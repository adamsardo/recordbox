use candle_core::Device;
use std::sync::OnceLock;

struct Backend {
    #[cfg(all(windows, feature = "directml"))]
    directml: Option<super::directml::Binding>,
    device: Device,
    label: String,
}

impl Backend {
    fn cpu(reason: &str) -> Self {
        Self { #[cfg(all(windows, feature = "directml"))] directml: None, device: Device::Cpu, label: format!("CPU · {reason}") }
    }
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

// A removed D3D device cannot be used again. Keep the first reason so that queued
// workers and the UI agree, even after the model that observed it is dropped.
static RADEON_HOLD: OnceLock<String> = OnceLock::new();

pub fn radeon_hold() -> Option<&'static str> {
    RADEON_HOLD.get().map(String::as_str)
}

#[cfg(all(windows, feature = "directml"))]
pub(super) fn hold_radeon(reason: String) {
    let _ = RADEON_HOLD.set(reason);
}

pub fn force_cpu() {
    let _ = BACKEND.set(Backend::cpu("selected for this session"));
}

pub fn device() -> Device {
    BACKEND.get_or_init(select).device.clone()
}

#[cfg(all(windows, feature = "directml"))]
pub(super) fn directml_device() -> Option<&'static super::directml::Binding> {
    BACKEND.get_or_init(select).directml.as_ref()
}

pub fn uses_directml() -> bool {
    #[cfg(all(windows, feature = "directml"))]
    { directml_device().is_some() && radeon_hold().is_none() }
    #[cfg(not(all(windows, feature = "directml")))]
    { false }
}

pub(super) fn gpu_uses_shared_memory() -> bool {
    #[cfg(all(windows, feature = "directml"))]
    { directml_device().is_some_and(|binding| binding.shared_memory) }
    #[cfg(not(all(windows, feature = "directml")))]
    { false }
}

pub fn gpu_active() -> bool {
    uses_directml() || !device().is_cpu()
}

#[cfg(all(windows, feature = "directml"))]
fn select() -> Backend {
    if let Some(reason) = radeon_hold() { return Backend::cpu(reason); }
    match super::directml::select() {
        Ok((binding, label)) => Backend { device: Device::Cpu, label, directml: Some(binding) },
        Err(error) => Backend::cpu(&format!("{error:#}")),
    }
}

/// Status for startup and Preferences. Radeon inference is opened by a real job or --stems-device.
pub fn status() -> (bool, &'static str) {
    #[cfg(all(windows, feature = "directml"))]
    {
        if let Some(reason) = radeon_hold() { return (false, reason); }
        if let Some(active) = BACKEND.get() {
            return (active.directml.is_some(), &active.label);
        }
        static DETECTED: OnceLock<(bool, String)> = OnceLock::new();
        let (available, label) = DETECTED.get_or_init(|| match super::directml::available() {
            Ok(name) => (true, format!("{name} · DirectML checked on first use")),
            Err(e) => (false, format!("CPU · {e:#}")),
        });
        radeon_hold().map_or((*available, label.as_str()), |reason| (false, reason))
    }
    #[cfg(not(all(windows, feature = "directml")))]
    { (gpu_active(), backend_label()) }
}

pub fn backend_label() -> &'static str {
    let selected = BACKEND.get_or_init(select);
    radeon_hold().unwrap_or(&selected.label)
}

#[cfg(target_os = "macos")]
fn select() -> Backend {
    match Device::new_metal(0) {
        Ok(device) => Backend { device, label: "Apple Metal".into() },
        Err(error) => Backend::cpu(&format!("Metal initialization failed ({error})")),
    }
}

#[cfg(all(not(target_os = "macos"), not(feature = "cuda12"), not(all(windows, feature = "directml"))))]
fn select() -> Backend {
    Backend::cpu("standard build")
}

#[cfg(all(not(target_os = "macos"), feature = "cuda12", not(all(windows, feature = "directml"))))]
fn select() -> Backend {
    use candle_core::cuda_backend::cudarc::driver::CudaContext;
    let count = match CudaContext::device_count() {
        Ok(0) => return Backend::cpu("CUDA 12 found no visible NVIDIA devices"),
        Ok(count) => count,
        Err(error) => return Backend::cpu(&format!("CUDA driver initialization failed ({error})")),
    };
    let mut reasons = Vec::new();
    for ordinal in 0..count {
        match cuda_device(ordinal as usize) {
            Ok(backend) => return backend,
            Err(error) => reasons.push(format!("device {ordinal}: {error}")),
        }
    }
    Backend::cpu(&format!("CUDA 12 · {}", reasons.join(", ")))
}

#[cfg(all(not(target_os = "macos"), feature = "cuda12", not(all(windows, feature = "directml"))))]
fn cuda_device(ordinal: usize) -> anyhow::Result<Backend> {
    use candle_core::Tensor;
    // One explicit stream can be used by both the assessment and separation threads.
    let device = Device::new_cuda_with_stream(ordinal)?;
    let stream = device.as_cuda_device()?.cuda_stream();
    let context = stream.context();
    let name = context.name()?;
    let (major, minor) = context.compute_capability()?;
    anyhow::ensure!((major, minor) >= (6, 1), "{name} is SM {major}.{minor}, this build needs 6.1 or newer");
    let mib = context.total_mem()? / (1024 * 1024);

    // Exercise both a Candle convolution kernel and cuBLAS, then synchronize by
    // reading the result. Enumeration alone cannot detect incompatible PTX or libraries.
    let x = Tensor::new(&[[[1f32, 2., 3.]]], &device)?;
    let weights = Tensor::new(&[[[2f32, 1.]]], &device)?;
    let conv = x.conv1d(&weights, 0, 1, 1, 1)?.reshape((1, 2))?;
    let column = Tensor::new(&[[1f32], [2.]], &device)?;
    let result = conv.matmul(&column)?.flatten_all()?.to_vec1::<f32>()?;
    anyhow::ensure!(result == [18.0], "{name} failed the FP32 calculation check");
    Ok(Backend { device, label: format!("CUDA 12 · {name} · SM {major}.{minor} · {mib} MiB VRAM") })
}

#[cfg(all(test, windows, feature = "directml"))]
mod tests {
    #[test]
    #[ignore = "run in a fresh native test process before any inference"]
    fn idle_radeon_assessment_does_not_open_a_compute_backend() {
        assert!(super::BACKEND.get().is_none());
        let hardware = crate::stems::assess();
        let (detected, label) = super::status();
        assert_eq!(hardware.gpu, detected);
        assert!(!label.is_empty());
        assert!(super::BACKEND.get().is_none(), "Startup or Preferences opened an inference device");
        super::force_cpu();
        assert!(!super::status().0);
        assert!(crate::stems::SeparationDevice::GpuOnly.device().is_err());
        assert!(crate::stems::SeparationDevice::Cpu.device().unwrap().is_cpu());
    }

    #[test]
    #[ignore = "requires an isolated model directory, the Windows DirectML runtime and WARP"]
    fn deck_separates_with_both_radeon_models_and_releases_the_gpu_graph() -> anyhow::Result<()> {
        use crate::stems::{self, compare::relative_error, directml::Binding, SeparationDevice};
        use std::sync::Arc;

        let fixture = std::path::PathBuf::from(std::env::var_os("RECORDBOX_TEST_CHECKPOINT").expect("checkpoint"));
        assert_eq!(stems::models_dir().join(stems::CATALOG[1].files[0]), fixture);
        let binding = Binding::warp()?.into_standby();
        assert!(super::BACKEND.set(super::Backend {
            directml: Some(binding),
            device: candle_core::Device::Cpu,
            label: "DirectML WARP with shared GPU memory".into(),
        }).is_ok());
        assert!(super::gpu_uses_shared_memory());

        let frames = 32_768;
        let audio: Vec<f32> = (0..frames).flat_map(|i| {
            let t = i as f32 / 44_100.;
            let kick = if i % 4096 < 32 { 0.25 } else { 0. };
            [(t * 440.).sin() * 0.21 + kick, (t * 1293.).sin() * 0.1 + kick]
        }).collect();
        for model in stems::CATALOG.iter().rev() {
            stems::download(model, &mut |_, _, _| {}, &Default::default())?;
            for (file, hash) in model.files.iter().zip(model.sha256) {
                if let Some(hash) = hash {
                    assert_eq!(stems::sha_hex(&stems::models_dir().join(file))?, *hash);
                }
            }
            let run = |mode| stems::separate_with_device(model, &audio, 44_100, &mut |_| true, mode);
            let actual = run(SeparationDevice::GpuOnly)?;
            let expected = run(SeparationDevice::Cpu)?;
            assert_eq!((actual.len, actual.data.len()), (frames, 8 * frames));
            assert_eq!(actual.data.len(), expected.data.len());
            for ((name, reference), result) in stems::htdemucs::SOURCES.iter()
                .zip(expected.data.chunks_exact(2 * frames)).zip(actual.data.chunks_exact(2 * frames)) {
                let error = relative_error(reference, result)?;
                println!("{} deck {name}: {:.4}% relative RMS difference", model.name, 100. * error);
                assert!(error <= 0.03);
            }
        }
        let file = stems::CATALOG[1].files[0];
        let gpu = stems::graph(file, SeparationDevice::GpuOnly)?;
        let weak = Arc::downgrade(&gpu);
        drop(gpu);
        assert!(weak.upgrade().is_none(), "Idle shared GPU still owns a compiled model");
        let cpu = stems::graph(file, SeparationDevice::Cpu)?;
        assert!(Arc::ptr_eq(&cpu, &stems::graph(file, SeparationDevice::Cpu)?));
        Ok(())
    }
}
