// Generated from rekordcrate src/setting.rs (MIT): option labels are the names rekordbox
// shows in Preferences > DJ System > My Settings; values are the stored bytes (high bit = set).
use super::settings::{Field, SettingFile};

pub const MY_SETTINGS: &[Field] = &[
    Field { file: SettingFile::My, offset: 8, name: "On Air Display", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 9, name: "LCD Brightness", options: &[("1", 0x81), ("2", 0x82), ("3", 0x83), ("4", 0x84), ("5", 0x85)] },
    Field { file: SettingFile::My, offset: 10, name: "Quantize", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 11, name: "Auto Cue Level", options: &[("-78dB", 0x87), ("-72dB", 0x86), ("-66dB", 0x85), ("-60dB", 0x84), ("-54dB", 0x83), ("-48dB", 0x82), ("-42dB", 0x81), ("-36dB", 0x80), ("MEMORY", 0x88)] },
    Field { file: SettingFile::My, offset: 12, name: "Language", options: &[("English", 0x81), ("Français", 0x82), ("Deutsch", 0x83), ("Italiano", 0x84), ("Nederlands", 0x85), ("Español", 0x86), ("Русский", 0x87), ("한국어", 0x88), ("简体中文", 0x89), ("繁體中文", 0x8A), ("日本語", 0x8B), ("Português", 0x8C), ("Svenska", 0x8D), ("Čeština", 0x8E), ("Magyar", 0x8F), ("Dansk", 0x90), ("Ελληνικά", 0x91), ("Türkçe", 0x92)] },
    Field { file: SettingFile::My, offset: 14, name: "Jog Ring Brightness", options: &[("OFF", 0x80), ("1 (Dark)", 0x81), ("2 (Bright)", 0x82)] },
    Field { file: SettingFile::My, offset: 15, name: "Jog Ring Indicator", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 16, name: "Slip Flashing", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 20, name: "Disc Slot Illumination", options: &[("OFF", 0x80), ("1 (Dark)", 0x81), ("2 (Bright)", 0x82)] },
    Field { file: SettingFile::My, offset: 21, name: "Eject/Load Lock", options: &[("UNLOCK", 0x80), ("LOCK", 0x81)] },
    Field { file: SettingFile::My, offset: 22, name: "Sync", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 23, name: "Play Mode / Auto Play Mode", options: &[("CONTINUE / ON", 0x80), ("SINGLE / OFF", 0x81)] },
    Field { file: SettingFile::My, offset: 24, name: "Quantize Beat Value", options: &[("1/8 Beat", 0x83), ("1/4 Beat", 0x82), ("1/2 Beat", 0x81), ("1 Beat", 0x80)] },
    Field { file: SettingFile::My, offset: 25, name: "Hot Cue Auto Load", options: &[("OFF", 0x80), ("rekordbox SETTING", 0x82), ("On", 0x81)] },
    Field { file: SettingFile::My, offset: 26, name: "Hot Cue Color", options: &[("OFF", 0x80), ("On", 0x81)] },
    Field { file: SettingFile::My, offset: 29, name: "Needle Lock", options: &[("UNLOCK", 0x80), ("LOCK", 0x81)] },
    Field { file: SettingFile::My, offset: 32, name: "Time Mode", options: &[("Elapsed", 0x80), ("REMAIN", 0x81)] },
    Field { file: SettingFile::My, offset: 33, name: "Jog Mode", options: &[("VINYL", 0x81), ("CDJ", 0x80)] },
    Field { file: SettingFile::My, offset: 34, name: "Auto Cue", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 35, name: "Master Tempo", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::My, offset: 36, name: "Tempo Range", options: &[("±6", 0x80), ("±10", 0x81), ("±16", 0x82), ("WIDE", 0x83)] },
    Field { file: SettingFile::My, offset: 37, name: "Phase Meter", options: &[("TYPE 1", 0x80), ("TYPE 2", 0x81)] },
    Field { file: SettingFile::My2, offset: 0, name: "Vinyl Speed Adjust", options: &[("TOUCH & RELEASE", 0x80), ("TOUCH", 0x81), ("RELEASE", 0x82)] },
    Field { file: SettingFile::My2, offset: 1, name: "Jog Display Mode", options: &[("AUTO", 0x80), ("INFO", 0x81), ("SIMPLE", 0x82), ("ARTWORK", 0x83)] },
    Field { file: SettingFile::My2, offset: 2, name: "Pad Button Brightness", options: &[("1", 0x81), ("2", 0x82), ("3", 0x83), ("4", 0x84)] },
    Field { file: SettingFile::My2, offset: 3, name: "Jog LCD Brightness", options: &[("1", 0x81), ("2", 0x82), ("3", 0x83), ("4", 0x84), ("5", 0x85)] },
    Field { file: SettingFile::My2, offset: 4, name: "Waveform Divisions", options: &[("TIME SCALE", 0x80), ("PHRASE", 0x81)] },
    Field { file: SettingFile::My2, offset: 10, name: "Waveform / Phase Meter", options: &[("WAVEFORM", 0x80), ("PHASE METER", 0x81)] },
    Field { file: SettingFile::My2, offset: 12, name: "Beat Jump Beat Value", options: &[("1/2 BEAT", 0x80), ("1 BEAT", 0x81), ("2 BEAT", 0x82), ("4 BEAT", 0x83), ("8 BEAT", 0x84), ("16 BEAT", 0x85), ("32 BEAT", 0x86), ("64 BEAT", 0x87)] },
];

/// DJM mixer settings (DJMMYSETTING.DAT), from rekordcrate src/setting.rs (MIT): payload
/// offsets 12..=24, in file order.
pub const DJM_SETTINGS: &[Field] = &[
    Field { file: SettingFile::Djm, offset: 12, name: "CH Fader Curve", options: &[("STEEP TOP", 0x80), ("LINEAR", 0x81), ("STEEP BOTTOM", 0x82)] },
    Field { file: SettingFile::Djm, offset: 13, name: "Crossfader Curve", options: &[("CONSTANT POWER", 0x80), ("SLOW CUT", 0x81), ("FAST CUT", 0x82)] },
    Field { file: SettingFile::Djm, offset: 14, name: "Headphones Pre EQ", options: &[("POST EQ", 0x80), ("PRE EQ", 0x81)] },
    Field { file: SettingFile::Djm, offset: 15, name: "Headphones Mono Split", options: &[("STEREO", 0x80), ("MONO SPLIT", 0x81)] },
    Field { file: SettingFile::Djm, offset: 16, name: "Beat FX Quantize", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::Djm, offset: 17, name: "Mic Low Cut", options: &[("OFF", 0x80), ("ON", 0x81)] },
    Field { file: SettingFile::Djm, offset: 18, name: "Talk Over Mode", options: &[("ADVANCED", 0x80), ("NORMAL", 0x81)] },
    Field { file: SettingFile::Djm, offset: 19, name: "Talk Over Level", options: &[("-24dB", 0x80), ("-18dB", 0x81), ("-12dB", 0x82), ("-6dB", 0x83)] },
    Field { file: SettingFile::Djm, offset: 20, name: "MIDI Channel", options: &[("1", 0x80), ("2", 0x81), ("3", 0x82), ("4", 0x83), ("5", 0x84), ("6", 0x85), ("7", 0x86), ("8", 0x87), ("9", 0x88), ("10", 0x89), ("11", 0x8A), ("12", 0x8B), ("13", 0x8C), ("14", 0x8D), ("15", 0x8E), ("16", 0x8F)] },
    Field { file: SettingFile::Djm, offset: 21, name: "MIDI Button Type", options: &[("TOGGLE", 0x80), ("TRIGGER", 0x81)] },
    Field { file: SettingFile::Djm, offset: 22, name: "Display Brightness", options: &[("WHITE", 0x80), ("1", 0x81), ("2", 0x82), ("3", 0x83), ("4", 0x84), ("5", 0x85)] },
    Field { file: SettingFile::Djm, offset: 23, name: "Indicator Brightness", options: &[("1", 0x80), ("2", 0x81), ("3", 0x82)] },
    Field { file: SettingFile::Djm, offset: 24, name: "CH Fader Curve (Long Fader)", options: &[("EXPONENTIAL", 0x80), ("SMOOTH", 0x81), ("LINEAR", 0x82)] },
];
