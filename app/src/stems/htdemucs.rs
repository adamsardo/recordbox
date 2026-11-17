//! HTDemucs (open model from facebookresearch/demucs, MIT licence) in pure Rust with Candle.
//!
//! The graph is the fixed profile validated in rekordbox-rea (analysis/open-model-harness,
//! reports/htdemucs-candle-graph.md and htdemucs-whole-track-contract.md): batch 1, stereo f32
//! at 44.1 kHz, sources drums/bass/other/vocals, 343,980-frame segments with 25 % overlap and
//! a triangular transition (power 1), no shifts. Research tracing/comparison code is removed.

#[cfg(target_os = "macos")]
#[path = "metal_fast.rs"]
mod metal_fast;

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

/// The model's short-time Fourier transform (4096 window, hop 1024, the upstream padding)
/// for one channel at a time, on the CPU.
pub(crate) struct Stft {
    win: Vec<f32>,
    fwd: std::sync::Arc<dyn rustfft::Fft<f32>>,
    inv: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl Stft {
    pub(crate) fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        Self { win: hann(NFFT), fwd: planner.plan_fft_forward(NFFT), inv: planner.plan_fft_inverse(NFFT) }
    }

    pub(crate) fn frames(len: usize) -> usize {
        len.div_ceil(HOP)
    }

    /// x -> real and imaginary planes, each [bin][frame] with NFFT/2 bins.
    pub(crate) fn forward(&self, x: &[f32], re: &mut [f32], im: &mut [f32]) {
        let len = x.len();
        let frames = Self::frames(len);
        let freq = NFFT / 2;
        let pad = HOP / 2 * 3;
        let norm = 1. / (NFFT as f32).sqrt();
        let padded = reflect(x, pad, pad + frames * HOP - len);
        let padded = reflect(&padded, NFFT / 2, NFFT / 2);
        let mut buffer = vec![Complex::new(0f32, 0.); NFFT];
        let mut scratch = vec![Complex::new(0f32, 0.); self.fwd.get_inplace_scratch_len()];
        for frame in 0..frames {
            let start = (frame + 2) * HOP;
            for (i, b) in buffer.iter_mut().enumerate() {
                *b = Complex::new(padded[start + i] * self.win[i], 0.);
            }
            self.fwd.process_with_scratch(&mut buffer, &mut scratch);
            for f in 0..freq {
                re[f * frames + frame] = buffer[f].re * norm;
                im[f * frames + frame] = buffer[f].im * norm;
            }
        }
    }

    /// Sum of squared windows at every output position (the inverse divides by it).
    pub(crate) fn envelope(&self, frames: usize) -> Vec<f32> {
        let mut envelope = vec![0.; NFFT + (frames + 3) * HOP];
        for frame in 0..frames + 4 {
            for i in 0..NFFT {
                envelope[frame * HOP + i] += self.win[i] * self.win[i];
            }
        }
        envelope
    }

    /// Planes [bin][frame] (each value taken as v * scale + shift) -> out.len() samples.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inverse(&self, re: &[f32], im: &[f32], frames: usize, scale: f32, shift: f32, envelope: &[f32], out: &mut [f32]) {
        let freq = NFFT / 2;
        let norm = 1. / (NFFT as f32).sqrt();
        let pad = HOP / 2 * 3;
        let mut wave = vec![0f32; NFFT + (frames + 3) * HOP];
        let mut buffer = vec![Complex::new(0f32, 0.); NFFT];
        let mut scratch = vec![Complex::new(0f32, 0.); self.inv.get_inplace_scratch_len()];
        for frame in 0..frames {
            buffer[freq] = Complex::new(0., 0.);
            for f in 0..freq {
                let v = Complex::new(re[f * frames + frame] * scale + shift, im[f * frames + frame] * scale + shift);
                buffer[f] = v;
                if f > 0 {
                    buffer[NFFT - f] = v.conj();
                }
            }
            self.inv.process_with_scratch(&mut buffer, &mut scratch);
            let w = &mut wave[(frame + 2) * HOP..(frame + 2) * HOP + NFFT];
            for i in 0..NFFT {
                w[i] += buffer[i].re * norm * self.win[i];
            }
        }
        let start = NFFT / 2 + pad;
        for (i, o) in out.iter_mut().enumerate() {
            *o = wave[start + i] / envelope[start + i];
        }
    }
}

fn spec(x: &Tensor) -> Result<Tensor> {
    let (b, c, len) = x.dims3()?;
    let frames = Stft::frames(len);
    let freq = NFFT / 2;
    let input = x.flatten_all()?.to_vec1::<f32>()?;
    let stft = Stft::new();
    let mut out = vec![0.; b * c * freq * frames * 2];
    let (mut re, mut im) = (vec![0f32; freq * frames], vec![0f32; freq * frames]);
    for bc in 0..b * c {
        stft.forward(&input[bc * len..(bc + 1) * len], &mut re, &mut im);
        for i in 0..freq * frames {
            out[(bc * freq * frames + i) * 2] = re[i];
            out[(bc * freq * frames + i) * 2 + 1] = im[i];
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
    let stft = Stft::new();
    let envelope = stft.envelope(frames);
    let mut out = vec![0f32; b * c * len];
    let (mut re, mut im) = (vec![0f32; freq * frames], vec![0f32; freq * frames]);
    for bc in 0..b * c {
        for i in 0..freq * frames {
            re[i] = input[(bc * freq * frames + i) * 2];
            im[i] = input[(bc * freq * frames + i) * 2 + 1];
        }
        stft.inverse(&re, &im, frames, 1., 0., &envelope, &mut out[bc * len..(bc + 1) * len]);
    }
    Tensor::from_vec(out, (b, c, len), z.device())
}

/// Sinusoidal embedding of the time-branch tokens: [t][c].
pub(crate) fn pos1(len: usize, c: usize) -> Vec<f32> {
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
    out
}

/// 2-D sinusoidal embedding of the spectrogram-branch tokens: [time][freq][c].
pub(crate) fn pos2(f: usize, t: usize, c: usize) -> Vec<f32> {
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
    out
}

/// One loaded HTDemucs network.
pub fn fast_report() { metal_fast::report() }
pub struct Graph {
    /// The checkpoint's tensors by name: the reference graph (any device).
    weights: HashMap<String, Tensor>,
    /// Weights prepared for the fused Metal kernels; when present, `separate` uses them and
    /// `weights` is empty.
    #[cfg(target_os = "macos")]
    fast: Option<metal_fast::Fast>,
}

impl Graph {
    /// Load weights from a converted `.safetensors` file or an original Demucs `.th` checkpoint.
    /// On a Metal device the fused kernels are used; elsewhere the reference graph.
    pub fn load(path: &Path, device: &Device) -> Result<Self> {
        Self::load_as(path, device, device.is_metal())
    }

    /// As `load`, but always the reference graph built from Candle's own operations.
    pub fn load_reference(path: &Path, device: &Device) -> Result<Self> {
        Self::load_as(path, device, false)
    }

    fn load_as(path: &Path, device: &Device, fast: bool) -> Result<Self> {
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
        Self::from_weights(weights, fast)
    }

    fn from_weights(weights: HashMap<String, Tensor>, fast: bool) -> Result<Self> {
        #[cfg(target_os = "macos")]
        if fast {
            return Ok(Self { fast: Some(metal_fast::Fast::new(&weights)?), weights: HashMap::new() });
        }
        let _ = fast;
        Ok(Self {
            weights,
            #[cfg(target_os = "macos")]
            fast: None,
        })
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
    fn transformer(&self, x: &Tensor, xt: &Tensor) -> Result<(Tensor, Tensor)> {
        let (b, c, f, t) = x.dims4()?;
        let tt = xt.dim(2)?;
        let x = x.permute((0, 3, 2, 1))?.contiguous()?.reshape((b, t * f, c))?;
        let mut x = self.ln(&x, "crosstransformer.norm_in")?.broadcast_add(&Tensor::from_vec(pos2(f, t, c), (1, t * f, c), x.device())?)?;
        let mut xt = self.ln(&xt.transpose(1, 2)?.contiguous()?, "crosstransformer.norm_in_t")?.broadcast_add(&Tensor::from_vec(pos1(tt, c), (1, tt, c), xt.device())?)?;
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

/// How a track is cut up and what is returned.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Share of each 7.8 s segment that overlaps the next (upstream default 0.25).
    pub overlap: f64,
    /// Return only this source (0 drums, 1 bass, 2 other, 3 vocals) instead of all four.
    pub source: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self { overlap: 0.25, source: None }
    }
}

/// Segment start offsets and the triangular weight of upstream `apply_model`.
fn plan(len: usize, overlap: f64) -> (Vec<usize>, Vec<f32>) {
    let segment = TRAINING_FRAMES;
    let stride = (((1.0 - overlap.clamp(0.0, 0.9)) * segment as f64) as usize).max(1);
    let half = segment / 2;
    let maximum = (segment - half) as f32;
    let weight = (0..segment).map(|i| (if i < half { i + 1 } else { segment - i }) as f32 / maximum).collect();
    ((0..len).step_by(stride).collect(), weight)
}

/// The segment for the chunk at `offset`, padded to the segment length from the surrounding
/// audio (centred), planar stereo.
fn segment_at(audio: &[f32], len: usize, offset: usize) -> Vec<f32> {
    let segment = TRAINING_FRAMES;
    let chunk_len = (len - offset).min(segment);
    let start = offset as isize - ((segment - chunk_len) / 2) as isize;
    let mut input = vec![0f32; 2 * segment];
    for ch in 0..2 {
        let (a, b) = (start.max(0) as usize, ((start + segment as isize) as usize).min(len));
        let at = (a as isize - start) as usize;
        input[ch * segment + at..ch * segment + at + (b - a)].copy_from_slice(&audio[ch * len + a..ch * len + b]);
    }
    input
}

/// Serialises GPU work between separation jobs (full stems, vocal detection, key analysis).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run a model over a whole track the way upstream `apply_model` does (segment 343,980,
/// overlapping segments with triangular weights, chunks padded from the surrounding audio).
/// `audio` is planar stereo: left samples then right samples, `len` frames each.
/// Returns planar [source][channel][frame] (4 × 2 × len), or [channel][frame] of the one
/// source asked for. `progress(done, total)` is called after each chunk; returning false
/// cancels.
pub fn separate(graph: &Graph, device: &Device, audio: &[f32], len: usize, opts: &Options, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<Vec<f32>> {
    if len == 0 || audio.len() != 2 * len {
        candle_core::bail!("empty or malformed input")
    }
    if opts.source.is_some_and(|s| s >= 4) {
        candle_core::bail!("no such source")
    }
    #[cfg(target_os = "macos")]
    if let Some(fast) = &graph.fast {
        return separate_fast(fast, device, audio, len, opts, progress);
    }
    let segment = TRAINING_FRAMES;
    let (offsets, weight) = plan(len, opts.overlap);
    let sources: Vec<usize> = opts.source.map(|s| vec![s]).unwrap_or_else(|| (0..4).collect());
    let mut out = vec![0f32; sources.len() * 2 * len];
    let mut sums = vec![0f32; len];
    for (k, &offset) in offsets.iter().enumerate() {
        let chunk_len = (len - offset).min(segment);
        let x = Tensor::from_vec(segment_at(audio, len, offset), (1, 2, segment), device)?;
        let y = {
            let _gpu = GPU.lock();
            graph.forward(&x)?.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?
        };
        let crop = (segment - chunk_len) / 2;
        for (n, &s) in sources.iter().enumerate() {
            for ch in 0..2 {
                let (dst, src) = (&mut out[(n * 2 + ch) * len + offset..][..chunk_len], &y[(s * 2 + ch) * segment + crop..][..chunk_len]);
                for i in 0..chunk_len {
                    dst[i] += weight[i] * src[i];
                }
            }
        }
        for i in 0..chunk_len {
            sums[offset + i] += weight[i];
        }
        if !progress(k + 1, offsets.len()) {
            candle_core::bail!("stopped")
        }
    }
    for ch in out.chunks_mut(len) {
        for (v, s) in ch.iter_mut().zip(&sums) {
            *v /= s.max(1e-9);
        }
    }
    Ok(out)
}

/// Mean and (unbiased) standard deviation, accumulated in f64.
fn mean_std(x: &[f32]) -> (f32, f32) {
    let n = x.len() as f64;
    let mean = x.iter().map(|v| *v as f64).sum::<f64>() / n;
    let var = x.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean as f32, var.sqrt() as f32)
}

/// One segment ready for the GPU: normalised spectrogram and waveform with their statistics.
#[cfg(target_os = "macos")]
struct Prepared {
    offset: usize,
    spec: Vec<f32>,
    wave: Vec<f32>,
    /// (mean, std) of the spectrogram and of the waveform.
    stats: [(f32, f32); 2],
}

/// The network's two outputs for one segment, still normalised.
#[cfg(target_os = "macos")]
struct Raw {
    offset: usize,
    spec: Vec<f32>,
    wave: Vec<f32>,
    stats: [(f32, f32); 2],
}

/// The Metal path: three stages run side by side so the GPU never waits for the CPU. A
/// thread prepares segments (normalisation and the forward transform), this thread runs the
/// network, and a third does the inverse transform and the overlap-add.
#[cfg(target_os = "macos")]
fn separate_fast(fast: &metal_fast::Fast, device: &Device, audio: &[f32], len: usize, opts: &Options, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<Vec<f32>> {
    let segment = TRAINING_FRAMES;
    let freq = NFFT / 2;
    let frames = Stft::frames(segment);
    let plane = freq * frames;
    let (offsets, weight) = plan(len, opts.overlap);
    let sources: Vec<usize> = opts.source.map(|s| vec![s]).unwrap_or_else(|| (0..4).collect());
    let n_src = sources.len();
    let stft = Stft::new();
    let envelope = stft.envelope(frames);
    let mut out = vec![0f32; n_src * 2 * len];
    let (stft, envelope, weight, offsets) = (&stft, &envelope, &weight, &offsets);

    let result = std::thread::scope(|scope| -> Result<()> {
        let (tx_in, rx_in) = std::sync::mpsc::sync_channel::<Prepared>(2);
        let (tx_out, rx_out) = std::sync::mpsc::sync_channel::<Raw>(2);
        scope.spawn(move || {
            for &offset in offsets {
                let input = segment_at(audio, len, offset);
                let mut spec = vec![0f32; 4 * plane];
                std::thread::scope(|s| {
                    for (ch, planes) in spec.chunks_mut(2 * plane).enumerate() {
                        let x = &input[ch * segment..(ch + 1) * segment];
                        s.spawn(move || {
                            let (re, im) = planes.split_at_mut(plane);
                            stft.forward(x, re, im);
                        });
                    }
                });
                let stats = [mean_std(&spec), mean_std(&input)];
                let mut wave = input;
                for (v, (mean, std)) in [(&mut spec, stats[0]), (&mut wave, stats[1])] {
                    let scale = 1.0 / (std + 1e-5);
                    v.iter_mut().for_each(|x| *x = (*x - mean) * scale);
                }
                if tx_in.send(Prepared { offset, spec, wave, stats }).is_err() {
                    return;
                }
            }
        });
        let out = &mut out;
        let post = scope.spawn(move || {
            for raw in rx_out {
                let chunk_len = (len - raw.offset).min(segment);
                let crop = (segment - chunk_len) / 2;
                let raw = &raw;
                std::thread::scope(|s| {
                    for (n, dst) in out.chunks_mut(len).enumerate() {
                        s.spawn(move || {
                            let (re, im) = raw.spec[n * 2 * plane..(n + 1) * 2 * plane].split_at(plane);
                            let mut wave = vec![0f32; segment];
                            stft.inverse(re, im, frames, raw.stats[0].1, raw.stats[0].0, envelope, &mut wave);
                            let time = &raw.wave[n * segment..(n + 1) * segment];
                            let (mean, std) = raw.stats[1];
                            let dst = &mut dst[raw.offset..raw.offset + chunk_len];
                            for i in 0..chunk_len {
                                dst[i] += weight[i] * (wave[crop + i] + time[crop + i] * std + mean);
                            }
                        });
                    }
                });
            }
        });
        let mut run = || -> Result<()> {
            let mut tw = std::time::Instant::now();
            for (k, p) in rx_in.iter().enumerate() {
                metal_fast::mark(device, &mut tw, "top wait for prep");
                let (spec, wave) = {
                    let _gpu = GPU.lock();
                    let x = Tensor::from_vec(p.spec, (4, freq, frames), device)?;
                    let xt = Tensor::from_vec(p.wave, (2, 1, segment), device)?;
                    let te = std::time::Instant::now();
                    let (mut y, mut yt) = fast.forward(&x, &xt)?;
                    if std::env::var("PROF").as_deref() == Ok("3") { eprintln!("encode {:.1} ms", te.elapsed().as_secs_f64() * 1e3); }
                    if let Some(s) = opts.source {
                        y = y.narrow(0, s * 4, 4)?;
                        yt = yt.narrow(0, s * 2, 2)?;
                    }
                    (y.flatten_all()?.to_vec1::<f32>()?, yt.flatten_all()?.to_vec1::<f32>()?)
                };
                metal_fast::mark(device, &mut tw, "top gpu total incl download");
                if tx_out.send(Raw { offset: p.offset, spec, wave, stats: p.stats }).is_err() {
                    candle_core::bail!("stem post-processing stopped")
                }
                if !progress(k + 1, offsets.len()) {
                    candle_core::bail!("stopped")
                }
                metal_fast::mark(device, &mut tw, "top send to post");
            }
            Ok(())
        };
        let r = run();
        // Closing both channels lets the other two stages finish.
        drop(rx_in);
        drop(tx_out);
        if post.join().is_err() {
            candle_core::bail!("stem post-processing failed")
        }
        r
    });
    result?;
    let mut sums = vec![0f32; len];
    for &offset in offsets {
        let chunk_len = (len - offset).min(segment);
        for i in 0..chunk_len {
            sums[offset + i] += weight[i];
        }
    }
    for ch in out.chunks_mut(len) {
        for (v, s) in ch.iter_mut().zip(&sums) {
            *v /= s.max(1e-9);
        }
    }
    Ok(out)
}
