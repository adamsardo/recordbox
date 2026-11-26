//! Sample layer outputs when native DirectML diverges from the CPU reference.
use super::{network, session, Binding, Stft, TRAINING_FRAMES};
use crate::stems::{compare::relative_error, htdemucs::Graph};
use anyhow::{Context, Result};
use bumpalo::Bump;
use onnx_rs::ast::{
    DataType, Node, OpType, TensorProto, TensorTypeProto, TypeProto, TypeValue, ValueInfo,
};
use ort::value::TensorRef;
use std::path::Path;

fn sampled_model(bytes: &[u8]) -> Result<Vec<u8>> {
    let names = Bump::new();
    let mut model = onnx_rs::parse(bytes)?;
    let graph = model.graph.as_mut().context("Missing ONNX graph")?;
    let stages: Vec<_> = graph
        .node
        .iter()
        .filter_map(|node| {
            let bias = node.input.iter().find_map(|s| s.strip_suffix(".bias"))?;
            match node.op_type {
                OpType::Conv | OpType::ConvTranspose => Some((bias, node.output[0])),
                OpType::Add if bias.starts_with("crosstransformer.") => {
                    Some((bias, node.output[0]))
                }
                _ => None,
            }
        })
        .collect();
    graph.output.clear();
    for (name, value) in [
        ("flat", -1),
        ("start", 0),
        ("end", 65536),
        ("axis", 0),
        ("step", 67),
    ] {
        graph
            .initializer
            .push(TensorProto::from_i64(name, vec![1], vec![value]));
    }
    for (stage, activation) in stages {
        let flat: &str = names.alloc_str(&format!("sample/{stage}/flat"));
        let sample: &str = names.alloc_str(&format!("sample/{stage}"));
        graph.node.push(Node {
            op_type: OpType::Reshape,
            input: vec![activation, "flat"],
            output: vec![flat],
            ..Default::default()
        });
        graph.node.push(Node {
            op_type: OpType::Slice,
            input: vec![flat, "start", "end", "axis", "step"],
            output: vec![sample],
            ..Default::default()
        });
        graph.output.push(ValueInfo {
            name: sample,
            r#type: Some(TypeProto {
                value: Some(TypeValue::Tensor(TensorTypeProto {
                    elem_type: DataType::Float,
                    shape: None,
                })),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    Ok(onnx_rs::encode(&model))
}

pub(super) fn trace(checkpoint: &Path, adapter: Option<&Binding>) -> Result<()> {
    let weights = Graph::read_weights(checkpoint, &candle_core::Device::Cpu)?;
    let model = sampled_model(&network::model_parts(weights, super::query_tile(adapter), false)?.remove(0).bytes)?;
    let frames = Stft::frames(TRAINING_FRAMES);
    let signal = |count| {
        (0..count)
            .map(|i| ((i % 101) as f32 - 50.) / 50.)
            .collect::<Vec<_>>()
    };
    let spec = signal(4 * network::BINS * frames);
    let wave = signal(2 * TRAINING_FRAMES);
    let run = |binding: Option<&Binding>| -> Result<Vec<(String, Vec<f32>)>> {
        let provider = binding.map(super::Provider::new).transpose()?;
        let mut engine = session(&model, provider.as_ref())?;
        let spec = TensorRef::from_array_view(([1, 4, network::BINS, frames], spec.as_slice()))?;
        let wave = TensorRef::from_array_view(([1, 2, TRAINING_FRAMES], wave.as_slice()))?;
        let outputs = engine.run(ort::inputs!["spec"=>spec,"wave"=>wave])?;
        outputs
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.to_owned(),
                    value.try_extract_tensor::<f32>()?.1.to_vec(),
                ))
            })
            .collect()
    };
    let reference = run(None)?;
    let actual = run(adapter)?;
    for ((name, expected), (other, values)) in reference.iter().zip(&actual) {
        anyhow::ensure!(name == other, "Different sampled outputs");
        println!(
            "{name}: {:?} relative RMS, first values {:.6} / {:.6}",
            relative_error(expected, values),
            expected[0],
            values[0]
        );
    }
    Ok(())
}
