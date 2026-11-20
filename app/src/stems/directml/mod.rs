//! ONNX Runtime DirectML execution, restricted to physical Radeon adapters.
#[cfg(windows)]
mod device;
#[cfg(test)]
mod diagnostics;
mod network;
#[cfg(windows)]
pub(super) use device::{select, Binding};
#[cfg(not(windows))]
#[derive(Clone)]
pub(super) enum Binding {}
use super::htdemucs::{Stft, TRAINING_FRAMES};
use anyhow::{ensure, Context, Result};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::TensorRef,
};
use std::sync::Mutex;

fn shared_memory(adapter: Option<&Binding>) -> bool {
    #[cfg(windows)]
    {
        adapter.is_some_and(|binding| binding.shared_memory)
    }
    #[cfg(not(windows))]
    {
        let _ = adapter;
        false
    }
}

fn query_tile(adapter: Option<&Binding>) -> i64 {
    if shared_memory(adapter) {
        network::SHARED_MEMORY_QUERY_TILE
    } else {
        network::QUERY_TILE
    }
}

fn runtime_error(adapter: Option<&Binding>, stage: &'static str, error: ort::Error) -> anyhow::Error {
    #[cfg(windows)]
    if let Some(binding) = adapter {
        return super::DirectMlFailure(format!("{stage}\n{}\n{error}", binding.failure_detail())).into();
    }
    let _ = adapter;
    anyhow::Error::new(error).context(stage)
}

fn session(model: &[u8], adapter: Option<&Binding>) -> ort::Result<Session> {
    let mut builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level1)?
        .with_parallel_execution(false)?
        .with_memory_pattern(false)?;
    if let Some(binding) = adapter {
        #[cfg(windows)]
        {
            if binding.shared_memory {
                // The arena can retain allocations between dispatches. The Vega APU and host share RAM.
                builder = builder.with_config_entry("ep.dml.disable_memory_arena", "1")?;
            }
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

pub(super) struct Engine {
    session: Mutex<Session>,
    adapter: Option<Binding>,
}

impl Engine {
    pub(super) fn new(
        weights: std::collections::HashMap<String, candle_core::Tensor>,
        adapter: Option<&Binding>,
    ) -> Result<Self> {
        let model = network::model(weights, query_tile(adapter))?;
        let session = session(&model, adapter)
            .map_err(|e| runtime_error(adapter, "Preparing the HTDemucs network", e))?;
        Ok(Self {
            session: Mutex::new(session),
            adapter: adapter.cloned(),
        })
    }
    pub(super) fn buffered_segments(&self) -> usize {
        // STFT, inference and overlap-add can still run concurrently on a rendezvous channel.
        if shared_memory(self.adapter.as_ref()) {
            0
        } else {
            2
        }
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
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("DirectML session interrupted"))?;
        let outputs = engine
            .run(ort::inputs!["spec"=>spec,"wave"=>wave])
            .map_err(|e| runtime_error(self.adapter.as_ref(), "Running an HTDemucs separation segment", e))?;
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
        let binding = Binding::warp()?;
        probe(Some(&binding))?;
        println!("DirectML convolution and matrix multiplication passed on D3D12 WARP");
        Ok(())
    }

    #[test]
    #[ignore = "requires RECORDBOX_TEST_CHECKPOINT and ONNX Runtime"]
    fn onnx_four_stems_match_candle() {
        use crate::stems::{compare::relative_error, htdemucs};
        use candle_core::Device;
        let path =
            std::path::PathBuf::from(std::env::var_os("RECORDBOX_TEST_CHECKPOINT").expect("checkpoint path"));
        // A real song crosses a segment boundary. Exercise repeat dispatch, crop and overlap-add.
        let len = TRAINING_FRAMES + 4096;
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
        let expected = run(crate::stems::SeparationDevice::Cpu.load(&path).unwrap());
        let compare = |label: &str, adapter| {
            let actual = run(htdemucs::Graph::load_onnx(&path, adapter).unwrap());
            let mut maximum = 0.0f64;
            for (name, (a, b)) in htdemucs::SOURCES
                .iter()
                .zip(expected.chunks_exact(2 * len).zip(actual.chunks_exact(2 * len)))
            {
                let error = relative_error(a, b).unwrap();
                println!("{label} {name}: {:.4}% relative RMS difference", 100. * error);
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
            let binding = Binding::warp().unwrap();
            let error = compare("DirectML WARP", Some(&binding));
            if error > 0.03 {
                diagnostics::trace(&path, Some(&binding)).unwrap();
            }
            assert!(error <= 0.03, "DirectML maximum relative error {error}");
        }
    }
}
