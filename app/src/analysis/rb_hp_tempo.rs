//! The tempo-change path of rekordbox's high precision BeatGrid analysis (rekordbox 7.2.19):
//! what `AnalyzeJobBeatDetector::highPrecisionBeatAnalyze` does when the tempo may change
//! (analysis mode Dynamic, or Auto when `BeatAnalyzer_2_0` judged the tempo varied).
//!
//! 1. `madmom::analyze` (PCM version, 0x102307f8c) runs the downbeat network `model7` over the
//!    whole track in pieces: whole minutes, then the last minute and the rest (split in two
//!    halves when the rest is shorter than 30 s); each piece is decoded by the DBN on its own and
//!    the pieces are joined (`network_beats`);
//! 2. the file version (0x10230a820) scales the times to 44.1 kHz and repairs the beat list at
//!    every seam between pieces and at single gaps (`seam_repair`), then `get_beat_list` /
//!    `get_bpm` as on the constant path.

use super::dbn::Dbn;
use super::net::Net;

/// `v84 = (float)sr * 0.0013605f` (60 / 44100): the start of whole minute `i` in the network's
/// time base (seconds at 44.1 kHz) is `v84 * i`.
const MINUTE_44K: f32 = f32::from_bits(0x3ab2544a);
/// `(float)start * 0.000022676f` (1 / 44100): the start of the last two pieces.
const SAMPLE_44K: f32 = f32::from_bits(0x37be37c6);

/// The pieces the PCM version cuts `n` frames into: (start, length) in frames, and whether the
/// piece is joined at the beat nearest to the previous piece's last beat (the last two pieces)
/// or appended whole (the whole minutes before them).
pub fn pieces(n: i64, sr: i64) -> Vec<(i64, i64, bool)> {
    let a5 = sr as i32;
    let a6 = n as i32;
    let v76 = 60 * a5;
    let v77 = a6 / v76;
    let mut out = vec![];
    let (v82, v350, v351, len_a6);
    if v77 <= 0 {
        v82 = 0;
        v350 = a6;
        v351 = 0;
        len_a6 = a6;
    } else {
        let v357 = v77 - 1;
        let v78 = v77 * v76;
        let v79 = a6 - v77 * v76;
        let v80 = (a6 - v357 * v76) / 2;
        v351 = v357 * v76;
        let v81 = v79 >= 1 && v79 < 30 * a5;
        v82 = if v81 { a6 - (v80 + v357 * v76) } else { a6 - v77 * v76 };
        v350 = if v81 { v80 + v357 * v76 } else { v78 };
        len_a6 = if v81 { v80 } else { 60 * a5 };
        for i in 0..v357 {
            out.push(((i * v76) as i64, v76 as i64, false));
        }
    }
    if len_a6 >= 1 {
        out.push((v351 as i64, len_a6 as i64, true));
    }
    if v82 >= 1 {
        out.push((v350 as i64, v82 as i64, true));
    }
    out
}

/// `madmom::analyze` (PCM version) with the tempo-change flag: the network's beats (time in
/// seconds at 44.1 kHz, beat number) over `len` frames of the stereo `samples`.
pub fn network_beats(net: &Net, dbn: &Dbn, samples: &[f32], sr: u32, len: usize) -> Vec<(f32, i32)> {
    let n = len as i64;
    let a5 = sr as i32;
    let v84 = (a5 as f32) * MINUTE_44K;
    let mut acc: Vec<(f32, i32)> = vec![];
    let mut minute = 0i32;
    for (start, plen, nearest) in pieces(n, sr as i64) {
        let (s, l) = (start as usize, plen as usize);
        let mono: Vec<f32> = (s..s + l).map(|i| (samples[2 * i + 1] + samples[2 * i]) * 0.5).collect();
        let act = net.run(&mono);
        let mut beats = dbn.track(&act);
        if !nearest {
            // whole minutes: offset by v84 * i, appended as they are
            let off = v84 * minute as f32;
            minute += 1;
            for b in beats.iter_mut() {
                b.0 += off;
            }
            acc.extend(beats);
            continue;
        }
        if start < 1 {
            // the first piece (tracks under two minutes): the result as it is
            acc = beats;
            continue;
        }
        let rows = beats.len();
        if rows < 1 {
            continue;
        }
        let off = (start as i32 as f32) * SAMPLE_44K;
        for b in beats.iter_mut() {
            b.0 += off;
        }
        // `size` of the joined array is rows * 2: more than one value means at least one row
        if !acc.is_empty() {
            let last = acc[acc.len() - 1].0;
            let mut v248 = rows as i64 - 1;
            if rows < 2 || (last - beats[0].0).abs() <= (last - beats[1].0).abs() {
                v248 = 0;
            } else {
                let mut v266 = (last - beats[1].0).abs();
                let mut k = 0usize;
                while rows - 2 != k {
                    let v270 = (last - beats[k + 2].0).abs();
                    k += 1;
                    let stop = v266 <= v270;
                    v266 = v270;
                    if stop {
                        v248 = k as i64;
                        break;
                    }
                }
            }
            if v248 + 1 >= rows as i64 {
                continue;
            }
            acc.extend_from_slice(&beats[(v248 + 1) as usize..]);
        } else {
            acc.extend(beats);
        }
    }
    acc
}

const EPS: f32 = f32::from_bits(0x34000000);

/// One seam: the first beat at or after `seam` (seconds) and its neighbours. Beats missing at
/// the seam (one interval 1.35-3.5 times the one before, the next 0.25-0.75 times it) are
/// filled in evenly, a missing beat right after it likewise, a beat closer than 0.3 intervals
/// to the one before it is removed (decompile lines ~540-700 and 740-940 of 0x10230a820).
fn repair_at(b: &mut Vec<(f32, i32)>, seam: f32) {
    if b.is_empty() {
        return;
    }
    let count = b.len();
    let v105 = count.max(1);
    let mut v104 = 0usize;
    while b[v104].0 < seam {
        v104 += 1;
        if v105 == v104 {
            return;
        }
    }
    if v104 < 2 || count <= v104 + 1 {
        return;
    }
    let v111 = b[v104 - 1].0;
    let v112 = b[v104].0;
    let v113 = b[v104 + 1].0;
    let v114 = v111 - b[v104 - 2].0;
    let v115 = v112 - v111;
    let v116 = if v114 <= EPS { 0.0 } else { v115 / v114 };
    let v117 = if v115 <= EPS { 0.0 } else { (v113 - v112) / v115 };
    let v118 = (v116 + 0.5) as i32;
    if v118 >= 2 && v116 >= 1.35 && v116 <= 3.5 && v117 >= 0.25 && v117 <= 0.75 {
        let inv = 1.0f32 / v118 as f32;
        for j in 1..v118 {
            b.insert(v104 + (j - 1) as usize, ((v115 * j as f32).mul_add(inv, v111), 1));
        }
        return;
    }
    let v123 = (v117 + 0.5) as i32;
    if v123 >= 2 && v116 >= 0.9 && v116 <= 1.1 && v117 >= 1.35 && v117 <= 3.5 {
        let v127 = v113 - v112;
        let inv = 1.0f32 / v123 as f32;
        for k in 1..v123 {
            b.insert(v104 + k as usize, ((v127 * k as f32).mul_add(inv, v112), 1));
        }
        return;
    }
    if v116 >= 0.3 {
        if v117 >= 0.3 {
            return;
        }
        b.remove(v104 + 1);
    } else {
        b.remove(v104);
    }
}

/// The file version of `madmom::analyze` after the network when the tempo may change
/// (0x10230a820, decompile lines ~510-1015): the seams between the network's pieces (every
/// whole minute but the last, then the start of the last piece) are repaired, then single
/// intervals more than 1.8 times both neighbours (or, for the last pair, the one before) are
/// split evenly. `n` = frames analysed, `sr` = the analysis rate. Times are seconds at 44.1 kHz
/// (after `scale_times`).
pub fn seam_repair(b: &mut Vec<(f32, i32)>, n: i64, sr: i32) {
    let v302 = sr;
    let m = 60 * v302;
    let v91 = (n as u64 / m as i64 as u64) as i32;
    let (mut v90, v93) = if v91 > 0 { (m, (v91 - 1).wrapping_mul(m)) } else { (n as i32, 0) };
    let v94 = (n as i32).wrapping_sub(v90.wrapping_add(v93));
    if v94 < 30 * v302 && v94 > 0 && v91 > 0 {
        v90 = ((n - v93 as i64) as u64 >> 1) as i32;
    }
    let v301 = v90;
    if v91 > 1 {
        for i in 0..(v91 - 1) {
            repair_at(b, (i + 1) as f32 * 60.0);
        }
    }
    if v301 >= 1 {
        repair_at(b, v301.wrapping_add(v93) as f32 / v302 as f32);
    }
    // LABEL_211: single long intervals, over the indices the list had before this pass
    let v236 = b.len() as i32;
    if v236 > 2 {
        let v237 = v236 - 1;
        let v238 = v236 - 2;
        let mut v240 = 1i32;
        loop {
            let i = v240 as usize;
            let v248 = b[i - 1].0;
            let v249 = b[i].0;
            let v250 = v240 + 1;
            if v240 >= v238 {
                let v257 = v249 - v248;
                if v257 > EPS {
                    let v258 = b[i + 1].0 - v249;
                    let v259 = v258 / v257;
                    let v260 = (v259 + 0.5) as i32;
                    if !(v260 < 1 || v259 <= 1.8) && v260 >= 2 {
                        let inv = 1.0f32 / v260 as f32;
                        for ii in 2..=v260 {
                            b.insert(i + (ii - 1) as usize, ((v258 * (ii - 1) as f32).mul_add(inv, v249), 1));
                        }
                    }
                }
            } else {
                let v251 = b[i + 1].0;
                let v252 = v249 - v248;
                let v254 = v251 - v249;
                let v255 = if v252 > EPS { v254 / v252 } else { 0.0 };
                let v256 = b[i + 2].0 - v251;
                let v253 = if v256 > EPS { v254 / v256 } else { 0.0 };
                let v241 = (v255 + 0.5) as i32;
                if v241 >= 1 && (v253 + 0.5) as i32 >= 1 && v255 > 1.8 && v253 > 1.8 && v241 >= 2 {
                    let inv = 1.0f32 / v241 as f32;
                    for jj in 2..=v241 {
                        b.insert(i + (jj - 1) as usize, ((v254 * (jj - 1) as f32).mul_add(inv, v249), 1));
                    }
                }
            }
            v240 = v250;
            if v250 == v237 {
                break;
            }
        }
    }
}

/// The network stage of the tempo-change path as the file version of `madmom::analyze` runs it
/// (window = the whole track but its last frame): the network's raw beats, then the beat list
/// handed to analyzeFocus and its bpm (`(int)get_bpm`).
pub fn madmom_tempo(net: &Net, dbn: &Dbn, samples: &[f32], sr: u32, total: usize) -> (Vec<(f32, i32)>, Vec<super::dbn::ListBeat>, i32) {
    let (_, len) = super::madmom_window(total as i64, sr as i64, 0, true);
    let raw = network_beats(net, dbn, samples, sr, len as usize);
    let mut beats = raw.clone();
    super::scale_times(&mut beats, sr as i32);
    seam_repair(&mut beats, len, sr as i32);
    let list = super::dbn::get_beat_list(&beats, len, sr as i32, 4);
    let bpm = super::dbn::get_bpm(&list) as i32;
    (raw, list, bpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_follow_the_minute_layout() {
        let sr = 44100i64;
        let m = 60 * sr;
        // under a minute: one piece
        assert_eq!(pieces(30 * sr, sr), vec![(0, 30 * sr, true)]);
        // 150 s: two whole minutes... the second is the last minute, the 30 s rest its own piece
        assert_eq!(pieces(150 * sr, sr), vec![(0, m, false), (m, m, true), (2 * m, 30 * sr, true)]);
        // 200 s: the last minute and the 20 s rest are split in two halves of 40 s
        assert_eq!(pieces(200 * sr, sr), vec![(0, m, false), (m, m, false), (2 * m, 40 * sr, true), (2 * m + 40 * sr, 40 * sr, true)]);
        // exactly three minutes: no rest piece
        assert_eq!(pieces(180 * sr, sr), vec![(0, m, false), (m, m, false), (2 * m, m, true)]);
    }

    #[test]
    fn seam_repair_fills_a_missing_beat_at_a_minute() {
        // 0.5 s beats with the one at 60.0 s missing (the next is at 60.5 s)
        let mut b: Vec<(f32, i32)> = (0..130).map(|k| k as f32 * 0.5).filter(|t| *t != 60.0).map(|t| (t, 1)).collect();
        let n = 66 * 44100;
        seam_repair(&mut b, n, 44100);
        assert!(b.iter().any(|x| (x.0 - 60.0).abs() < 1e-4), "beat at the seam not filled");
        assert!(b.windows(2).all(|w| (w[1].0 - w[0].0 - 0.5).abs() < 1e-3));
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    /// The oracle dumps of the tempo-change corpus (`scratch/o4/<name>.m<mode>.txt`, written by
    /// `scripts/orun4.py` with rbhp): every one whose analysis took the tempo-change path.
    fn hendou_dumps() -> Vec<(String, super::super::stage_tests::Dump)> {
        let Ok(dir) = std::env::var("PREPDECK_HP_ORACLE") else { return vec![] };
        let only: Vec<String> = std::env::var("PREPDECK_HP_ONLY").map(|v| v.split(',').map(|s| s.to_string()).collect()).unwrap_or_default();
        let mut v: Vec<(String, super::super::stage_tests::Dump)> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| {
                let e = e.ok()?;
                let name = e.file_name().to_str()?.strip_suffix(".txt")?.to_string();
                if !only.is_empty() && !only.iter().any(|o| name.starts_with(o.as_str())) {
                    return None;
                }
                let d = super::super::stage_tests::read_dump(&e.path())?;
                d.hendou.then_some((name, d))
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Network pieces + DBN ("M": raw beats) and the file version's repair + beat list ("J")
    /// against rekordbox's own routines on the same PCM. PREPDECK_HP_ORACLE = o4 folder.
    #[test]
    #[ignore]
    fn tempo_madmom_against_oracle() {
        let all = hendou_dumps();
        if all.is_empty() {
            return;
        }
        let m = super::super::load_models();
        let net = m.net7.clone().expect("model7");
        let (mut n, mut m_ok, mut j_ok, mut b_ok) = (0, 0, 0, 0);
        let mut cpu = 0f64;
        for (name, d) in &all {
            let Some(dec) = corpus::case_pcm(&d.path) else { continue };
            let c0 = super::super::thread_cpu();
            let (raw, list, bpm) = madmom_tempo(&net, &m.dbn, &dec.samples, dec.sr, dec.frames());
            cpu += super::super::thread_cpu() - c0;
            n += 1;
            let mo = raw.len() == d.m.len() && raw.iter().zip(&d.m).all(|(a, b)| a.1 == b.1 && (a.0 - b.0).abs() < 2e-6);
            let jl: Vec<(i64, i32, i32)> = list.iter().map(|b| (b.pos, b.num_bar, b.bpm)).collect();
            let jo = jl == d.j;
            let bo = bpm == d.j_bpm as i32;
            m_ok += mo as usize;
            j_ok += jo as usize;
            b_ok += bo as usize;
            if !(mo && jo && bo) {
                let fm = raw.iter().zip(&d.m).position(|(a, b)| a.1 != b.1 || (a.0 - b.0).abs() >= 2e-6);
                let fj = jl.iter().zip(&d.j).position(|(a, b)| a != b);
                eprintln!("  {name}: M {} vs {} first {:?} | J {} vs {} first {:?} | bpm {bpm} vs {}", raw.len(), d.m.len(), fm.map(|i| (i, raw[i], d.m[i])), jl.len(), d.j.len(), fj.map(|i| (i, jl[i], d.j[i])), d.j_bpm);
            }
        }
        eprintln!("tempo madmom stage: cases {n}, raw beats identical {m_ok}, beat list identical {j_ok}, bpm identical {b_ok}; port CPU {:.2} s per case", cpu / n.max(1) as f64);
    }

    /// analyzeFocus' tempo-change half on the oracle's own input (its J list, legacy count and
    /// bpm): the final list (F: position, beat number, tempo x 100) and BA_GetBpm_Focus.
    #[test]
    #[ignore]
    fn tempo_focus_against_oracle() {
        let all = hendou_dumps();
        if all.is_empty() {
            return;
        }
        let env = super::super::uba::Uba::new(44100, &[0i16; 4096]);
        let (mut n, mut same, mut unported, mut grid_same) = (0, 0, 0, 0);
        let mut why: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
        for (name, d) in &all {
            let ml: Vec<(i32, i32, i32)> = d.j.iter().map(|b| (b.0 as i32, b.1, b.2)).collect();
            let text = std::fs::read_to_string(format!("{}/{name}.txt", std::env::var("PREPDECK_HP_ORACLE").unwrap())).unwrap_or_default();
            let total: i32 = text.lines().find(|l| l.starts_with("L bpm")).and_then(|l| l.split(" total ").nth(1)).and_then(|v| v.split_whitespace().next()?.parse().ok()).unwrap_or(0);
            let inp = super::super::focus::FocusInput { sr: d.sr as i32, total, offset: d.offset as i32, hendou: true, legacy_bpm100: d.legacy_bpm, legacy_count: d.beat_num, madmom: &ml, madmom_bpm: d.j_bpm as i32 };
            n += 1;
            match super::super::focus::analyze_focus(&env, &inp) {
                Ok(o) => {
                    let fm: Vec<(i64, i32, i32)> = o.beats.iter().map(|b| (b.0 as i64, b.1, b.2)).collect();
                    let ok = fm == d.f && o.bpm100 == d.f_bpm;
                    same += ok as usize;
                    // what the export writes (PQTZ) from the grid made of the port's list, against
                    // convertToMicrosec of the oracle's list
                    let sr = d.sr as u32;
                    let want: Vec<(u16, u16, u32)> = d.f.iter().map(|b| (b.1.clamp(1, 4) as u16, b.2 as u16, (super::super::beat_us(b.0 as i32, sr) / 1000) as u32)).collect();
                    let g = super::super::grid_from_list(&o.beats, sr);
                    let mine: Vec<(u16, u16, u32)> = g.beats(total as f64 / sr as f64).iter().map(|b| (b.beat as u16, (b.bpm * 100.0).round() as u16, (((b.time.max(0.0) * 1e6).round() as u64) / 1000) as u32)).collect();
                    let g_ok = mine.len() >= want.len() && mine[..want.len()] == want[..];
                    grid_same += g_ok as usize;
                    if !g_ok {
                        let gfirst = mine.iter().zip(&want).position(|(a, b)| a != b);
                        eprintln!("  {name}: exported beats {} vs {} first {:?}", mine.len(), want.len(), gfirst.map(|i| (i, mine[i], want[i])));
                    }
                    if !ok {
                        let first = fm.iter().zip(&d.f).position(|(a, b)| a != b);
                        eprintln!("  {name}: {} vs {} beats, bpm {} vs {}, first diff {:?}", fm.len(), d.f.len(), o.bpm100, d.f_bpm, first.map(|i| (i, fm[i], d.f[i])));
                    }
                }
                Err(e) if e == super::super::focus::NO_INPUT && d.f.is_empty() => {
                    // no list on either side: nothing exported
                    same += 1;
                    grid_same += 1;
                }
                Err(e) => {
                    unported += 1;
                    why.entry(e).or_default().push(name.clone());
                }
            }
        }
        for (e, v) in &why {
            eprintln!("  not built: {e}: {}", v.join(" "));
        }
        eprintln!("tempo focus stage: cases {n}, final list identical {same}, exported beats identical {grid_same}, not built {unported}");
    }

    /// The whole high precision analysis of each corpus case in its mode (`rb_hp::analyze`:
    /// legacy, verdict, network, seam repair, focus) against the oracle's final list, and every
    /// beat the export writes from the grid (ms, beat number, tempo x 100) against the list as
    /// convertToMicrosec turns it into milliseconds. Also process CPU against the original's
    /// time in the harness (legacy + file version of madmom::analyze + focus).
    #[test]
    #[ignore]
    fn tempo_pipeline_against_oracle() {
        let all = hendou_dumps();
        if all.is_empty() {
            return;
        }
        #[repr(C)]
        struct Rusage([i64; 18]);
        extern "C" {
            fn getrusage(who: i32, usage: *mut Rusage) -> i32;
        }
        let cpu = || {
            let mut r = Rusage([0; 18]);
            unsafe { getrusage(0, &mut r) };
            let us = |i: usize| (r.0[i + 1] & 0xffff_ffff) as f64 * 1e-6 + r.0[i] as f64;
            us(0) + us(2)
        };
        let (mut n, mut same, mut grid_same, mut fell) = (0, 0, 0, 0);
        let (mut t_mine, mut t_rb, mut w_mine) = (0f64, 0f64, 0f64);
        for (name, d) in &all {
            let Some(dec) = corpus::case_pcm(&d.path) else { continue };
            let mode: u8 = name.rsplit(".m").next().and_then(|m| m.parse().ok()).unwrap_or(4);
            let c0 = cpu();
            let w0 = std::time::Instant::now();
            let out = super::super::analyze(&dec, mode, (120.0, 0.0, 1));
            t_mine += cpu() - c0;
            w_mine += w0.elapsed().as_secs_f64();
            let text = std::fs::read_to_string(format!("{}/{name}.txt", std::env::var("PREPDECK_HP_ORACLE").unwrap())).unwrap_or_default();
            let time_of = |tag: &str| text.lines().find(|l| l.starts_with(tag)).and_then(|l| l.split(" time ").nth(1)).and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(0.0);
            t_rb += time_of("L bpm") + time_of("J bpm") + time_of("F bpm");
            n += 1;
            match out {
                Ok(o) => {
                    let fm: Vec<(i64, i32, i32)> = o.beats.iter().map(|b| (b.0 as i64, b.1, b.2)).collect();
                    let ok = fm == d.f && (o.bpm * 100.0).round() as i32 == d.f_bpm;
                    same += ok as usize;
                    // what the export writes (PQTZ) against convertToMicrosec of the oracle's list
                    let want: Vec<(u16, u16, u32)> = d.f.iter().map(|b| (b.1.clamp(1, 4) as u16, b.2 as u16, (super::super::beat_us(b.0 as i32, dec.sr) / 1000) as u32)).collect();
                    let dur = dec.duration();
                    let mine: Vec<(u16, u16, u32)> = o.grid.beats(dur).iter().map(|b| (b.beat as u16, (b.bpm * 100.0).round() as u16, (((b.time.max(0.0) * 1e6).round() as u64) / 1000) as u32)).collect();
                    let g_ok = mine.len() >= want.len() && mine[..want.len()] == want[..];
                    grid_same += g_ok as usize;
                    if !ok || !g_ok {
                        let first = fm.iter().zip(&d.f).position(|(a, b)| a != b);
                        let gfirst = mine.iter().zip(&want).position(|(a, b)| a != b);
                        eprintln!("  {name}: list {} vs {} first {:?} | grid {} vs {} first {:?}", fm.len(), d.f.len(), first.map(|i| (i, fm[i], d.f[i])), mine.len(), want.len(), gfirst.map(|i| (i, mine[i], want[i])));
                    }
                }
                Err(e) => {
                    if super::super::is_tempo_fallback(e) {
                        fell += 1;
                    } else if d.f.is_empty() && e == super::super::NO_LEGACY_BEATS {
                        same += 1;
                        grid_same += 1;
                        continue;
                    }
                    eprintln!("  {name}: no result ({e})");
                }
            }
        }
        eprintln!("tempo pipeline: cases {n}, final list identical {same}, exported beats identical {grid_same}, fell back (branch not built) {fell}");
        eprintln!("tempo pipeline time per case: port {:.2} s process CPU, {:.2} s wall; original {:.2} s wall in the harness (legacy + madmom file version + focus)", t_mine / n.max(1) as f64, w_mine / n.max(1) as f64, t_rb / n.max(1) as f64);
    }
}

#[cfg(test)]
pub(crate) mod corpus {
    /// The PCM of one corpus case: "synth:<k>" (the beat track's synthetic tempo programs) or a
    /// file path (decoded by recordbox's decoder).
    pub fn case_pcm(case: &str) -> Option<crate::audio::Decoded> {
        if let Some(k) = case.strip_prefix("synth:") {
            use super::super::super::rb_beat1::oracle::{synth, SYNTH};
            let (secs, f, seed, gap) = SYNTH[k.parse::<usize>().ok()?];
            let samples = synth(secs, &f, seed, gap);
            let frames = samples.len() / 2;
            return Some(crate::audio::Decoded { sr: 44100, samples, raw_frames: frames, damaged: 0, truncated: false });
        }
        crate::audio::decode_file(case).ok()
    }

    /// Writes one corpus case's PCM (interleaved stereo f32) for the oracle harness:
    /// PREPDECK_HP_CASE = case, PREPDECK_HP_PCMOUT = file. Prints "DUMP <sr> <frames>".
    #[test]
    #[ignore]
    fn hp_case_pcm() {
        let (Ok(case), Ok(out)) = (std::env::var("PREPDECK_HP_CASE"), std::env::var("PREPDECK_HP_PCMOUT")) else { return };
        let Some(d) = case_pcm(&case) else {
            eprintln!("DUMP ERR {case}");
            return;
        };
        let bytes: Vec<u8> = d.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&out, bytes).unwrap();
        eprintln!("DUMP\t{}\t{}", d.sr, d.frames());
    }
}
