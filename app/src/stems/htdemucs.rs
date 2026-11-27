//! HTDemucs (open model from facebookresearch/demucs, MIT licence) in pure Rust with Candle.
//!
//! The graph is the fixed profile validated in rekordbox-rea (analysis/open-model-harness,
//! reports/htdemucs-candle-graph.md and htdemucs-whole-track-contract.md): batch 1, stereo f32
//! at 44.1 kHz, sources drums/bass/other/vocals, 343,980-frame segments with 25 % overlap and
//! a triangular transition (power 1), no shifts. Research tracing/comparison code is removed.
//!
//! There are two implementations of the same network. `Graph::forward` is the reference,
//! written with Candle's own operations; it runs on any device and is what the CPU uses. On
//! Metal, `separate` uses the fused kernels in `metal_fast.rs` instead (about 7× faster on
//! an M5 Pro; outputs agree with the reference to better than 100 dB, which the test
//! `metal_kernels_match_reference_graph` guards) and overlaps the transforms on the CPU
//! with the network on the GPU.
//!
//! `separate_with` runs one or more networks over a track in any segment order (the caller
//! picks the next one) and reports frames as soon as every segment covering them is in, so
//! the player can use the stems while they are made; the overlap-add is exact in any order.

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

/// The tensors of an original Demucs checkpoint (`torch.save({"klass", "args", "kwargs",
/// "state": state_dict, ...})`), on the CPU. Candle's `PthTensors::get` opens the archive and
/// parses its directory again for every tensor (1.2-1.4 s for HTDemucs's 533); this opens it
/// once and reads every tensor the same way (same bytes, same layout handling).
fn read_th(path: &Path) -> Result<Vec<(String, Tensor)>> {
    use std::io::Read;
    let infos = candle_core::pickle::read_pth_tensor_info(path, false, Some("state"))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path)?)).map_err(|e| candle_core::Error::Msg(format!("{e}")))?;
    let mut out = Vec::with_capacity(infos.len());
    for info in infos {
        let mut reader = zip.by_name(&info.path).map_err(|e| candle_core::Error::Msg(format!("{e}")))?;
        let fortran = info.layout.is_fortran_contiguous();
        let rank = info.layout.shape().rank();
        if !info.layout.is_contiguous() && !fortran {
            candle_core::bail!("cannot read non-contiguous tensor {}", info.name)
        }
        // As Candle's reader does (it skips `start_offset` bytes).
        let skip = info.layout.start_offset();
        if skip > 0 {
            std::io::copy(&mut reader.by_ref().take(skip as u64), &mut std::io::sink())?;
        }
        // Little-endian on disk, as in memory here.
        let mut bytes = vec![0u8; info.layout.shape().elem_count() * info.dtype.size_in_bytes()];
        reader.read_exact(&mut bytes)?;
        let mut t = Tensor::from_raw_buffer(&bytes, info.dtype, info.layout.dims(), &Device::Cpu)?;
        if rank > 1 && fortran {
            let reversed: Vec<usize> = info.layout.dims().iter().rev().cloned().collect();
            t = t.reshape(reversed)?.permute((0..rank).rev().collect::<Vec<_>>())?;
        }
        out.push((info.name, t));
    }
    Ok(out)
}

/// One loaded HTDemucs network.
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

    /// As `load`, but always the reference graph built from Candle's own operations (for
    /// comparing the two on a real checkpoint).
    #[allow(dead_code)]
    pub fn load_reference(path: &Path, device: &Device) -> Result<Self> {
        Self::load_as(path, device, false)
    }

    fn load_as(path: &Path, device: &Device, fast: bool) -> Result<Self> {
        // Uploading weights encodes GPU work too.
        let _gpu = GPU.lock();
        let raw: HashMap<String, Tensor> = match path.extension().and_then(|e| e.to_str()) {
            Some("safetensors") => candle_core::safetensors::load(path, device)?,
            _ => {
                let mut m = HashMap::new();
                for (name, t) in read_th(path)? {
                    m.insert(name, t.to_device(device)?);
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

/// Profiling of the Metal path (see `metal_fast::profile_begin`); no effect elsewhere.
#[cfg(test)]
pub(crate) fn profile_begin(ops: bool) {
    #[cfg(target_os = "macos")]
    metal_fast::profile_begin(ops);
}

#[cfg(test)]
pub(crate) fn profile_take() -> Vec<(String, f64, usize)> {
    #[cfg(target_os = "macos")]
    return metal_fast::profile_take();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// Time of one empty profiling mark (a GPU round trip with nothing to do).
#[cfg(test)]
pub(crate) fn profile_mark_cost(device: &Device) -> f64 {
    #[cfg(target_os = "macos")]
    if let Ok(t) = Tensor::zeros(16, DType::F32, device) {
        metal_fast::profile_begin(false);
        for _ in 0..200 {
            metal_fast::mark(&t, || "empty".into());
        }
        let rows = metal_fast::profile_take();
        return rows.first().map(|r| r.1 / r.2 as f64).unwrap_or(0.0);
    }
    let _ = device;
    0.0
}

/// Free the GPU memory kept for intermediates between segments (call when a job ends).
pub fn release_scratch() {
    #[cfg(target_os = "macos")]
    metal_fast::release_buffers();
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
    if opts.source.is_some_and(|s| s >= 4) {
        candle_core::bail!("no such source")
    }
    let planes = if opts.source.is_some() { 2 } else { 8 };
    let mut out = vec![0f32; planes * len];
    let sink = Sink { out: out.as_mut_ptr(), ready: &|_, _| {}, next: None };
    separate_with(&[Part { graph, source: opts.source }], device, audio, len, opts.overlap, &sink, progress)?;
    Ok(out)
}

/// One network and what is taken from it.
pub struct Part<'a> {
    pub graph: &'a Graph,
    /// One source (0 drums, 1 bass, 2 other, 3 vocals), or all four.
    pub source: Option<usize>,
}

impl Part<'_> {
    fn sources(&self) -> Vec<usize> {
        self.source.map(|s| vec![s]).unwrap_or_else(|| (0..4).collect())
    }
}

/// Where a separation's results go and in which order its segments are made.
pub struct Sink<'a> {
    /// Output: one plane of `len` frames for each channel of each source taken, in the order
    /// of the parts (planar [source][channel][frame]), zero at the start. Nothing else may
    /// write it while the separation runs, and a frame's value is final only once `ready`
    /// has reported it.
    pub out: *mut f32,
    /// Frames [a, b) are final. Called on a worker thread, once per frame, in no fixed order.
    pub ready: &'a (dyn Fn(usize, usize) + Sync),
    /// The next segment to make, given which are taken already; without it (or when it
    /// answers None or a taken one) segments are made in order.
    pub next: Option<&'a (dyn Fn(&[bool]) -> Option<usize> + Sync)>,
}

/// How a track is cut: the frames each segment's chunk covers, the triangular weight and the
/// sum of weights at every frame.
pub(crate) struct Plan {
    pub spans: Vec<(usize, usize)>,
    weight: Vec<f32>,
    sums: Vec<f32>,
}

impl Plan {
    pub fn new(len: usize, overlap: f64) -> Self {
        let (offsets, weight) = plan(len, overlap);
        let spans: Vec<(usize, usize)> = offsets.iter().map(|&o| (o, o + (len - o).min(TRAINING_FRAMES))).collect();
        // Summed in segment order, as the whole-track division always was.
        let mut sums = vec![0f32; len];
        for &(a, b) in &spans {
            for (s, w) in sums[a..b].iter_mut().zip(&weight) {
                *s += w;
            }
        }
        Self { spans, weight, sums }
    }
}

/// The first segment `next` names that is not taken, else the first not taken.
fn pick(next: Option<&(dyn Fn(&[bool]) -> Option<usize> + Sync)>, taken: &[bool]) -> Option<usize> {
    next.and_then(|f| f(taken)).filter(|&k| k < taken.len() && !taken[k]).or_else(|| taken.iter().position(|t| !t))
}

#[derive(Clone, Copy)]
struct OutPtr(*mut f32);
// SAFETY: the pointer is only used while the separation that owns the output runs, and each
// plane is written by one thread at a time (see `Assembly`).
unsafe impl Send for OutPtr {}
unsafe impl Sync for OutPtr {}

/// Overlap-add of segment results in any order. Each output frame is the weighted sum of the
/// (one or two) segments that cover it divided by the summed weights; it is divided and
/// reported ready as soon as every segment covering it is in. With at most two segments per
/// frame (overlap up to 50 %) the order of the two additions cannot change the sum, so the
/// result is the same bit for bit whatever the order.
struct Assembly<'a> {
    plan: &'a Plan,
    out: OutPtr,
    planes: usize,
    len: usize,
    /// Parts still to come, per segment.
    left: Vec<usize>,
    done: Vec<bool>,
    ready: &'a (dyn Fn(usize, usize) + Sync),
}

impl<'a> Assembly<'a> {
    fn new(plan: &'a Plan, out: *mut f32, planes: usize, len: usize, parts: usize, ready: &'a (dyn Fn(usize, usize) + Sync)) -> Self {
        Self { plan, out: OutPtr(out), planes, len, left: vec![parts; plan.spans.len()], done: vec![false; plan.spans.len()], ready }
    }

    /// Output plane `p`.
    ///
    /// SAFETY: the caller must be the only one using plane `p` until the returned slice is
    /// dropped (planes are handed to one thread each, and `part_done` runs after they join).
    #[allow(clippy::mut_from_ref)]
    unsafe fn plane(&self, p: usize) -> &mut [f32] {
        assert!(p < self.planes);
        std::slice::from_raw_parts_mut(self.out.0.add(p * self.len), self.len)
    }

    /// One part of segment `k` has been added; when it was the last, finish what became final.
    fn part_done(&mut self, k: usize) {
        self.left[k] -= 1;
        if self.left[k] > 0 {
            return;
        }
        self.done[k] = true;
        let (a, b) = self.plan.spans[k];
        // The boundaries of the overlapping segments cut the chunk into pieces that each have
        // one set of covering segments.
        let mut cuts = vec![a, b];
        for &(s, e) in &self.plan.spans {
            for c in [s, e] {
                if c > a && c < b {
                    cuts.push(c);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (x, y) = (w[0], w[1]);
            if self.plan.spans.iter().zip(&self.done).all(|(&(s, e), &d)| d || e <= x || s >= y) {
                for p in 0..self.planes {
                    // SAFETY: no plane is borrowed elsewhere while `part_done` runs.
                    let plane = unsafe { self.plane(p) };
                    for (v, s) in plane[x..y].iter_mut().zip(&self.plan.sums[x..y]) {
                        *v /= s.max(1e-9);
                    }
                }
                (self.ready)(x, y);
            }
        }
    }
}

/// Run one or more networks over a track, each segment through all of them before the next
/// (so with several specialists a stretch of the track is complete after one segment), in
/// the order `sink.next` asks for, with results written to `sink.out` as they finish.
pub fn separate_with(parts: &[Part], device: &Device, audio: &[f32], len: usize, overlap: f64, sink: &Sink, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<()> {
    if len == 0 || audio.len() != 2 * len || parts.is_empty() {
        candle_core::bail!("empty or malformed input")
    }
    if parts.iter().any(|p| p.source.is_some_and(|s| s >= 4)) {
        candle_core::bail!("no such source")
    }
    // Three segments over one frame would make the order of additions matter.
    let in_order;
    let sink = if overlap > 0.5 {
        in_order = Sink { out: sink.out, ready: sink.ready, next: None };
        &in_order
    } else {
        sink
    };
    let plan = Plan::new(len, overlap);
    #[cfg(target_os = "macos")]
    {
        let fast: Option<Vec<(&metal_fast::Fast, Option<usize>)>> = parts.iter().map(|p| p.graph.fast.as_ref().map(|f| (f, p.source))).collect();
        if let Some(fast) = fast {
            return separate_fast(&fast, device, audio, len, &plan, sink, progress);
        }
    }
    let segment = TRAINING_FRAMES;
    let planes: usize = parts.iter().map(|p| 2 * p.sources().len()).sum();
    let mut asm = Assembly::new(&plan, sink.out, planes, len, parts.len(), sink.ready);
    let n_seg = plan.spans.len();
    let mut taken = vec![false; n_seg];
    let mut count = 0;
    while let Some(k) = pick(sink.next, &taken) {
        taken[k] = true;
        let (offset, end) = plan.spans[k];
        let chunk_len = end - offset;
        let crop = (segment - chunk_len) / 2;
        let x = Tensor::from_vec(segment_at(audio, len, offset), (1, 2, segment), device)?;
        let mut base = 0;
        for part in parts {
            let y = {
                let _gpu = GPU.lock();
                part.graph.forward(&x)?.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?
            };
            let sources = part.sources();
            for (n, &s) in sources.iter().enumerate() {
                for ch in 0..2 {
                    // SAFETY: this thread is the only user of the output.
                    let dst = &mut unsafe { asm.plane(base + n * 2 + ch) }[offset..][..chunk_len];
                    let src = &y[(s * 2 + ch) * segment + crop..][..chunk_len];
                    for i in 0..chunk_len {
                        dst[i] += plan.weight[i] * src[i];
                    }
                }
            }
            base += 2 * sources.len();
            asm.part_done(k);
        }
        count += 1;
        if !progress(count, n_seg) {
            candle_core::bail!("stopped")
        }
    }
    Ok(())
}

/// Mean and (unbiased) standard deviation, accumulated in f64.
fn mean_std(x: &[f32]) -> (f32, f32) {
    let n = x.len() as f64;
    let mean = x.iter().map(|v| *v as f64).sum::<f64>() / n;
    let var = x.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean as f32, var.sqrt() as f32)
}

/// One segment ready for the GPU (its normalised spectrogram and waveform are in an input
/// slot) with the statistics they were normalised with.
#[cfg(target_os = "macos")]
struct Prepared {
    k: usize,
    slot: usize,
    /// (mean, std) of the spectrogram and of the waveform.
    stats: [(f32, f32); 2],
}

/// One network's two outputs for one segment, still normalised and still in GPU memory
/// (finished: the GPU thread waited for them).
#[cfg(target_os = "macos")]
struct Raw {
    k: usize,
    part: usize,
    spec: Tensor,
    wave: Tensor,
    stats: [(f32, f32); 2],
}

/// The Metal path: three stages run side by side so the GPU never waits for the CPU. A
/// thread prepares segments (normalisation and the forward transform) straight into GPU-
/// visible input buffers, this thread runs the networks, and a third does the inverse
/// transform and the overlap-add reading the results where the GPU left them.
#[cfg(target_os = "macos")]
fn separate_fast(parts: &[(&metal_fast::Fast, Option<usize>)], device: &Device, audio: &[f32], len: usize, plan: &Plan, sink: &Sink, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<()> {
    let segment = TRAINING_FRAMES;
    let freq = NFFT / 2;
    let frames = Stft::frames(segment);
    let plane = freq * frames;
    let n_seg = plan.spans.len();
    let srcs: Vec<usize> = parts.iter().map(|p| if p.1.is_some() { 1 } else { 4 }).collect();
    let bases: Vec<usize> = srcs.iter().scan(0, |acc, n| {
        let b = *acc;
        *acc += 2 * n;
        Some(b)
    }).collect();
    let planes: usize = srcs.iter().map(|n| 2 * n).sum();
    let stft = Stft::new();
    let envelope = stft.envelope(frames);
    // Three input slots: one being filled, one waiting, one on the GPU.
    const SLOTS: usize = 3;
    let slots = metal_fast::Slots::new(device, SLOTS, 4 * plane, 2 * segment)?;
    let mut asm = Assembly::new(plan, sink.out, planes, len, parts.len(), sink.ready);
    let (stft, envelope, slots, bases) = (&stft, &envelope, &slots, &bases);
    let next = sink.next;

    std::thread::scope(|scope| -> Result<()> {
        let (tx_free, rx_free) = std::sync::mpsc::sync_channel::<usize>(SLOTS);
        for i in 0..SLOTS {
            let _ = tx_free.send(i);
        }
        // One prepared segment waiting is enough to keep the GPU busy, and a short queue
        // lets a new playhead position take effect sooner.
        let (tx_in, rx_in) = std::sync::mpsc::sync_channel::<Prepared>(1);
        // One segment's results waiting (each holds its network's whole output buffer).
        let (tx_out, rx_out) = std::sync::mpsc::sync_channel::<Raw>(parts.len());
        scope.spawn(move || {
            let mut taken = vec![false; n_seg];
            while let Some(k) = pick(next, &taken) {
                taken[k] = true;
                let Ok(slot) = rx_free.recv() else { return };
                let offset = plan.spans[k].0;
                let input = segment_at(audio, len, offset);
                // SAFETY: the slot came from the free list, so neither the GPU nor another
                // thread uses it until it is sent on.
                let (spec, wave) = unsafe { slots.fill(slot) };
                std::thread::scope(|s| {
                    for (ch, planes) in spec.chunks_mut(2 * plane).enumerate() {
                        let x = &input[ch * segment..(ch + 1) * segment];
                        s.spawn(move || {
                            let (re, im) = planes.split_at_mut(plane);
                            stft.forward(x, re, im);
                        });
                    }
                });
                let stats = [mean_std(spec), mean_std(&input)];
                wave.copy_from_slice(&input);
                for (v, (mean, std)) in [(&mut *spec, stats[0]), (&mut *wave, stats[1])] {
                    let scale = 1.0 / (std + 1e-5);
                    v.iter_mut().for_each(|x| *x = (*x - mean) * scale);
                }
                if tx_in.send(Prepared { k, slot, stats }).is_err() {
                    return;
                }
            }
        });
        let asm = &mut asm;
        let post = scope.spawn(move || -> Result<()> {
            for raw in rx_out {
                let (offset, end) = plan.spans[raw.k];
                let chunk_len = end - offset;
                let crop = (segment - chunk_len) / 2;
                let base = bases[raw.part];
                let n = srcs_of(&raw);
                metal_fast::with_view(&raw.spec, |spec| {
                    metal_fast::with_view(&raw.wave, |time_all| {
                        let asm = &*asm;
                        std::thread::scope(|s| {
                            for q in 0..2 * n {
                                // SAFETY: each plane goes to one thread; they are joined
                                // before `part_done`.
                                let dst = unsafe { asm.plane(base + q) };
                                let (stats, weight) = (raw.stats, &plan.weight);
                                s.spawn(move || {
                                    let (re, im) = spec[q * 2 * plane..(q + 1) * 2 * plane].split_at(plane);
                                    let mut wave = vec![0f32; segment];
                                    stft.inverse(re, im, frames, stats[0].1, stats[0].0, envelope, &mut wave);
                                    let time = &time_all[q * segment..(q + 1) * segment];
                                    let (mean, std) = stats[1];
                                    let dst = &mut dst[offset..offset + chunk_len];
                                    for i in 0..chunk_len {
                                        dst[i] += weight[i] * (wave[crop + i] + time[crop + i] * std + mean);
                                    }
                                });
                            }
                        });
                    })
                })??;
                asm.part_done(raw.k);
            }
            Ok(())
        });
        let mut run = || -> Result<()> {
            for (count, p) in rx_in.iter().enumerate() {
                let outs = {
                    let _gpu = GPU.lock();
                    let (x, xt) = slots.tensors(p.slot, (4, freq, frames), (2, 1, segment))?;
                    let mut outs = Vec::with_capacity(parts.len());
                    for (i, (fast, source)) in parts.iter().enumerate() {
                        let (mut y, mut yt) = fast.forward(&x, &xt)?;
                        if let Some(s) = source {
                            y = y.narrow(0, s * 4, 4)?;
                            yt = yt.narrow(0, s * 2, 2)?;
                        }
                        outs.push((i, y, yt));
                    }
                    device.synchronize()?;
                    metal_fast::mark(&xt, || "wait for the GPU".into());
                    outs
                };
                // The GPU is done with the slot.
                let _ = tx_free.send(p.slot);
                for (part, spec, wave) in outs {
                    if tx_out.send(Raw { k: p.k, part, spec, wave, stats: p.stats }).is_err() {
                        candle_core::bail!("stem post-processing stopped")
                    }
                }
                if !progress(count + 1, n_seg) {
                    candle_core::bail!("stopped")
                }
            }
            Ok(())
        };
        let r = run();
        // Closing the channels lets the other two stages finish.
        drop(rx_in);
        drop(tx_out);
        drop(tx_free);
        match post.join() {
            Ok(Ok(())) => r,
            Ok(Err(e)) => r.and(Err(e)),
            Err(_) => candle_core::bail!("stem post-processing failed"),
        }
    })
}

/// Sources in one raw result (one or all four).
#[cfg(target_os = "macos")]
fn srcs_of(raw: &Raw) -> usize {
    raw.wave.dim(0).unwrap_or(2) / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic values in [-1, 1).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    /// A network with HTDemucs's exact tensor names and shapes and random values, so the two
    /// implementations can be compared without a downloaded checkpoint.
    fn random_weights(device: &Device) -> HashMap<String, Tensor> {
        let mut rng = Lcg(7);
        let mut w = HashMap::new();
        // kind 0: weight scaled by fan-in; 1: norm weight near 1; 2: bias; 3: layer scale.
        let mut put = |name: String, dims: &[usize], kind: u8| {
            let n: usize = dims.iter().product();
            let fan = (n / dims[0]).max(1) as f32;
            let (mean, amp) = match kind {
                0 => (0.0, (3.0 / fan).sqrt()),
                1 => (1.0, 0.2),
                2 => (0.0, 0.1),
                _ => (0.3, 0.1),
            };
            let v: Vec<f32> = (0..n).map(|_| mean + amp * rng.next()).collect();
            w.insert(name, Tensor::from_vec(v, dims, device).unwrap());
        };
        let mut dconv = |put: &mut dyn FnMut(String, &[usize], u8), name: String, c: usize| {
            for l in 0..2 {
                let (p, h) = (format!("{name}.layers.{l}"), c / 8);
                put(format!("{p}.0.weight"), &[h, c, 3], 0);
                put(format!("{p}.0.bias"), &[h], 2);
                put(format!("{p}.1.weight"), &[h], 1);
                put(format!("{p}.1.bias"), &[h], 2);
                put(format!("{p}.3.weight"), &[2 * c, h, 1], 0);
                put(format!("{p}.3.bias"), &[2 * c], 2);
                put(format!("{p}.4.weight"), &[2 * c], 1);
                put(format!("{p}.4.bias"), &[2 * c], 2);
                put(format!("{p}.6.scale"), &[c], 3);
            }
        };
        for (prefix, first, freq) in [("encoder", 4, true), ("tencoder", 2, false)] {
            for i in 0..4 {
                let c = 48 << i;
                let cin = if i == 0 { first } else { c / 2 };
                let name = format!("{prefix}.{i}");
                put(format!("{name}.conv.weight"), &[c, cin, 8, 1][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.conv.bias"), &[c], 2);
                dconv(&mut put, format!("{name}.dconv"), c);
                put(format!("{name}.rewrite.weight"), &[2 * c, c, 1, 1][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.rewrite.bias"), &[2 * c], 2);
            }
        }
        for (prefix, last, freq) in [("decoder", 16, true), ("tdecoder", 8, false)] {
            for j in 0..4 {
                let c = 384 >> j;
                let cout = if j == 3 { last } else { c / 2 };
                let name = format!("{prefix}.{j}");
                put(format!("{name}.conv_tr.weight"), &[c, cout, 8, 1][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.conv_tr.bias"), &[cout], 2);
                dconv(&mut put, format!("{name}.dconv"), c);
                put(format!("{name}.rewrite.weight"), &[2 * c, c, 3, 3][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.rewrite.bias"), &[2 * c], 2);
            }
        }
        put("freq_emb.embedding.weight".into(), &[512, 48], 2);
        for t in ["", "_t"] {
            put(format!("channel_upsampler{t}.weight"), &[512, 384, 1], 0);
            put(format!("channel_upsampler{t}.bias"), &[512], 2);
            put(format!("channel_downsampler{t}.weight"), &[384, 512, 1], 0);
            put(format!("channel_downsampler{t}.bias"), &[384], 2);
            put(format!("crosstransformer.norm_in{t}.weight"), &[512], 1);
            put(format!("crosstransformer.norm_in{t}.bias"), &[512], 2);
            for i in 0..5 {
                let name = format!("crosstransformer.layers{t}.{i}");
                let cross = i % 2 == 1;
                let attn = format!("{name}.{}", if cross { "cross_attn" } else { "self_attn" });
                put(format!("{attn}.in_proj_weight"), &[1536, 512], 0);
                put(format!("{attn}.in_proj_bias"), &[1536], 2);
                put(format!("{attn}.out_proj.weight"), &[512, 512], 0);
                put(format!("{attn}.out_proj.bias"), &[512], 2);
                put(format!("{name}.linear1.weight"), &[2048, 512], 0);
                put(format!("{name}.linear1.bias"), &[2048], 2);
                put(format!("{name}.linear2.weight"), &[512, 2048], 0);
                put(format!("{name}.linear2.bias"), &[512], 2);
                put(format!("{name}.gamma_1.scale"), &[512], 3);
                put(format!("{name}.gamma_2.scale"), &[512], 3);
                for n in ["norm1", "norm2", "norm_out"].into_iter().chain(cross.then_some("norm3")) {
                    put(format!("{name}.{n}.weight"), &[512], 1);
                    put(format!("{name}.{n}.bias"), &[512], 2);
                }
            }
        }
        w
    }

    /// Planar stereo test signal: tones, a sweep and noise, different in the two channels.
    fn signal(len: usize) -> Vec<f32> {
        let mut rng = Lcg(3);
        (0..2 * len)
            .map(|n| {
                let (ch, i) = (n / len, (n % len) as f32);
                let t = i / SAMPLE_RATE as f32;
                0.3 * (2.0 * std::f32::consts::PI * (110.0 + 40.0 * ch as f32) * t).sin() + 0.2 * (2.0 * std::f32::consts::PI * (300.0 + 900.0 * t) * t).sin() + 0.1 * rng.next()
            })
            .collect()
    }

    fn snr_db(reference: &[f32], other: &[f32]) -> f64 {
        let (mut s, mut e) = (0f64, 0f64);
        for (a, b) in reference.iter().zip(other) {
            s += (*a as f64).powi(2);
            e += ((*a - *b) as f64).powi(2);
        }
        10.0 * (s / e.max(1e-30)).log10()
    }

    #[test]
    fn transform_round_trip() {
        let len = 20 * HOP + 300;
        let x: Vec<f32> = signal(len)[..len].to_vec();
        let stft = Stft::new();
        let frames = Stft::frames(len);
        let (mut re, mut im) = (vec![0f32; NFFT / 2 * frames], vec![0f32; NFFT / 2 * frames]);
        stft.forward(&x, &mut re, &mut im);
        let mut y = vec![0f32; len];
        stft.inverse(&re, &im, frames, 1.0, 0.0, &stft.envelope(frames), &mut y);
        // As upstream, the frames that hang over the ends are dropped, so only the interior
        // comes back (and without the Nyquist bin).
        let snr = snr_db(&x[NFFT..len - NFFT], &y[NFFT..len - NFFT]);
        assert!(snr > 40.0, "round trip {snr:.1} dB");
    }

    #[test]
    fn segments_cover_the_track() {
        let len = 3 * TRAINING_FRAMES + 1000;
        for overlap in [0.25, 0.1] {
            let (offsets, weight) = plan(len, overlap);
            assert_eq!(weight.len(), TRAINING_FRAMES);
            assert_eq!(offsets[0], 0);
            assert!(offsets.windows(2).all(|w| w[1] - w[0] <= TRAINING_FRAMES));
            assert!(offsets.last().unwrap() + TRAINING_FRAMES >= len);
        }
        // A short last chunk is padded from the audio around it, centred.
        let audio: Vec<f32> = (0..2 * len).map(|i| (i % len) as f32).collect();
        let offset = 3 * TRAINING_FRAMES - 500;
        let seg = segment_at(&audio, len, offset);
        let crop = (TRAINING_FRAMES - (len - offset)) / 2;
        assert_eq!(seg[crop], offset as f32);
        assert_eq!(seg[TRAINING_FRAMES + crop], offset as f32);
        assert_eq!(seg[0], (offset - crop) as f32);
    }

    /// The fused Metal kernels against the reference graph built from Candle's operations,
    /// on the same (random) weights: outputs must agree far below audibility. Skipped where
    /// there is no Metal device.
    #[test]
    fn metal_kernels_match_reference_graph() {
        let Ok(device) = Device::new_metal(0) else { return };
        let weights = random_weights(&device);
        let reference = Graph::from_weights(weights.clone(), false).unwrap();
        let fast = Graph::from_weights(weights, true).unwrap();
        // One full segment and a short, padded one.
        let len = TRAINING_FRAMES + SAMPLE_RATE as usize;
        let audio = signal(len);
        let opts = Options::default();
        let a = separate(&reference, &device, &audio, len, &opts, &mut |_, _| true).unwrap();
        let b = separate(&fast, &device, &audio, len, &opts, &mut |_, _| true).unwrap();
        assert_eq!(a.len(), 8 * len);
        for s in 0..4 {
            let snr = snr_db(&a[s * 2 * len..(s + 1) * 2 * len], &b[s * 2 * len..(s + 1) * 2 * len]);
            eprintln!("{}: fused kernels vs reference {snr:.1} dB", SOURCES[s]);
            assert!(snr > 60.0, "{}: fused kernels differ from the reference ({snr:.1} dB)", SOURCES[s]);
        }
        // One source alone is the same signal as that source of the full result.
        let one = separate(&fast, &device, &audio, len, &Options { source: Some(3), ..opts }, &mut |_, _| true).unwrap();
        assert!(snr_db(&b[6 * len..], &one) > 100.0);
        // Returning false from the progress callback stops the job.
        let mut calls = 0;
        let stopped = separate(&fast, &device, &audio, len, &opts, &mut |_, _| {
            calls += 1;
            false
        });
        assert!(stopped.is_err_and(|e| e.to_string().contains("stopped")) && calls == 1);
        release_scratch();
    }
}

#[cfg(test)]
mod load_bench {
    use super::*;

    /// Where loading a checkpoint goes: reading the .th, converting, uploading, preparing the
    /// fused kernels' weights. PREPDECK_STEM_TH = path to a .th file.
    #[test]
    #[ignore]
    fn load_breakdown() {
        let Ok(path) = std::env::var("PREPDECK_STEM_TH") else { return };
        let device = Device::new_metal(0).unwrap();
        for pass in 0..2 {
            let t = std::time::Instant::now();
            let t_index = 0.0;
            let cpu: HashMap<String, Tensor> = read_th(Path::new(&path)).unwrap().into_iter().collect();
            let names: Vec<String> = cpu.keys().cloned().collect();
            let t_read = t.elapsed().as_secs_f64();
            if pass == 0 {
                // Same tensors as Candle's own reader, bit for bit.
                let pth = candle_core::pickle::PthTensors::new(&path, Some("state")).unwrap();
                for n in &names {
                    let a = pth.get(n).unwrap().unwrap().to_dtype(DType::F32).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
                    let b = cpu[n].to_dtype(DType::F32).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
                    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()) && a.len() == b.len(), "{n} differs");
                }
                println!("LB all {} tensors identical to Candle's reader", names.len());
            }
            let dtypes: std::collections::BTreeSet<String> = cpu.values().map(|v| format!("{:?}", v.dtype())).collect();
            let up: HashMap<String, Tensor> = cpu.iter().map(|(k, v)| (k.clone(), v.to_device(&device).unwrap().to_dtype(DType::F32).unwrap())).collect();
            device.synchronize().unwrap();
            let t_up = t.elapsed().as_secs_f64();
            let g = Graph::from_weights(up, true).unwrap();
            device.synchronize().unwrap();
            let t_prep = t.elapsed().as_secs_f64();
            drop(g);
            println!("LB pass {pass}: index {:.3} s, read {:.3} s, upload+convert {:.3} s, prepare {:.3} s, total {:.3} s ({} tensors, {dtypes:?})", t_index, t_read - t_index, t_up - t_read, t_prep - t_up, t_prep, names.len());
        }
    }
}
