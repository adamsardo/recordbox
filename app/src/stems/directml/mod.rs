//! ONNX Runtime DirectML execution, restricted to physical Radeon adapters.
mod network;
use super::htdemucs::{Stft, TRAINING_FRAMES};
use anyhow::{ensure, Context, Result};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::TensorRef,
};
use std::sync::Mutex;

fn session(model: &[u8], adapter: Option<i32>) -> ort::Result<Session> {
    let mut builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level1)?
        .with_parallel_execution(false)?
        .with_memory_pattern(false)?;
    if let Some(index) = adapter {
        let ep = ort::ep::DirectML::default()
            .with_device_id(index)
            .build()
            .error_on_failure();
        builder = builder
            .with_execution_providers([ep])?
            .with_disable_cpu_fallback()?;
    } else {
        // Only the numerical regression test uses the ONNX CPU execution provider.
        builder = builder.with_intra_threads(2)?;
    }
    builder.commit_from_memory(model)
}

fn probe(adapter: Option<i32>) -> Result<()> {
    let mut session = session(&network::probe_model(), adapter)?;
    let input = TensorRef::from_array_view(([1, 1, 3], &[1f32, 2., 3.][..]))?;
    let outputs = session.run(ort::inputs![input])?;
    let (_, result) = outputs["result"].try_extract_tensor::<f32>()?;
    ensure!(result == [18.], "DirectML calculation check failed");
    Ok(())
}

#[cfg(windows)]
pub(super) fn select() -> Result<(i32, String)> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
    };
    let exe = std::env::current_exe()?;
    let dll = exe.with_file_name("onnxruntime.dll");
    ensure!(
        dll.is_file(),
        "Extract the complete DirectML ZIP with onnxruntime.dll beside recordbox.exe"
    );
    ort::init_from(&dll)
        .context("Loading the packaged ONNX Runtime")?
        .commit();
    // The legacy DirectML device ID is the DXGI enumeration index, including non-AMD entries.
    // The factory owns the returned COM references throughout each query.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut failures = Vec::new();
    for index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(e.into()),
        };
        let desc = unsafe { adapter.GetDesc1()? };
        if desc.VendorId != 0x1002 || desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]);
        match probe(Some(index as i32)) {
            Ok(()) => return Ok((index as i32, format!("DirectML · {name} · DXGI adapter {index}"))),
            Err(error) => failures.push(format!("{name}: {error:#}")),
        }
    }
    ensure!(
        !failures.is_empty(),
        "DirectML found no hardware AMD Radeon adapter"
    );
    anyhow::bail!("DirectML device check failed: {}", failures.join(", "))
}

pub(super) struct Engine(Mutex<Session>);

impl Engine {
    pub(super) fn new(
        weights: std::collections::HashMap<String, candle_core::Tensor>,
        adapter: Option<i32>,
    ) -> Result<Self> {
        let model = network::model(weights)?;
        Ok(Self(Mutex::new(
            session(&model, adapter).context("Preparing the HTDemucs network")?,
        )))
    }
    pub(super) fn forward(
        &self,
        spec: Vec<f32>,
        wave: Vec<f32>,
        source: Option<usize>,
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        let frames = Stft::frames(TRAINING_FRAMES);
        let spec = TensorRef::from_array_view(([1, 4, network::BINS, frames], spec.as_slice()))?;
        let wave = TensorRef::from_array_view(([1, 2, TRAINING_FRAMES], wave.as_slice()))?;
        let mut engine = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("DirectML session interrupted"))?;
        let outputs = engine.run(ort::inputs!["spec"=>spec,"wave"=>wave])?;
        let take = |name: &str, shape: &[usize]| -> Result<Vec<f32>> {
            let (actual, data) = outputs[name].try_extract_tensor::<f32>()?;
            ensure!(
                actual.iter().copied().eq(shape.iter().map(|&n| n as i64)),
                "Unexpected {name} output shape"
            );
            let stride = data.len() / 4;
            Ok(match source {
                Some(index) => data[index * stride..(index + 1) * stride].to_vec(),
                None => data.to_vec(),
            })
        };
        Ok((
            take("spec_out", &[1, 16, network::BINS, frames])?,
            take("wave_out", &[1, 8, TRAINING_FRAMES])?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the packaged ONNX Runtime or ORT_DYLIB_PATH"]
    fn runtime_executes_probe() {
        probe(None).unwrap();
    }
    #[test]
    #[ignore = "requires RECORDBOX_TEST_CHECKPOINT and the ONNX Runtime CPU library"]
    fn onnx_four_stems_match_candle() {
        use crate::stems::{compare::relative_error, htdemucs};
        use candle_core::Device;
        let path =
            std::path::PathBuf::from(std::env::var_os("RECORDBOX_TEST_CHECKPOINT").expect("checkpoint path"));
        let len = 32768;
        let audio: Vec<f32> = (0..2 * len)
            .map(|i| {
                let t = (i % len) as f32 / 44100.;
                let ch = (i / len + 1) as f32;
                (t * 440. * ch).sin() * 0.21 + (t * 1293.).sin() * 0.1 + if i % 4096 < 32 { 0.25 } else { 0. }
            })
            .collect();
        let run = |model: htdemucs::Graph| {
            htdemucs::separate(
                &model,
                &Device::Cpu,
                &audio,
                len,
                &Default::default(),
                &mut |_, _| true,
            )
            .unwrap()
        };
        let expected = run(htdemucs::Graph::load(&path, &Device::Cpu).unwrap());
        let actual = run(htdemucs::Graph::load_onnx(&path, None).unwrap());
        for (name, (a, b)) in htdemucs::SOURCES
            .iter()
            .zip(expected.chunks_exact(2 * len).zip(actual.chunks_exact(2 * len)))
        {
            let error = relative_error(a, b).unwrap();
            println!("{name}: {:.4}% relative RMS difference", 100. * error);
            assert!(error <= 0.03, "{name}: {error}");
        }
    }
}
