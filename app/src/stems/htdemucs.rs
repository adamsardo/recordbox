//! HTDemucs (open model from facebookresearch/demucs, MIT licence) in pure Rust with Candle.
//!
//! The graph is the fixed profile validated in rekordbox-rea (analysis/open-model-harness,
//! reports/htdemucs-candle-graph.md and htdemucs-whole-track-contract.md): batch 1, stereo f32
//! at 44.1 kHz, sources drums/bass/other/vocals, 343,980-frame segments with 25 % overlap and
//! a triangular transition (power 1), no shifts. Research tracing/comparison code is removed.

use candle_core::{DType, Device, Module, Result, Tensor};
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::HashMap;
use std::path::Path;

pub const TRAINING_FRAMES: usize = 343980;
pub const SAMPLE_RATE: u32 = 44100;
/// Output order of every HTDemucs model.
pub const SOURCES: [&str; 4] = ["drums", "bass", "other", "vocals"];
const NFFT: usize = 4096;
const HOP: usize = 1024;

fn hann(n: usize) -> Vec<f32> {
    let step = (2. * std::f64::consts::PI / n as f64) as f32;
    (0..n).map(|i| 0.5 - 0.5 * (i as f32 * step).cos()).collect()
}

fn reflect(x: &[f32], left: usize, right: usize) -> Vec<f32> {
    (0..x.len() + left + right)
        .map(|i| {
            let p = i as isize - left as isize;
            let n = x.len() as isize;
            let j = if p < 0 { -p } else if p >= n { 2 * n - 2 - p } else { p };
            x[j.clamp(0, n - 1) as usize]
        })
        .collect()
}

fn spec(x: &Tensor) -> Result<Tensor> {
    let (b, c, len) = x.dims3()?;
    let frames = len.div_ceil(HOP);
    let freq = NFFT / 2;
    let pad = HOP / 2 * 3;
    let input = x.flatten_all()?.to_vec1::<f32>()?;
    let win = hann(NFFT);
    let norm = 1. / (NFFT as f32).sqrt();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let mut out = vec![0.; b * c * freq * frames * 2];
    for bc in 0..b * c {
        let padded = reflect(&input[bc * len..(bc + 1) * len], pad, pad + frames * HOP - len);
        let padded = reflect(&padded, NFFT / 2, NFFT / 2);
        for frame in 0..frames {
            let start = (frame + 2) * HOP;
            let mut buffer: Vec<_> = (0..NFFT).map(|i| Complex::new(padded[start + i] * win[i], 0.)).collect();
            fft.process(&mut buffer);
            for f in 0..freq {
                let offset = ((bc * freq + f) * frames + frame) * 2;
                out[offset] = buffer[f].re * norm;
                out[offset + 1] = buffer[f].im * norm;
            }
        }
    }
    Tensor::from_vec(out, (b, c, freq, frames, 2), x.device())
}

fn ispec(z: &Tensor, len: usize) -> Result<Tensor> {
    let (b, c, freq, frames, ri) = z.dims5()?;
    if freq != NFFT / 2 || ri != 2 {
        candle_core::bail!("wrong spectral shape")
    }
    let input = z.flatten_all()?.to_vec1::<f32>()?;
    let win = hann(NFFT);
    let norm = 1. / (NFFT as f32).sqrt();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_inverse(NFFT);
    let full_len = NFFT + (frames + 3) * HOP;
    let pad = HOP / 2 * 3;
    let mut envelope = vec![0.; full_len];
    for frame in 0..frames + 4 {
        for i in 0..NFFT {
            envelope[frame * HOP + i] += win[i] * win[i];
        }
    }
    let mut out = Vec::with_capacity(b * c * len);
    for bc in 0..b * c {
        let mut wave = vec![0.; full_len];
        for frame in 0..frames {
            let mut buffer = vec![Complex::new(0., 0.); NFFT];
            for f in 0..freq {
                let offset = ((bc * freq + f) * frames + frame) * 2;
                buffer[f] = Complex::new(input[offset], input[offset + 1]);
                if f > 0 {
                    buffer[NFFT - f] = buffer[f].conj();
                }
            }
            fft.process(&mut buffer);
            for i in 0..NFFT {
                wave[(frame + 2) * HOP + i] += buffer[i].re * norm * win[i];
            }
        }
        let start = NFFT / 2 + pad;
        for i in start..start + len {
            out.push(wave[i] / envelope[i]);
        }
    }
    Tensor::from_vec(out, (b, c, len), z.device())
}

/// One loaded HTDemucs network.
pub struct Graph {
    weights: HashMap<String, Tensor>,
}

impl Graph {
    /// Load weights from a converted `.safetensors` file or an original Demucs `.th` checkpoint.
    pub fn load(path: &Path, device: &Device) -> Result<Self> {
        let raw: HashMap<String, Tensor> = match path.extension().and_then(|e| e.to_str()) {
            Some("safetensors") => candle_core::safetensors::load(path, device)?,
            _ => {
                // torch.save({"klass", "args", "kwargs", "state": state_dict, ...})
                let pth = candle_core::pickle::PthTensors::new(path, Some("state"))?;
                let mut m = HashMap::new();
                for name in pth.tensor_infos().keys() {
                    if let Some(t) = pth.get(name)? {
                        m.insert(name.clone(), t.to_device(device)?);
                    }
                }
                m
            }
        };
        let weights = raw.into_iter().map(|(k, v)| Ok((k, v.to_dtype(DType::F32)?))).collect::<Result<HashMap<_, _>>>()?;
        if weights.len() != 533 {
            candle_core::bail!("expected an HTDemucs checkpoint with 533 tensors, found {}", weights.len())
        }
        Ok(Self { weights })
    }

    fn w(&self, name: &str) -> Result<Tensor> {
        self.weights.get(name).cloned().ok_or_else(|| candle_core::Error::Msg(format!("missing weight {name}")))
    }
    fn conv1(&self, x: &Tensor, name: &str, pad: usize, stride: usize, dilation: usize) -> Result<Tensor> {
        let w = self.w(&format!("{name}.weight"))?;
        let co = w.dim(0)?;
        x.conv1d(&w, pad, stride, dilation, 1)?.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)
    }
    fn conv2(&self, x: &Tensor, name: &str, pad: usize) -> Result<Tensor> {
        let w = self.w(&format!("{name}.weight"))?;
        let co = w.dim(0)?;
        x.conv2d(&w, pad, 1, 1, 1)?.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1, 1))?)
    }
    fn freq_conv(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        let (b, c, f, t) = x.dims4()?;
        let w = self.w(&format!("{name}.weight"))?.squeeze(3)?;
        let co = w.dim(0)?;
        let y = x.permute((0, 3, 1, 2))?.contiguous()?.reshape((b * t, c, f))?.conv1d(&w, 2, 4, 1, 1)?;
        let fo = y.dim(2)?;
        y.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)?.reshape((b, t, co, fo))?.permute((0, 2, 3, 1))?.contiguous()
    }
    fn transpose1(&self, x: &Tensor, w: &Tensor) -> Result<Tensor> {
        if matches!(x.device(), Device::Cpu) {
            x.unsqueeze(3)?.conv_transpose2d(&w.unsqueeze(3)?, 0, 0, 4, 1)?.squeeze(3)
        } else {
            x.conv_transpose1d(w, 0, 0, 4, 1, 1)
        }
    }
    fn transposed(&self, x: &Tensor, name: &str, freq: bool, length: usize) -> Result<Tensor> {
        let mut w = self.w(&format!("{name}.weight"))?;
        let co = w.dim(1)?;
        if freq {
            let (b, c, f, t) = x.dims4()?;
            w = w.squeeze(3)?;
            let y = self.transpose1(&x.permute((0, 3, 1, 2))?.contiguous()?.reshape((b * t, c, f))?, &w)?;
            let fo = y.dim(2)?;
            y.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)?.narrow(2, 2, fo - 4)?.reshape((b, t, co, fo - 4))?.permute((0, 2, 3, 1))?.contiguous()
        } else {
            self.transpose1(x, &w)?.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)?.narrow(2, 2, length)?.contiguous()
        }
    }
    fn gn(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        // CPU normalises in f64 (large near-constant groups amplify f32 reduction error).
        let internal = if matches!(x.device(), Device::Cpu) { DType::F64 } else { DType::F32 };
        candle_nn::GroupNorm::new(self.w(&format!("{name}.weight"))?.to_dtype(internal)?, self.w(&format!("{name}.bias"))?.to_dtype(internal)?, x.dim(1)?, 1, 1e-5)?
            .forward(&x.to_dtype(internal)?)?
            .to_dtype(DType::F32)
    }
    fn ln(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        candle_nn::LayerNorm::new(self.w(&format!("{name}.weight"))?, self.w(&format!("{name}.bias"))?, 1e-5).forward(x)
    }
    fn glu(&self, x: &Tensor) -> Result<Tensor> {
        let half = x.dim(1)? / 2;
        x.narrow(1, 0, half)?.mul(&candle_nn::ops::sigmoid(&x.narrow(1, half, half)?)?)
    }
    fn dconv(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        let mut x = x.clone();
        for i in 0..2 {
            let p = format!("{name}.layers.{i}");
            let dilation = 1 << i;
            let y = self.conv1(&x, &format!("{p}.0"), dilation, 1, dilation)?;
            let y = self.gn(&y, &format!("{p}.1"))?.gelu_erf()?;
            let y = self.conv1(&y, &format!("{p}.3"), 0, 1, 1)?;
            let y = self.glu(&self.gn(&y, &format!("{p}.4"))?)?;
            let scale = self.w(&format!("{p}.6.scale"))?.reshape((1, x.dim(1)?, 1))?;
            x = (x + y.broadcast_mul(&scale)?)?;
        }
        Ok(x)
    }
    fn spatial_dconv(&self, x: &Tensor, name: &str, freq: bool) -> Result<Tensor> {
        if !freq {
            return self.dconv(x, name);
        }
        let (b, c, f, t) = x.dims4()?;
        self.dconv(&x.permute((0, 2, 1, 3))?.contiguous()?.reshape((b * f, c, t))?, name)?.reshape((b, f, c, t))?.permute((0, 2, 1, 3))?.contiguous()
    }
    fn encode(&self, x: &Tensor, name: &str, freq: bool) -> Result<Tensor> {
        let y = if freq {
            self.freq_conv(x, &format!("{name}.conv"))?
        } else {
            let len = x.dim(2)?;
            let x = x.pad_with_zeros(2, 0, (4 - len % 4) % 4)?;
            self.conv1(&x, &format!("{name}.conv"), 2, 4, 1)?
        }
        .gelu_erf()?;
        let y = self.spatial_dconv(&y, &format!("{name}.dconv"), freq)?;
        let y = if freq { self.conv2(&y, &format!("{name}.rewrite"), 0)? } else { self.conv1(&y, &format!("{name}.rewrite"), 0, 1, 1)? };
        self.glu(&y)
    }
    fn decode(&self, x: &Tensor, skip: &Tensor, name: &str, freq: bool, length: usize, last: bool) -> Result<Tensor> {
        let y = (x + skip)?;
        let y = if freq { self.conv2(&y, &format!("{name}.rewrite"), 1)? } else { self.conv1(&y, &format!("{name}.rewrite"), 1, 1, 1)? };
        let pre = self.spatial_dconv(&self.glu(&y)?, &format!("{name}.dconv"), freq)?;
        let z = self.transposed(&pre, &format!("{name}.conv_tr"), freq, length)?;
        if last {
            Ok(z)
        } else {
            z.gelu_erf()
        }
    }
    fn linear_weight(&self, x: &Tensor, w: &Tensor, bias: &Tensor) -> Result<Tensor> {
        let (b, t, c) = x.dims3()?;
        let co = w.dim(0)?;
        x.reshape((b * t, c))?.matmul(&w.t()?.contiguous()?)?.broadcast_add(bias)?.reshape((b, t, co))
    }
    fn linear(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        self.linear_weight(x, &self.w(&format!("{name}.weight"))?, &self.w(&format!("{name}.bias"))?)
    }
    fn attention(&self, q: &Tensor, k: &Tensor, name: &str) -> Result<Tensor> {
        let (b, l, c) = q.dims3()?;
        let s = k.dim(1)?;
        let h = 8;
        let head = c / h;
        let w = self.w(&format!("{name}.in_proj_weight"))?;
        let bias = self.w(&format!("{name}.in_proj_bias"))?;
        let q = self.linear_weight(q, &w.narrow(0, 0, c)?.contiguous()?, &bias.narrow(0, 0, c)?)?.reshape((b, l, h, head))?.permute((0, 2, 1, 3))?.contiguous()?;
        let kp = self.linear_weight(k, &w.narrow(0, c, c)?.contiguous()?, &bias.narrow(0, c, c)?)?.reshape((b, s, h, head))?.permute((0, 2, 1, 3))?.contiguous()?;
        let vp = self.linear_weight(k, &w.narrow(0, 2 * c, c)?.contiguous()?, &bias.narrow(0, 2 * c, c)?)?.reshape((b, s, h, head))?.permute((0, 2, 1, 3))?.contiguous()?;
        let scores = (q.matmul(&kp.transpose(2, 3)?.contiguous()?)? / (head as f64).sqrt())?;
        let y = candle_nn::ops::softmax_last_dim(&scores)?.matmul(&vp)?.permute((0, 2, 1, 3))?.contiguous()?.reshape((b, l, c))?;
        self.linear(&y, &format!("{name}.out_proj"))
    }
    fn transformer_layer(&self, q: &Tensor, k: Option<&Tensor>, name: &str) -> Result<Tensor> {
        let nq = self.ln(q, &format!("{name}.norm1"))?;
        let attn = if let Some(k) = k { self.attention(&nq, &self.ln(k, &format!("{name}.norm2"))?, &format!("{name}.cross_attn"))? } else { self.attention(&nq, &nq, &format!("{name}.self_attn"))? };
        let x = (q + attn.broadcast_mul(&self.w(&format!("{name}.gamma_1.scale"))?)?)?;
        let norm_name = if k.is_some() { "norm3" } else { "norm2" };
        let y = self.ln(&x, &format!("{name}.{norm_name}"))?;
        let y = self.linear(&y, &format!("{name}.linear1"))?.gelu_erf()?;
        let y = self.linear(&y, &format!("{name}.linear2"))?;
        let x = (x + y.broadcast_mul(&self.w(&format!("{name}.gamma_2.scale"))?)?)?;
        self.gn(&x.transpose(1, 2)?.contiguous()?, &format!("{name}.norm_out"))?.transpose(1, 2)?.contiguous()
    }
    fn pos1(&self, len: usize, c: usize, device: &Device) -> Result<Tensor> {
        let half = c / 2;
        let mut out = vec![0.; len * c];
        for t in 0..len {
            for i in 0..half {
                let denom = 10000f32.powf(i as f32 / (half - 1) as f32);
                let phase = t as f32 / denom;
                out[t * c + i] = phase.cos();
                out[t * c + half + i] = phase.sin();
            }
        }
        Tensor::from_vec(out, (1, len, c), device)
    }
    fn pos2(&self, f: usize, t: usize, c: usize, device: &Device) -> Result<Tensor> {
        let half = c / 2;
        let mut out = vec![0.; f * t * c];
        let scale = -(10000f64.ln() / half as f64) as f32;
        for time in 0..t {
            for freq in 0..f {
                for i in 0..half / 2 {
                    let div = ((2 * i) as f32 * scale).exp();
                    let pt = time as f32 * div;
                    let pf = freq as f32 * div;
                    let offset = (time * f + freq) * c + 2 * i;
                    out[offset] = pt.sin();
                    out[offset + 1] = pt.cos();
                    out[offset + half] = pf.sin();
                    out[offset + half + 1] = pf.cos();
                }
            }
        }
        Tensor::from_vec(out, (1, t * f, c), device)
    }
    fn transformer(&self, x: &Tensor, xt: &Tensor) -> Result<(Tensor, Tensor)> {
        let (b, c, f, t) = x.dims4()?;
        let tt = xt.dim(2)?;
        let x = x.permute((0, 3, 2, 1))?.contiguous()?.reshape((b, t * f, c))?;
        let mut x = self.ln(&x, "crosstransformer.norm_in")?.broadcast_add(&self.pos2(f, t, c, x.device())?)?;
        let mut xt = self.ln(&xt.transpose(1, 2)?.contiguous()?, "crosstransformer.norm_in_t")?.broadcast_add(&self.pos1(tt, c, xt.device())?)?;
        for i in 0..5 {
            let prefix = format!("crosstransformer.layers.{i}");
            let tp = format!("crosstransformer.layers_t.{i}");
            if i % 2 == 0 {
                x = self.transformer_layer(&x, None, &prefix)?;
                xt = self.transformer_layer(&xt, None, &tp)?;
            } else {
                let old_x = x;
                x = self.transformer_layer(&old_x, Some(&xt), &prefix)?;
                xt = self.transformer_layer(&xt, Some(&old_x), &tp)?;
            }
        }
        Ok((x.reshape((b, t, f, c))?.permute((0, 3, 2, 1))?.contiguous()?, xt.transpose(1, 2)?.contiguous()?))
    }
    fn normalize(&self, x: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let internal = if matches!(x.device(), Device::Cpu) { DType::F64 } else { DType::F32 };
        let flat = x.flatten_from(1)?.to_dtype(internal)?;
        let mean = flat.mean_keepdim(1)?.to_dtype(DType::F32)?;
        let std = flat.var_keepdim(1)?.sqrt()?.to_dtype(DType::F32)?;
        let mut dims = vec![1; x.rank()];
        dims[0] = x.dim(0)?;
        let mean = mean.reshape(dims.clone())?;
        let std = std.reshape(dims)?;
        let y = x.broadcast_sub(&mean)?.broadcast_div(&(&std + 1e-5)?)?;
        Ok((y, mean, std))
    }

    /// One model pass: [1, 2, L] (L <= 343980) -> [1, 4, 2, L].
    pub fn forward(&self, input: &Tensor) -> Result<Tensor> {
        let (b, c, original_len) = input.dims3()?;
        if b != 1 || c != 2 || original_len == 0 || original_len > TRAINING_FRAMES {
            candle_core::bail!("HTDemucs input must be [1, 2, L] with 1 <= L <= {TRAINING_FRAMES}")
        }
        let mix = input.pad_with_zeros(2, 0, TRAINING_FRAMES - original_len)?;
        let spectral = spec(&mix)?;
        let frames = spectral.dim(3)?;
        let freq = spectral.dim(2)?;
        let magnitude = spectral.permute((0, 1, 4, 2, 3))?.contiguous()?.reshape((b, c * 2, freq, frames))?;
        let (mut x, mean, std) = self.normalize(&magnitude)?;
        let (mut xt, meant, stdt) = self.normalize(&mix)?;
        let mut saved = vec![];
        let mut saved_t = vec![];
        let mut lengths_t = vec![];
        for i in 0..4 {
            lengths_t.push(xt.dim(2)?);
            xt = self.encode(&xt, &format!("tencoder.{i}"), false)?;
            x = self.encode(&x, &format!("encoder.{i}"), true)?;
            if i == 0 {
                let embedding = (self.w("freq_emb.embedding.weight")?.t()?.unsqueeze(0)?.unsqueeze(3)? * 10.)?;
                x = x.broadcast_add(&(embedding * 0.2)?)?;
            }
            saved.push(x.clone());
            saved_t.push(xt.clone());
        }
        let (_, co, f, t) = x.dims4()?;
        x = self.conv1(&x.reshape((b, co, f * t))?, "channel_upsampler", 0, 1, 1)?.reshape((b, 512, f, t))?;
        xt = self.conv1(&xt, "channel_upsampler_t", 0, 1, 1)?;
        (x, xt) = self.transformer(&x, &xt)?;
        x = self.conv1(&x.reshape((b, 512, f * t))?, "channel_downsampler", 0, 1, 1)?.reshape((b, co, f, t))?;
        xt = self.conv1(&xt, "channel_downsampler_t", 0, 1, 1)?;
        for i in 0..4 {
            x = self.decode(&x, &saved.pop().unwrap(), &format!("decoder.{i}"), true, t, i == 3)?;
            xt = self.decode(&xt, &saved_t.pop().unwrap(), &format!("tdecoder.{i}"), false, lengths_t.pop().unwrap(), i == 3)?;
        }
        let x = x.reshape((b, 4, 4, freq, frames))?.broadcast_mul(&std.unsqueeze(1)?)?.broadcast_add(&mean.unsqueeze(1)?)?;
        let complex = x.reshape((b * 4, 2, 2, freq, frames))?.permute((0, 1, 3, 4, 2))?.contiguous()?;
        let spectral_wave = ispec(&complex, TRAINING_FRAMES)?.reshape((b, 4, 2, TRAINING_FRAMES))?;
        let temporal_wave = xt.reshape((b, 4, 2, TRAINING_FRAMES))?.broadcast_mul(&stdt.unsqueeze(1)?)?.broadcast_add(&meant.unsqueeze(1)?)?;
        (spectral_wave + temporal_wave)?.narrow(3, 0, original_len)?.contiguous()
    }
}

/// Run a model over a whole track the way upstream `apply_model` does (segment 343,980,
/// 25 % overlap, triangular weights, chunks padded from the surrounding audio).
/// `audio` is planar stereo: left samples then right samples, `len` frames each.
/// Returns planar [source][channel][frame] (4 × 2 × len). `progress(done, total)` is called
/// after each chunk; returning false cancels.
pub fn separate(graph: &Graph, device: &Device, audio: &[f32], len: usize, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<Vec<f32>> {
    if len == 0 || audio.len() != 2 * len {
        candle_core::bail!("empty or malformed input")
    }
    let segment = TRAINING_FRAMES;
    let stride = (0.75 * segment as f64) as usize;
    let half = segment / 2;
    let maximum = (segment - half) as f32;
    let weight: Vec<f32> = (0..segment).map(|i| (if i < half { i + 1 } else { segment - i }) as f32 / maximum).collect();
    let mut out = vec![0f32; 8 * len];
    let mut sums = vec![0f32; len];
    let offsets: Vec<usize> = (0..len).step_by(stride).collect();
    for (k, &offset) in offsets.iter().enumerate() {
        let chunk_len = (len - offset).min(segment);
        // Pad the chunk to the segment length from the surrounding audio, centred.
        let start = offset as isize - ((segment - chunk_len) / 2) as isize;
        let mut input = vec![0f32; 2 * segment];
        for ch in 0..2 {
            for i in 0..segment {
                let p = start + i as isize;
                if p >= 0 && (p as usize) < len {
                    input[ch * segment + i] = audio[ch * len + p as usize];
                }
            }
        }
        let x = Tensor::from_vec(input, (1, 2, segment), device)?;
        let y = graph.forward(&x)?.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?;
        let crop = (segment - chunk_len) / 2;
        for ch in 0..8 {
            for i in 0..chunk_len {
                out[ch * len + offset + i] += weight[i] * y[ch * segment + crop + i];
            }
        }
        for i in 0..chunk_len {
            sums[offset + i] += weight[i];
        }
        if !progress(k + 1, offsets.len()) {
            candle_core::bail!("stopped")
        }
    }
    for ch in 0..8 {
        for i in 0..len {
            out[ch * len + i] /= sums[i].max(1e-9);
        }
    }
    Ok(out)
}
