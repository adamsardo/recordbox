//! Calibration harness: least-squares fit of the original's waveform channels from this
//! app's band levels, for several band split points. Read-only on all inputs.
//! PREPDECK_ANLZ_PAIRS = tsv "audio\toriginal ANLZ base".

use super::anlz_compare::{pearson, sections};

/// Features per 1/150 s frame for one band split: peak and RMS of full, low, mid, high.
fn frame_features(mono: &[f32], sr: f64, lo: f64, hi: f64) -> Vec<[f64; 8]> {
    use crate::analysis::Biquad;
    let hop = sr / 150.0;
    let n = (mono.len() as f64 / hop).ceil() as usize;
    let mut f = [Biquad::new("lp", lo, 0.707, sr, 0.0), Biquad::new("lp", lo, 0.707, sr, 0.0), Biquad::new("hp", hi, 0.707, sr, 0.0), Biquad::new("hp", hi, 0.707, sr, 0.0), Biquad::new("hp", lo, 0.707, sr, 0.0), Biquad::new("lp", hi, 0.707, sr, 0.0)];
    let mut out = vec![[0f64; 8]; n];
    let mut cnt = vec![0f64; n];
    for (i, &x) in mono.iter().enumerate() {
        let x = x as f64;
        let t0 = f[0].run(x);
        let l = f[1].run(t0);
        let t2 = f[2].run(x);
        let h = f[3].run(t2);
        let t4 = f[4].run(x);
        let m = f[5].run(t4);
        let k = ((i as f64) / hop) as usize;
        if k >= n {
            break;
        }
        let o = &mut out[k];
        for (j, v) in [x, l, m, h].iter().enumerate() {
            o[j] = o[j].max(v.abs());
            o[4 + j] += v * v;
        }
        cnt[k] += 1.0;
    }
    for (o, c) in out.iter_mut().zip(cnt) {
        for j in 4..8 {
            o[j] = (o[j] / c.max(1.0)).sqrt();
        }
    }
    out
}

fn bucket(fr: &[[f64; 8]], cols: usize) -> Vec<[f64; 8]> {
    let n = fr.len();
    (0..cols)
        .map(|i| {
            let a = i * n / cols;
            let b = ((i + 1) * n / cols).max(a + 1).min(n);
            let mut m = [0f64; 8];
            for f in &fr[a..b] {
                for j in 0..4 {
                    m[j] = m[j].max(f[j]);
                }
                for j in 4..8 {
                    m[j] += f[j] / (b - a) as f64;
                }
            }
            m
        })
        .collect()
}

/// Normal equations for y = w·[1, x0..x7] (+ squared peaks).
#[derive(Clone)]
struct Fit {
    ata: Vec<Vec<f64>>,
    aty: Vec<f64>,
    xs: Vec<Vec<f64>>,
    ys: Vec<f64>,
}
const NF: usize = 13;
fn feats(x: &[f64; 8]) -> [f64; NF] {
    [1.0, x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7], x[0].sqrt(), x[1].sqrt(), x[2].sqrt(), x[3].sqrt()]
}
impl Fit {
    fn new() -> Fit {
        Fit { ata: vec![vec![0.0; NF]; NF], aty: vec![0.0; NF], xs: vec![], ys: vec![] }
    }
    fn add(&mut self, x: &[f64; 8], y: f64) {
        let f = feats(x);
        for i in 0..NF {
            self.aty[i] += f[i] * y;
            for j in 0..NF {
                self.ata[i][j] += f[i] * f[j];
            }
        }
        if self.ys.len() < 400_000 {
            self.xs.push(f.to_vec());
            self.ys.push(y);
        }
    }
    fn solve(&self) -> Vec<f64> {
        let n = NF;
        let mut a = self.ata.clone();
        for i in 0..n {
            a[i][i] += 1e-6;
        }
        let mut b = self.aty.clone();
        for c in 0..n {
            let p = (c..n).max_by(|i, j| a[*i][c].abs().partial_cmp(&a[*j][c].abs()).unwrap()).unwrap();
            a.swap(c, p);
            b.swap(c, p);
            for r in 0..n {
                if r != c {
                    let k = a[r][c] / a[c][c];
                    for cc in c..n {
                        a[r][cc] -= k * a[c][cc];
                    }
                    b[r] -= k * b[c];
                }
            }
        }
        (0..n).map(|i| b[i] / a[i][i]).collect()
    }
    fn corr(&self, w: &[f64]) -> f64 {
        let pred: Vec<f64> = self.xs.iter().map(|x| x.iter().zip(w).map(|(a, b)| a * b).sum()).collect();
        pearson(&pred, &self.ys)
    }
}

#[test]
#[ignore]
fn fit_waveforms() {
    let Ok(f) = std::env::var("PREPDECK_ANLZ_PAIRS") else { return };
    let pairs: Vec<(String, String)> = std::fs::read_to_string(f).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string()))).take(70).collect();
    let splits: [(f64, f64); 6] = [(200.0, 2500.0), (150.0, 1500.0), (300.0, 4000.0), (120.0, 3000.0), (250.0, 6000.0), (400.0, 8000.0)];
    // targets: (name, file, tag, stride, channel, mask/shift fn id, columns or 0 for 150 fps)
    let targets: Vec<(&str, &str, &[u8; 4], usize, usize, u8)> = vec![
        ("PWAV height", "DAT", b"PWAV", 1, 0, 0),
        ("PWAV white", "DAT", b"PWAV", 1, 0, 1),
        ("PWV2 height", "DAT", b"PWV2", 1, 0, 2),
        ("PWV3 height", "EXT", b"PWV3", 1, 0, 0),
        ("PWV3 white", "EXT", b"PWV3", 1, 0, 1),
        ("PWV4 b0", "EXT", b"PWV4", 6, 0, 9),
        ("PWV4 b1", "EXT", b"PWV4", 6, 1, 9),
        ("PWV4 b2", "EXT", b"PWV4", 6, 2, 9),
        ("PWV4 b3", "EXT", b"PWV4", 6, 3, 9),
        ("PWV4 b4", "EXT", b"PWV4", 6, 4, 9),
        ("PWV4 b5", "EXT", b"PWV4", 6, 5, 9),
        ("PWV5 height", "EXT", b"PWV5", 2, 0, 3),
        ("PWV5 red", "EXT", b"PWV5", 2, 0, 4),
        ("PWV5 green", "EXT", b"PWV5", 2, 0, 5),
        ("PWV5 blue", "EXT", b"PWV5", 2, 0, 6),
        ("PWV6 low", "2EX", b"PWV6", 3, 0, 9),
        ("PWV6 mid", "2EX", b"PWV6", 3, 1, 9),
        ("PWV6 high", "2EX", b"PWV6", 3, 2, 9),
        ("PWV7 low", "2EX", b"PWV7", 3, 0, 9),
        ("PWV7 mid", "2EX", b"PWV7", 3, 1, 9),
        ("PWV7 high", "2EX", b"PWV7", 3, 2, 9),
    ];
    let fits = std::sync::Mutex::new(vec![vec![Fit::new(); targets.len()]; splits.len()]);
    std::thread::scope(|s| {
        for ch in pairs.chunks((pairs.len() / 8).max(1)) {
            let fits = &fits;
            let targets = &targets;
            s.spawn(move || {
                for (audio, base) in ch {
                    let Ok(d) = crate::audio::decode_file(audio) else { continue };
                    let mono = d.mono();
                    let files: Vec<(String, Vec<u8>)> = ["DAT", "EXT", "2EX"].iter().map(|e| (e.to_string(), std::fs::read(format!("{base}.{e}")).unwrap_or_default())).collect();
                    for (si, (lo, hi)) in splits.iter().enumerate() {
                        let fr = frame_features(&mono, d.sr as f64, *lo, *hi);
                        let mut local = vec![Fit::new(); targets.len()];
                        for (ti, (_, file, tag, stride, ch, kind)) in targets.iter().enumerate() {
                            let Some(body) = files.iter().find(|f| f.0 == *file).and_then(|f| sections(&f.1).into_iter().find(|x| &x.0 == *tag)).map(|x| x.2) else { continue };
                            let vals: Vec<f64> = body
                                .chunks_exact(*stride)
                                .map(|c| {
                                    let w = if *stride == 2 { u16::from_be_bytes([c[0], c[1]]) as u32 } else { c[*ch] as u32 };
                                    (match kind {
                                        0 => w & 31,
                                        1 => w >> 5,
                                        2 => w & 15,
                                        3 => (w >> 2) & 31,
                                        4 => (w >> 13) & 7,
                                        5 => (w >> 10) & 7,
                                        6 => (w >> 7) & 7,
                                        _ => w,
                                    }) as f64
                                })
                                .collect();
                            let cols = if vals.len() == 400 || vals.len() == 100 || vals.len() == 1200 { bucket(&fr, vals.len()) } else { fr.clone() };
                            for (i, y) in vals.iter().enumerate() {
                                if let Some(x) = cols.get(i) {
                                    if i % 3 == 0 || vals.len() <= 1200 {
                                        local[ti].add(x, *y);
                                    }
                                }
                            }
                        }
                        let mut g = fits.lock().unwrap();
                        for (ti, l) in local.into_iter().enumerate() {
                            let gf = &mut g[si][ti];
                            for i in 0..NF {
                                gf.aty[i] += l.aty[i];
                                for j in 0..NF {
                                    gf.ata[i][j] += l.ata[i][j];
                                }
                            }
                            if gf.ys.len() < 200_000 {
                                gf.xs.extend(l.xs);
                                gf.ys.extend(l.ys);
                            }
                        }
                    }
                }
            });
        }
    });
    let fits = fits.into_inner().unwrap();
    for (ti, t) in targets.iter().enumerate() {
        let mut best = (0usize, -2.0, vec![]);
        for si in 0..splits.len() {
            let w = fits[si][ti].solve();
            let c = fits[si][ti].corr(&w);
            if c > best.1 {
                best = (si, c, w);
            }
        }
        let w0 = fits[0][ti].solve();
        let c0 = fits[0][ti].corr(&w0);
        eprintln!("{:<12} split0 r={:.3} best split {:?} r={:.3}", t.0, c0, splits[best.0], best.1);
        eprintln!("RUST (\"{}\", [{}]),", t.0, w0.iter().map(|x| format!("{:.5}", x)).collect::<Vec<_>>().join(", "));
    }
}
