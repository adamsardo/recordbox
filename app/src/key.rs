//! Key detection: tuning-corrected pitch-class profile (from spectral peaks) matched against
//! major/minor key templates. Parameters were chosen by benchmarking against the original's
//! own key results for 37 tracks (see MECHANICS.md).

use rustfft::{num_complex::Complex, FftPlanner};

#[derive(Clone, Copy, Debug)]
pub struct KeyParams {
    /// Estimate and correct global tuning (deviation from A=440).
    pub tuning: bool,
    /// Use only spectral peaks (local maxima) instead of every bin.
    pub peaks: bool,
    /// Weight of the bass-register profile added to the full profile.
    pub bass_weight: f64,
    /// Template set (see PROFILES).
    pub profile: usize,
    /// Exponent applied to magnitudes before summing (compression).
    pub power: f64,
    /// Score bonus for minor keys (dance music is mostly minor).
    pub minor_bias: f64,
}

/// Harmonic/percussive separation before the pitch profile: keep energy that is steady in
/// time (notes, chords) and suppress energy that is spread across frequency (drums).
fn hpss_enabled() -> bool {
    std::env::var("PREPDECK_KEY_HPSS").map(|v| v != "0").unwrap_or(HPSS_DEFAULT)
}
const HPSS_DEFAULT: bool = false;

fn median(v: &mut [f32]) -> f32 {
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).1
}

/// Soft-mask each frame with H² / (H² + P²), H = median over ±k frames, P = median over ±k bins.
fn hpss(frames: &mut [Vec<f32>], k: usize) {
    let nf = frames.len();
    if nf == 0 {
        return;
    }
    let nb = frames[0].len();
    let mut h = vec![vec![0f32; nb]; nf];
    let mut buf = Vec::with_capacity(2 * k + 1);
    for b in 0..nb {
        for f in 0..nf {
            buf.clear();
            for g in f.saturating_sub(k)..(f + k + 1).min(nf) {
                buf.push(frames[g][b]);
            }
            h[f][b] = median(&mut buf);
        }
    }
    for f in 0..nf {
        let row = frames[f].clone();
        for b in 0..nb {
            buf.clear();
            buf.extend_from_slice(&row[b.saturating_sub(k)..(b + k + 1).min(nb)]);
            let p = median(&mut buf);
            let hh = h[f][b] * h[f][b];
            let mask = hh / (hh + p * p + 1e-12);
            frames[f][b] = row[b] * mask;
        }
    }
}

pub const DEFAULT: KeyParams = KeyParams { tuning: false, peaks: true, bass_weight: 0.45, profile: 2, power: 1.0, minor_bias: 0.15 };

/// (name, major, minor) templates, index 0 = tonic.
pub const PROFILES: [(&str, [f64; 12], [f64; 12]); 5] = [
    (
        "krumhansl",
        [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88],
        [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17],
    ),
    (
        "temperley",
        [5.0, 2.0, 3.5, 2.0, 4.5, 4.0, 2.0, 4.5, 2.0, 3.5, 1.5, 4.0],
        [5.0, 2.0, 3.5, 4.5, 2.0, 4.0, 2.0, 4.5, 3.5, 2.0, 1.5, 4.0],
    ),
    (
        "diatonic-triad",
        [3.0, 0.0, 1.0, 0.0, 2.0, 1.0, 0.0, 2.0, 0.0, 1.0, 0.0, 1.0],
        [3.0, 0.0, 1.0, 2.0, 0.0, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0, 0.0],
    ),
    (
        // Natural-minor biased template suited to dance music (minor keys with flat 6/7).
        "dance",
        [6.0, 1.0, 3.5, 1.0, 4.5, 4.0, 1.0, 5.0, 1.0, 3.5, 1.5, 2.5],
        [6.0, 1.0, 3.0, 5.0, 1.0, 4.0, 1.0, 5.0, 4.0, 1.0, 3.5, 1.5],
    ),
    (
        "shaath-like",
        [6.6, 2.0, 3.5, 2.3, 4.6, 4.0, 2.5, 5.2, 2.4, 3.7, 2.3, 3.4],
        [6.5, 2.8, 3.5, 5.4, 2.7, 3.5, 2.5, 5.2, 4.0, 2.7, 4.3, 3.2],
    ),
];

pub struct KeyFeatures {
    pub full: [f64; 12],
    pub bass: [f64; 12],
}

/// Pitch-class energy of a whole track.
pub fn features(mono: &[f32], sr: f64, p: &KeyParams) -> KeyFeatures {
    // Downsample to ~11 kHz.
    let factor = ((sr / 11025.0).round() as usize).max(1);
    let mut lp = [0f64; 4];
    let a = (-2.0 * std::f64::consts::PI * 3500.0 / sr).exp();
    let mut ds = Vec::with_capacity(mono.len() / factor + 1);
    for (i, &x) in mono.iter().enumerate() {
        // 4 one-pole stages
        let mut y = x as f64;
        for s in lp.iter_mut() {
            *s = a * *s + (1.0 - a) * y;
            y = *s;
        }
        if i % factor == 0 {
            ds.push(y as f32);
        }
    }
    let dsr = sr / factor as f64;
    let nfft = 8192usize;
    let hop = 2048usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    let bin_hz = dsr / nfft as f64;
    let lo_bin = (50.0 / bin_hz) as usize;
    let hi_bin = ((2000.0 / bin_hz) as usize).min(nfft / 2 - 2);
    // Collect per-frame magnitudes (only the needed bins) once.
    let mut frames: Vec<Vec<f32>> = vec![];
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut start = 0;
    while start + nfft <= ds.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(ds[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        frames.push((lo_bin..=hi_bin + 1).map(|b| buf[b].norm()).collect());
        start += hop;
    }
    if hpss_enabled() {
        hpss(&mut frames, 8);
    }
    // Peak list per frame: (fractional bin, magnitude)
    let peaks_of = |m: &Vec<f32>| -> Vec<(f64, f64)> {
        let mut out = vec![];
        for i in 1..m.len() - 1 {
            if p.peaks {
                if m[i] > m[i - 1] && m[i] >= m[i + 1] {
                    // parabolic interpolation
                    let (l, c, r) = (m[i - 1] as f64, m[i] as f64, m[i + 1] as f64);
                    let d = 0.5 * (l - r) / (l - 2.0 * c + r).min(-1e-12);
                    out.push(((lo_bin + i) as f64 + d, c));
                }
            } else {
                out.push(((lo_bin + i) as f64, m[i] as f64));
            }
        }
        out
    };
    // Tuning: histogram of peak deviations (cents) weighted by magnitude.
    let mut tune_cents = 0.0;
    if p.tuning {
        let mut hist = [0f64; 20];
        for (fi, m) in frames.iter().enumerate() {
            if fi % 4 != 0 {
                continue;
            }
            for (b, mag) in peaks_of(m) {
                let f = b * bin_hz;
                if f < 100.0 {
                    continue;
                }
                let midi = 69.0 + 12.0 * (f / 440.0).log2();
                let dev = midi - midi.round(); // -0.5..0.5
                let k = (((dev + 0.5) * 20.0) as usize).min(19);
                hist[k] += mag;
            }
        }
        // circular smoothing and argmax
        let mut best = (0usize, -1.0);
        for k in 0..20 {
            let s = hist[(k + 19) % 20] + 2.0 * hist[k] + hist[(k + 1) % 20];
            if s > best.1 {
                best = (k, s);
            }
        }
        tune_cents = ((best.0 as f64 + 0.5) / 20.0 - 0.5) * 100.0;
    }
    let mut full = [0f64; 12];
    let mut bass = [0f64; 12];
    for m in &frames {
        let mut fr = [0f64; 12];
        let mut bs = [0f64; 12];
        for (b, mag) in peaks_of(m) {
            let f = b * bin_hz;
            let midi = 69.0 + 12.0 * (f / 440.0).log2() - tune_cents / 100.0;
            let near = midi.round();
            let dev = (midi - near).abs();
            if dev > 0.4 {
                continue;
            }
            let w = 1.0 - dev / 0.4;
            let pc = (near as i64).rem_euclid(12) as usize;
            let v = mag.powf(p.power) * w;
            if f < 220.0 {
                bs[pc] += v;
            }
            if f >= 80.0 {
                fr[pc] += v;
            }
        }
        let s: f64 = fr.iter().sum();
        if s > 1e-9 {
            for k in 0..12 {
                full[k] += fr[k] / s;
            }
        }
        let s: f64 = bs.iter().sum();
        if s > 1e-9 {
            for k in 0..12 {
                bass[k] += bs[k] / s;
            }
        }
    }
    KeyFeatures { full, bass }
}

fn corr(x: &[f64; 12], prof: &[f64; 12], root: usize) -> f64 {
    let xm = x.iter().sum::<f64>() / 12.0;
    let ym = prof.iter().sum::<f64>() / 12.0;
    let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let a = x[(i + root) % 12] - xm;
        let b = prof[i] - ym;
        num += a * b;
        dx += a * a;
        dy += b * b;
    }
    num / (dx * dy).sqrt().max(1e-12)
}

/// Camelot-ordered key index (see model::key_from_pitch), or -1.
pub fn key_from_features(f: &KeyFeatures, p: &KeyParams) -> i32 {
    if f.full.iter().sum::<f64>() <= 0.0 {
        return -1;
    }
    let (_, maj, min) = PROFILES[p.profile.min(PROFILES.len() - 1)];
    let mut best = (f64::MIN, 0usize, false);
    for root in 0..12 {
        for (minor, prof) in [(false, &maj), (true, &min)] {
            let mut s = corr(&f.full, prof, root);
            if p.bass_weight > 0.0 && f.bass.iter().sum::<f64>() > 0.0 {
                s += p.bass_weight * corr(&f.bass, prof, root);
            }
            if minor {
                s += p.minor_bias;
            }
            if s > best.0 {
                best = (s, root, minor);
            }
        }
    }
    crate::model::key_from_pitch(best.1 as i32, best.2)
}

pub fn detect_key(mono: &[f32], sr: f64) -> i32 {
    let f = features(mono, sr, &DEFAULT);
    key_from_features(&f, &DEFAULT)
}

#[cfg(test)]
mod bench {
    use super::*;
    /// PREPDECK_KEYGT = tsv "path\tCamelot key" (reference keys from the original).
    #[test]
    #[ignore]
    fn key_benchmark() {
        let Ok(f) = std::env::var("PREPDECK_KEYGT") else { return };
        let rows: Vec<(String, i32)> = std::fs::read_to_string(f)
            .unwrap()
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(p, k)| (p.to_string(), crate::model::parse_key(k))))
            .collect();
        let variants: Vec<KeyParams> = {
            let mut v = vec![];
            for tuning in [false, true] {
                for peaks in [false, true] {
                    for power in [0.5, 1.0] {
                        v.push(KeyParams { tuning, peaks, power, bass_weight: 0.0, profile: 0, minor_bias: 0.0 });
                    }
                }
            }
            v
        };
        let results = std::sync::Mutex::new(vec![]);
        std::thread::scope(|s| {
            for ch in rows.chunks((rows.len() / 8).max(1)) {
                let results = &results;
                let variants = &variants;
                s.spawn(move || {
                    for (p, k) in ch {
                        let Ok(d) = crate::audio::decode_file(p) else { continue };
                        let mono = d.mono();
                        let feats: Vec<KeyFeatures> = variants.iter().map(|v| features(&mono, d.sr as f64, v)).collect();
                        results.lock().unwrap().push((*k, feats, p.rsplit('/').next().unwrap().to_string()));
                    }
                });
            }
        });
        let res = results.into_inner().unwrap();
        let n = res.len();
        let mut table = vec![];
        for (vi, v) in variants.iter().enumerate() {
            for prof in 0..PROFILES.len() {
                for (bw, mb) in [0.0, 0.3, 0.45, 0.6, 0.8, 1.0].iter().flat_map(|b| [0.0, 0.03, 0.06, 0.1, 0.15].map(|m| (*b, m))) {
                    let pp = KeyParams { profile: prof, bass_weight: bw, minor_bias: mb, ..*v };
                    let mut exact = 0;
                    let mut close = 0;
                    for (k, feats, _) in &res {
                        let got = key_from_features(&feats[vi], &pp);
                        if got == *k {
                            exact += 1;
                        } else if crate::app::compatible_key(*k, got) {
                            close += 1;
                        }
                    }
                    table.push((exact, close, format!("tuning {} peaks {} power {} profile {} bass {} minor {}", v.tuning, v.peaks, v.power, PROFILES[prof].0, bw, mb)));
                }
            }
        }
        table.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
        eprintln!("tracks {n}");
        for t in table.iter().take(15) {
            eprintln!("exact {:2} related {:2} :: {}", t.0, t.1, t.2);
        }
        // detail for the current default
        let vi = variants.iter().position(|v| v.tuning == DEFAULT.tuning && v.peaks == DEFAULT.peaks && v.power == DEFAULT.power).unwrap();
        for (k, feats, name) in &res {
            let got = key_from_features(&feats[vi], &DEFAULT);
            if got != *k {
                eprintln!("  ref {:>4} got {:>4} :: {}", crate::model::key_alnum(*k), crate::model::key_alnum(got), name);
            }
        }
    }
}
