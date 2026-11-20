//! ONNX Runtime DirectML execution, restricted to physical Radeon adapters.
#[cfg(windows)]
mod device;
#[cfg(test)]
mod diagnostics;
mod network;
#[cfg(windows)]
pub(super) use device::{select, Binding};
#[cfg(not(windows))]
pub(super) enum Binding {}
use super::htdemucs::{Stft, TRAINING_FRAMES};
use anyhow::{ensure, Context, Result};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::TensorRef,
};
use std::sync::Mutex;

fn session(model: &[u8], adapter: Option<&Binding>) -> ort::Result<Session> {
    let mut builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level1)?
        .with_parallel_execution(false)?
        .with_memory_pattern(false)?;
    if let Some(binding) = adapter {
        #[cfg(windows)]
        {
            binding.attach(&mut builder)?;
            // Keep the FP32 operators and their intermediate results explicit.
            // DirectML's whole-graph fusion can alter HTDemucs separation results.
            builder = builder
                .with_config_entry("ep.dml.disable_graph_fusion", "1")?
                .with_disable_cpu_fallback()?;
        }
        #[cfg(not(windows))]
        match *binding {}
    } else {
        // Only the numerical regression test uses the ONNX CPU execution provider.
        builder = builder.with_intra_threads(2)?;
    }
    builder.commit_from_memory(model)
}

fn probe(adapter: Option<&Binding>) -> Result<()> {
    let mut session = session(&network::probe_model(), adapter)?;
    let input = TensorRef::from_array_view(([1, 1, 3], &[1f32, 2., 3.][..]))?;
    let outputs = session.run(ort::inputs![input])?;
    let (_, result) = outputs["result"].try_extract_tensor::<f32>()?;
    ensure!(result == [18.], "DirectML calculation check failed");
    Ok(())
}

pub(super) struct Engine(Mutex<Session>);

impl Engine {
    pub(super) fn new(
        weights: std::collections::HashMap<String, candle_core::Tensor>,
        adapter: Option<&Binding>,
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
    // WARP executes DirectML operators in software on a hosted Windows runner.
    // The application only enumerates physical Radeon adapters.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires the Windows DirectML runtime and D3D12 WARP"]
    fn runtime_executes_directml() -> Result<()> {
        let binding = Binding::Warp;
        probe(Some(&binding))?;
        println!("DirectML convolution and matrix multiplication passed on D3D12 WARP");
        Ok(())
    }

    #[test]
    #[ignore = "requires RECORDBOX_TEST_CHECKPOINT and ONNX Runtime"]
    fn onnx_four_stems_match_candle() {
        use crate::stems::{compare::relative_error, htdemucs};
        use candle_core::Device;
        let path = std::path::PathBuf::from(
            std::env::var_os("RECORDBOX_TEST_CHECKPOINT").expect("checkpoint path"),
        );
        let len = 32768;
        let audio: Vec<f32> = (0..2 * len)
            .map(|i| {
                let t = (i % len) as f32 / 44100.;
                let ch = (i / len + 1) as f32;
                (t * 440. * ch).sin() * 0.21
                    + (t * 1293.).sin() * 0.1
                    + if i % 4096 < 32 { 0.25 } else { 0. }
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
        let compare = |label: &str, adapter| {
            let actual = run(htdemucs::Graph::load_onnx(&path, adapter).unwrap());
            let mut maximum = 0.0f64;
            for (name, (a, b)) in htdemucs::SOURCES.iter().zip(
                expected
                    .chunks_exact(2 * len)
                    .zip(actual.chunks_exact(2 * len)),
            ) {
                let error = relative_error(a, b).unwrap();
                println!(
                    "{label} {name}: {:.4}% relative RMS difference",
                    100. * error
                );
                maximum = maximum.max(error);
            }
            maximum
        };
        assert!(compare("ONNX CPU", None) <= 0.03);
        if std::env::var_os("RECORDBOX_TRACE_STEMS").is_some() {
            diagnostics::trace(&path, None).unwrap();
        }
        #[cfg(windows)]
        {
            let binding = Binding::Warp;
            let error = compare("DirectML WARP", Some(&binding));
            if error > 0.03 {
                diagnostics::trace(&path, Some(&binding)).unwrap();
            }
            assert!(error <= 0.03, "DirectML maximum relative error {error}");
        }
    }
}
