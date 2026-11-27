//! The legacy analysis rekordbox's high precision analysis starts from.
//!
//! With high precision on, `CBeatAnalyzeThread::analyze` (0x1001528c4) switches its beat library
//! to `BeatAnalyzer_2_0` (`BALibWrapper::SetVersion(2)` when the analysis mode is 2..4), created
//! for constant tempo in Auto / Normal and for varied tempo in Dynamic, and feeds it the same
//! 16-bit samples as the standard analysis. `BeatAnalyzer_2_0::Analyzer::analyze(bool&)`
//! (0x10116be98) runs `BeatSensor`, `BeatInfoAnalyzer` and `UnitEnvelopeMaker`, and unlike
//! `BeatAnalyzer_1_0` no `UnitBeatAdjust` (2_0's `UnitBeatAdjust` only collects the envelopes
//! that `BeatInfoAnalyzer` and `analyzeFocus` read). `highPrecisionBeatAnalyze` (0x102327e50)
//! then reads `BA_GetBpm`, `BA_GetBeatNum`, `BA_GetOffsetCalc` (from the beat list) and, in Auto,
//! `GetBpmHendou`.
//!
//! The sensor, `UnitEnvelopeMaker`, gap correction and `findNearPos` of 2_0 are the 1_0
//! functions (masked byte compare; `BeatSensor::create` and `Analyzer::create` differ only in
//! the object layout), so those stages are recordbox's port of 1_0 (`rb_beat1`), run once for
//! both analyses (`rb_beat1::front`). 2_0's own `BeatInfoAnalyzer::analyze` (0x101198e4c,
//! 52 KB) is `bia2`.

pub use super::super::rb_beat1::BeatInfo;
use super::super::rb_beat1::front;

#[path = "rb_hp_bia2.rs"]
pub mod bia2;

/// `BeatAnalyzer_2_0` after `setSample`: the float BPM (`Analyzer` +72), the beat list (+88), the
/// tempo-change verdict (+45, `getBpmHendou`) and the flag `analyze(bool&)` hands back (set when
/// the tempo-change check halved a tempo above 180 BPM).
#[derive(Clone, Debug)]
pub struct Legacy2 {
    pub bpm: f32,
    pub beats: Vec<BeatInfo>,
    pub hendou: bool,
    pub halved: bool,
}

impl Legacy2 {
    /// `BA_GetBpm`: BPM x 100 rounded.
    pub fn bpm100(&self) -> i32 {
        self.bpm.mul_add(100.0, 0.5) as i32
    }
}

/// The legacy analysis of interleaved stereo at 44.1 or 48 kHz. `varied` = created for varied
/// tempo (Analysis Mode Dynamic); `low` = the low-band envelope of 2_0's `UnitBeatAdjust`
/// (`get_pdata_low`, `Uba::low`), which only exists when created for constant tempo.
pub fn analyze(samples: &[f32], sr: u32, varied: bool, low: Option<&[i16]>) -> Option<Legacy2> {
    // the reader's length minus one frame, as for every analyser (`analysis::analysed_frames`)
    let samples = &samples[..2 * crate::analysis::analysed_frames(samples.len() / 2)];
    let mut front = front(samples, sr)?;
    let r = bia2::analyze(sr as i32, &front.list, front.total, if varied { None } else { low });
    let mut grid = r.list;
    front.uem_analyze(r.bpm, &mut grid, varied);
    if grid.is_empty() {
        return None;
    }
    Some(Legacy2 { bpm: r.bpm, beats: grid, hendou: r.hendou, halved: r.halved })
}

#[cfg(test)]
mod oracle {
    use super::*;

    /// One `[ section ]` of 2_0's `BeatInfoAnalyzer` debug file (RB_BIALOG of rbbeat): its name in
    /// the port's trace and its lines (stat, 60000 / interval, f1, pos, band), or a BPM.
    fn parse_bialog(text: &str) -> Vec<(&'static str, Vec<(i32, f64, f64, i32, i32)>, f64)> {
        let names = [
            ("BPMの一致しない値削除後のデータ", "BPM mismatch removed"),
            ("近い値調整後のデータ", "near values adjusted"),
            ("代表BPM", "representative BPM"),
            ("アジャスト前区間情報", "segments before adjust"),
            ("アジャスト後区間情報", "segments after adjust"),
            ("区間をBPM値で分割したデータ", "split by BPM"),
            ("位置補正後のデータ", "position corrected"),
            ("連続しないBPM区間削除後のデータ", "non-continuous BPM runs removed"),
            ("8個以上連続しない値削除後のデータ", "runs under 8 removed"),
        ];
        let mut out: Vec<(&'static str, Vec<(i32, f64, f64, i32, i32)>, f64)> = vec![];
        let mut cur: Option<usize> = None;
        for l in text.split('\n') {
            let l = l.trim_end_matches('\r');
            if let Some(t) = l.strip_prefix("[ ").and_then(|x| x.strip_suffix(" ]")) {
                cur = names.iter().find(|n| n.0 == t).map(|n| {
                    out.push((n.1, vec![], 0.0));
                    out.len() - 1
                });
                continue;
            }
            let Some(k) = cur else { continue };
            let w: Vec<&str> = l.split('\t').collect();
            if w.len() == 5 {
                if let (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e)) = (w[0].parse(), w[1].parse(), w[2].parse(), w[3].parse(), w[4].parse()) {
                    out[k].1.push((a, b, c, d, e));
                }
            } else if w.len() == 1 && out[k].0 == "representative BPM" {
                if let Ok(b) = w[0].trim().parse() {
                    out[k].2 = b;
                }
            }
        }
        out
    }

    fn parse_list(text: &str) -> (f32, Vec<BeatInfo>) {
        let mut bpm = 0f32;
        let mut v = vec![];
        for l in text.lines() {
            if let Some(b) = l.strip_prefix("# bpm ") {
                bpm = b.trim().parse().unwrap_or(0.0);
                continue;
            }
            let w: Vec<&str> = l.split_whitespace().collect();
            if w.len() == 6 {
                v.push(BeatInfo { pos: w[0].parse().unwrap(), f1: w[1].parse().unwrap(), iv: w[2].parse().unwrap(), stat: w[3].parse().unwrap(), band: w[4].parse().unwrap(), flag: (w[5].parse::<f32>().unwrap_or(0.0).to_bits() & 0xff) as u8 });
            }
        }
        (bpm, v)
    }

    struct TmpDir(std::path::PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 2_0's `BeatInfoAnalyzer` (UnitEnvelopeMaker skipped) and the whole library (API values)
    /// against rekordbox's own routines run on the same samples (rbbeat ver 2), constant and
    /// varied creation. Stage by stage through the debug sections. PREPDECK_RBBEAT = harness dir
    /// (rbbeat, fixups.txt), PREPDECK_RBSLICE = slice, PREPDECK_RBTMP = scratch for the PCM,
    /// PREPDECK_GT = track list (first column) or PREPDECK_AUDIO = '|'-separated files,
    /// PREPDECK_HP_ONLY = track numbers, PREPDECK_RBVARIED = 0 / 1 (default both).
    #[test]
    #[ignore]
    fn legacy2_oracle() {
        let (Ok(dir), Ok(slice)) = (std::env::var("PREPDECK_RBBEAT"), std::env::var("PREPDECK_RBSLICE")) else { return };
        let tmp_root = std::path::PathBuf::from(std::env::var("PREPDECK_RBTMP").unwrap_or_else(|_| "/tmp".into()));
        let mut paths: Vec<String> = std::env::var("PREPDECK_GT").ok().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| t.lines().filter_map(|l| l.split('\t').next()).map(String::from).collect()).unwrap_or_default();
        paths.extend(std::env::var("PREPDECK_AUDIO").unwrap_or_default().split('|').filter(|s| !s.is_empty()).map(String::from));
        // PREPDECK_RBSYNTH=1: the beat track's synthetic tempo programs (steps, ramps, jitter, a
        // silent break, slow and fast tempi), plus PREPDECK_HP_SYNTH2=1: longer step programs
        if std::env::var("PREPDECK_RBSYNTH").is_ok() {
            paths.extend((0..super::super::super::rb_beat1::oracle::SYNTH.len()).map(|k| format!("synth:{k}")));
        }
        let only: Vec<usize> = std::env::var("PREPDECK_HP_ONLY").map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
        let modes: Vec<bool> = match std::env::var("PREPDECK_RBVARIED").as_deref() {
            Ok("0") => vec![false],
            Ok("1") => vec![true],
            _ => vec![false, true],
        };
        let verbose = std::env::var("PREPDECK_HP_VERBOSE").is_ok();
        let (mut n, mut stage_ok, mut bia_ok, mut api_ok, mut hendou_ok) = ([0usize; 2], [0usize; 2], [0usize; 2], [0usize; 2], [0usize; 2]);
        for (k, p) in paths.iter().enumerate() {
            if !only.is_empty() && !only.contains(&k) {
                continue;
            }
            let dec = if let Some(k) = p.strip_prefix("synth:") {
                let (secs, f, seed, gap) = super::super::super::rb_beat1::oracle::SYNTH[k.parse::<usize>().unwrap()];
                let samples = super::super::super::rb_beat1::oracle::synth(secs, &f, seed, gap);
                let frames = samples.len() / 2;
                crate::audio::Decoded { sr: 44100, samples, raw_frames: frames, damaged: 0, truncated: false }
            } else {
                let Ok(d) = crate::audio::decode_file(p) else { continue };
                d
            };
            if !(dec.sr == 44100 || dec.sr == 48000) {
                continue;
            }
            let tmp = TmpDir(tmp_root.join(format!("leg2-{}-{k}", std::process::id())));
            std::fs::create_dir_all(&tmp.0).unwrap();
            let pcm = tmp.0.join("pcm.f32");
            std::fs::write(&pcm, dec.samples.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
            let pcm16 = super::super::uba::to_i16(&dec.samples);
            let env_ = super::super::uba::Uba::new(dec.sr as i32, &pcm16);
            let name = p.rsplit('/').next().unwrap_or("");
            for &varied in &modes {
                let m = varied as usize;
                let log = tmp.0.join("bia.log");
                let lst = tmp.0.join("bia.list");
                let run = |extra: &[(&str, &std::path::Path)], skip_uem: bool| {
                    let mut c = std::process::Command::new(format!("{dir}/rbbeat"));
                    c.current_dir(&dir).env("RB_TRUNC", "1").env("RB_ALL", "1");
                    if skip_uem {
                        c.env("RB_SKIP_UEM", "1");
                    }
                    for (a, b) in extra {
                        c.env(a, b);
                    }
                    c.args([slice.as_str(), "fixups.txt", pcm.to_str().unwrap(), &dec.sr.to_string(), "2", if varied { "1" } else { "0" }]).output().unwrap()
                };
                run(&[("RB_BIALOG", &log), ("RB_DUMP_LIST", &lst)], true);
                let api_out = run(&[], false);
                let want_log = parse_bialog(&std::fs::read_to_string(&log).unwrap_or_default());
                let (want_bpm, want_list) = parse_list(&std::fs::read_to_string(&lst).unwrap_or_default());
                let so = String::from_utf8_lossy(&api_out.stdout).to_string();
                let mut api = so.lines().skip_while(|l| !l.starts_with("bpm "));
                let head: Vec<String> = api.next().unwrap_or("").split_whitespace().map(String::from).collect();
                let want_api_bpm: i32 = head.get(1).and_then(|x| x.parse().ok()).unwrap_or(-1);
                let want_hendou = head.get(9).map(|x| x == "1").unwrap_or(false);
                let want_api: Vec<(i32, i32)> = api.filter_map(|l| {
                    let w: Vec<i32> = l.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
                    (w.len() == 3).then(|| (w[0], w[1]))
                }).collect();
                // the port
                let mut fr = front(&dec.samples, dec.sr).unwrap();
                super::super::super::rb_beat1::trace_on();
                let r = bia2::analyze(dec.sr as i32, &fr.list, fr.total, if varied { None } else { Some(&env_.low) });
                let trace = super::super::super::rb_beat1::trace_take();
                n[m] += 1;
                // stages
                let mut first_bad: Option<String> = None;
                for (i, (wn, wl, wb)) in want_log.iter().enumerate() {
                    let Some((gn, gl, gb)) = trace.get(i) else {
                        first_bad.get_or_insert(format!("section {i} {wn} missing in the port ({} vs {} sections)", trace.len(), want_log.len()));
                        break;
                    };
                    if wn != gn {
                        first_bad.get_or_insert(format!("section {i}: {wn} vs port {gn}"));
                        break;
                    }
                    if *wn == "representative BPM" {
                        if (*wb - *gb as f64).abs() > 1e-5 * wb.abs().max(1.0) {
                            first_bad.get_or_insert(format!("{wn}: {wb} vs port {gb}"));
                            break;
                        }
                        continue;
                    }
                    let same = |a: &(i32, f64, f64, i32, i32), b: &BeatInfo| {
                        let bb = 60000.0f32 / b.iv;
                        a.0 == b.stat && a.3 == b.pos && a.4 == b.band && (a.1 - bb as f64).abs() <= 2e-6 * a.1.abs().max(1.0) && (a.2 - b.f1 as f64).abs() < 2e-6
                    };
                    if wl.len() != gl.len() || wl.iter().zip(gl).any(|(a, b)| !same(a, b)) {
                        let j = wl.iter().zip(gl).position(|(a, b)| !same(a, b));
                        first_bad.get_or_insert(format!("{wn} (section {i}): {} vs port {} entries, first diff {:?}", wl.len(), gl.len(), j.map(|j| (j, wl[j], gl[j]))));
                        break;
                    }
                }
                if want_log.len() != trace.len() && first_bad.is_none() {
                    first_bad = Some(format!("sections {} vs port {}", want_log.len(), trace.len()));
                }
                stage_ok[m] += first_bad.is_none() as usize;
                let bia_same = r.bpm.to_bits() == want_bpm.to_bits() && r.list == want_list;
                bia_ok[m] += bia_same as usize;
                let mut grid = r.list.clone();
                fr.uem_analyze(r.bpm, &mut grid, varied);
                let mine_bpm = r.bpm.mul_add(100.0, 0.5) as i32;
                let mine_api: Vec<(i32, i32)> = grid.iter().map(|b| (b.pos, b.stat)).collect();
                let api_same = mine_bpm == want_api_bpm && mine_api == want_api;
                api_ok[m] += api_same as usize;
                hendou_ok[m] += (r.hendou == want_hendou) as usize;
                if verbose || first_bad.is_some() || !bia_same || !api_same || r.hendou != want_hendou {
                    let fd = r.list.iter().zip(&want_list).position(|(a, b)| a != b);
                    eprintln!(
                        "  #{k} {} stages: {} | BIA bpm {} vs {} list {} vs {} first diff {:?} | API bpm {mine_bpm} vs {want_api_bpm} beats {} vs {} | hendou {} vs {want_hendou} :: {name}",
                        if varied { "varied" } else { "const " },
                        first_bad.as_deref().unwrap_or("ok"),
                        r.bpm,
                        want_bpm,
                        r.list.len(),
                        want_list.len(),
                        fd.map(|i| (i, r.list[i], want_list[i])),
                        mine_api.len(),
                        want_api.len(),
                        r.hendou
                    );
                }
            }
        }
        for m in 0..2 {
            if n[m] > 0 {
                eprintln!("legacy 2_0 {}: tracks {}, every stage {}, BeatInfoAnalyzer output (bits) {}, API bpm + list {}, hendou {}", if m == 1 { "varied" } else { "constant" }, n[m], stage_ok[m], bia_ok[m], api_ok[m], hendou_ok[m]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// 2_0's verdict on two of the synthetic tempo programs, as rekordbox's own routine gives it
    /// (legacy2_oracle, PREPDECK_RBSYNTH=1): constant 124 BPM -> not varied, 124.00; a step from
    /// 128 to 86 BPM -> varied, the varied pass' 128.00.
    #[test]
    fn tempo_change_verdict_on_synthetic_programs() {
        use super::super::super::rb_beat1::oracle::{synth, SYNTH};
        for (k, want_hendou, want_bpm100) in [(0usize, false, 12400), (3, true, 12800)] {
            let (secs, f, seed, gap) = SYNTH[k];
            let samples = synth(secs, &f, seed, gap);
            let env = super::super::uba::Uba::new(44100, &super::super::uba::to_i16(&samples[..2 * crate::analysis::analysed_frames(samples.len() / 2)]));
            let l = super::analyze(&samples, 44100, false, Some(&env.low)).unwrap();
            assert_eq!((l.hendou, l.bpm100()), (want_hendou, want_bpm100), "program {k}");
            // created for varied tempo there is no envelope: always varied
            let v = super::analyze(&samples, 44100, true, None).unwrap();
            assert!(v.hendou, "program {k} varied");
        }
    }
}
