use candle_core::Device;
use std::sync::OnceLock;

struct Backend {
    #[cfg(all(windows, feature = "directml"))]
    directml: Option<i32>,
    device: Device,
    label: String,
}

impl Backend {
    fn cpu(reason: &str) -> Self {
        Self { #[cfg(all(windows, feature = "directml"))] directml: None, device: Device::Cpu, label: format!("CPU · {reason}") }
    }
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

pub fn force_cpu() {
    let _ = BACKEND.set(Backend::cpu("selected for this session"));
}

pub fn device() -> Device {
    BACKEND.get_or_init(select).device.clone()
}

pub fn directml_adapter() -> Option<i32> {
    #[cfg(all(windows, feature = "directml"))]
    { BACKEND.get_or_init(select).directml }
    #[cfg(not(all(windows, feature = "directml")))]
    { None }
}

pub fn gpu_active() -> bool {
    directml_adapter().is_some() || !device().is_cpu()
}

#[cfg(all(windows, feature = "directml"))]
fn select() -> Backend {
    match super::directml::select() {
        Ok((index, label)) => Backend { device: Device::Cpu, label, directml: Some(index) },
        Err(error) => Backend::cpu(&format!("{error:#}")),
    }
}

pub fn backend_label() -> &'static str {
    &BACKEND.get_or_init(select).label
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
