//! rekordbox's high precision BeatGrid analysis ("Use high precision BeatGrid analysis",
//! analysis modes 2 / 3 / 4 = Auto / Normal / Dynamic), ported from rekordbox 7.2.19.
//!
//! `AnalyzeJobBeatDetector::highPrecisionBeatAnalyze` (0x102327e50):
//! 1. the legacy `BeatAnalyzer_2_0` analysis of the whole track gives a BPM, a beat count and a
//!    start offset (`Analyzer::getOffsetCalc`);
//! 2. `madmom::analyze` (0x10230a820) runs madmom's downbeat network (`net`) on a 60 s window from
//!    that offset (the whole track when the tempo may change), decodes it with a bar-pointer DBN
//!    (`dbn`) and turns the beat times into a beat list (`madmom::get_beat_list`, `get_bpm`);
//! 3. `Analyzer::analyzeFocus` merges that list with the legacy analysis into the final grid
//!    (`focus`, its bar-phase vote in `vote`), and `convertToMicrosec` stores every beat
//!    (`grid_from_list`).
//!
//! The legacy analysis is `legacy` (BeatAnalyzer_2_0, with its tempo-change verdict that Auto
//! follows). The tempo-change path (Dynamic, and Auto when 2_0 judges the tempo varied) is
//! `tempo` (network over the whole track, seam repair) and `focus2` (analyzeFocus' second
//! half); where it reaches a branch not built yet the standard Dynamic grid is kept and the
//! reason is reported (`take_notes`). Other rates than 44.1 / 48
//! kHz go through rekordbox's converter (`rb_reader::for_analysis`) first, as in rekordbox.

#[path = "rb_hp_net.rs"]
pub mod net;
#[path = "rb_hp_dbn.rs"]
pub mod dbn;
#[path = "rb_hp_uba.rs"]
pub mod uba;

#[path = "rb_hp_focus.rs"]
pub mod focus;
#[path = "rb_hp_vote.rs"]
pub mod vote;
#[path = "rb_hp_legacy.rs"]
pub mod legacy;
#[path = "rb_hp_tempo.rs"]
pub mod tempo;
#[path = "rb_hp_focus2.rs"]
pub mod focus2;

use crate::model::BeatGrid;
use std::sync::{Arc, Mutex, OnceLock};
use uba::Uba;

/// Analysis modes as rekordbox numbers them (`CBeatAnalyzeThread::analyze`, field +14544).
pub const MODE_AUTO: u8 = 2;
#[allow(dead_code)]
pub const MODE_NORMAL: u8 = 3;
pub const MODE_DYNAMIC: u8 = 4;

/// The legacy analysis rekordbox runs first (`BeatAnalyzer_2_0`): its BPM and beat positions
/// (`legacy::analyze`; `legacy_from_grid` makes a stand-in from the standard grid when that
/// gives nothing, or with PREPDECK_HP_LEGACY=standard). rekordbox uses the BPM for its octave
/// and integer-snap rules and the beats to pick the 60 s window.
#[derive(Clone, Debug)]
pub struct Legacy {
    /// BPM as the legacy library stores it (float, the same value x100 rounded is `BA_GetBpm`).
    pub bpm: f32,
    /// Beat positions in samples from the first beat on.
    pub beats: Vec<i32>,
}

pub fn legacy_from_grid(bpm: f64, first_beat: f64, sr: u32, total: usize) -> Option<Legacy> {
    if !(bpm > 0.0) {
        return None;
    }
    let bpm100 = (bpm * 100.0).round();
    let b = (bpm100 / 100.0) as f32;
    let period = 60.0 / bpm100 * 100.0;
    let mut t = first_beat;
    while t - period >= 0.0 {
        t -= period;
    }
    while t < 0.0 {
        t += period;
    }
    let mut beats = vec![];
    while ((t * sr as f64) as usize) < total {
        beats.push((t * sr as f64) as i32);
        t += period;
    }
    if beats.is_empty() {
        return None;
    }
    Some(Legacy { bpm: b, beats })
}

/// `Analyzer::getOffsetCalc` (0x101182100): where the 60 s window of the network starts.
/// Integrates the low band over half a beat at every legacy beat, normalises, and finds the
/// first loud section (thresholds 0.2 / 0.4 / 0.15 on the integrals and their differences);
/// the offset is 4 beats before it, at `sr * 60 * (index - 4) / bpm` samples.
pub fn get_offset_calc(env: &Uba, sr: i32, legacy: &Legacy) -> i32 {
    let n = legacy.beats.len();
    if n == 0 {
        return 0;
    }
    let bpm = legacy.bpm;
    let mut v19 = vec![0f64; n];
    let mut v20 = vec![0f64; n];
    let v21 = n - 1;
    let mut v22 = 0f64;
    if n >= 2 {
        for i in 0..v21 {
            let mut x = env.calc_low_integ(bpm, legacy.beats[i]);
            if i != 0 && x < 0.0 {
                x = v19[i - 1];
            }
            v19[i] = x;
        }
        for &x in &v19[..v21] {
            v22 = v22.max(x);
        }
    }
    let mut v42: i64 = -1;
    let mut v68: i64;
    'done: {
        if v22.abs() > 2.22044605e-16 {
            if n <= 1 {
                v20[0] = 0.0;
                v68 = v42;
                break 'done;
            }
            let inv = 1.0 / v22;
            for x in v19[..v21].iter_mut() {
                *x *= inv;
            }
        }
        v20[0] = 0.0;
        if n < 3 {
            v68 = -1;
            break 'done;
        }
        for k in 1..v21 {
            v20[k] = v19[k] - v19[k - 1];
        }
        let g = |v: &Vec<f64>, k: usize| v.get(k).copied().unwrap_or(0.0);
        let mut v64 = false;
        let mut v65 = 0i64;
        let mut v66 = 0i64;
        v68 = -1;
        let mut v63 = 0usize;
        loop {
            let i = v63;
            // labels of the original: 39 (quiet: reset), 40 (next), 52 (loud), 64, 65, 69
            enum Next {
                L39,
                L40,
                L52,
                L64,
                L65,
                L69,
                L76,
            }
            let mut next;
            if v63 != 0 {
                let v69 = g(&v19, i);
                if !v64 {
                    if v69 <= 0.2 {
                        v64 = false;
                        next = Next::L64;
                    } else if g(&v20, i) <= 0.15 {
                        v64 = false;
                        next = if g(&v19, i) >= 0.2 { Next::L69 } else { Next::L65 };
                    } else {
                        if v65 > 32 || v68 < 0 {
                            v42 = v63 as i64;
                        }
                        v65 = 0;
                        v68 = v63 as i64;
                        next = if v66 > 11 { Next::L76 } else { Next::L52 };
                    }
                } else if v69 <= 0.4 && (v69 <= 0.2 || g(&v19, i + 1) <= 0.2 || g(&v19, i + 2) <= 0.2) {
                    v64 = true;
                    next = if g(&v19, i) >= 0.2 { Next::L69 } else { Next::L65 };
                } else {
                    next = if v66 > 11 { Next::L76 } else { Next::L52 };
                }
            } else if g(&v19, 0) <= 0.4 {
                next = Next::L64;
            } else {
                v68 = 0;
                v42 = 0;
                next = if v66 > 11 { Next::L76 } else { Next::L52 };
            }
            loop {
                match next {
                    Next::L64 => next = if g(&v19, i) >= 0.2 { Next::L69 } else { Next::L65 },
                    Next::L52 => {
                        v66 += 1;
                        v64 = true;
                        next = if g(&v19, i) < 0.2 { Next::L65 } else { Next::L69 };
                    }
                    Next::L65 => next = if g(&v19, i + 1) < 0.2 && g(&v19, i + 2) < 0.2 { Next::L39 } else { Next::L69 },
                    Next::L69 => next = if g(&v20, i) < -0.4 && g(&v20, i + 1) < 0.2 && g(&v20, i + 2) < 0.2 { Next::L39 } else { Next::L40 },
                    Next::L39 => {
                        v66 = 0;
                        v64 = false;
                        v65 += 1;
                        next = Next::L40;
                    }
                    Next::L40 | Next::L76 => break,
                }
            }
            if matches!(next, Next::L76) {
                break 'done;
            }
            v63 += 1;
            if n - 2 == v63 {
                break;
            }
        }
        if v66 <= 12 {
            v68 = v42;
        }
    }
    let v71 = v68.max(0);
    let v72 = (sr as f64 * 60.0 * (v71 - 4) as f64 / bpm as f64) as i32;
    v72.max(0)
}

/// The window `madmom::analyze` (file version, 0x10230a820) runs the network on:
/// 60 s from the offset (the last 60 s when that runs past the end), the whole track when the
/// tempo may change. Returns (start, length) in frames.
pub fn madmom_window(total: i64, sr: i64, offset: i64, hendou: bool) -> (i64, i64) {
    let v62 = total;
    let mut v63 = v62 - 1;
    let mut v64 = 60 * sr;
    let mut v65 = v62 - 1 - v64;
    let v66 = if v62 - 1 > v64 { v64 } else { v62 - 1 };
    if v62 - 1 <= v64 {
        v65 = 0;
    }
    let v67;
    if v62 - 1 - offset > v64 {
        v67 = offset;
    } else {
        v64 = v66;
        v67 = v65;
    }
    let v68;
    if hendou {
        v68 = 0;
    } else {
        v63 = v64;
        v68 = v67;
    }
    if v62 != 0 {
        (v68, v63)
    } else {
        (0, 0)
    }
}

/// Times of the network's beats at other rates than 44.1 kHz: rekordbox scales them by
/// 44100 / sr, the vectorised part as `t * (44100 / sr)`, the last 1-4 values as
/// `(t * 44100) * (1 / sr)`.
pub fn scale_times(beats: &mut [(f32, i32)], sr: i32) {
    if sr == 44100 || beats.is_empty() {
        return;
    }
    let n = beats.len();
    let v80 = sr as f32;
    let tail = if n > 4 {
        let r = n & 3;
        if r == 0 { 4 } else { r }
    } else {
        n
    };
    let k = 44100.0f32 / v80;
    let inv = 1.0f32 / v80;
    for (i, b) in beats.iter_mut().enumerate() {
        if i < n - tail {
            b.0 *= k;
        } else {
            b.0 = (b.0 * 44100.0) * inv;
        }
    }
}

struct Models {
    net8: Option<Arc<net::Net>>,
    net7: Option<Arc<net::Net>>,
    dbn: Arc<dbn::Dbn>,
}

fn models() -> &'static Mutex<Option<Arc<Models>>> {
    static M: OnceLock<Mutex<Option<Arc<Models>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(None))
}

fn load_models() -> Arc<Models> {
    let mut g = models().lock().unwrap();
    if let Some(m) = g.as_ref() {
        return m.clone();
    }
    let dir = net::models_dir();
    let m = Arc::new(Models {
        net8: net::Net::load(&dir.join("model8")).ok().map(Arc::new),
        net7: net::Net::load(&dir.join("model7")).ok().map(Arc::new),
        dbn: Arc::new(dbn::Dbn::rekordbox()),
    });
    *g = Some(m.clone());
    m
}

/// True when rekordbox's beat models are installed and readable.
pub fn available() -> bool {
    load_models().net8.is_some()
}

/// The network + DBN + beat list stage for one window of the decode.
pub fn madmom_stage(samples: &[f32], sr: u32, start: usize, len: usize, hendou: bool) -> Option<(Vec<dbn::ListBeat>, f64, Vec<(f32, i32)>)> {
    let m = load_models();
    let net = if hendou { m.net7.as_ref().or(m.net8.as_ref())? } else { m.net8.as_ref()? };
    let mono: Vec<f32> = (start..start + len).map(|i| (samples[2 * i + 1] + samples[2 * i]) * 0.5).collect();
    let act = net.run(&mono);
    let mut beats = m.dbn.track(&act);
    scale_times(&mut beats, sr as i32);
    let list = dbn::get_beat_list(&beats, len as i64, sr as i32, 4);
    let bpm = dbn::get_bpm(&list);
    Some((list, bpm, beats))
}

/// Everything the high precision analysis produced for one track.
#[allow(dead_code)]
pub struct HpOutcome {
    pub bpm: f64,
    pub grid: BeatGrid,
    /// Beat positions (samples), beat numbers and bpm x 100 as rekordbox's `BA_GetBeatPos_Focus`.
    pub beats: Vec<(i32, i32, i32)>,
    pub offset: i32,
    pub window: (usize, usize),
    /// The tempo-change path was taken (convertToMicrosec then keeps the grid as it is).
    pub hendou: bool,
}

/// Why the high precision analysis gave no result (the caller then keeps the standard one).
pub const NOT_44_48: &str = "the audio's sample rate could not be converted to 44.1 or 48 kHz";
pub const NO_MODELS: &str = "rekordbox's beat models were not found";
pub const NO_GRID: &str = "the standard analysis found no beats to start from";
pub const NO_LEGACY_BEATS: &str = "rekordbox's first analysis (BeatAnalyzer_2_0) found no beats, so its high precision analysis gives no grid";
pub const NO_NET_BEATS: &str = "the network found no beats";

/// The reasons the tempo-change path gives up in a branch that is not built yet.
pub fn is_tempo_fallback(why: &str) -> bool {
    why == focus2::UNPORTED_CLASS || why == focus2::UNPORTED_HALFBEAT || why == focus2::UNPORTED_SHORT
}

/// rekordbox's high precision analysis of decoded audio. `legacy` is the standard analysis'
/// tempo and first beat, used only when `legacy::analyze` gives nothing. The error says
/// why there is no result.
pub fn analyze(dec: &crate::audio::Decoded, mode: u8, legacy: (f64, f64, u8)) -> Result<HpOutcome, &'static str> {
    // other rates: the whole pipeline runs on what rekordbox's reader delivers at the analysis
    // rate (multiples of 8 kHz at 48 kHz, others at 44.1 kHz) through its converter; madmom's
    // file reader is opened at that rate too (`PcmReaderTemplate(path, rate, ...)` in
    // madmom::analyze 0x10230a820). `analyze_decoded` has usually converted already.
    let conv;
    let dec = if dec.sr != 44100 && dec.sr != 48000 {
        conv = super::rb_reader::for_analysis(dec).ok_or(NOT_44_48)?;
        &conv
    } else {
        dec
    };
    let sr = dec.sr;
    if !available() {
        return Err(NO_MODELS);
    }
    let total = dec.frames();
    // BeatAnalyzer_2_0 (and with it the envelopes of its UnitBeatAdjust and analyzeFocus' track
    // length) gets the analysis thread's samples: the reader's length minus one frame; madmom's
    // own reader sees the whole file (`madmom_window`)
    let ana = super::analysed_frames(total);
    let times = std::env::var("PREPDECK_HP_TIMES").is_ok();
    let mut tt = vec![("start", thread_cpu())];
    let pcm16 = uba::to_i16(&dec.samples[..2 * ana]);
    let env = Uba::new(sr as i32, &pcm16);
    tt.push(("envelopes", thread_cpu()));
    // the legacy analysis (BeatAnalyzer_2_0, `legacy.rs`); PREPDECK_HP_LEGACY=standard uses
    // recordbox's standard grid in its place (as before)
    let leg2 = if std::env::var("PREPDECK_HP_LEGACY").as_deref() == Ok("standard") { None } else { legacy::analyze(&dec.samples, sr, mode == MODE_DYNAMIC, Some(&env.low)) };
    // highPrecisionBeatAnalyze: Dynamic always takes the tempo-change path, Auto when 2_0's
    // BeatInfoAnalyzer judged the tempo varied (GetBpmHendou), Normal never
    let auto_hendou = leg2.as_ref().map(|l| l.hendou).unwrap_or(false);
    let (legacy, bpm100) = match leg2 {
        Some(l) => (Legacy { bpm: l.bpm, beats: l.beats.iter().map(|b| b.pos).collect() }, l.bpm100()),
        // BeatAnalyzer_2_0 found no beats: analyzeFocus then gets a count of 0 and returns
        // nothing (BA_GetBeatNum_Focus 0), as for short loops in Dynamic
        None if std::env::var("PREPDECK_HP_LEGACY").as_deref() != Ok("standard") => return Err(NO_LEGACY_BEATS),
        None => {
            let l = legacy_from_grid(legacy.0, legacy.1, sr, total).ok_or(NO_GRID)?;
            let b = (l.bpm as f64 * 100.0).round() as i32;
            (l, b)
        }
    };
    tt.push(("legacy", thread_cpu()));
    let offset = get_offset_calc(&env, sr as i32, &legacy);
    let hendou = match mode {
        MODE_DYNAMIC => true,
        MODE_AUTO => auto_hendou,
        _ => false,
    };
    let (start, len) = madmom_window(total as i64, sr as i64, offset as i64, hendou);
    let (list, mbpm) = if hendou {
        // the tempo-change path: model7 over the whole track in pieces, the seam repair
        // (`tempo.rs`), then analyzeFocus' second half (`focus2.rs`)
        let m = load_models();
        let net = m.net7.as_ref().ok_or(NO_MODELS)?;
        let (_, list, b) = tempo::madmom_tempo(net, &m.dbn, &dec.samples, sr, total);
        (list, b as f64)
    } else {
        let (list, mbpm, _) = madmom_stage(&dec.samples, sr, start as usize, len as usize, hendou).ok_or(NO_MODELS)?;
        (list, mbpm)
    };
    tt.push(("network+dbn", thread_cpu()));
    let ml: Vec<(i32, i32, i32)> = list.iter().map(|b| (b.pos as i32, b.num_bar, b.bpm)).collect();
    let input = focus::FocusInput { sr: sr as i32, total: ana as i32, offset, hendou, legacy_bpm100: bpm100, legacy_count: legacy.beats.len() as i32, madmom: &ml, madmom_bpm: mbpm as i32 };
    // a reading past the end of a list in a branch not yet seen must not take the analysis down
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| focus::analyze_focus(&env, &input))).unwrap_or(Err(focus::OUT_OF_RANGE))?;
    tt.push(("focus", thread_cpu()));
    if times {
        eprintln!("HPTIMES {}", tt.windows(2).map(|w| format!("{} {:.3}", w[1].0, w[1].1 - w[0].1)).collect::<Vec<_>>().join(" "));
    }
    if out.beats.is_empty() {
        return Err(NO_NET_BEATS);
    }
    let beats = out.beats;
    let bpm = out.bpm100 as f64 / 100.0;
    let grid = grid_from_list(&beats, sr);
    Ok(HpOutcome { bpm, grid, beats, offset, window: (start as usize, len as usize), hendou })
}

/// CPU seconds of the calling thread (for PREPDECK_HP_TIMES; worker threads of a stage are
/// not counted).
fn thread_cpu() -> f64 {
    #[repr(C)]
    struct Ts {
        s: i64,
        n: i64,
    }
    extern "C" {
        fn clock_gettime(id: u32, t: *mut Ts) -> i32;
    }
    let mut t = Ts { s: 0, n: 0 };
    // CLOCK_THREAD_CPUTIME_ID = 16 on macOS
    unsafe { clock_gettime(16, &mut t) };
    t.s as f64 + t.n as f64 * 1e-9
}

/// Microseconds of a beat position as `AnalyzeJobBeatDetector::convertToMicrosec` (0x102328b20)
/// computes them: `1000000 * pos / sr` in integers; the analysis file keeps `us / 1000` (PQTZ).
pub fn beat_us(pos: i32, sr: u32) -> i64 {
    1_000_000i64 * pos as i64 / sr as i64
}

/// The grid for rekordbox's beat list (position, beat number, tempo x 100 per beat): rekordbox
/// stores every beat, recordbox a grid. An anchor at the first beat and wherever the running
/// grid would give a beat another whole millisecond (or the listed tempo changes), so every
/// beat the export writes (`BeatGrid::beats`, ms = round(t * 1e6) / 1000) has rekordbox's
/// millisecond, beat number and tempo. Anchors sit on the beats' own microseconds.
pub fn grid_from_list(beats: &[(i32, i32, i32)], sr: u32) -> BeatGrid {
    let mut anchors: Vec<crate::model::GridAnchor> = vec![];
    let ms_of = |t: f64| ((t.max(0.0) * 1e6).round() as i64) / 1000;
    // `pt`: where `BeatGrid::beats` puts this beat (it adds the period beat by beat)
    let (mut pt, mut a_bpm100, mut pn) = (0f64, 0i32, 0i32);
    let mut force = false;
    for (j, &(pos, num, bpm100)) in beats.iter().enumerate() {
        let us = beat_us(pos, sr);
        let t = us as f64 / 1e6;
        let num = num.clamp(1, 4);
        if j > 0 {
            pt += 60.0 / (a_bpm100 as f64 / 100.0).max(1.0);
            pn = pn % 4 + 1;
        }
        let fits = !force && j > 0 && bpm100 == a_bpm100 && bpm100 > 0 && num == pn && ms_of(pt) == us / 1000;
        force = false;
        let mut bpm = if bpm100 > 0 { bpm100 as f64 / 100.0 } else { anchors.last().map(|a| a.bpm).unwrap_or(120.0) };
        // `BeatGrid::beats` lays a beat every period from an anchor until half a period before
        // the next one: with the tempo changing per beat (the tempo-change path) a gap to the
        // next listed beat of 1.5 periods or more would add a beat, one of half a period or
        // less would drop this one. Such a beat gets its own anchor at the nearest tempo that
        // keeps the list (its written tempo then differs from rekordbox's).
        let gap = beats.get(j + 1).map(|b| beat_us(b.0, sr) as f64 / 1e6 - t);
        let p = 60.0 / bpm.max(1.0);
        let bad = match gap {
            Some(g) if g > 0.0 && g >= 1.5 * p - 1e-9 => {
                bpm = (9000.0 / g).floor() / 100.0;
                true
            }
            Some(g) if g > 0.0 && g <= 0.5 * p + 1e-9 => {
                bpm = ((3000.0 / g).floor() + 1.0) / 100.0;
                true
            }
            _ => false,
        };
        if !fits || bad {
            anchors.push(crate::model::GridAnchor { time: t, bpm, beat: num as u8 });
            (pt, a_bpm100, pn) = (t, (bpm * 100.0).round() as i32, num);
            force = bad;
        }
    }
    BeatGrid { anchors }
}

/// The analysis mode the next analyses use: 0 = high precision off, 2 / 3 / 4 = Auto / Normal /
/// Dynamic. Set from the preferences (`App::analysis_settings`); `PREPDECK_HP_MODE` overrides it.
static MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// rekordbox's setting `trackHiPrecisionBeatAnalysisMode` (the Analysis Mode while high precision
/// is on; `AnalysisTrackAnalysisModeComponent2`, 0x100505568): 0 Normal, 1 Dynamic, 2 Auto. Its
/// getter defaults to 2. Kept apart from the Analysis Mode used with high precision off, as
/// rekordbox does (`trackAnalysisMode`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HpPrefs {
    pub mode: u8,
}

impl Default for HpPrefs {
    fn default() -> Self {
        HpPrefs { mode: 2 }
    }
}

/// The combo box items (`ItemData2` table built in `_GLOBAL__sub_I_AnalysisTrackAnalysisModeComponent2.cpp`
/// 0x100507148): name, setting value, item id. "Auto " (with its space, as the key in the
/// locale files) is only listed while high precision is on.
pub const MODE_ITEMS: [(&str, u8, i32); 3] = [("Normal", 0, 1), ("Dynamic", 1, 2), ("Auto", 2, 3)];

/// The tooltip of "Track Analysis Mode" (built in `lookAndFeelChanged` 0x100505e04).
pub const MODE_TOOLTIP: &str = "When selecting auto while using high precision BeatGrid analysis, \nautomatically switches to an optimal analysis mode based on the track.\nFor tracks with a constant tempo : Normal\nFor tracks that change tempo : Dynamic";

/// Preferences -> mode (`CBeatAnalyzeThread::analyze` field +14544): high precision off 0, on
/// 3 / 4 / 2 for its Analysis Mode Normal / Dynamic / Auto.
pub fn set_mode(high_precision: bool, hp_mode: u8) {
    let m = if !high_precision {
        0
    } else {
        match hp_mode {
            0 => MODE_NORMAL,
            1 => MODE_DYNAMIC,
            _ => MODE_AUTO,
        }
    };
    MODE.store(m, std::sync::atomic::Ordering::Relaxed);
}

/// Reasons the high precision analysis fell back to the standard one since the last
/// [`take_notes`], with a count each.
fn notes() -> &'static Mutex<std::collections::BTreeMap<&'static str, usize>> {
    static N: OnceLock<Mutex<std::collections::BTreeMap<&'static str, usize>>> = OnceLock::new();
    N.get_or_init(|| Mutex::new(Default::default()))
}

fn note(why: &'static str) {
    eprintln!("high precision BeatGrid: {why}; standard analysis used");
    if let Ok(mut n) = notes().lock() {
        *n.entry(why).or_insert(0) += 1;
    }
}

/// One line per reason for the status line / a toast, e.g. "High precision BeatGrid analysis
/// was not used for 2 tracks: ...; their standard grid was kept." Empties the list.
pub fn take_notes() -> Vec<String> {
    let Ok(mut n) = notes().lock() else { return vec![] };
    let out = n
        .iter()
        .map(|(why, k)| {
            let what = if *k == 1 { "1 track".to_string() } else { format!("{k} tracks") };
            format!("High precision BeatGrid analysis was not used for {what}: {why}. The standard beat grid was kept.")
        })
        .collect();
    n.clear();
    out
}

pub fn current_mode() -> u8 {
    if let Ok(v) = std::env::var("PREPDECK_HP_MODE") {
        if let Ok(m) = v.parse::<u8>() {
            return m;
        }
    }
    // Unit tests build Apps with default preferences (switch on) in parallel: only the
    // environment variable turns the high precision path on there, so analyses stay
    // independent of test order.
    if cfg!(test) {
        return 0;
    }
    MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Hook of `analysis::analyze_decoded`: with high precision on, the standard estimate becomes the
/// legacy input of rekordbox's high precision analysis and its grid replaces the standard one.
/// Falls back to the standard grid (and says so on stderr) when rekordbox's models are missing
/// or a part of the pipeline that is not ported is needed.
pub fn dispatch(dec: &crate::audio::Decoded, s: &super::AnalysisSettings, bpm: f64, grid: Option<BeatGrid>) -> (f64, Option<BeatGrid>) {
    let mode = current_mode();
    if mode == 0 || !s.do_grid {
        return (bpm, grid);
    }
    let Some((first, beat)) = grid.as_ref().and_then(|g| g.anchors.first()).map(|a| (a.time, a.beat)) else {
        note(NO_GRID);
        return (bpm, grid);
    };
    if !available() {
        note(NO_MODELS);
        return (bpm, grid);
    }
    // "Analyze Track (Dynamic)" / the Dynamic analysis mode ask for the tempo-change path
    let mode = if s.dynamic { MODE_DYNAMIC } else { mode };
    match analyze(dec, mode, (bpm, first, beat)) {
        Ok(o) => {
            let mut g = o.grid;
            let mut b = o.bpm;
            // convertToMicrosec (0x102328b20): halve or double into the BPM range, only when the
            // tempo-change path was not taken (Analyzer +48 clear)
            let dur = dec.duration();
            while !o.hendou && b < s.bpm_min && b > 0.0 {
                g = g.doubled(dur);
                b *= 2.0;
            }
            while !o.hendou && b > s.bpm_max {
                g = g.halved(dur);
                b *= 0.5;
            }
            (b, Some(g))
        }
        Err(why) => {
            note(why);
            if is_tempo_fallback(why) {
                // the tempo-change path hit a branch that is not built: the standard analysis in
                // Dynamic mode is the nearest grid (recordbox's choice, not traced)
                if let Some((b, Some(g))) = super::rb_beat1::tempo_and_grid(&dec.samples, dec.sr, true, s.bpm_min, s.bpm_max) {
                    return (b, Some(g));
                }
            }
            (bpm, grid)
        }
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::net::*;

    fn read_f32(p: &str) -> Vec<f32> {
        std::fs::read(p).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// Network stage against rekordbox's own TensorFlow run (`checks/beat-oracle-harness/rbtf`):
    /// PREPDECK_HP_PCM = stereo f32 PCM, PREPDECK_HP_TF = rbtf output prefix, PREPDECK_HP_START /
    /// PREPDECK_HP_LEN = the window rbtf was given, PREPDECK_HP_MODEL = model directory.
    #[test]
    #[ignore]
    fn net_against_tensorflow() {
        let (Ok(pcm), Ok(tf)) = (std::env::var("PREPDECK_HP_PCM"), std::env::var("PREPDECK_HP_TF")) else { return };
        let start: usize = std::env::var("PREPDECK_HP_START").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let len: usize = std::env::var("PREPDECK_HP_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let model = std::env::var("PREPDECK_HP_MODEL").map(std::path::PathBuf::from).unwrap_or_else(|_| models_dir().join("model8"));
        let st = read_f32(&pcm);
        let n = if len == 0 { st.len() / 2 - start } else { len };
        let mono: Vec<f32> = (start..start + n).map(|i| (st[2 * i + 1] + st[2 * i]) * 0.5).collect();
        let t0 = std::time::Instant::now();
        let net = Net::load(&model).unwrap();
        let t1 = std::time::Instant::now();
        let feats = net.features(&mono);
        let t2 = std::time::Instant::now();
        let act = net.activations(&feats);
        let t3 = std::time::Instant::now();
        let want_f = read_f32(&format!("{tf}.feat.f32"));
        let want_a = read_f32(&format!("{tf}.act.f32"));
        assert_eq!(want_f.len(), feats.len(), "feature count");
        let mut worst = (0f32, 0usize);
        let mut sum = 0f64;
        for (i, (a, b)) in feats.iter().zip(&want_f).enumerate() {
            let d = (a - b).abs();
            sum += d as f64;
            if d > worst.0 {
                worst = (d, i);
            }
        }
        eprintln!(
            "features: {} frames, max |diff| {:.3e} at frame {} col {}, mean {:.3e}",
            feats.len() / NUM_FEATURES,
            worst.0,
            worst.1 / NUM_FEATURES,
            worst.1 % NUM_FEATURES,
            sum / feats.len() as f64
        );
        let mut worst_a = 0f32;
        let mut worst_at = 0usize;
        let mut over = 0usize;
        for (i, (a, b)) in act.iter().flatten().zip(&want_a).enumerate() {
            let d = (a - b).abs();
            over += (d > 1e-4) as usize;
            if d > worst_a {
                worst_a = d;
                worst_at = i;
            }
        }
        eprintln!("activations: worst at frame {} col {}, {over} values differ by more than 1e-4", worst_at / 2, worst_at % 2);
        if let Ok(p) = std::env::var("PREPDECK_HP_ACTOUT") {
            let b: Vec<u8> = act.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(p, b).unwrap();
        }
        // The same network on TensorFlow's own features isolates the LSTM part.
        let act_tf = net.activations(&want_f);
        let mut worst_b = 0f32;
        for (a, b) in act_tf.iter().flatten().zip(&want_a) {
            worst_b = worst_b.max((a - b).abs());
        }
        eprintln!("activations: max |diff| {worst_a:.3e} (on TF's features {worst_b:.3e})");
        eprintln!("time: load {:?} features {:?} network {:?}", t1 - t0, t2 - t1, t3 - t2);
    }
}

#[cfg(test)]
mod pcm_dump {
    /// Writes recordbox's own decode (interleaved stereo f32) of each PREPDECK_AUDIO file
    /// ('|'-separated) to PREPDECK_HP_DUMP/<n>.f32, so the oracle harness runs on the same PCM.
    #[test]
    #[ignore]
    fn dump_pcm() {
        let Ok(dir) = std::env::var("PREPDECK_HP_DUMP") else { return };
        let list = std::env::var("PREPDECK_AUDIO").unwrap_or_default();
        for (i, p) in list.split('|').filter(|s| !s.is_empty()).enumerate() {
            let Ok(d) = crate::audio::decode_file(p) else {
                eprintln!("DUMP\t{i}\tERR\t{p}");
                continue;
            };
            let bytes: Vec<u8> = d.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(format!("{dir}/{i}.f32"), bytes).unwrap();
            eprintln!("DUMP\t{i}\t{}\t{}\t{p}", d.sr, d.frames());
        }
    }
}

#[cfg(test)]
mod stage_tests {
    use super::{dbn, net};

    /// One oracle dump (`checks/beat-oracle-harness/rbhp` output, RB_ALL=1).
    pub struct Dump {
        pub path: String,
        pub sr: u32,
        pub legacy_bpm: i32,
        pub beat_num: i32,
        pub offset: i64,
        pub hendou: bool,
        pub win: (usize, usize),
        pub m: Vec<(f32, i32)>,
        pub j: Vec<(i64, i32, i32)>,
        pub j_bpm: f64,
        pub f: Vec<(i64, i32, i32)>,
        pub f_bpm: i32,
    }

    pub fn read_dump(p: &std::path::Path) -> Option<Dump> {
        let text = std::fs::read_to_string(p).ok()?;
        let mut d = Dump { path: String::new(), sr: 0, legacy_bpm: 0, beat_num: 0, offset: 0, hendou: false, win: (0, 0), m: vec![], j: vec![], j_bpm: 0.0, f: vec![], f_bpm: 0 };
        for l in text.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            match (w.first().copied(), w.get(1).copied()) {
                (Some("P"), _) => d.path = l[2..].to_string(),
                (Some("SR"), Some(v)) => d.sr = v.parse().ok()?,
                (Some("L"), Some("bpm")) => {
                    d.legacy_bpm = w[2].parse().ok()?;
                    d.beat_num = w[4].parse().ok()?;
                    d.offset = w[6].parse().ok()?;
                    d.hendou = w.iter().position(|x| *x == "hendou").map(|i| w[i + 1] == "1").unwrap_or(false);
                }
                (Some("W"), _) => d.win = (w[2].parse().ok()?, w[4].parse().ok()?),
                (Some("M"), Some("beats")) => {}
                (Some("M"), Some(_)) => d.m.push((w[1].parse().ok()?, w[2].parse().ok()?)),
                (Some("J"), Some("bpm")) => d.j_bpm = w[2].parse().ok()?,
                (Some("J"), Some(_)) => d.j.push((w[1].parse().ok()?, w[2].parse().ok()?, w[3].parse().ok()?)),
                (Some("F"), Some("bpm")) => d.f_bpm = w[2].parse().ok()?,
                (Some("F"), Some(_)) => d.f.push((w[1].parse().ok()?, w[2].parse().ok()?, w[3].parse().ok()?)),
                _ => {}
            }
        }
        Some(d)
    }

    pub fn dumps() -> Vec<(usize, Dump)> {
        let Ok(dir) = std::env::var("PREPDECK_HP_ORACLE") else { return vec![] };
        let mut v: Vec<(usize, Dump)> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| {
                let e = e.ok()?;
                let k: usize = e.file_name().to_str()?.strip_suffix(".txt")?.parse().ok()?;
                Some((k, read_dump(&e.path())?))
            })
            .collect();
        v.sort_by_key(|x| x.0);
        v
    }

    /// madmom stage against the oracle: the same window of recordbox's decode through the ported
    /// network + DBN (raw beats, "M") and get_beat_list / get_bpm ("J").
    /// PREPDECK_HP_ORACLE = directory of rbhp dumps.
    /// Focus stage against the oracle on the oracle's own inputs (legacy BPM, count, offset,
    /// window): the debug csv lines rekordbox wrote (PREPDECK_HP_PROP/<n>/csv/) and the final
    /// beat list. PREPDECK_HP_ORACLE = rbhp dumps, PREPDECK_HP_PROP = their csv folders,
    /// PREPDECK_HP_ONLY = comma list of track numbers.
    #[test]
    #[ignore]
    fn focus_stage_against_oracle() {
        let all = dumps();
        let Ok(prop) = std::env::var("PREPDECK_HP_PROP") else { return };
        let only: Vec<usize> = std::env::var("PREPDECK_HP_ONLY").map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
        let names = ["interval.csv", "interval2.csv", "interval3.csv", "interval4.csv", "interval5.csv"];
        let mut same = [0usize; 5];
        let (mut n, mut f_same, mut first_same) = (0, 0, 0);
        for (k, d) in &all {
            if d.hendou || d.m.is_empty() || (!only.is_empty() && !only.contains(k)) {
                continue;
            }
            let Ok(dec) = crate::audio::decode_file(&d.path) else { continue };
            let env = super::uba::Uba::new(dec.sr as i32, &super::uba::to_i16(&dec.samples));
            // PREPDECK_HP_USEJ=1: the dump's network beat list (J lines, e.g. one the oracle was
            // given with RB_MLFILE) instead of running the network
            let (ml, mbpm): (Vec<(i32, i32, i32)>, f64) = if std::env::var("PREPDECK_HP_USEJ").is_ok() {
                (d.j.iter().map(|b| (b.0 as i32, b.1, b.2)).collect(), d.j_bpm)
            } else {
                let Some((list, mbpm, _)) = super::madmom_stage(&dec.samples, dec.sr, d.win.0, d.win.1, false) else { continue };
                (list.iter().map(|b| (b.pos as i32, b.num_bar, b.bpm)).collect(), mbpm)
            };
            let inp = super::focus::FocusInput { sr: dec.sr as i32, total: dec.frames() as i32, offset: d.offset as i32, hendou: false, legacy_bpm100: d.legacy_bpm, legacy_count: d.beat_num, madmom: &ml, madmom_bpm: mbpm as i32 };
            let out = match super::focus::analyze_focus(&env, &inp) {
                Ok(o) => o,
                Err(why) => {
                    eprintln!("  #{k} focus returned nothing: {why}");
                    continue;
                }
            };
            n += 1;
            if let Ok(dir) = std::env::var("PREPDECK_HP_DUMPFOCUS") {
                // the lists around UnitBeatAdjust in the probe format (pos:interval:number)
                let fmt = |v: &[super::uba::BeatInfo]| v.iter().map(|b| format!("{}:{}:{}", b.pos, b.interval, b.num)).collect::<Vec<_>>().join(" ");
                let text = format!("B n={} {}\nA n={} {}\nV {:?}\n", out.before_uba.len(), fmt(&out.before_uba), out.after_uba.len(), fmt(&out.after_uba), out.vote);
                std::fs::write(format!("{dir}/{k}.focus"), text).unwrap();
            }
            let mut line = format!("  #{k}");
            for (c, name) in names.iter().enumerate() {
                let want: Vec<String> = std::fs::read_to_string(format!("{prop}/{k}/csv/{name}")).unwrap_or_default().split("\r\n").filter(|l| !l.is_empty()).map(|l| l.trim_end_matches('\r').to_string()).collect();
                let want: Vec<String> = if c == 2 { want.iter().map(|l| l.split(',').next().unwrap_or("").to_string()).collect() } else { want };
                let ok = want == out.csv[c];
                same[c] += ok as usize;
                if !ok {
                    let first = want.iter().zip(&out.csv[c]).position(|(a, b)| a != b);
                    line += &format!(" {name}: {} vs {} lines, first diff {:?}", out.csv[c].len(), want.len(), first.map(|i| (i, out.csv[c][i].clone(), want[i].clone())));
                }
            }
            let fm: Vec<(i64, i32, i32)> = out.beats.iter().map(|b| (b.0 as i64, b.1, b.2)).collect();
            let fok = fm == d.f && out.bpm100 == d.f_bpm;
            f_same += fok as usize;
            let fs = fm.first().map(|b| b.0) == d.f.first().map(|b| b.0);
            first_same += fs as usize;
            if !fok {
                line += &format!(" | F {} beats bpm {} first {:?} vs oracle {} beats bpm {} first {:?}", fm.len(), out.bpm100, fm.first(), d.f.len(), d.f_bpm, d.f.first());
            }
            if line.len() > 8 {
                eprintln!("{line}");
            }
        }
        eprintln!("focus stage: tracks {n}; csv identical {same:?}; final list identical {f_same}, first beat identical {first_same}");
    }

    #[test]
    #[ignore]
    fn madmom_stage_against_oracle() {
        let all = dumps();
        if all.is_empty() {
            return;
        }
        let model = net::Net::load(&net::models_dir().join("model8")).unwrap();
        let hmm = dbn::Dbn::rekordbox();
        let (mut n, mut m_same, mut j_same, mut bpm_same) = (0, 0, 0, 0);
        let mut t_net = std::time::Duration::ZERO;
        let mut t_dbn = std::time::Duration::ZERO;
        for (k, d) in &all {
            if d.hendou || d.m.is_empty() {
                continue;
            }
            let Ok(dec) = crate::audio::decode_file(&d.path) else { continue };
            let (st, len) = d.win;
            let mono: Vec<f32> = (st..st + len).map(|i| (dec.samples[2 * i + 1] + dec.samples[2 * i]) * 0.5).collect();
            let t0 = std::time::Instant::now();
            let act = model.run(&mono);
            let t1 = std::time::Instant::now();
            let beats = hmm.track(&act);
            t_dbn += t1.elapsed();
            t_net += t1 - t0;
            let list = dbn::get_beat_list(&beats, len as i64, d.sr as i32, 4);
            let bpm = dbn::get_bpm(&list);
            n += 1;
            let m_ok = beats.len() == d.m.len() && beats.iter().zip(&d.m).all(|(a, b)| a.1 == b.1 && (a.0 - b.0).abs() < 1e-6);
            let j_ok = list.len() == d.j.len() && list.iter().zip(&d.j).all(|(a, b)| (a.pos, a.num_bar, a.bpm) == *b);
            m_same += m_ok as usize;
            j_same += j_ok as usize;
            bpm_same += (bpm == d.j_bpm) as usize;
            if !m_ok || !j_ok {
                let first = beats.iter().zip(&d.m).position(|(a, b)| a.1 != b.1 || (a.0 - b.0).abs() >= 1e-6);
                eprintln!(
                    "  #{k} M {} vs {} first diff {:?} | J {} vs {} bpm {bpm} vs {} :: {}",
                    beats.len(),
                    d.m.len(),
                    first.map(|i| (i, beats[i], d.m[i])),
                    list.len(),
                    d.j.len(),
                    d.j_bpm,
                    d.path.rsplit('/').next().unwrap_or("")
                );
            }
        }
        eprintln!("madmom stage: tracks {n}, raw beats identical {m_same}, beat list identical {j_same}, bpm identical {bpm_same}");
        eprintln!("time: network {:.2} s, dbn {:.2} s (total over {n} windows)", t_net.as_secs_f64(), t_dbn.as_secs_f64());
    }
}

#[cfg(test)]
mod reference_benchmark {
    #[repr(C)]
    struct Rusage([i64; 18]);
    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    /// User + system CPU seconds of the whole process (`getrusage(RUSAGE_SELF)`; timeval is
    /// 16 bytes on macOS: seconds, then microseconds in the low 32 bits).
    fn process_cpu() -> f64 {
        let mut r = Rusage([0; 18]);
        unsafe { getrusage(0, &mut r) };
        let us = |i: usize| (r.0[i + 1] & 0xffff_ffff) as f64 * 1e-6 + r.0[i] as f64;
        us(0) + us(2)
    }

    /// End to end against rekordbox's stored high precision grids, exact in rekordbox's own units:
    /// BPM x100, first beat in whole ms (`convertToMicrosec`), beat number of the first beat.
    /// PREPDECK_GT = oracle/rbgrids.tsv; PREPDECK_HP_MODE = 2 | 3 (default 2).
    /// Also times the standard analysis and the high precision one per track.
    #[test]
    #[ignore]
    fn hp_reference_grids() {
        let Ok(f) = std::env::var("PREPDECK_GT") else { return };
        let mode: u8 = std::env::var("PREPDECK_HP_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let text = std::fs::read_to_string(f).unwrap();
        let rows: Vec<(String, (i64, i64, i64))> = text
            .lines()
            .filter_map(|l| {
                let (p, b) = l.split_once('\t')?;
                let v: Vec<i64> = b.split(';').next()?.split(',').filter_map(|n| n.parse().ok()).collect();
                (v.len() == 3).then(|| (p.to_string(), (v[0], v[1], v[2])))
            })
            .collect();
        // PREPDECK_ANLZ_PAIRS (audio path \t rekordbox ANLZ base): also compare every beat the
        // export would write (PQTZ: beat number, tempo x 100, ms) with rekordbox's stored list.
        let pairs: std::collections::HashMap<String, String> = std::env::var("PREPDECK_ANLZ_PAIRS")
            .ok()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| t.lines().filter_map(|l| l.split_once('\t')).map(|(a, b)| (a.to_string(), b.to_string())).collect())
            .unwrap_or_default();
        let pqtz_of = |grid: &crate::model::BeatGrid, dur: f64| -> Vec<(u16, u16, u32)> {
            grid.beats(dur).iter().map(|b| (b.beat as u16, (b.bpm * 100.0).round() as u16, (((b.time.max(0.0) * 1e6).round() as u64) / 1000) as u32)).collect()
        };
        let beat_stats = std::sync::Mutex::new((0usize, 0usize, 0usize, 0usize, 0usize, 0usize));
        let wall = std::sync::Mutex::new(Vec::<f64>::new());
        let results = std::sync::Mutex::new(vec![]);
        let threads: usize = std::env::var("PREPDECK_HP_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
        let chunks: Vec<_> = rows.chunks(rows.len().div_ceil(threads).max(1)).collect();
        std::thread::scope(|s| {
            for ch in chunks {
                let results = &results;
                let (pairs, beat_stats, pqtz_of, wall) = (&pairs, &beat_stats, &pqtz_of, &wall);
                s.spawn(move || {
                    for (p, (rb, rbpm, rms)) in ch {
                        let Ok(dec) = crate::audio::decode_file(p) else { continue };
                        // the standard analysis (rekordbox's BeatAnalyzer_1_0, Normal) and then the high
                        // precision one; process CPU seconds (all threads), one track at a time with
                        // PREPDECK_HP_THREADS=1
                        let c0 = process_cpu();
                        let std_res = super::super::rb_beat1::tempo_and_grid(&dec.samples, dec.sr, false, 70.0, 180.0);
                        let t_std = process_cpu() - c0;
                        let Some((bpm, Some(g))) = std_res else { continue };
                        let c1 = process_cpu();
                        let w1 = std::time::Instant::now();
                        let out = super::analyze(&dec, mode, (bpm, g.anchors[0].time, g.anchors[0].beat));
                        let t_hp = process_cpu() - c1;
                        wall.lock().unwrap().push(w1.elapsed().as_secs_f64());
                        if let (Some(o), Some(base)) = (out.as_ref().ok(), pairs.get(p)) {
                            let secs = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                            let want = secs.get("PQTZ").map(|s| crate::export::anlz::pqtz_entries(s)).unwrap_or_default();
                            if !want.is_empty() {
                                let dur = dec.duration();
                                let mine = pqtz_of(&o.grid, dur);
                                let (p0, n0, _) = o.beats[0];
                                let plain = crate::model::BeatGrid::constant(p0 as f64 / dec.sr as f64, o.bpm, n0.clamp(1, 4) as u8);
                                let old = pqtz_of(&plain, dur);
                                let diff = |v: &[(u16, u16, u32)]| v.iter().zip(&want).filter(|(a, b)| a != b).count() + v.len().abs_diff(want.len());
                                let (d_new, d_old) = (diff(&mine), diff(&old));
                                let mut s = beat_stats.lock().unwrap();
                                s.0 += 1;
                                s.1 += want.len();
                                s.2 += (d_new == 0) as usize;
                                s.3 += d_new;
                                s.4 += (d_old == 0) as usize;
                                s.5 += d_old;
                                if d_new != 0 {
                                    let first = mine.iter().zip(&want).position(|(a, b)| a != b);
                                    eprintln!("  beats: {d_new} of {} differ (mine {} beats), first {:?} :: {}", want.len(), mine.len(), first.map(|i| (i, mine[i], want[i])), p.rsplit('/').next().unwrap_or(""));
                                }
                            }
                        }
                        let row = match out.as_ref().ok() {
                            Some(o) => {
                                let (pos, num, _) = o.beats[0];
                                let ms = (1_000_000i64 * pos as i64 / dec.sr as i64) / 1000;
                                (p.clone(), *rb, *rbpm, *rms, Some(((o.bpm * 100.0).round() as i64, ms, num as i64)), t_std, t_hp)
                            }
                            None => {
                                eprintln!("  no high precision result ({}) :: {}", out.as_ref().err().copied().unwrap_or(""), p.rsplit('/').next().unwrap_or(""));
                                (p.clone(), *rb, *rbpm, *rms, None, t_std, t_hp)
                            }
                        };
                        results.lock().unwrap().push(row);
                    }
                });
            }
        });
        let mut res = results.into_inner().unwrap();
        res.sort_by(|a, b| a.0.cmp(&b.0));
        let (mut n, mut bpm_ok, mut ms_ok, mut num_ok, mut all_ok, mut none) = (0, 0, 0, 0, 0, 0);
        let (mut ts, mut th) = (0f64, 0f64);
        for (p, rb, rbpm, rms, got, t_std, t_hp) in &res {
            n += 1;
            ts += t_std;
            th += t_hp;
            let name = p.rsplit('/').next().unwrap_or("");
            match got {
                None => {
                    none += 1;
                }
                Some((b, ms, num)) => {
                    let (a, m, u) = (b == rbpm, ms == rms, num == rb);
                    bpm_ok += a as usize;
                    ms_ok += m as usize;
                    num_ok += u as usize;
                    all_ok += (a && m && u) as usize;
                    if !(a && m && u) {
                        eprintln!("  mine bpm {b} first {ms} ms beat {num} | rekordbox bpm {rbpm} first {rms} ms beat {rb} :: {name}");
                    }
                }
            }
        }
        eprintln!("high precision mode {mode}: tracks {n}, no result {none}, bpm identical {bpm_ok}, first beat ms identical {ms_ok}, beat number identical {num_ok}, all three {all_ok}");
        eprintln!("CPU time per track (process, all threads): standard {:.2} s, high precision extra {:.2} s ({:+.0} %)", ts / n.max(1) as f64, th / n.max(1) as f64, 100.0 * th / ts.max(1e-9));
        let w = wall.into_inner().unwrap();
        eprintln!("wall time of the high precision part per track: mean {:.2} s", w.iter().sum::<f64>() / w.len().max(1) as f64);
        let s = beat_stats.into_inner().unwrap();
        if s.0 > 0 {
            eprintln!("every stored beat (PQTZ): tracks {}, beats {}; grid from the list: tracks identical {}, beats differing {}; first beat + k * 60 / BPM: tracks identical {}, beats differing {}", s.0, s.1, s.2, s.3, s.4, s.5);
        }
    }
}

#[cfg(test)]
mod rate_check {
    /// High precision on files at other rates: PREPDECK_AUDIO ('|'-separated) through recordbox's
    /// decoder, rekordbox's converter port and the pipeline (Auto); writes the final beat list
    /// (pos num bpm per line, as the oracle's F lines) to PREPDECK_HP_RATEOUT/<file name>.f and
    /// prints one line per file.
    #[test]
    #[ignore]
    fn hp_rate_check() {
        let (Ok(list), Ok(out)) = (std::env::var("PREPDECK_AUDIO"), std::env::var("PREPDECK_HP_RATEOUT")) else { return };
        for p in list.split('|').filter(|p| !p.is_empty()) {
            let d = crate::audio::decode_file(p).unwrap();
            let name = p.rsplit('/').next().unwrap();
            match super::analyze(&d, 2, (120.0, 0.0, 1)) {
                Ok(o) => {
                    let text: String = o.beats.iter().map(|b| format!("{} {} {}\n", b.0, b.1, b.2)).collect();
                    std::fs::write(format!("{out}/{name}.f"), text).unwrap();
                    eprintln!("RATE {} -> {} beats {} bpm {} first {:?} :: {name}", d.sr, super::super::rb_reader::analysis_rate(d.sr), o.beats.len(), o.bpm, o.beats.first());
                }
                Err(e) => eprintln!("RATE {} no result: {e} :: {name}", d.sr),
            }
        }
    }
}

#[cfg(test)]
mod std_dump {
    /// The standard analysis' grid head for every track of PREPDECK_GT, as
    /// "STD <k> <bpm x 100> <first beat ms> <beat number>" (for the stage-B correction probe).
    #[test]
    #[ignore]
    fn std_grid_dump() {
        // PREPDECK_ANLZ_PAIRS (audio \t ANLZ base): also "RB <k> <bpm x 100> <ms> <beat> <path>"
        // from rekordbox's stored PQTZ, and the path on the STD lines
        let pairs = std::env::var("PREPDECK_ANLZ_PAIRS").ok();
        let Some(f) = std::env::var("PREPDECK_GT").ok().or(pairs.clone()) else { return };
        for (k, l) in std::fs::read_to_string(f).unwrap().lines().enumerate() {
            let p = l.split('\t').next().unwrap();
            if pairs.is_some() {
                let base = l.split('\t').nth(1).unwrap_or("");
                let secs = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                if let Some(e) = secs.get("PQTZ").map(|s| crate::export::anlz::pqtz_entries(s)).and_then(|v| v.first().copied()) {
                    eprintln!("RB {k} {} {} {} {p}", e.1, e.2, e.0);
                }
            }
            let Ok(d) = crate::audio::decode_file(p) else { continue };
            let conv = super::super::rb_reader::for_analysis(&d);
            let a = conv.as_ref().unwrap_or(&d);
            if let Some((bpm, Some(g))) = super::super::rb_beat1::tempo_and_grid(&a.samples, a.sr, false, 70.0, 180.0) {
                let a0 = g.anchors[0];
                eprintln!("STD {k} {} {} {} {p}", (bpm * 100.0).round() as i64, ((a0.time * 1e6).round() as i64) / 1000, a0.beat);
            }
        }
    }
}

#[cfg(test)]
mod env_dump {
    /// Writes the UnitBeatAdjust envelopes of PREPDECK_AUDIO (one file) to PREPDECK_HP_ENVOUT in the
    /// layout of the oracle's RB_DUMPENV (count, cap, then the three i16 buffers +112, +120, +128).
    #[test]
    #[ignore]
    fn dump_envelopes() {
        let (Ok(p), Ok(out)) = (std::env::var("PREPDECK_AUDIO"), std::env::var("PREPDECK_HP_ENVOUT")) else { return };
        let dec = crate::audio::decode_file(&p).unwrap();
        let env = super::uba::Uba::new(dec.sr as i32, &super::uba::to_i16(&dec.samples));
        let mut b = vec![];
        b.extend_from_slice(&(env.high.len() as i32).to_le_bytes());
        b.extend_from_slice(&env.cap.to_le_bytes());
        for buf in [&env.high, &env.low, &env.all] {
            for v in buf.iter() {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(out, b).unwrap();
    }
}

#[cfg(test)]
mod legacy_tests {
    /// The legacy stage (`legacy.rs` + getOffsetCalc) against the oracle's L lines: BA_GetBpm,
    /// BA_GetBeatNum, BA_GetOffsetCalc and every listed beat (position, number).
    /// PREPDECK_HP_ORACLE = rbhp dumps (RB_ALL=1), PREPDECK_HP_ONLY = track numbers.
    #[test]
    #[ignore]
    fn legacy_against_oracle() {
        let Ok(dir) = std::env::var("PREPDECK_HP_ORACLE") else { return };
        let only: Vec<usize> = std::env::var("PREPDECK_HP_ONLY").map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
        let (mut n, mut bpm_ok, mut cnt_ok, mut off_ok, mut list_ok) = (0, 0, 0, 0, 0);
        let mut ents: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
        ents.sort_by_key(|e| e.file_name());
        for e in ents {
            let Some(k) = e.file_name().to_str().and_then(|f| f.strip_suffix(".txt")).and_then(|f| f.parse::<usize>().ok()) else { continue };
            if !only.is_empty() && !only.contains(&k) {
                continue;
            }
            let text = std::fs::read_to_string(e.path()).unwrap();
            let mut path = String::new();
            let (mut lb, mut lc, mut lo, mut hendou) = (0i32, 0usize, 0i64, false);
            let mut list: Vec<(i32, i32)> = vec![];
            for l in text.lines() {
                let w: Vec<&str> = l.split_whitespace().collect();
                if l.starts_with("P ") {
                    path = l[2..].to_string();
                } else if w.len() > 6 && w[0] == "L" && w[1] == "bpm" {
                    lb = w[2].parse().unwrap();
                    lc = w[4].parse().unwrap();
                    lo = w[6].parse().unwrap();
                    hendou = w.iter().position(|x| *x == "hendou").map(|i| w[i + 1] == "1").unwrap_or(false);
                } else if w.len() == 4 && w[0] == "L" {
                    list.push((w[1].parse().unwrap(), w[2].parse().unwrap()));
                }
            }
            let Ok(dec) = crate::audio::decode_file(&path) else { continue };
            let env = super::uba::Uba::new(dec.sr as i32, &super::uba::to_i16(&dec.samples));
            let Some(l2) = super::legacy::analyze(&dec.samples, dec.sr, false, Some(&env.low)) else {
                eprintln!("  #{k} no legacy result");
                continue;
            };
            let leg = super::Legacy { bpm: l2.bpm, beats: l2.beats.iter().map(|b| b.pos).collect() };
            let off = super::get_offset_calc(&env, dec.sr as i32, &leg) as i64;
            let mine: Vec<(i32, i32)> = l2.beats.iter().map(|b| (b.pos, b.stat)).collect();
            n += 1;
            let (a, b, c, d) = (l2.bpm100() == lb, mine.len() == lc, off == lo, mine == list);
            bpm_ok += a as usize;
            cnt_ok += b as usize;
            off_ok += c as usize;
            list_ok += d as usize;
            if !(a && b && c && d) {
                let first = mine.iter().zip(&list).position(|(x, y)| x != y);
                eprintln!("  #{k} bpm {} vs {lb}, beats {} vs {lc}, offset {off} vs {lo}, first list diff {:?}, oracle hendou {hendou} :: {}", l2.bpm100(), mine.len(), first.map(|i| (i, mine[i], list[i])), path.rsplit('/').next().unwrap_or(""));
            }
        }
        eprintln!("legacy stage: tracks {n}; BA_GetBpm identical {bpm_ok}, beat count {cnt_ok}, offset {off_ok}, beat list {list_ok}");
    }
}

#[cfg(test)]
mod grid_tests {
    #[test]
    fn grid_from_list_writes_every_listed_millisecond() {
        // 129.97 BPM at 44.1 kHz, positions rounded to whole samples as rekordbox's list
        let sr = 44100u32;
        let per = sr as f64 * 60.0 / 129.97;
        let beats: Vec<(i32, i32, i32)> = (0..700).map(|k| ((4321.6 + k as f64 * per + 0.5) as i32, (k % 4 + 1) as i32, 12997)).collect();
        let g = super::grid_from_list(&beats, sr);
        let dur = (beats.last().unwrap().0 as f64 + per) / sr as f64;
        let got: Vec<(u8, u32)> = g.beats(dur).iter().map(|b| (b.beat, (((b.time * 1e6).round() as u64) / 1000) as u32)).collect();
        let want: Vec<(u8, u32)> = beats.iter().map(|b| (b.1 as u8, (super::beat_us(b.0, sr) / 1000) as u32)).collect();
        assert_eq!(&got[..want.len()], &want[..]);
        assert!(g.anchors.len() < 60, "{} anchors", g.anchors.len());
        assert!(g.anchors.iter().all(|a| (a.bpm - 129.97).abs() < 1e-9));
    }

    #[test]
    fn reader16_clamps_like_rekordbox() {
        use super::uba::reader16;
        assert_eq!(reader16(1.0), 32767);
        assert_eq!(reader16(-1.0), -32768);
        assert_eq!(reader16(-1.5), -32768);
        assert_eq!(reader16(f32::NAN), -32768);
        assert_eq!(reader16(0.5), 16384);
        assert_eq!(reader16(-0.99999), -32767);
    }
}
