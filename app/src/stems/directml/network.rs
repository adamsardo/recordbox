//! Fixed-shape FP32 network compiled from the existing HTDemucs checkpoint.
//! The host shares spectral transforms and overlap-add with the Metal path.
use super::super::htdemucs::{pos1, pos2, Stft, TRAINING_FRAMES};
use bumpalo::Bump;
use candle_core::Tensor;
use onnx_rs::ast::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

pub(super) const BINS: usize = 2048;
const WIDTH: i64 = 512;
const HEADS: i64 = 8;
pub(super) const QUERY_TILE: i64 = 256;
pub(super) const SHARED_MEMORY_QUERY_TILE: i64 = 64;

fn tensor_info<'a>(name: &'a str, dims: &[i64]) -> ValueInfo<'a> {
    let dim = dims
        .iter()
        .map(|&n| TensorShapeDimension {
            value: Dimension::Value(n),
            ..Default::default()
        })
        .collect();
    ValueInfo {
        name,
        r#type: Some(TypeProto {
            value: Some(TypeValue::Tensor(TensorTypeProto {
                elem_type: DataType::Float,
                shape: Some(TensorShape { dim }),
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn integer(name: &'static str, i: i64) -> Attribute<'static> {
    Attribute {
        name,
        i,
        r#type: AttributeType::Int,
        ..Default::default()
    }
}
fn integers(name: &'static str, values: &[i64]) -> Attribute<'static> {
    Attribute {
        name,
        ints: values.to_vec(),
        r#type: AttributeType::Ints,
        ..Default::default()
    }
}

struct Network<'a> {
    names: &'a Bump,
    query_tile: i64,
    next: Cell<usize>,
    graph: RefCell<Graph<'a>>,
}
impl<'a> Network<'a> {
    fn new(names: &'a Bump, query_tile: i64) -> Self {
        Self {
            names,
            query_tile,
            next: Cell::new(0),
            graph: RefCell::new(Graph {
                name: "recordbox_htdemucs",
                ..Default::default()
            }),
        }
    }
    fn id(&self) -> &'a str {
        let n = self.next.get();
        self.next.set(n + 1);
        self.names.alloc_str(&format!("v{n}"))
    }
    fn weight(&self, prefix: &str, field: &str) -> &'a str {
        self.names.alloc_str(&format!("{prefix}.{field}"))
    }
    fn node(&self, op: &'static str, inputs: &[&'a str], outputs: &[&'a str], attrs: Vec<Attribute<'a>>) {
        self.graph.borrow_mut().node.push(Node {
            name: outputs[0],
            op_type: op.into(),
            input: inputs.to_vec(),
            output: outputs.to_vec(),
            attribute: attrs,
            ..Default::default()
        });
    }
    fn op(&self, op: &'static str, inputs: &[&'a str], attrs: Vec<Attribute<'a>>) -> &'a str {
        let out = self.id();
        self.node(op, inputs, &[out], attrs);
        out
    }
    fn math(&self, op: &'static str, inputs: &[&'a str]) -> &'a str {
        self.op(op, inputs, vec![])
    }
    fn shape(&self, values: &[i64]) -> &'a str {
        let id = self.id();
        self.graph.borrow_mut().initializer.push(TensorProto::from_i64(
            id,
            vec![values.len() as i64],
            values.to_vec(),
        ));
        id
    }
    fn floats(&self, name: &'a str, dims: &[i64], values: Vec<f32>) {
        self.graph
            .borrow_mut()
            .initializer
            .push(TensorProto::from_f32(name, dims.to_vec(), values));
    }
    fn reshape(&self, x: &'a str, dims: &[i64]) -> &'a str {
        self.math("Reshape", &[x, self.shape(dims)])
    }
    fn transpose(&self, x: &'a str, perm: &[i64]) -> &'a str {
        self.op("Transpose", &[x], vec![integers("perm", perm)])
    }
    fn slice(&self, x: &'a str, axis: i64, start: i64, end: i64) -> &'a str {
        self.math(
            "Slice",
            &[x, self.shape(&[start]), self.shape(&[end]), self.shape(&[axis])],
        )
    }
    fn gelu(&self, x: &'a str) -> &'a str {
        let erf = self.math("Erf", &[self.math("Mul", &[x, "inv_sqrt2"])]);
        self.math(
            "Mul",
            &[self.math("Mul", &[x, "half"]), self.math("Add", &[erf, "one"])],
        )
    }
    fn glu(&self, x: &'a str) -> &'a str {
        let (a, b) = (self.id(), self.id());
        self.node("Split", &[x], &[a, b], vec![integer("axis", 1)]);
        self.math("Mul", &[a, self.math("Sigmoid", &[b])])
    }
    fn norm(&self, x: &'a str, p: &str, axes: &[i64], channels_first: bool) -> &'a str {
        let mean = |t| {
            self.op(
                "ReduceMean",
                &[t],
                vec![integers("axes", axes), integer("keepdims", 1)],
            )
        };
        let centered = self.math("Sub", &[x, mean(x)]);
        let variance = mean(self.math("Mul", &[centered, centered]));
        let std = self.math("Sqrt", &[self.math("Add", &[variance, "eps"])]);
        let affine = |field| {
            let w = self.weight(p, field);
            if channels_first {
                self.reshape(w, &[1, -1, 1])
            } else {
                w
            }
        };
        self.math(
            "Add",
            &[
                self.math("Mul", &[self.math("Div", &[centered, std]), affine("weight")]),
                affine("bias"),
            ],
        )
    }
    fn conv(
        &self,
        op: &'static str,
        x: &'a str,
        p: &str,
        pads: &[i64],
        strides: &[i64],
        dilations: &[i64],
    ) -> &'a str {
        self.op(
            op,
            &[x, self.weight(p, "weight"), self.weight(p, "bias")],
            vec![
                integers("pads", pads),
                integers("strides", strides),
                integers("dilations", dilations),
            ],
        )
    }
    fn dconv(&self, mut x: &'a str, p: &str, channels: i64, freq: Option<i64>, frames: i64) -> &'a str {
        if let Some(f) = freq {
            x = self.reshape(self.transpose(x, &[0, 2, 1, 3]), &[f, channels, frames]);
        }
        for (i, d) in [1, 2].into_iter().enumerate() {
            let layer = format!("{p}.layers.{i}");
            let y = self.conv("Conv", x, &format!("{layer}.0"), &[d, d], &[1], &[d]);
            let y = self.gelu(self.norm(y, &format!("{layer}.1"), &[1, 2], true));
            let y = self.conv("Conv", y, &format!("{layer}.3"), &[0, 0], &[1], &[1]);
            let y = self.glu(self.norm(y, &format!("{layer}.4"), &[1, 2], true));
            let scale = self.reshape(self.weight(&layer, "6.scale"), &[1, -1, 1]);
            x = self.math("Add", &[x, self.math("Mul", &[y, scale])]);
        }
        if let Some(f) = freq {
            x = self.transpose(self.reshape(x, &[1, f, channels, frames]), &[0, 2, 1, 3]);
        }
        x
    }
    fn rewrite(&self, x: &'a str, p: &str, spectral: bool, pad: i64) -> &'a str {
        if spectral {
            self.conv("Conv", x, p, &[pad; 4], &[1, 1], &[1, 1])
        } else {
            self.conv("Conv", x, p, &[pad; 2], &[1], &[1])
        }
    }
    fn linear(&self, x: &'a str, w: &'a str, b: &'a str) -> &'a str {
        self.math("Add", &[self.math("MatMul", &[x, self.transpose(w, &[1, 0])]), b])
    }
    fn linear_layer(&self, x: &'a str, p: &str) -> &'a str {
        self.linear(x, self.weight(p, "weight"), self.weight(p, "bias"))
    }
    fn attention(&self, q: &'a str, k: &'a str, p: &str, queries: i64, keys: i64) -> &'a str {
        let w = self.weight(p, "in_proj_weight");
        let b = self.weight(p, "in_proj_bias");
        let project = |x, i, tokens| {
            let projected = self.linear(
                x,
                self.slice(w, 0, i * WIDTH, (i + 1) * WIDTH),
                self.slice(b, 0, i * WIDTH, (i + 1) * WIDTH),
            );
            self.transpose(
                self.reshape(projected, &[1, tokens, HEADS, WIDTH / HEADS]),
                &[0, 2, 1, 3],
            )
        };
        let q = project(q, 0, queries);
        let kt = self.transpose(project(k, 1, keys), &[0, 1, 3, 2]);
        let v = project(k, 2, keys);
        let mut parts = Vec::new();
        // Tile only the query rows. Every query still attends to every key.
        for start in (0..queries).step_by(self.query_tile as usize) {
            let tile = self.slice(q, 2, start, (start + self.query_tile).min(queries));
            let scores = self.math("Mul", &[self.math("MatMul", &[tile, kt]), "attn_scale"]);
            let probs = self.op("Softmax", &[scores], vec![integer("axis", -1)]);
            parts.push(self.math("MatMul", &[probs, v]));
        }
        let merged = self.op("Concat", &parts, vec![integer("axis", 2)]);
        let y = self.reshape(self.transpose(merged, &[0, 2, 1, 3]), &[1, queries, WIDTH]);
        self.linear_layer(y, &format!("{p}.out_proj"))
    }
    fn layer(&self, x: &'a str, tokens: i64, cross: Option<(&'a str, i64)>, p: &str) -> &'a str {
        let q = self.norm(x, &format!("{p}.norm1"), &[-1], false);
        let (k, keys, attn, norm) = match cross {
            Some((k, n)) => (
                self.norm(k, &format!("{p}.norm2"), &[-1], false),
                n,
                "cross_attn",
                "norm3",
            ),
            None => (q, tokens, "self_attn", "norm2"),
        };
        let y = self.attention(q, k, &format!("{p}.{attn}"), tokens, keys);
        let x = self.math(
            "Add",
            &[x, self.math("Mul", &[y, self.weight(p, "gamma_1.scale")])],
        );
        let y = self.norm(x, &format!("{p}.{norm}"), &[-1], false);
        let y = self.gelu(self.linear_layer(y, &format!("{p}.linear1")));
        let y = self.linear_layer(y, &format!("{p}.linear2"));
        let x = self.math(
            "Add",
            &[x, self.math("Mul", &[y, self.weight(p, "gamma_2.scale")])],
        );
        self.norm(x, &format!("{p}.norm_out"), &[1, 2], false)
    }
    fn htdemucs(&self) -> Vec<Boundary<'a>> {
        let mut boundaries = Vec::new();
        let mut finish = |label, outputs| {
            boundaries.push(Boundary {
                label,
                end: self.graph.borrow().node.len(),
                outputs,
            })
        };
        let frames = Stft::frames(TRAINING_FRAMES) as i64;
        let mut freq = BINS as i64;
        let mut len = TRAINING_FRAMES as i64;
        let (mut x, mut t) = ("spec", "wave");
        self.graph.borrow_mut().input = vec![
            tensor_info(x, &[1, 4, freq, frames]),
            tensor_info(t, &[1, 2, len]),
        ];
        for (name, value) in [
            ("one", 1.),
            ("two", 2.),
            ("half", 0.5),
            ("eps", 1e-5),
            ("inv_sqrt2", std::f32::consts::FRAC_1_SQRT_2),
            ("attn_scale", 1.0 / ((WIDTH / HEADS) as f32).sqrt()),
        ] {
            self.floats(name, &[], vec![value]);
        }
        let mut skips = Vec::new();
        for (i, channels) in [48, 96, 192, 384].into_iter().enumerate() {
            let (p, pt) = (format!("encoder.{i}"), format!("tencoder.{i}"));
            let extra = (4 - len % 4) % 4;
            freq /= 4;
            x = self.gelu(self.conv("Conv", x, &format!("{p}.conv"), &[2, 0, 2, 0], &[4, 1], &[1, 1]));
            t = self.gelu(self.conv("Conv", t, &format!("{pt}.conv"), &[2, 2 + extra], &[4], &[1]));
            let original = len;
            len = (len + extra) / 4;
            x = self.dconv(x, &format!("{p}.dconv"), channels, Some(freq), frames);
            t = self.dconv(t, &format!("{pt}.dconv"), channels, None, len);
            x = self.glu(self.rewrite(x, &format!("{p}.rewrite"), true, 0));
            t = self.glu(self.rewrite(t, &format!("{pt}.rewrite"), false, 0));
            if i == 0 {
                let emb = self.reshape(
                    self.transpose("freq_emb.embedding.weight", &[1, 0]),
                    &[1, channels, freq, 1],
                );
                x = self.math("Add", &[x, self.math("Mul", &[emb, "two"])]);
            }
            skips.push((x, t, channels, freq, original, extra));
            finish(
                ["Encoder 1", "Encoder 2", "Encoder 3", "Encoder 4"][i],
                [
                    tensor_info(x, &[1, channels, freq, frames]),
                    tensor_info(t, &[1, channels, len]),
                ],
            );
        }
        let tokens = freq * frames;
        x = self.reshape(x, &[1, 384, tokens]);
        x = self.reshape(
            self.rewrite(x, "channel_upsampler", false, 0),
            &[1, WIDTH, freq, frames],
        );
        t = self.rewrite(t, "channel_upsampler_t", false, 0);
        x = self.reshape(self.transpose(x, &[0, 3, 2, 1]), &[1, tokens, WIDTH]);
        t = self.transpose(t, &[0, 2, 1]);
        self.floats(
            "pos2",
            &[1, tokens, WIDTH],
            pos2(freq as usize, frames as usize, WIDTH as usize),
        );
        self.floats("pos1", &[1, len, WIDTH], pos1(len as usize, WIDTH as usize));
        x = self.math(
            "Add",
            &[self.norm(x, "crosstransformer.norm_in", &[-1], false), "pos2"],
        );
        t = self.math(
            "Add",
            &[self.norm(t, "crosstransformer.norm_in_t", &[-1], false), "pos1"],
        );
        let transformer_outputs = |x, t| {
            [
                tensor_info(x, &[1, tokens, WIDTH]),
                tensor_info(t, &[1, len, WIDTH]),
            ]
        };
        finish("Transformer inputs", transformer_outputs(x, t));
        for i in 0..5 {
            let cross = i % 2 == 1;
            let old_x = x;
            x = self.layer(
                x,
                tokens,
                cross.then_some((t, len)),
                &format!("crosstransformer.layers.{i}"),
            );
            t = self.layer(
                t,
                len,
                cross.then_some((old_x, tokens)),
                &format!("crosstransformer.layers_t.{i}"),
            );
            finish(
                [
                    "Transformer 1",
                    "Transformer 2",
                    "Transformer 3",
                    "Transformer 4",
                    "Transformer 5",
                ][i],
                transformer_outputs(x, t),
            );
        }
        x = self.transpose(self.reshape(x, &[1, frames, freq, WIDTH]), &[0, 3, 2, 1]);
        x = self.reshape(
            self.rewrite(
                self.reshape(x, &[1, WIDTH, tokens]),
                "channel_downsampler",
                false,
                0,
            ),
            &[1, 384, freq, frames],
        );
        t = self.rewrite(self.transpose(t, &[0, 2, 1]), "channel_downsampler_t", false, 0);
        for (i, (sx, st, channels, f, original, extra)) in skips.into_iter().rev().enumerate() {
            let (p, pt) = (format!("decoder.{i}"), format!("tdecoder.{i}"));
            x = self.glu(self.rewrite(self.math("Add", &[x, sx]), &format!("{p}.rewrite"), true, 1));
            t = self.glu(self.rewrite(self.math("Add", &[t, st]), &format!("{pt}.rewrite"), false, 1));
            x = self.dconv(x, &format!("{p}.dconv"), channels, Some(f), frames);
            t = self.dconv(t, &format!("{pt}.dconv"), channels, None, len);
            x = self.conv(
                "ConvTranspose",
                x,
                &format!("{p}.conv_tr"),
                &[2, 0, 2, 0],
                &[4, 1],
                &[1, 1],
            );
            t = self.conv(
                "ConvTranspose",
                t,
                &format!("{pt}.conv_tr"),
                &[2, 2 + extra],
                &[4],
                &[1],
            );
            len = original;
            if i < 3 {
                x = self.gelu(x);
                t = self.gelu(t);
                finish(
                    ["Decoder 1", "Decoder 2", "Decoder 3"][i],
                    [
                        tensor_info(x, &[1, channels / 2, f * 4, frames]),
                        tensor_info(t, &[1, channels / 2, original]),
                    ],
                );
            }
        }
        self.node("Identity", &[x], &["spec_out"], vec![]);
        self.node("Identity", &[t], &["wave_out"], vec![]);
        let outputs = [
            tensor_info("spec_out", &[1, 16, BINS as i64, frames]),
            tensor_info("wave_out", &[1, 8, len]),
        ];
        self.graph.borrow_mut().output = outputs.to_vec();
        finish("Decoder 4", outputs);
        boundaries
    }
}

struct Boundary<'a> {
    label: &'static str,
    end: usize,
    outputs: [ValueInfo<'a>; 2],
}

pub(super) struct ModelPart {
    pub label: &'static str,
    pub bytes: Vec<u8>,
    pub inputs: Vec<String>,
}

fn onnx_model(graph: Graph<'_>) -> Model<'_> {
    Model {
        ir_version: 8,
        producer_name: "Recordbox",
        opset_import: vec![OperatorSetId {
            domain: "",
            version: 13,
        }],
        graph: Some(graph),
        ..Default::default()
    }
}

pub(super) fn model_parts(
    weights: HashMap<String, Tensor>,
    query_tile: i64,
    staged: bool,
) -> anyhow::Result<Vec<ModelPart>> {
    let names = Bump::new();
    let net = Network::new(&names, query_tile);
    let boundaries = net.htdemucs();
    for (name, tensor) in weights {
        let dims = tensor.dims().iter().map(|&d| d as i64).collect::<Vec<_>>();
        net.floats(
            names.alloc_str(&name),
            &dims,
            tensor.flatten_all()?.to_vec1::<f32>()?,
        );
    }
    let graph = net.graph.into_inner();
    if !staged {
        let inputs = graph.input.iter().map(|v| v.name.to_owned()).collect();
        return Ok(vec![ModelPart {
            label: "HTDemucs network",
            inputs,
            bytes: onnx_rs::encode(&onnx_model(graph)),
        }]);
    }
    // Each boundary exposes the two branch activations. Encoder outputs also feed the
    // matching decoder through skip connections. Constants and weights stay in the graph.
    let mut shapes: HashMap<_, _> = graph.input.into_iter().map(|v| (v.name, v)).collect();
    let mut tensors: HashMap<_, _> = graph.initializer.into_iter().map(|t| (t.name(), t)).collect();
    let mut nodes = graph.node.into_iter();
    let mut start = 0;
    let mut parts = Vec::with_capacity(boundaries.len());
    for Boundary { label, end, outputs } in boundaries {
        let mut part = Graph {
            name: label,
            output: outputs.to_vec(),
            ..Default::default()
        };
        let mut seen = HashSet::new();
        for node in nodes.by_ref().take(end - start) {
            for &name in &node.input {
                if name.is_empty() || !seen.insert(name) {
                    continue;
                }
                if let Some(tensor) = tensors.remove(name) {
                    part.initializer.push(tensor);
                } else if let Some(info) = shapes.get(name) {
                    part.input.push(info.clone());
                } else {
                    anyhow::bail!("Missing {label} input {name}");
                }
            }
            seen.extend(node.output.iter().copied());
            part.node.push(node);
        }
        start = end;
        let inputs = part.input.iter().map(|v| v.name.to_owned()).collect();
        // Return the initializers to the pool after encoding. Weights move between the
        // pool and the graph, so partitioning does not clone their backing arrays.
        let mut model = onnx_model(part);
        let bytes = onnx_rs::encode(&model);
        let encoded = model.graph.take().expect("constructed with a graph");
        tensors.extend(encoded.initializer.into_iter().map(|t| (t.name(), t)));
        shapes.extend(outputs.into_iter().map(|v| (v.name, v)));
        parts.push(ModelPart { label, bytes, inputs });
    }
    Ok(parts)
}

pub(super) fn probe_model() -> Vec<u8> {
    let names = Bump::new();
    let net = Network::new(&names, QUERY_TILE);
    net.graph.borrow_mut().input = vec![tensor_info("x", &[1, 1, 3])];
    net.floats("kernel", &[1, 1, 2], vec![2., 1.]);
    net.floats("column", &[2, 1], vec![1., 2.]);
    let conv = net.math("Conv", &["x", "kernel"]);
    let flat = net.reshape(conv, &[1, 2]);
    net.node("MatMul", &[flat, "column"], &["result"], vec![]);
    net.graph.borrow_mut().output = vec![tensor_info("result", &[1, 1])];
    onnx_rs::encode(&onnx_model(net.graph.into_inner()))
}
