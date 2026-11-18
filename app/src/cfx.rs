//! SOUND COLOR FX (the CFX row of the FX panel): one effect per channel, driven by its COLOR
//! knob (centre = off) and the shared PARAM knob. In the signal path it sits after the EQ and
//! before the channel fader, as in rekordbox 7.2.19 (`checks/re-7.2.19/mixer-fx-recording.md`
//! 2.6); the master's sits on the master mix.
//!
//! FILTER follows rekordbox's traced rules exactly in its blend zones, cutoff curves and
//! resonance law (the filters themselves are ordinary second-order sections; rekordbox's own
//! filter shape was not traced). NOISE, CRUSH and DUB ECHO follow the traced outlines; JET,
//! SPACE, SWEEP, PITCH and GATE COMP are recordbox's own takes on the effects of those names
//! (GATE COMP with the thresholds and time constants of rekordbox's `GateCore` / `CompCore`
//! parameter lists).

/// The USER menu's nine types in rekordbox's order (the effect if recordbox has it).
pub const LIST: [(&str, Option<u8>); 9] = [("FILTER", Some(0)), ("JET", Some(1)), ("CRUSH", Some(2)), ("NOISE", Some(3)), ("PITCH", Some(4)), ("SPACE", Some(5)), ("DUB ECHO", Some(6)), ("SWEEP", Some(7)), ("GATE COMP", Some(8))];

/// The effects DEFAULT offers for a mixer, in the order of its buttons. Without hardware
/// rekordbox shows FILTER alone; the lists of its supported mixers were not traced, so this is
/// the one list there is.
pub const DEFAULT_LIST: [u8; 1] = [0];

pub fn name(kind: u8) -> &'static str {
    LIST.iter().find(|x| x.1 == Some(kind)).map(|x| x.0).unwrap_or("FILTER")
}

#[derive(Clone, Copy, Default)]
struct Bq {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}

impl Bq {
    /// Second-order low-pass (`hp` false) or high-pass at `fc` with resonance `q`.
    fn set(&mut self, rate: f32, fc: f32, q: f32, hp: bool) {
        let w = std::f32::consts::TAU * (fc / rate).clamp(0.0005, 0.45);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.05));
        let a0 = 1.0 + alpha;
        self.b = if hp { [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0] } else { [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0] };
        self.a = [-2.0 * c / a0, (1.0 - alpha) / a0];
    }

    /// Band-pass (constant peak) at `fc`.
    fn band(&mut self, rate: f32, fc: f32, q: f32) {
        let w = std::f32::consts::TAU * (fc / rate).clamp(0.0005, 0.45);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.05));
        let a0 = 1.0 + alpha;
        self.b = [alpha / a0, 0.0, -alpha / a0];
        self.a = [-2.0 * c / a0, (1.0 - alpha) / a0];
    }

    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x + z[1] - self.a[0] * y;
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// rekordbox's CFX FILTER as numbers: for a COLOR position `c` (0..255) and PARAM `p`
/// (0..255), the share of low-pass, dry and high-pass, the two cutoffs and the resonance.
pub fn filter_law(c: f32, p: f32) -> (f32, f32, f32, f32, f32, f32) {
    let (lp, dry, hp) = if c < 110.0 {
        (1.0, 0.0, 0.0)
    } else if c <= 125.0 {
        let d = (c - 110.0) / 16.0;
        (1.0 - d, d, 0.0)
    } else if c <= 130.0 {
        (0.0, 1.0, 0.0)
    } else if c <= 146.0 {
        let h = (c - 130.0) / 16.0;
        (0.0, 1.0 - h, h)
    } else {
        (0.0, 0.0, 1.0)
    };
    let cl = c.min(125.0);
    let lp_fc = 100.0 + 198.41 * cl * (cl / 126.0).powi(2);
    let ch = ((c.max(131.0) - 130.0) / 126.0).powi(3);
    let hp_fc = 20.0 + 8200.0 * ch;
    let mut q = if p <= 127.0 { 0.5 + 3.5 * (1.0 - p / 128.0).powi(2) } else { 0.5 - 0.0036621 * (p - 128.0) };
    // Extra resonance at the low end of the high-pass.
    if c >= 131.0 && hp_fc < 249.9 {
        let t = 35.652 * ch;
        let r = (4.0 * (1.0 - t).powi(3) + 0.03).min(2.0);
        q = q.max(r);
    }
    (lp, dry, hp, lp_fc, hp_fc, q.max(0.05))
}

const ECHO_LEN: usize = 24000;

/// One channel's Sound Color FX.
pub struct Cfx {
    kind: u8,
    /// COLOR in use, moving toward the knob.
    color: f32,
    lp: Bq,
    hp: Bq,
    band: Bq,
    seed: u32,
    hold: [f32; 2],
    hold_n: f32,
    delay: Vec<f32>,
    w: usize,
    fb_lp: [f32; 2],
    combs: Vec<Vec<f32>>,
    comb_pos: [usize; 8],
    comb_lp: [f32; 8],
    made: (f32, f32, u32),
    /// PITCH: the shifter's line, write position and window phase.
    ps: Vec<f32>,
    ps_w: usize,
    ps_ph: f32,
    /// GATE COMP: the level follower and the gain in use.
    env: f32,
    gain: f32,
}

const PS_LEN: usize = 4096;
const PS_WIN: f32 = 2048.0;

impl Cfx {
    pub fn new() -> Cfx {
        let lens = [1117usize, 1291, 1433, 1559];
        let mut combs = Vec::new();
        for ch in 0..2 {
            for l in lens {
                combs.push(vec![0.0; l + ch * 31]);
            }
        }
        Cfx { kind: 0, color: 0.5, lp: Bq::default(), hp: Bq::default(), band: Bq::default(), seed: 0x1234_5678, hold: [0.0; 2], hold_n: 0.0, delay: vec![0.0; ECHO_LEN * 2], w: 0, fb_lp: [0.0; 2], combs, comb_pos: [0; 8], comb_lp: [0.0; 8], made: (-1.0, -1.0, 0), ps: vec![0.0; PS_LEN * 2], ps_w: 0, ps_ph: 0.0, env: 0.0, gain: 1.0 }
    }

    fn reset(&mut self) {
        self.lp.z = [[0.0; 2]; 2];
        self.hp.z = [[0.0; 2]; 2];
        self.band.z = [[0.0; 2]; 2];
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.combs.iter_mut().for_each(|c| c.iter_mut().for_each(|v| *v = 0.0));
        self.comb_lp = [0.0; 8];
        self.fb_lp = [0.0; 2];
        self.hold = [0.0; 2];
        self.made = (-1.0, -1.0, 0);
        self.ps.iter_mut().for_each(|v| *v = 0.0);
        self.env = 0.0;
        self.gain = 1.0;
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Process `buf` (stereo interleaved) in place. `color` and `param`: the knobs, 0..1.
    pub fn process(&mut self, kind: u8, color: f32, param: f32, buf: &mut [f32], rate: u32) {
        if kind != self.kind {
            self.kind = kind;
            self.reset();
        }
        // The effect is out of the path while the knob rests in the centre and nothing rings.
        let centred = (color - 0.5).abs() < 0.012;
        if centred && (self.color - 0.5).abs() < 0.012 {
            self.color = 0.5;
            return;
        }
        let r = rate as f32;
        let frames = buf.len() / 2;
        // The knob glides (rekordbox: 0.3333 ms per 8-bit step, 85 ms for the whole travel).
        let step = 1.0 / (0.085 * r);
        let mut n = 0;
        while n < frames {
            let end = (n + 32).min(frames);
            let target = if centred { 0.5 } else { color.clamp(0.0, 1.0) };
            let span = (end - n) as f32;
            self.color += (target - self.color).clamp(-step * span, step * span);
            let c = self.color;
            // How far from the centre, 0..1, and which side.
            let depth = ((c - 0.5).abs() * 2.0).min(1.0);
            let right = c > 0.5;
            let p = param.clamp(0.0, 1.0);
            match self.kind {
                0 => {
                    let (lp, dry, hp, lfc, hfc, q) = filter_law(c * 255.0, p * 255.0);
                    if self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        self.lp.set(r, lfc.min(0.45 * r), q, false);
                        self.hp.set(r, hfc, q, true);
                    }
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let x = *v;
                            let l = self.lp.run(ch, x);
                            let h = self.hp.run(ch, x);
                            *v = l * lp + x * dry + h * hp;
                        }
                    }
                }
                1 => {
                    // JET: a short feedback comb whose delay the knob sets (left: with the
                    // comb's phase turned over); PARAM is the feedback.
                    let d = (0.0003 + 0.006 * (1.0 - depth)) * r;
                    let fbk = 0.3 + 0.6 * p;
                    let sign = if right { 1.0 } else { -1.0 };
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let i = d as usize;
                            let t = d - i as f32;
                            let a = self.delay[((self.w + ECHO_LEN - i) % ECHO_LEN) * 2 + ch];
                            let b = self.delay[((self.w + ECHO_LEN - i - 1) % ECHO_LEN) * 2 + ch];
                            let e = a + (b - a) * t;
                            self.delay[self.w * 2 + ch] = *v + e * fbk * sign;
                            *v += e * depth.min(0.9) * sign;
                        }
                        self.w = (self.w + 1) % ECHO_LEN;
                    }
                }
                2 => {
                    // CRUSH: sample and hold, up to 10 ms at the knob's ends (rekordbox's
                    // range); to the right the result is high-passed too. PARAM mixes it in.
                    let hold = (depth * depth * 0.010 * r).max(1.0);
                    let wet = 0.4 + 0.6 * p;
                    if right && self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        self.hp.set(r, 20.0 + 4000.0 * depth * depth, 0.7, true);
                    }
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        self.hold_n += 1.0;
                        if self.hold_n >= hold {
                            self.hold_n -= hold;
                            self.hold = [f[0], f[1]];
                        }
                        for (ch, v) in f.iter_mut().enumerate() {
                            let mut y = self.hold[ch];
                            if right {
                                y = self.hp.run(ch, y);
                            }
                            *v = *v * (1.0 - wet * depth.min(1.0).sqrt()) + y * wet * depth.min(1.0).sqrt();
                        }
                    }
                }
                3 => {
                    // NOISE: white noise through a filter the knob sweeps (left: darker,
                    // right: brighter), added to the sound; PARAM is its level.
                    if self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        let fc = if right { 800.0 * (0.45 * r / 800.0).powf(depth) } else { 8000.0 * (120.0f32 / 8000.0).powf(depth) };
                        self.band.band(r, fc, 1.2);
                    }
                    let level = depth.min(1.0) * (0.15 + 0.6 * p);
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        let x = self.noise();
                        let y = self.band.run(0, x) * level;
                        f[0] += y;
                        f[1] += y;
                    }
                }
                5 => {
                    // SPACE: a reverb whose amount the knob sets; left keeps the lows of the
                    // reverb, right the highs. PARAM is the decay.
                    let fbk = 0.80 + 0.17 * p;
                    let k = 1.0 - (-std::f32::consts::TAU * 600.0 / r).exp();
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let x = *v * 0.25;
                            let mut acc = 0.0;
                            for j in 0..4 {
                                let i = ch * 4 + j;
                                let line = &mut self.combs[i];
                                let len = line.len();
                                let y = line[self.comb_pos[i]];
                                self.comb_lp[i] = y * 0.7 + self.comb_lp[i] * 0.3;
                                line[self.comb_pos[i]] = x + self.comb_lp[i] * fbk;
                                self.comb_pos[i] = (self.comb_pos[i] + 1) % len;
                                acc += y;
                            }
                            self.fb_lp[ch] += (acc - self.fb_lp[ch]) * k;
                            let wet = if right { acc - self.fb_lp[ch] } else { self.fb_lp[ch] * 1.5 };
                            *v += wet * depth;
                        }
                    }
                }
                6 => {
                    // DUB ECHO: a filtered feedback echo; left low-passes the repeats, right
                    // high-passes them. PARAM is the feedback.
                    let d = ((0.375 * r) as usize).min(ECHO_LEN - 1);
                    let fbk = 0.35 + 0.5 * p;
                    let k = 1.0 - (-std::f32::consts::TAU * if right { 900.0 } else { 1800.0 } / r).exp();
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let e = self.delay[((self.w + ECHO_LEN - d) % ECHO_LEN) * 2 + ch];
                            self.fb_lp[ch] += (e - self.fb_lp[ch]) * k;
                            let coloured = if right { e - self.fb_lp[ch] } else { self.fb_lp[ch] };
                            self.delay[self.w * 2 + ch] = *v * depth + coloured * fbk;
                            *v += coloured * depth.min(1.0);
                        }
                        self.w = (self.w + 1) % ECHO_LEN;
                    }
                }
                4 => {
                    // PITCH: left lowers the pitch, right raises it, an octave at the ends;
                    // PARAM mixes the shifted sound in (all of it from the centre up).
                    let ratio = 2f32.powf(if right { depth } else { -depth });
                    let wet = (p * 2.0).min(1.0) * (depth * 8.0).min(1.0);
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        self.ps[self.ps_w * 2] = f[0];
                        self.ps[self.ps_w * 2 + 1] = f[1];
                        self.ps_ph += (1.0 - ratio) / PS_WIN;
                        self.ps_ph -= self.ps_ph.floor();
                        let mut o = [0.0f32; 2];
                        for k in 0..2 {
                            let ph = (self.ps_ph + k as f32 * 0.5).fract();
                            let d = 1.0 + ph * PS_WIN;
                            let i = d as usize;
                            let t = d - i as f32;
                            let g = 0.5 - 0.5 * (std::f32::consts::TAU * ph).cos();
                            let (a, b) = ((self.ps_w + PS_LEN - i) % PS_LEN, (self.ps_w + PS_LEN - i - 1) % PS_LEN);
                            for ch in 0..2 {
                                o[ch] += (self.ps[a * 2 + ch] + (self.ps[b * 2 + ch] - self.ps[a * 2 + ch]) * t) * g;
                            }
                        }
                        self.ps_w = (self.ps_w + 1) % PS_LEN;
                        f[0] += (o[0] - f[0]) * wet;
                        f[1] += (o[1] - f[1]) * wet;
                    }
                }
                8 => {
                    // GATE COMP: left a gate whose threshold rises with the knob, right a
                    // compressor that squeezes harder with it and makes the level up. The
                    // follower's release (0.99964 / 0.99919 a sample at 44.1 kHz), the gate's
                    // opening and closing (0.0026571, 0.99982), the compressor's threshold
                    // (0.24) and its make-up limit (3.6) are rekordbox's defaults; PARAM is
                    // the gate's closing time or the compressor's make-up.
                    let per = 44100.0 / r;
                    let rel = if right { 0.99919f32 } else { 0.99964f32 }.powf(per);
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        let a = f[0].abs().max(f[1].abs());
                        self.env = if a > self.env { a } else { self.env * rel };
                        let want = if right {
                            let th = 0.24 * (1.0 - 0.75 * depth);
                            let ratio = 1.0 - 0.85 * depth;
                            let g = if self.env > th { (th + (self.env - th) * ratio) / self.env } else { 1.0 };
                            g * (1.0 + (3.6 - 1.0) * depth * p).min(1.0 / g.max(0.05)).min(3.6)
                        } else if self.env >= 0.5 * depth * depth {
                            1.0
                        } else {
                            0.0
                        };
                        if want > self.gain {
                            self.gain += (want - self.gain) * (0.0026571 * per * 4.0).min(1.0);
                        } else {
                            let close = 0.99982f32.powf(per * (0.25 + 3.0 * (1.0 - p)));
                            self.gain = want + (self.gain - want) * close;
                        }
                        f[0] *= self.gain;
                        f[1] *= self.gain;
                    }
                }
                _ => {
                    // SWEEP: left takes a band out of the sound (a notch moving down), right
                    // leaves only a band (moving up). PARAM is the band's width.
                    if self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        let fc = if right { 150.0 * (9000.0f32 / 150.0).powf(depth) } else { 9000.0 * (150.0f32 / 9000.0).powf(depth) };
                        self.band.band(r, fc, 0.6 + 4.0 * (1.0 - p));
                    }
                    let amount = (depth * 4.0).min(1.0);
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let b = self.band.run(ch, *v);
                            *v = if right { *v * (1.0 - amount) + b * amount } else { *v - b * amount };
                        }
                    }
                }
            }
            n = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(kind: u8, color: f32, param: f32, freq: f32) -> f32 {
        let mut fx = Cfx::new();
        let mut x: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * freq * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect();
        fx.process(kind, color, param, &mut x, 48000);
        let tail = &x[48000..];
        20.0 * ((tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt() / (0.5 * std::f32::consts::FRAC_1_SQRT_2)).log10()
    }

    #[test]
    fn filter_law_matches_rekordbox() {
        // Blend zones.
        assert_eq!(filter_law(60.0, 128.0).0, 1.0);
        let (lp, dry, hp, ..) = filter_law(118.0, 128.0);
        assert!((lp - 0.5).abs() < 1e-6 && (dry - 0.5).abs() < 1e-6 && hp == 0.0);
        assert_eq!(filter_law(128.0, 128.0).1, 1.0);
        assert!((filter_law(138.0, 128.0).2 - 0.5).abs() < 1e-6);
        assert_eq!(filter_law(200.0, 128.0).2, 1.0);
        // Cutoffs: low-pass 100 Hz at the left end and about 24.5 kHz at 125; high-pass about
        // 8 kHz at the right end.
        assert!((filter_law(0.0, 128.0).3 - 100.0).abs() < 0.01);
        assert!((filter_law(125.0, 128.0).3 - 24509.0).abs() < 60.0, "{}", filter_law(125.0, 128.0).3);
        assert!((filter_law(255.0, 128.0).4 - 8026.0).abs() < 30.0, "{}", filter_law(255.0, 128.0).4);
        // Resonance: 4.0 with PARAM at the left end, 0.5 at the centre, falling to the right.
        assert!((filter_law(60.0, 0.0).5 - 4.0).abs() < 1e-5);
        assert!((filter_law(60.0, 128.0).5 - 0.5).abs() < 1e-5);
        assert!(filter_law(60.0, 255.0).5 < 0.1);
    }

    #[test]
    fn filter_is_out_of_the_path_at_the_centre_and_cuts_at_the_ends() {
        assert!(level(0, 0.5, 0.5, 1000.0).abs() < 0.001);
        // Fully left: a 100 Hz low-pass, so 5 kHz is gone and 50 Hz stays.
        assert!(level(0, 0.0, 0.5, 5000.0) < -50.0);
        assert!(level(0, 0.0, 0.5, 50.0).abs() < 3.0);
        // Fully right: an 8 kHz high-pass, so 200 Hz is gone and 15 kHz stays.
        assert!(level(0, 1.0, 0.5, 200.0) < -50.0);
        assert!(level(0, 1.0, 0.5, 15000.0).abs() < 3.0);
    }

    #[test]
    fn every_effect_stays_bounded_and_does_something() {
        for (name, kind) in LIST {
            let Some(kind) = kind else { continue };
            for color in [0.05, 0.95] {
                let mut fx = Cfx::new();
                let x: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect();
                let mut y = x.clone();
                fx.process(kind, color, 0.5, &mut y, 48000);
                let peak = y.iter().fold(0.0f32, |a, b| a.max(b.abs()));
                assert!(peak.is_finite() && peak < 4.0, "{name} at {color}: peak {peak}");
                let diff = (x.iter().zip(&y).map(|(a, b)| (a - b) * (a - b)).sum::<f32>() / x.len() as f32).sqrt();
                assert!(diff > 0.003, "{name} at {color} changes the sound ({diff})");
            }
        }
    }
}
