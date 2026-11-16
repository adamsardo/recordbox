//! End-to-end export test: two synthetic tracks into a folder inside target/.

use super::usb::*;
use crate::model::*;
use std::collections::HashMap;

#[test]
fn full_export_to_folder() {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test");
    let run = chrono::Local::now().format("%H%M%S%3f").to_string();
    let root = base.join(format!("usb_{run}"));
    std::fs::create_dir_all(&root).unwrap();
    let src_dir = base.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let mut tracks = HashMap::new();
    for (i, d) in crate::demo::DEMOS.iter().take(2).enumerate() {
        let p = src_dir.join(format!("{}.wav", d.name));
        if !p.exists() {
            crate::demo::write_wav(&p, 44100, &crate::demo::synth(d, 44100)).unwrap();
        }
        let mut t = crate::importer::read_track(&p);
        t.id = i as i64 + 1;
        t.artist = "Test Artist".into();
        t.album = if i == 0 { "Album: One?".into() } else { String::new() };
        t.key = 7;
        let r = crate::analysis::analyze_file(&t.path, &Default::default()).unwrap();
        t.bpm = r.bpm;
        let cues = vec![
            Cue { id: 1, track_id: t.id, kind: CueKind::Hot, slot: 0, start: 1.0, end: None, color: 43, comment: "Intro".into(), active_loop: false },
            Cue { id: 2, track_id: t.id, kind: CueKind::Memory, slot: -1, start: 8.0, end: Some(12.0), color: 3, comment: String::new(), active_loop: false },
        ];
        tracks.insert(t.id, ExportTrack { track: t, cues, grid: r.grid, wave: Some(r.wave), tags: vec![("Genre".into(), "Techno".into())], rb_anlz: None, grid_same: false });
    }
    let nodes = vec![ExportNode { name: "Sets".into(), folder: true, children: vec![ExportNode { name: "Friday".into(), folder: false, children: vec![], tracks: vec![2, 1] }], tracks: vec![] }];
    let req = ExportRequest { root: root.clone(), nodes: nodes.clone(), tracks: tracks.clone(), tag_categories: vec![("Genre".into(), vec!["Techno".into(), "House".into()])], skipped: vec![], analysis: Default::default(), settings: Default::default(), browse: Default::default(), phrases: true };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let msg = run_export(req, &|_| {}, &cancel).unwrap();
    assert!(msg.contains("2 tracks"), "{msg}");
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.tracks.len(), 2);
    let fri = lib.nodes.iter().find(|n| n.name == "Friday").unwrap();
    assert_eq!(fri.entries.len(), 2);
    for t in lib.tracks.values() {
        assert!(root.join(t.file_path.trim_start_matches('/')).exists(), "{}", t.file_path);
        assert!(root.join(t.analyze_path.trim_start_matches('/')).exists());
        assert_eq!(t.tags, vec![tag_key("Genre", "Techno")]);
        assert!(!t.source.is_empty());
    }
    assert!(lib.tracks.values().any(|t| t.file_path.starts_with("/Contents/Test Artist/Album_ One_/")));
    assert!(lib.tracks.values().any(|t| t.file_path.starts_with("/Contents/Test Artist/UnknownAlbum/")));
    // Exporting the same playlist again updates in place instead of duplicating.
    let req2 = ExportRequest { root: root.clone(), nodes, tracks, tag_categories: vec![], skipped: vec![], analysis: Default::default(), settings: Default::default(), browse: Default::default(), phrases: true };
    run_export(req2, &|_| {}, &cancel).unwrap();
    let lib2 = read_device(&root).unwrap();
    assert_eq!(lib2.tracks.len(), 2);
    assert_eq!(lib2.nodes.len(), 2);
    // Player settings files are written with valid checksums.
    for name in ["MYSETTING.DAT", "MYSETTING2.DAT", "DEVSETTING.DAT", "DJMMYSETTING.DAT"] {
        let b = std::fs::read(root.join("PIONEER").join(name)).unwrap();
        let n = u32::from_le_bytes(b[100..104].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(b[104 + n..108 + n].try_into().unwrap()) as u16;
        let covered = if name == "DJMMYSETTING.DAT" { &b[..104 + n] } else { &b[104..104 + n] };
        assert_eq!(crc, super::settings::crc16_xmodem(covered), "{name}");
    }
    // Re-export keeps each track's analysis path and the My Tag IDs, and backs up the old DB.
    for (id, t) in &lib.tracks {
        assert_eq!(lib2.tracks[id].analyze_path, t.analyze_path);
    }
    assert_eq!(lib2.tag_ids, lib.tag_ids);
    assert!(std::fs::read_dir(root.join(".PrepDeck/backup")).unwrap().count() >= 2);
    // No temporary files are left behind.
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            assert!(!n.ends_with(".part") && !n.ends_with(".tmp_write"), "left over: {n}");
            if e.path().is_dir() {
                stack.push(e.path());
            }
        }
    }
    std::fs::write(base.join("LAST"), root.to_string_lossy().as_bytes()).unwrap();
}
