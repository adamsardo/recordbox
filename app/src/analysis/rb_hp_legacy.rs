//! The legacy analysis rekordbox's high precision analysis starts from.
//!
//! With high precision on, `CBeatAnalyzeThread::analyze` (0x1001528c4) switches its beat library
//! to `BeatAnalyzer_2_0` (`BALibWrapper::SetVersion(2)` when the analysis mode is 2..4), created
//! for constant tempo in Auto / Normal and for varied tempo in Dynamic, and feeds it the same
//! 16-bit samples as the standard analysis. `BeatAnalyzer_2_0::Analyzer::analyze(bool&)`
//! (0x10116be98) runs `BeatSensor`, `BeatInfoAnalyzer` and `UnitEnvelopeMaker`, and unlike
//! `BeatAnalyzer_1_0` no `UnitBeatAdjust` (2_0's `UnitBeatAdjust` only serves `analyzeFocus`).
//! `highPrecisionBeatAnalyze` (0x102327e50) then reads `BA_GetBpm`, `BA_GetBeatNum`,
//! `BA_GetOffsetCalc` (from the beat list) and, in Auto, `BA_GetBpmHendou`.
//!
//! The sensor, `UnitEnvelopeMaker`, gap correction and `findNearPos` of 2_0 are byte for byte
//! the 1_0 functions (114 functions compared with masked branch targets), so the stages are
//! compiled here from recordbox's port of 1_0 (`rb_beat1/`). 2_0's `BeatInfoAnalyzer::analyze`
//! (0x101198e4c, 52 KB) is a larger function than 1_0's (18 KB); it is not ported. Run on the
//! same samples by the original routines (107 benchmark tracks, constant and varied), 2_0's
//! output equals 1_0's without `UnitBeatAdjust` on 206 of 214 runs; in constant mode the beat
//! lists are equal on all of them and the tempo differs only where 2_0 snaps a tempo within
//! 0.05 % of a whole BPM to it (4 tracks) and on two Sampler preset WAVs. That snap is applied
//! here. Not reproduced: 2_0's tempo-change verdict (`judgeHendouDynamicPeak`, Auto) and its
//! varied-mode list where it differs (2 of 107 tracks).

pub use super::super::rb_beat1::BeatInfo;

#[path = "rb_beat1/sensor.rs"]
#[allow(dead_code)]
mod sensor;
#[path = "rb_beat1/info.rs"]
#[allow(dead_code)]
mod info;
#[path = "rb_beat1/env.rs"]
#[allow(dead_code)]
mod env;

/// `BeatAnalyzer_2_0` after `setSample`: the float BPM (`Analyzer` +72) and the beat list (+88).
#[derive(Clone, Debug)]
pub struct Legacy2 {
    pub bpm: f32,
    pub beats: Vec<BeatInfo>,
}

impl Legacy2 {
    /// `BA_GetBpm`: BPM x 100 rounded.
    pub fn bpm100(&self) -> i32 {
        self.bpm.mul_add(100.0, 0.5) as i32
    }
}

/// The legacy analysis of interleaved stereo at 44.1 or 48 kHz (`varied` = created for varied
/// tempo, Analysis Mode Dynamic).
pub fn analyze(samples: &[f32], sr: u32, varied: bool) -> Option<Legacy2> {
    let total = (samples.len() / 2) as i32;
    if !(sr == 44100 || sr == 48000) || total < 1 {
        return None;
    }
    let block = (sr / 75) as usize;
    let mut sensor = sensor::Sensor::new(sr as i32, total);
    let mut uem = env::Uem::new(sr as i32, total);
    let mut pcm16 = vec![0i16; 2 * block];
    let mut mono = vec![0f32; block];
    let conv = super::super::rb_beat1::reader_sample;
    let frames = total as usize;
    let mut done = 0usize;
    while done < frames {
        let n = block.min(frames - done);
        for i in 0..n {
            let l = conv(samples[2 * (done + i)]);
            let r = conv(samples[2 * (done + i) + 1]);
            mono[i] = (r + l) * (1.0 / 65536.0);
            pcm16[2 * i] = (l as i32) as i16;
            pcm16[2 * i + 1] = (r as i32) as i16;
        }
        sensor.put(&mono[..n]);
        uem.put(&pcm16[..2 * n]);
        done += n;
    }
    let bia = info::Bia { sr: sr as i32 };
    let mut list = std::mem::take(&mut sensor.list);
    let mut grid = vec![];
    let mut bpm = bia.analyze(&mut list, &mut grid, total, 4, 16, 12, varied);
    uem.analyze(bpm, &mut grid, varied);
    // 2_0 keeps a tempo within 0.05 % of a whole BPM as that BPM (in 2_0's
    // BeatInfoAnalyzer::analyze: `roundf(bpm) * 1.0005 >= bpm && roundf(bpm) * 0.9995 <= bpm`;
    // matches the original's output on 105 of the 107 constant-tempo runs, the other two are
    // Sampler preset WAVs that 2_0 sends down its tempo-change branch)
    let r = bpm.round();
    if r * 1.0005 >= bpm && r * 0.9995 <= bpm {
        bpm = r;
    }
    if grid.is_empty() {
        return None;
    }
    Some(Legacy2 { bpm, beats: grid })
}
