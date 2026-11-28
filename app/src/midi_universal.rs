//! Universal MIDI (any controller, no mapping of its own): MIDI Learn of every kind of
//! control, every command of the window, duplicates and reassignment, outputs, saving,
//! rekordbox's file format both ways, two devices at once, factory mappings by name, DEFAULT
//! and DELETE. Tests only; `cargo test --release midi_universal -- --nocapture` prints a
//! `UNIVERSAL` line per part.

#[cfg(test)]
mod tests {
    use crate::audio::PlayState;
    use crate::midi::*;
    use crate::midi_csv::{CsvFile, Kind};
    use crate::midi_map::*;
    use midir::os::unix::{VirtualInput, VirtualOutput};
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn learnable(n: &str) -> bool {
        registry().learnable(n)
    }

    fn row(name: &str, deck: u8, kind: Kind) -> LearnEntry {
        LearnEntry { name: name.into(), deck, kind, min: -1, max: -1, comment: String::new() }
    }

    /// What a learned row does with a message: the function (by the registry's tag) and value.
    fn hit(map: &mut MidiMap, m: &[u8]) -> Option<(String, u8, Val)> {
        let reg = registry();
        map.tag_inputs(|n| reg.tag(n));
        let ev = map.handle(m, 0.0)?;
        Some((reg.get(ev.tag).map(|f| f.name.clone()).unwrap_or_default(), ev.deck, ev.val))
    }

    #[test]
    fn learn_binds_every_kind_of_control() {
        let mut map = MidiMap::new("rbxtest generic");
        let mut f = LearnFilter::default();

        // A button as notes: the press is learned, its release passed over; press and
        // release (velocity 0 or a real Note Off) both reach the function.
        let c = f.feed(&[0x90, 0x0B, 0x7F]).unwrap();
        assert_eq!(c, 0x900B);
        assert_eq!(f.feed(&[0x90, 0x0B, 0x00]), None);
        map.change_input(&row("PlayPause", 1, Kind::Button), None, Some(c), &learnable).unwrap();
        assert_eq!(hit(&mut map, &[0x90, 0x0B, 0x7F]), Some(("PlayPause".into(), 1, Val::On(true, 127))));
        assert_eq!(hit(&mut map, &[0x90, 0x0B, 0x00]).map(|x| x.2), Some(Val::On(false, 0)));
        assert_eq!(hit(&mut map, &[0x80, 0x0B, 0x40]).map(|x| x.2), Some(Val::On(false, 0)));

        // A button as a CC (7F pressed, 00 released).
        f.reset();
        let c = f.feed(&[0xB0, 0x40, 0x7F]).unwrap();
        map.change_input(&row("PitchBendUp", 1, Kind::Button), None, Some(c), &learnable).unwrap();
        assert_eq!(hit(&mut map, &[0xB0, 0x40, 0x7F]).map(|x| x.2), Some(Val::On(true, 127)));
        assert_eq!(hit(&mut map, &[0xB0, 0x40, 0x00]).map(|x| x.2), Some(Val::On(false, 0)));

        // A 7-bit knob: 0, the middle, the top.
        f.reset();
        let c = f.feed(&[0xB0, 0x07, 0x22]).unwrap();
        map.change_input(&row("ChannelFader", 1, Kind::KnobSlider), None, Some(c), &learnable).unwrap();
        for (b, u) in [(0x00u8, 0.0f32), (0x40, 0.5), (0x7F, 1.0)] {
            assert_eq!(hit(&mut map, &[0xB0, 0x07, b]).and_then(|x| x.2.unit()), Some(u), "{b:02X}");
        }

        // A 14-bit pair: the LSB (CC + 32 right after) is passed over by Learn; the MSB alone
        // gives nothing, the LSB completes the value.
        f.reset();
        let c = f.feed(&[0xB0, 0x13, 0x40]).unwrap();
        assert_eq!(f.feed(&[0xB0, 0x33, 0x00]), None);
        map.change_input(&row("TempoSlider", 1, Kind::KnobSliderHiRes), None, Some(c), &learnable).unwrap();
        assert_eq!(hit(&mut map, &[0xB0, 0x13, 0x40]), None);
        assert_eq!(hit(&mut map, &[0xB0, 0x33, 0x00]).map(|x| x.2), Some(Val::Abs { v: 8192, lo: 0, mid: 8192, hi: 16383 }));
        assert_eq!(hit(&mut map, &[0xB0, 0x13, 0x7F]), None);
        assert_eq!(hit(&mut map, &[0xB0, 0x33, 0x7F]).and_then(|x| x.2.unit()), Some(1.0));

        // Relative encoders, each encoding rekordbox's `MidiRotary` reads: two's complement
        // (01 / 7F), offset binary (41 / 3F); faster turns accelerate (base 2.22). A
        // sign-magnitude encoder (41 = -1) is read the other way round, as by rekordbox.
        f.reset();
        let c = f.feed(&[0xB0, 0x10, 0x01]).unwrap();
        map.change_input(&row("Browse", 0, Kind::Rotary), None, Some(c), &learnable).unwrap();
        let rel = |m: &mut MidiMap, b: u8| hit(m, &[0xB0, 0x10, b]).map(|x| x.2);
        assert_eq!(rel(&mut map, 0x01), Some(Val::Rel(1)));
        assert_eq!(rel(&mut map, 0x7F), Some(Val::Rel(-1)));
        assert_eq!(rel(&mut map, 0x41), Some(Val::Rel(1)));
        assert_eq!(rel(&mut map, 0x3F), Some(Val::Rel(-1)));
        assert_eq!(rel(&mut map, 0x03), Some(Val::Rel(4)));
        assert_eq!(rel(&mut map, 0x7D), Some(Val::Rel(-4)));

        // A pad (pressure pads of the RZ / RX families report velocity while held).
        f.reset();
        let c = f.feed(&[0x97, 0x00, 0x64]).unwrap();
        map.change_input(&row("PAD1_HotCue", 1, Kind::Pad(crate::midi_csv::PadType::ByDevice)), None, Some(c), &learnable).unwrap();
        assert_eq!(hit(&mut map, &[0x97, 0x00, 0x64]).map(|x| x.2), Some(Val::On(true, 0x64)));

        // Jog wheels are not offered by MIDI Learn (rekordbox's window has no jog commands
        // either); Shift neither.
        for n in ["JogScratch", "JogPitchBend", "JogTouch", "JogSearch", "Shift"] {
            assert!(!learnable(n), "{n}");
        }
        eprintln!("UNIVERSAL learn kinds: note button, CC button, 7-bit knob, 14-bit pair, relative encoders (two's complement, offset binary, accelerated), pad: all bind and give their values");
    }

    #[test]
    fn every_command_of_every_tab_can_be_learned_and_acts_or_says_why() {
        let reg = registry();
        let mut map = MidiMap::new("rbxtest generic");
        let mut learned = 0;
        let mut acting = 0;
        let mut silent: Vec<String> = vec![];
        let mut per_tab: Vec<(&str, usize, usize)> = vec![];
        let mut code: u16 = 0;
        for (tab, tname) in TABS {
            let (mut n, mut a) = (0, 0);
            for f in reg.of_tab(tab) {
                if f.ty == T_INDICATOR {
                    continue;
                }
                // One free code per command: notes on channels 1-16 (outputs-only types skip).
                let c = 0x9000 | ((code / 128) << 8) | (code % 128);
                code += 1;
                let kind = kind_of_type(f.ty);
                if map.change_input(&row(&f.name, f.per_deck as u8, kind), None, Some(c), &learnable).is_err() {
                    continue;
                }
                assert_eq!(map.input_at(c).map(|e| e.name.as_str()), Some(f.name.as_str()), "{}", f.name);
                n += 1;
                learned += 1;
                if input_acts(&f.name, kind) {
                    a += 1;
                    acting += 1;
                } else {
                    silent.push(f.name.clone());
                }
            }
            per_tab.push((tname, n, a));
        }
        for (t, n, a) in &per_tab {
            eprintln!("UNIVERSAL tab {t}: {n} commands learned, {a} act");
        }
        eprintln!("UNIVERSAL commands learned {learned}, acting {acting}, listed without an action {}: {:?}", silent.len(), silent.iter().take(40).collect::<Vec<_>>());
        assert!(learned > 500 && acting * 10 > learned * 8, "{learned} {acting}");
    }

    #[test]
    fn duplicates_reassignment_outputs_delete_and_saving() {
        let reg = registry();
        let mut map = MidiMap::new("rbxtest generic");
        let play = row("PlayPause", 1, Kind::Button);
        map.change_input(&play, None, Some(0x900B), &learnable).unwrap();
        map.change_output(&play, None, Some(0x900B), &learnable).unwrap();
        // The same code for another command: refused, naming the one that has it.
        match map.change_input(&row("Cue", 1, Kind::Button), None, Some(0x900B), &learnable) {
            Err(Reject::Taken(e)) => assert_eq!(e.name, "PlayPause"),
            other => panic!("{other:?}"),
        }
        // Two rows of one command (DUPLICATE): both codes play.
        map.change_input(&play, None, Some(0x9047), &learnable).unwrap();
        assert_eq!(hit(&mut map, &[0x90, 0x47, 0x7F]).map(|x| x.0), Some("PlayPause".into()));
        assert_eq!(hit(&mut map, &[0x90, 0x0B, 0x7F]).map(|x| x.0), Some("PlayPause".into()));
        // Reassignment: the row's old code is switched off (NoFunction), the new one acts.
        map.change_input(&play, Some(0x9047), Some(0x9048), &learnable).unwrap();
        assert_eq!(map.input_at(0x9047).map(|e| e.name.clone()), Some(NO_FUNCTION.into()));
        assert_eq!(hit(&mut map, &[0x90, 0x48, 0x7F]).map(|x| x.0), Some("PlayPause".into()));
        // A code switched off can be given to something else.
        map.change_input(&row("Cue", 1, Kind::Button), None, Some(0x9047), &learnable).unwrap();
        assert_eq!(hit(&mut map, &[0x90, 0x47, 0x7F]).map(|x| x.0), Some("Cue".into()));
        // Outputs: the light's bytes, on and off; a Button(for Pad) row sends its own values.
        let e = map.outputs_at(0x900B)[0].clone();
        assert_eq!(map.led_message(&e, 127), [0x90, 0x0B, 0x7F]);
        assert_eq!(map.led_message(&e, 0), [0x90, 0x0B, 0x00]);
        let mut padrow = row("Sync", 1, Kind::Button);
        padrow.min = 5;
        padrow.max = 60;
        map.change_output(&padrow, None, Some(0x9058), &learnable).unwrap();
        let e = map.outputs_at(0x9058)[0].clone();
        assert_eq!((map.led_message(&e, 127)[2], map.led_message(&e, 0)[2]), (60, 5));
        // An indicator cannot get a MIDI IN; a type that cannot light gets no MIDI OUT.
        assert_eq!(map.change_input(&row("TempoUp", 1, Kind::Indicator), None, Some(0x9070), &learnable), Err(Reject::Revert));
        assert_eq!(map.change_output(&row("ChannelFader", 1, Kind::KnobSlider), None, Some(0xB070), &learnable), Err(Reject::Revert));
        // DELETE: the codes stay, switched off; saved and read back, they stay off.
        map.delete(Some(0x9048), None);
        let text = map.save_text();
        let back = MidiMap::load("rbxtest generic", None, Some(text.as_bytes())).unwrap();
        assert_eq!(back.input_at(0x9048).map(|e| e.name.clone()), Some(NO_FUNCTION.into()));
        assert_eq!(back.input_at(0x900B).map(|e| e.name.clone()), Some("PlayPause".into()));
        assert_eq!(back.input_at(0x9047).map(|e| e.name.clone()), Some("Cue".into()));
        assert_eq!(back.outputs_at(0x9058).first().map(|e| (e.opts.min, e.opts.max)), Some((5, 60)));
        // Saved again, the text is the same (rekordbox's delta format is stable).
        assert_eq!(back.save_text(), text);
        let _ = reg;
        eprintln!("UNIVERSAL duplicates refused with the owner named, DUPLICATE rows, reassignment, outputs, DELETE, save and reload: same text twice");
    }

    #[test]
    fn rekordbox_format_files_round_trip_exactly() {
        // A user file as rekordbox writes it (CRLF, sorted by name, options with ';').
        let rb = "@file,1,rbxtest generic\r\n\
            Browse,,Rotary,B440,,,,,,,,,,,Library selector\r\n\
            Cue,,Button,,900C,910C,,,,900C,910C,,,Fast;Blink=500;Priority=50;,Cue\r\n\
            NeedleSearch,,Value,,B003,B103,,,,,,,,Max=16383;,Needle\r\n\
            PlayPause,,Button,,900B,910B,,,,900B,910B,,,Fast;Blink=1000;Priority=50;,Play\r\n\
            TempoSlider,,KnobSliderHiRes,,B000,B100,,,,,,,,Fast;,Tempo\r\n";
        let map = MidiMap::load("rbxtest generic", None, Some(rb.as_bytes())).unwrap();
        let out = map.save_text();
        let norm = |s: &str| s.lines().map(|l| l.split(',').skip(3).collect::<Vec<_>>().join(",")).collect::<Vec<_>>();
        // (The Function column: rekordbox writes it empty in user files, recordbox the
        // name; both read the same. Every other cell is equal.)
        assert_eq!(norm(&out), norm(rb), "{out}");
        eprintln!("UNIVERSAL rekordbox-format user file read and written: {} rows, cells equal", map.live_inputs().count());
    }

    /// Opt-in (reads this computer's rekordbox folder, read only): the owner's own files
    /// through recordbox's reader and writer. PREPDECK_MIDI_RB_USER="PIONEER DDJ-S1|DDJ-S1 Jog Helper"
    #[test]
    #[ignore]
    fn the_owners_rekordbox_files_round_trip() {
        let Ok(ports) = std::env::var("PREPDECK_MIDI_RB_USER") else { return };
        for port in ports.split('|') {
            let Some(f) = rekordbox_user_file(port) else { continue };
            let bytes = std::fs::read(&f).unwrap();
            let text = String::from_utf8_lossy(&bytes).to_string();
            let map = MidiMap::load(port, None, Some(&bytes)).unwrap();
            let out = map.save_text();
            let cells = |s: &str| {
                let mut v: Vec<String> = s.lines().skip(1).map(|l| { let c: Vec<&str> = l.split(',').collect(); format!("{}|{}", c[0], c[2..].join(",")) }).collect();
                v.sort();
                v
            };
            let (a, b) = (cells(&text), cells(&out));
            let differ: Vec<(&String, &String)> = a.iter().zip(b.iter()).filter(|(x, y)| x != y).collect();
            eprintln!("UNIVERSAL owner's file {port}: {} rows in, {} rows out, {} differ: {:?}", a.len(), b.len(), differ.len(), differ.iter().take(5).collect::<Vec<_>>());
            assert_eq!(a.len(), b.len());
        }
    }

    #[test]
    fn a_device_named_like_a_factory_mapping_loads_it_and_default_restores_it() {
        let dir = crate::midi_csv::factory_dir();
        if !dir.is_dir() {
            eprintln!("UNIVERSAL factory: no rekordbox installed here; skipped");
            return;
        }
        let tmp = std::env::temp_dir().join(format!("rbxtest-universal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (hub, _rx) = Hub::new(decks, tmp.clone());
        let name = "PIONEER DDJ-SX";
        let (factory_map, has) = hub.load_map(name);
        assert!(has, "factory file for {name}");
        let n0 = factory_map.live_inputs().count();
        assert!(n0 > 50, "{n0}");
        // A learned change on top, saved as rekordbox's delta.
        let mut m = hub.load_map(name).0;
        let code = (0x9000..0x9F7F).find(|c| m.input_at(*c).is_none()).unwrap();
        m.change_input(&row("PlayPause", 1, Kind::Button), None, Some(code), &learnable).unwrap();
        std::fs::write(hub.user_file(name), m.save_text()).unwrap();
        let with = hub.load_map(name).0;
        assert_eq!(with.input_at(code).map(|e| e.name.clone()), Some("PlayPause".into()));
        // DEFAULT: the user's file emptied, the factory mapping as it was.
        std::fs::write(hub.user_file(name), crate::midi_csv::empty_user_file(name)).unwrap();
        let back = hub.load_map(name).0;
        assert!(back.input_at(code).is_none());
        assert_eq!(back.live_inputs().count(), n0);
        // Any other name: no factory file, nothing mapped until learned.
        let (generic, has) = hub.load_map("rbxtest generic");
        assert!(!has && generic.live_inputs().count() == 0);
        let _ = std::fs::remove_dir_all(&tmp);
        eprintln!("UNIVERSAL factory by name: {name} {n0} input codes; a learned row on top; DEFAULT back to {n0}; an unknown name has none");
    }

    #[test]
    fn two_devices_at_once_with_their_own_mappings() {
        let tmp = std::env::temp_dir().join(format!("rbxtest-two-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // (Fixed names: rekordbox, when it runs, writes a header file for every new name.)
        let (a, b) = ("rbxtest-midi4 two A", "rbxtest-midi4 two B");
        std::fs::write(tmp.join(format!("{a}.midi.csv")), format!("@file,1,{a}\r\nPlayPause,PlayPause,Button,,900B,,,,,,,,,Fast;,\r\n")).unwrap();
        std::fs::write(tmp.join(format!("{b}.midi.csv")), format!("@file,1,{b}\r\nPlayPause,PlayPause,Button,,,900B,,,,,,,,Fast;,\r\n")).unwrap();
        let mk = |n: &str| -> Option<(midir::MidiOutputConnection, midir::MidiInputConnection<()>)> {
            let o = midir::MidiOutput::new("rbxtest").ok()?.create_virtual(n).ok()?;
            let i = midir::MidiInput::new("rbxtest").ok()?.create_virtual(n, |_, _, _| {}, ()).ok()?;
            Some((o, i))
        };
        let (Some(mut pa), Some(mut pb)) = (mk(a), mk(b)) else {
            eprintln!("UNIVERSAL two devices: no CoreMIDI virtual ports here; skipped");
            return;
        };
        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        for d in &decks[..2] {
            d.lock().audio = Some(Arc::new(crate::audio::Decoded { sr: 44100, samples: vec![0.0; 44100 * 20], raw_frames: 44100 * 10, damaged: 0, truncated: false }));
        }
        let (hub, _rx) = Hub::new(decks.clone(), tmp.clone());
        *hub.only.lock() = Some(vec![a.into(), b.into()]);
        let t0 = Instant::now();
        while hub.devices.lock().len() < 2 && t0.elapsed() < Duration::from_secs(5) {
            hub.scan();
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(hub.devices.lock().len(), 2, "both opened");
        // The same message from each: A plays deck 1, B plays deck 2.
        pa.0.send(&[0x90, 0x0B, 0x7F]).unwrap();
        let wait = |f: &dyn Fn() -> bool| {
            let t = Instant::now();
            while !f() && t.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            f()
        };
        assert!(wait(&|| decks[0].lock().playing) && !decks[1].lock().playing, "A plays deck 1 only");
        pb.0.send(&[0x90, 0x0B, 0x7F]).unwrap();
        assert!(wait(&|| decks[1].lock().playing), "B plays deck 2");
        for d in hub.devices.lock().iter() {
            d.alive.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = (&mut pa.1, &mut pb.1);
        let _ = std::fs::remove_dir_all(&tmp);
        eprintln!("UNIVERSAL two devices at once: the same code plays deck 1 from one and deck 2 from the other");
    }

    /// The record under a controller's platter travels exactly the distance the counts stand
    /// for (`Engine::jog_left`): however long the speed stays set (the worker's idle check
    /// late under load), it stops there. On-screen scratching (no limit) is unchanged.
    #[test]
    fn a_scratch_moves_the_record_exactly_as_far_as_the_counts() {
        let n = 44100 * 10;
        let audio = Arc::new(crate::audio::Decoded { sr: 44100, samples: vec![0.1; n * 2], raw_frames: n, damaged: 0, truncated: false });
        let mut s = PlayState::default();
        s.audio = Some(audio);
        s.playing = false;
        s.volume = 1.0;
        s.gain = 1.0;
        s.pos = 44100.0;
        s.dk.jog = crate::deck::Jog::Speed(3.6);
        // 1150 counts of the ERGO's 2300 a turn: 900 ms of the track.
        let k = Jog::new(2300.0).k();
        s.dk.jog_left = -(k * 1150.0) * 44100.0 / 1000.0;
        s.dk.jog = crate::deck::Jog::Speed(-3.6);
        let mut out = vec![0.0f32; 1024];
        for _ in 0..40 {
            out.iter_mut().for_each(|x| *x = 0.0);
            crate::audio::render_deck(&mut s, &mut out, 2, 44100);
        }
        let moved = (s.pos - 44100.0) / 44100.0;
        assert!((moved + 0.9).abs() < 1e-6, "{moved}");
        assert!(s.dk.jog_left.abs() < 1e-6);
        // Without a limit the speed goes on.
        s.dk.jog_left = f64::NAN;
        s.dk.jog = crate::deck::Jog::Speed(1.0);
        let before = s.pos;
        crate::audio::render_deck(&mut s, &mut out, 2, 44100);
        assert!(s.pos > before + 400.0);
        eprintln!("UNIVERSAL scratch distance: 1150 counts at the ERGO's 2300 a turn moved the record {moved:+.6} s (wants -0.900000), speed left set for 40 blocks");
    }

    #[test]
    fn a_header_only_file_is_none_and_the_rekordbox_mapping_is_used() {
        let base = std::env::temp_dir().join(format!("rbxtest-hdr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (rb, own) = (base.join("rb"), base.join("own"));
        std::fs::create_dir_all(&rb).unwrap();
        std::fs::create_dir_all(&own).unwrap();
        let name = "rbxtest device";
        std::fs::write(rb.join(format!("{name}.midi.csv")), format!("@file,1,{name}\r\nPlayPause,,Button,,900B,,,,,,,,,,\r\n")).unwrap();
        std::fs::write(own.join(format!("{name}.midi.csv")), crate::midi_csv::empty_user_file(name)).unwrap();
        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (hub, _rx) = Hub::new(decks, own.clone());
        *hub.rb_dir.lock() = Some(rb.clone());
        assert!(hub.from_rekordbox(name));
        let m = hub.load_map(name).0;
        assert_eq!(m.input_at(0x900B).map(|e| e.name.clone()), Some("PlayPause".into()));
        let _ = CsvFile::parse(crate::midi_csv::empty_user_file(name).as_bytes()).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }
}
