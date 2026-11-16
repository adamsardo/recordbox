//! Original synthetic demo loops written on first start (like the original's bundled demo
//! tracks), so a fresh library has something to analyse, prepare and export.

use std::path::{Path, PathBuf};

pub struct DemoSpec {
    pub name: &'static str,
    pub bpm: f64,
    /// Root pitch class (C = 0) and minor flag.
    pub root: i32,
    pub minor: bool,
    pub bars: usize,
}

pub const DEMOS: [DemoSpec; 4] = [
    DemoSpec { name: "Demo Loop 1 - Night Drive", bpm: 124.0, root: 9, minor: true, bars: 24 },
    DemoSpec { name: "Demo Loop 2 - Warehouse", bpm: 128.0, root: 0, minor: true, bars: 24 },
    DemoSpec { name: "Demo Loop 3 - Sunrise", bpm: 120.0, root: 7, minor: false, bars: 20 },
    DemoSpec { name: "Demo Loop 4 - Breaks", bpm: 132.0, root: 2, minor: true, bars: 24 },
];

pub fn write_wav(path: &Path, sr: u32, stereo: &[f32]) -> std::io::Result<()> {
    let n = stereo.len();
    let mut b = Vec::with_capacity(44 + n * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + n * 2) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&sr.to_le_bytes());
    b.extend_from_slice(&(sr * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&((n * 2) as u32).to_le_bytes());
    for s in stereo {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32000.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b)
}

fn freq(midi: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0) / 12.0)
}

pub fn synth(spec: &DemoSpec, sr: u32) -> Vec<f32> {
    let srf = sr as f64;
    let beat = 60.0 / spec.bpm;
    let lead_in = 0.0;
    let total = lead_in + spec.bars as f64 * 4.0 * beat + 1.0;
    let n = (total * srf) as usize;
    let mut l = vec![0f32; n];
    let mut seed: u32 = 12345 + spec.root as u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let third = if spec.minor { 3 } else { 4 };
    let chord = [0, third, 7];
    let prog: [i32; 4] = if spec.minor { [0, 5, 0, 7] } else { [0, 5, 7, 0] };
    let base = 36 + spec.root as i64;
    let beats = spec.bars * 4;
    for k in 0..beats {
        let t0 = lead_in + k as f64 * beat;
        let s0 = (t0 * srf) as usize;
        let bar = k / 4;
        let section = bar / 8; // 8-bar phrases
        // Kick (skip in a breakdown section for variety).
        let breakdown = section == 1 && spec.bars > 16 && bar % 8 >= 4;
        if !breakdown || (spec.name.contains("Breaks") && k % 4 != 1) {
            let amp = if k % 4 == 0 { 1.0 } else { 0.85 };
            for i in 0..(0.25 * srf) as usize {
                let t = i as f64 / srf;
                let f = 50.0 + 90.0 * (-t * 30.0).exp();
                let ph = 2.0 * std::f64::consts::PI * (50.0 * t + 90.0 / 30.0 * (1.0 - (-t * 30.0).exp()));
                let _ = f;
                let env = (-t * 9.0).exp();
                if s0 + i < n {
                    l[s0 + i] += (amp * 0.8 * env * ph.sin()) as f32;
                }
            }
        }
        // Off-beat hat.
        let h0 = ((t0 + beat / 2.0) * srf) as usize;
        for i in 0..(0.04 * srf) as usize {
            let env = (-(i as f64) / (0.008 * srf)).exp() as f32;
            if h0 + i < n {
                l[h0 + i] += rnd() * 0.18 * env;
            }
        }
        // Clap on 2 and 4.
        if k % 2 == 1 && !breakdown {
            for i in 0..(0.12 * srf) as usize {
                let env = (-(i as f64) / (0.03 * srf)).exp() as f32;
                if s0 + i < n {
                    l[s0 + i] += rnd() * 0.22 * env;
                }
            }
        }
        // Bass on off-beat 16ths, following the progression.
        let root = base + prog[bar % 4] as i64;
        let bf = freq(root as f64);
        for sub in [1usize, 2, 3] {
            let st = ((t0 + sub as f64 * beat / 4.0) * srf) as usize;
            for i in 0..(beat / 4.0 * srf * 0.9) as usize {
                let t = i as f64 / srf;
                let env = (-t * 6.0).exp();
                let v = (2.0 * std::f64::consts::PI * bf * t).sin() * 0.5 + (4.0 * std::f64::consts::PI * bf * t).sin() * 0.15;
                if st + i < n {
                    l[st + i] += (v * env * 0.35) as f32;
                }
            }
        }
        // Pad chord, one per bar.
        if k % 4 == 0 {
            for (ci, c) in chord.iter().enumerate() {
                let f = freq((base + 24 + prog[bar % 4] as i64 + *c as i64) as f64);
                let len = (4.0 * beat * srf) as usize;
                for i in 0..len {
                    let t = i as f64 / srf;
                    let env = (t * 8.0).min(1.0) * (1.0 - t / (4.0 * beat)).max(0.0);
                    let v = (2.0 * std::f64::consts::PI * f * t + ci as f64).sin() * 0.06 * env;
                    if s0 + i < n {
                        l[s0 + i] += v as f32;
                    }
                }
            }
        }
    }
    let peak = l.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    let mut out = Vec::with_capacity(n * 2);
    for v in l {
        let s = (v / peak * 0.89).tanh();
        out.push(s);
        out.push(s);
    }
    out
}

/// Write the demo files into the app's data folder; returns their paths.
pub fn ensure_demo_files() -> Vec<PathBuf> {
    let dir = crate::db::data_dir().join("Demo Tracks");
    let _ = std::fs::create_dir_all(&dir);
    let mut out = vec![];
    for d in DEMOS.iter() {
        let p = dir.join(format!("{}.wav", d.name));
        if !p.exists() {
            let audio = synth(d, 44100);
            if write_wav(&p, 44100, &audio).is_err() {
                continue;
            }
        }
        out.push(p);
    }
    out
}
