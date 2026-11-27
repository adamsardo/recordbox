//! How much of rekordbox's controller vocabulary recordbox acts on: every function name and
//! row type of the factory mappings in the installed rekordbox (and of user mappings), with
//! whether recordbox has an action for the input rows and a source for the output rows, and
//! for those it has neither, why. Read-only; nothing here is shipped data.

use crate::midi::{in_class, input_acts, output_source, InClass};
use crate::midi_csv::{CsvFile, Kind};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One function name with one row type, as the mapping files use it.
#[derive(Clone, Debug)]
pub struct Use {
    pub name: String,
    pub kind: Kind,
    /// The files with such a row.
    pub files: BTreeSet<String>,
    /// Some row of this name and type has an input code / an output code.
    pub input: bool,
    pub output: bool,
}

impl Use {
    /// recordbox does what the rows of this name and type ask (every side they have).
    pub fn acts(&self) -> bool {
        (!self.input || input_acts(&self.name, self.kind)) && (!self.output || output_source(&self.name)) && (self.input || self.output)
    }
}

/// The names and types of every mapping file given.
pub fn survey(files: &[std::path::PathBuf]) -> Vec<Use> {
    let mut m: BTreeMap<(String, String), Use> = BTreeMap::new();
    for f in files {
        let Ok(bytes) = std::fs::read(f) else { continue };
        let Ok(csv) = CsvFile::parse(&bytes) else { continue };
        let fname = f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        for r in csv.rows() {
            let has_in = r.input.iter().any(|c| c.is_some()) && in_class(r.kind).is_some();
            let has_out = r.output.iter().any(|c| c.is_some());
            let u = m.entry((r.name.clone(), r.kind.text().to_string())).or_insert_with(|| Use { name: r.name.clone(), kind: r.kind, files: BTreeSet::new(), input: false, output: false });
            u.files.insert(fname.clone());
            u.input |= has_in;
            u.output |= has_out;
        }
    }
    let mut v: Vec<Use> = m.into_values().collect();
    v.sort_by(|a, b| b.files.len().cmp(&a.files.len()).then(a.name.cmp(&b.name)));
    v
}

/// Why recordbox has no action for a function, when the feature is not one recordbox has.
pub fn reason(name: &str) -> Option<&'static str> {
    let n = name;
    let has = |s: &str| n.contains(s);
    let starts = |s: &str| n.starts_with(s);
    if has("Video") || has("TouchFX") || has("TransitionFX") || has("Camera") || starts("Text") || starts("Image") {
        return Some("video (out of scope)");
    }
    if has("Lighting") || has("DMX") || has("Strobe") || has("Ambient") || starts("Mood") || has("LightingBank") || has("Light") {
        return Some("lighting (out of scope)");
    }
    if has("Dvs") || has("DVS") || has("Relative") || has("Absolute") || has("Thru") && !starts("CrossfaderAssign") || starts("Control") && has("Vinyl") || has("Phono") || starts("LinePhono") || has("DeckDvs") {
        return Some("DVS (no timecode vinyl in recordbox)");
    }
    if has("Kuvo") || has("Tidal") || has("Beatport") || has("Soundcloud") || has("Streaming") || has("Internet") {
        return Some("streaming (out of scope)");
    }
    if n.ends_with(".MIC") || n.ends_with("Mic") {
        return Some("hardware-only (the controller's mic input: recordbox has no mic channel)");
    }
    if has("Setting") || starts("PeerUsb") || starts("@") || has("FailSafe") || starts("DeckState") || starts("DualDeckState") || starts("PlayingState") || has("Mic") || starts("InputSelect") || starts("AUXInput") || starts("AuxInput") || starts("Talkover") || has("Booth") || starts("Isolator") || starts("FaderStart") || starts("ChannelFaderStart") || has("Demo") || has("Version") || has("Dump") || starts("MixerMode") || starts("AudioOutput") || starts("PcMix") || starts("ShiftFuncDisp") || starts("LevelMeter") || starts("Headphone") && has("Split") || starts("SmoothEchoTiming") {
        return Some("hardware-only (a setting, state or input of the device itself)");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<std::path::PathBuf> {
        let dir = crate::midi_csv::factory_dir();
        let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".midi.csv")).collect()).unwrap_or_default();
        v.sort();
        // The user's two mappings in rekordbox's folder (read only), when they are there.
        if let Some(home) = std::env::var_os("HOME") {
            let u = Path::new(&home).join("Library/Application Support/Pioneer/rekordbox6/MidiMappings");
            for p in ["PIONEER DDJ-S1.midi.csv", "DDJ-S1 Jog Helper.midi.csv"] {
                if u.join(p).is_file() {
                    v.push(u.join(p));
                }
            }
        }
        v
    }

    /// `cargo test --release midi_cover -- --nocapture`: the table. Lines `COVER` (one per
    /// name and type: files, acts, sides, reason) and the totals line `COVER TOTAL`.
    #[test]
    fn coverage_table() {
        let fs = files();
        if fs.is_empty() {
            eprintln!("COVER: no rekordbox mapping files on this computer; skipped");
            return;
        }
        let uses = survey(&fs);
        let names: BTreeSet<&str> = uses.iter().map(|u| u.name.as_str()).collect();
        let acting: BTreeSet<&str> = names.iter().copied().filter(|n| uses.iter().filter(|u| u.name == *n).all(|u| u.acts())).collect();
        let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
        let mut open = vec![];
        for u in &uses {
            let side = format!("{}{}", if u.input { "in" } else { "" }, if u.output { if u.input { "+out" } else { "out" } } else { "" });
            let r = if u.acts() { "acts" } else { reason(&u.name).unwrap_or("NOT BUILT") };
            if !u.acts() {
                *reasons.entry(r).or_default() += 1;
                if r == "NOT BUILT" {
                    open.push(u);
                }
            }
            eprintln!("COVER {:>3} {:<40} {:<18} {:<6} {}", u.files.len(), u.name, u.kind.text(), side, r);
        }
        let kinds_acting = uses.iter().filter(|u| u.acts()).count();
        eprintln!("COVER TOTAL files {} | functions {} | acting {} | name+type pairs {} | acting pairs {} | not acting by reason {:?}", fs.len(), names.len(), acting.len(), uses.len(), kinds_acting, reasons);
        eprintln!("COVER OPEN (most used first): {}", open.iter().take(80).map(|u| format!("{}:{}({})", u.name, u.kind.text(), u.files.len())).collect::<Vec<_>>().join(" "));
        let _ = InClass::Press;
        assert!(acting.len() > 300);
    }
}
