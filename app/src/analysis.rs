//! Track analysis: waveform bands, tempo + beat grid, key and loudness.
//! All algorithms are original DSP (see MECHANICS.md for the parameters).

use crate::audio::{decode_file, Decoded};
use crate::model::{BeatGrid, Waveform, WAVE_FPS};
use anyhow::Result;
use rustfft::{num_complex::Complex, FftPlanner};

#[derive(Clone, Debug)]
pub struct AnalysisSettings {
    pub bpm_min: f64,
    pub bpm_max: f64,
    pub do_grid: bool,
    pub do_key: bool,
    /// Loudness the auto gain aims for (LUFS).
    pub target_lufs: f64,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        AnalysisSettings { bpm_min: 70.0, bpm_max: 180.0, do_grid: true, do_key: true, target_lufs: -8.0 }
    }
}

pub const BPM_RANGES: [(f64, f64); 10] = [
    (70.0, 180.0),
    (48.0, 95.0),
    (58.0, 115.0),
    (68.0, 135.0),
    (78.0, 155.0),
    (88.0, 175.0),
    (98.0, 195.0),
    (108.0, 215.0),
    (118.0, 235.0),
    (128.0, 255.0),
];

pub struct AnalysisResult {
    pub duration: f64,
    pub sample_rate: u32,
    pub bpm: f64,
    pub grid: Option<BeatGrid>,
    pub key: i32,
    pub wave: Waveform,
    pub gain_db: f32,
}

pub fn analyze_file(path: &str, s: &AnalysisSettings) -> Result<AnalysisResult> {
    let dec = decode_file(path)?;
    Ok(analyze_decoded(&dec, s))
}

pub fn analyze_decoded(dec: &Decoded, s: &AnalysisSettings) -> AnalysisResult {
    let mono = dec.mono();
    let sr = dec.sr as f64;
    let wave = waveform(&mono, sr);
    let (bpm, grid) = if s.do_grid { tempo_and_grid(&mono, sr, s.bpm_min, s.bpm_max) } else { (0.0, None) };
    let key = if s.do_key { detect_key(&mono, sr) } else { -1 };
    let lufs = loudness(dec);
    let gain_db = if lufs.is_finite() { (s.target_lufs - lufs).clamp(-24.0, 24.0) as f32 } else { 0.0 };
    AnalysisResult { duration: dec.duration(), sample_rate: dec.sr, bpm, grid, key, wave, gain_db }
}

// ------------------------------------------------------------------------------------------
// Filters
// ------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(kind: &str, f: f64, q: f64, sr: f64, gain_db: f64) -> Biquad {
        let w = 2.0 * std::f64::consts::PI * f / sr;
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            "lp" => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            "hp" => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            "hs" => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - sq),
                    (a + 1.0) - (a - 1.0) * cs + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - sq,
                )
            }
            _ => (1.0, 0.0, 0.0, 1.0, 0.0, 0.0),
        };
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }
    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

// ------------------------------------------------------------------------------------------
// Waveform: 150 frames per second, peak of full / low / mid / high bands.
// ------------------------------------------------------------------------------------------

pub const LOW_CUT: f64 = 200.0;
pub const HIGH_CUT: f64 = 2500.0;

pub fn waveform(mono: &[f32], sr: f64) -> Waveform {
    let hop = sr / WAVE_FPS;
    let n = (mono.len() as f64 / hop).ceil() as usize;
    let mut lp1 = Biquad::new("lp", LOW_CUT, 0.707, sr, 0.0);
    let mut lp2 = Biquad::new("lp", LOW_CUT, 0.707, sr, 0.0);
    let mut hp1 = Biquad::new("hp", HIGH_CUT, 0.707, sr, 0.0);
    let mut hp2 = Biquad::new("hp", HIGH_CUT, 0.707, sr, 0.0);
    let mut mh = Biquad::new("hp", LOW_CUT, 0.707, sr, 0.0);
    let mut ml = Biquad::new("lp", HIGH_CUT, 0.707, sr, 0.0);
    let mut raw = vec![[0f32; 4]; n];
    for (i, &x) in mono.iter().enumerate() {
        let x = x as f64;
        let lo = lp2.run(lp1.run(x));
        let hi = hp2.run(hp1.run(x));
        let mi = ml.run(mh.run(x));
        let fi = ((i as f64) / hop) as usize;
        if fi >= n {
            break;
        }
        let r = &mut raw[fi];
        r[0] = r[0].max(x.abs() as f32);
        r[1] = r[1].max(lo.abs() as f32);
        r[2] = r[2].max(mi.abs() as f32);
        r[3] = r[3].max(hi.abs() as f32);
    }
    // Band display gains (estimated so that typical dance music fills the height).
    let gains = [1.0f32, 1.0, 1.5, 2.6];
    let frames = raw
        .iter()
        .map(|r| {
            let mut o = [0u8; 4];
            for k in 0..4 {
                let v = (r[k] * gains[k]).min(1.0).powf(0.85);
                o[k] = (v * 255.0).round() as u8;
            }
            o
        })
        .collect();
    Waveform { frames }
}

// ------------------------------------------------------------------------------------------
// Tempo and beat grid
// ------------------------------------------------------------------------------------------

const ONSET_FPS: f64 = 200.0;

struct Onsets {
    all: Vec<f32>,
    low: Vec<f32>,
    fps: f64,
    /// Time of frame 0 (seconds)
    t0: f64,
}

fn onsets(mono: &[f32], sr: f64) -> Onsets {
    let nfft = 1024usize;
    let hop = (sr / ONSET_FPS).round() as usize;
    let fps = sr / hop as f64;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    let nb = nfft / 2;
    let max_bin = ((8000.0 / sr) * nfft as f64) as usize;
    let low_bin = ((160.0 / sr) * nfft as f64).ceil() as usize;
    let mut prev = vec![0f32; nb];
    let mut all = vec![];
    let mut low = vec![];
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut start = 0usize;
    while start + nfft <= mono.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(mono[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut fa = 0f32;
        let mut fl = 0f32;
        for b in 1..max_bin.min(nb) {
            let m = (1.0 + 100.0 * buf[b].norm()).ln();
            let d = m - prev[b];
            if d > 0.0 {
                fa += d;
                if b <= low_bin {
                    fl += d;
                }
            }
            prev[b] = m;
        }
        all.push(fa);
        low.push(fl);
        start += hop;
    }
    Onsets { all, low, fps, t0: nfft as f64 / 2.0 / sr }
}

fn normalize(v: &mut [f32]) {
    if v.is_empty() {
        return;
    }
    // Subtract a moving average so that only local peaks remain.
    let w = 16usize;
    let mut out = vec![0f32; v.len()];
    let mut acc = 0f64;
    let mut q = std::collections::VecDeque::new();
    for i in 0..v.len() + w {
        if i < v.len() {
            acc += v[i] as f64;
            q.push_back(v[i]);
        }
        if q.len() > 2 * w + 1 {
            acc -= q.pop_front().unwrap() as f64;
        }
        if i >= w {
            let c = i - w;
            let mean = acc / q.len() as f64;
            out[c] = (v[c] as f64 - mean).max(0.0) as f32;
        }
    }
    let mx = out.iter().cloned().fold(0f32, f32::max).max(1e-9);
    for (a, b) in v.iter_mut().zip(out) {
        *a = b / mx;
    }
}

fn interp(v: &[f32], x: f64) -> f32 {
    if x < 0.0 {
        return 0.0;
    }
    let i = x as usize;
    if i + 1 >= v.len() {
        return 0.0;
    }
    let f = (x - i as f64) as f32;
    v[i] * (1.0 - f) + v[i + 1] * f
}

/// Score of a beat comb with period `p` frames: best phase and its score.
fn comb_score(env: &[f32], p: f64) -> (f64, f64) {
    let steps = (p * 2.0).ceil() as usize;
    let mut best = (0.0, -1.0);
    for s in 0..steps {
        let ph = s as f64 * 0.5;
        let mut x = ph;
        let mut sc = 0f64;
        let mut n = 0;
        while x < env.len() as f64 {
            sc += interp(env, x) as f64;
            n += 1;
            x += p;
        }
        let sc = if n > 0 { sc / n as f64 } else { 0.0 };
        if sc > best.1 {
            best = (ph, sc);
        }
    }
    best
}

pub fn tempo_and_grid(mono: &[f32], sr: f64, bpm_min: f64, bpm_max: f64) -> (f64, Option<BeatGrid>) {
    if mono.len() < (sr * 4.0) as usize {
        return (0.0, None);
    }
    let mut o = onsets(mono, sr);
    normalize(&mut o.all);
    normalize(&mut o.low);
    let env: Vec<f32> = o.all.iter().zip(&o.low).map(|(a, l)| a + 0.6 * l).collect();
    let fps = o.fps;

    // 1) Autocorrelation over the allowed range, with harmonics.
    let max_lag = (60.0 * fps / bpm_min * 4.0) as usize + 2;
    let n = env.len();
    let mean = env.iter().map(|v| *v as f64).sum::<f64>() / n as f64;
    let e: Vec<f64> = env.iter().map(|v| *v as f64 - mean).collect();
    let mut ac = vec![0f64; max_lag + 1];
    for lag in 1..=max_lag.min(n - 1) {
        let mut s = 0.0;
        for i in 0..n - lag {
            s += e[i] * e[i + lag];
        }
        ac[lag] = s / (n - lag) as f64;
    }
    let ac_at = |l: f64| -> f64 {
        let i = l as usize;
        if i + 1 >= ac.len() {
            return 0.0;
        }
        let f = l - i as f64;
        ac[i] * (1.0 - f) + ac[i + 1] * f
    };
    let mut cands: Vec<(f64, f64)> = vec![];
    let mut bpm = bpm_min;
    while bpm <= bpm_max {
        let lag = 60.0 * fps / bpm;
        let sc = ac_at(lag) + 0.5 * ac_at(lag * 2.0) + 0.33 * ac_at(lag * 4.0) + 0.25 * ac_at(lag * 0.5);
        // Mild preference for the 110..140 region (estimated), common for dance music.
        let w = (-0.5 * ((bpm / 125.0).log2() / 0.9).powi(2)).exp();
        cands.push((bpm, sc * (0.7 + 0.3 * w)));
        bpm += 0.25;
    }
    cands.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let coarse = cands.first().map(|c| c.0).unwrap_or(120.0);

    // 2) Refine with a phase-aligned comb on a 0.01 BPM grid.
    let mut best = (coarse, 0.0, -1.0);
    let mut b = coarse - 0.5;
    while b <= coarse + 0.5 {
        let p = 60.0 * fps / b;
        let (ph, sc) = comb_score(&env, p);
        if sc > best.2 {
            best = (b, ph, sc);
        }
        b += 0.01;
    }
    let mut bpm = (best.0 * 100.0).round() / 100.0;
    // Prefer a whole BPM when it explains the beats nearly as well.
    let whole = bpm.round();
    if (whole - bpm).abs() > 1e-6 && (whole - bpm).abs() < 0.35 {
        let (_, sc) = comb_score(&env, 60.0 * fps / whole);
        if sc >= best.2 * 0.97 {
            bpm = whole;
        }
    }
    let period = 60.0 * fps / bpm;
    let (mut phase, _) = comb_score(&env, period);
    // Beats sit on the kick: compare low-band attack on this phase and the half-beat offset.
    let low_at = |ph: f64| -> f64 {
        let mut x = ph;
        let mut s = 0.0;
        while x < o.low.len() as f64 {
            s += interp(&o.low, x) as f64;
            x += period;
        }
        s
    };
    if low_at(phase + period / 2.0) > low_at(phase) * 1.15 {
        phase += period / 2.0;
    }

    // 3) Sub-frame refinement against low-band transients in the time domain.
    let period_s = 60.0 / bpm;
    let mut first = o.t0 + phase / fps;
    first = refine_phase(mono, sr, first, period_s);
    // Bring the first beat to the earliest position >= 0.
    first -= (first / period_s).floor() * period_s;

    // 4) Downbeat: which of the 4 beat positions carries the most low-frequency attack.
    let mut sums = [0f64; 4];
    let mut t = first;
    let mut k = 0usize;
    while t < mono.len() as f64 / sr {
        let fi = (t - o.t0) * fps;
        let mut v = 0f64;
        for d in -4..=4 {
            let x = fi + d as f64;
            v = v.max(interp(&o.low, x) as f64 + 0.5 * interp(&o.all, x) as f64);
        }
        // Bar starts often coincide with changes (crashes, new parts): weight 16-beat phrase starts.
        sums[k % 4] += v;
        t += period_s;
        k += 1;
    }
    // Harmony tends to change at the start of a bar: add the chroma change at each beat.
    let times: Vec<f64> = (0..k).map(|i| first + i as f64 * period_s).collect();
    let nov = chroma_novelty(mono, sr, &times);
    let mut hsum = [0f64; 4];
    for (i, v) in nov.iter().enumerate() {
        hsum[i % 4] += v;
    }
    let z = |a: [f64; 4]| -> [f64; 4] {
        let m = a.iter().sum::<f64>() / 4.0;
        let sd = (a.iter().map(|x| (x - m).powi(2)).sum::<f64>() / 4.0).sqrt().max(1e-9);
        [(a[0] - m) / sd, (a[1] - m) / sd, (a[2] - m) / sd, (a[3] - m) / sd]
    };
    let (zl, zh) = (z(sums), z(hsum));
    let score: Vec<f64> = (0..4).map(|j| zl[j] * 0.6 + zh[j]).collect();
    let down = (0..4).max_by(|a, b| score[*a].partial_cmp(&score[*b]).unwrap()).unwrap_or(0);
    // Beat number of the first beat so that beat index `down` is a "1".
    let first_beat_no = ((4 - down) % 4) as u8 + 1;
    (bpm, Some(BeatGrid::constant(first, bpm, first_beat_no)))
}

/// Find the offset that best aligns predicted beats with the steepest rise of the
/// low-passed amplitude envelope (median over the track).
fn refine_phase(mono: &[f32], sr: f64, first: f64, period: f64) -> f64 {
    let mut lp1 = Biquad::new("lp", 150.0, 0.707, sr, 0.0);
    let mut lp2 = Biquad::new("lp", 150.0, 0.707, sr, 0.0);
    let ms = (sr / 1000.0) as usize;
    let mut env = Vec::with_capacity(mono.len() / ms + 1);
    let mut acc = 0f32;
    for (i, &x) in mono.iter().enumerate() {
        let y = lp2.run(lp1.run(x as f64)).abs() as f32;
        acc = acc.max(y);
        if (i + 1) % ms == 0 {
            env.push(acc);
            acc = 0.0;
        }
    }
    let mut offsets = vec![];
    let mut t = first;
    let dur = mono.len() as f64 / sr;
    while t < dur - 0.1 {
        let c = (t * sr / ms as f64) as isize;
        let mut best = (0isize, 0f32);
        for d in -40isize..=40 {
            let i = c + d;
            if i < 2 || i as usize + 1 >= env.len() {
                continue;
            }
            let rise = env[i as usize + 1] - env[i as usize - 2];
            if rise > best.1 {
                best = (d, rise);
            }
        }
        if best.1 > 0.02 {
            offsets.push(best.0);
        }
        t += period;
    }
    if offsets.len() < 8 {
        return first;
    }
    offsets.sort();
    let med = offsets[offsets.len() / 2];
    first + med as f64 * ms as f64 / sr
}

/// Cosine distance between the pitch-class profile of each beat and the previous beat.
fn chroma_novelty(mono: &[f32], sr: f64, times: &[f64]) -> Vec<f64> {
    let nfft = 4096usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut prev: Option<[f64; 12]> = None;
    let mut out = Vec::with_capacity(times.len());
    for &t in times {
        let start = ((t + 0.03) * sr) as usize;
        if start + nfft > mono.len() {
            out.push(0.0);
            continue;
        }
        for i in 0..nfft {
            buf[i] = Complex::new(mono[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut c = [0f64; 12];
        for b in 1..nfft / 2 {
            let f = b as f64 * sr / nfft as f64;
            if !(90.0..=2000.0).contains(&f) {
                continue;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            let pc = (midi.round() as i64).rem_euclid(12) as usize;
            c[pc] += buf[b].norm() as f64;
        }
        let n = c.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-9);
        for x in c.iter_mut() {
            *x /= n;
        }
        let v = match prev {
            Some(p) => 1.0 - (0..12).map(|i| p[i] * c[i]).sum::<f64>(),
            None => 0.0,
        };
        out.push(v);
        prev = Some(c);
    }
    out
}

/// Recompute a grid's BPM by tapping intervals etc. is UI work; this helper fits a constant
/// tempo for "Analyze from current position" style edits.
pub fn shift_grid(g: &BeatGrid, seconds: f64) -> BeatGrid {
    BeatGrid { anchors: g.anchors.iter().map(|a| crate::model::GridAnchor { time: a.time + seconds, ..*a }).collect() }
}

// ------------------------------------------------------------------------------------------
// Key detection: chroma profile correlated with major/minor key templates.
// ------------------------------------------------------------------------------------------

const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

pub fn detect_key(mono: &[f32], sr: f64) -> i32 {
    // Downsample by an integer factor to ~11 kHz with a simple low-pass.
    let factor = ((sr / 11025.0).round() as usize).max(1);
    let mut lp1 = Biquad::new("lp", 4000.0, 0.707, sr, 0.0);
    let mut lp2 = Biquad::new("lp", 4000.0, 0.707, sr, 0.0);
    let mut ds = Vec::with_capacity(mono.len() / factor + 1);
    for (i, &x) in mono.iter().enumerate() {
        let y = lp2.run(lp1.run(x as f64));
        if i % factor == 0 {
            ds.push(y as f32);
        }
    }
    let dsr = sr / factor as f64;
    let nfft = 8192usize;
    let hop = 4096usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    // Bin -> pitch class weights (55 Hz .. 1760 Hz).
    let mut map: Vec<(usize, usize, f32)> = vec![];
    for b in 1..nfft / 2 {
        let f = b as f64 * dsr / nfft as f64;
        if !(55.0..=1760.0).contains(&f) {
            continue;
        }
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        let nearest = midi.round();
        let dev = (midi - nearest).abs();
        if dev > 0.35 {
            continue;
        }
        let pc = (nearest as i64).rem_euclid(12) as usize;
        let w = (1.0 - dev / 0.35) as f32;
        // Emphasise bass/mid register where harmony lives.
        let reg = if f < 220.0 { 1.4 } else if f < 900.0 { 1.0 } else { 0.6 };
        map.push((b, pc, w * reg));
    }
    let mut chroma = [0f64; 12];
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut start = 0;
    while start + nfft <= ds.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(ds[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut frame = [0f64; 12];
        let mut tot = 0.0;
        for &(b, pc, w) in &map {
            let m = buf[b].norm() as f64;
            frame[pc] += (m * w as f64).sqrt();
            tot += m;
        }
        if tot > 1e-3 {
            let s: f64 = frame.iter().sum();
            for k in 0..12 {
                chroma[k] += frame[k] / s.max(1e-9);
            }
        }
        start += hop;
    }
    if chroma.iter().sum::<f64>() <= 0.0 {
        return -1;
    }
    let corr = |prof: &[f64; 12], root: usize| -> f64 {
        let xm = chroma.iter().sum::<f64>() / 12.0;
        let ym = prof.iter().sum::<f64>() / 12.0;
        let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            let x = chroma[(i + root) % 12] - xm;
            let y = prof[i] - ym;
            num += x * y;
            dx += x * x;
            dy += y * y;
        }
        num / (dx * dy).sqrt().max(1e-12)
    };
    let mut best = (-2.0, 0usize, false);
    for root in 0..12 {
        let a = corr(&MAJOR, root);
        if a > best.0 {
            best = (a, root, false);
        }
        let b = corr(&MINOR, root);
        if b > best.0 {
            best = (b, root, true);
        }
    }
    crate::model::key_from_pitch(best.1 as i32, best.2)
}

// ------------------------------------------------------------------------------------------
// Loudness (ITU-R BS.1770 style: K-weighting, 400 ms blocks, absolute + relative gating)
// ------------------------------------------------------------------------------------------

pub fn loudness(dec: &Decoded) -> f64 {
    let sr = dec.sr as f64;
    let mut ch: Vec<(Biquad, Biquad)> = (0..2)
        .map(|_| (Biquad::new("hs", 1681.97, 0.7072, sr, 4.0), Biquad::new("hp", 38.13, 0.5003, sr, 0.0)))
        .collect();
    let block = (sr * 0.4) as usize;
    let step = (sr * 0.1) as usize;
    let frames = dec.frames();
    let mut sq = vec![0f64; frames];
    for i in 0..frames {
        let mut s = 0.0;
        for c in 0..2 {
            let x = dec.samples[i * 2 + c] as f64;
            let f = &mut ch[c];
            let y0 = f.0.run(x);
            let y = f.1.run(y0);
            s += y * y;
        }
        sq[i] = s;
    }
    // Prefix sums for block energies.
    let mut pre = vec![0f64; frames + 1];
    for i in 0..frames {
        pre[i + 1] = pre[i] + sq[i];
    }
    let mut blocks = vec![];
    let mut s = 0;
    while s + block <= frames {
        let z = (pre[s + block] - pre[s]) / block as f64;
        blocks.push(z);
        s += step;
    }
    let lk = |z: f64| -0.691 + 10.0 * z.max(1e-12).log10();
    let abs: Vec<f64> = blocks.into_iter().filter(|z| lk(*z) > -70.0).collect();
    if abs.is_empty() {
        return f64::NEG_INFINITY;
    }
    let mean = abs.iter().sum::<f64>() / abs.len() as f64;
    let rel = lk(mean) - 10.0;
    let gated: Vec<f64> = abs.into_iter().filter(|z| lk(*z) > rel).collect();
    if gated.is_empty() {
        return lk(mean);
    }
    lk(gated.iter().sum::<f64>() / gated.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_track(bpm: f64, secs: f64, sr: f64, offset: f64) -> Vec<f32> {
        let n = (secs * sr) as usize;
        let mut v = vec![0f32; n];
        let p = 60.0 / bpm;
        let mut t = offset;
        let mut k = 0;
        while t < secs {
            let start = (t * sr) as usize;
            let amp = if k % 4 == 0 { 1.0 } else { 0.6 };
            for i in 0..(0.08 * sr) as usize {
                if start + i < n {
                    let env = (-(i as f64) / (0.015 * sr)).exp();
                    v[start + i] += (amp * env * (2.0 * std::f64::consts::PI * 60.0 * i as f64 / sr).sin()) as f32;
                }
            }
            t += p;
            k += 1;
        }
        // hats on offbeats
        let mut t = offset + p / 2.0;
        while t < secs {
            let start = (t * sr) as usize;
            for i in 0..(0.01 * sr) as usize {
                if start + i < n {
                    let noise = ((i * 7919 % 97) as f32 / 97.0 - 0.5) * 0.2;
                    v[start + i] += noise;
                }
            }
            t += p;
        }
        v
    }

    #[test]
    fn detects_tempo_and_phase() {
        for &(bpm, off) in &[(128.0, 0.31), (122.0, 0.05), (140.0, 0.2), (174.0, 0.12)] {
            let sr = 44100.0;
            let x = click_track(bpm, 40.0, sr, off);
            let (got, grid) = tempo_and_grid(&x, sr, 70.0, 180.0);
            assert!((got - bpm).abs() < 0.02, "bpm {bpm} got {got}");
            let g = grid.unwrap();
            let first = g.anchors[0].time;
            let p = 60.0 / bpm;
            let err = ((first - off) / p - ((first - off) / p).round()).abs() * p;
            assert!(err < 0.012, "phase err {err} for {bpm}");
            // the loud beat is the downbeat
            let beats = g.beats(40.0);
            let b1 = beats.iter().find(|b| (b.time - off).abs() < 0.02).unwrap();
            assert_eq!(b1.beat, 1, "downbeat for {bpm}");
        }
    }

    #[test]
    fn detects_key_of_chord() {
        let sr = 22050.0;
        let n = (sr * 10.0) as usize;
        // A minor triad: A3 C4 E4 with octaves
        let freqs = [220.0, 261.63, 329.63, 110.0, 440.0];
        let x: Vec<f32> = (0..n)
            .map(|i| freqs.iter().map(|f| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() as f32 * 0.15).sum())
            .collect();
        let k = detect_key(&x, sr);
        assert_eq!(crate::model::key_name(k, false), "Am");
    }
}

#[cfg(test)]
mod real_files {
    #[test]
    #[ignore]
    fn analyze_env_files() {
        let list = std::env::var("PREPDECK_AUDIO").unwrap_or_default();
        for p in list.split('|').filter(|s| !s.is_empty()) {
            let t0 = std::time::Instant::now();
            match super::analyze_file(p, &Default::default()) {
                Ok(r) => {
                    let g = r.grid.unwrap();
                    eprintln!("{:.1}s bpm {:.2} first {:.3} beat {} key {} gain {:.1} dur {:.1} :: {}", t0.elapsed().as_secs_f64(), r.bpm, g.anchors[0].time, g.anchors[0].beat, crate::model::key_name(r.key, false) + "/" + &crate::model::key_alnum(r.key), r.gain_db, r.duration, p.rsplit('/').next().unwrap());
                }
                Err(e) => eprintln!("ERR {e} :: {p}"),
            }
        }
    }
}
