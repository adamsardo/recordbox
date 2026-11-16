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
        tracks.insert(t.id, ExportTrack { track: t, cues, grid: r.grid, wave: Some(r.wave), tags: vec![("Genre".into(), "Techno".into())] });
    }
    let nodes = vec![ExportNode { name: "Sets".into(), folder: true, children: vec![ExportNode { name: "Friday".into(), folder: false, children: vec![], tracks: vec![2, 1] }], tracks: vec![] }];
    let req = ExportRequest { root: root.clone(), nodes: nodes.clone(), tracks: tracks.clone(), tag_categories: vec![("Genre".into(), vec!["Techno".into(), "House".into()])] };
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
        assert_eq!(t.tags, vec!["Techno".to_string()]);
        assert!(!t.source.is_empty());
    }
    assert!(lib.tracks.values().any(|t| t.file_path.starts_with("/Contents/Test Artist/Album_ One_/")));
    assert!(lib.tracks.values().any(|t| t.file_path.starts_with("/Contents/Test Artist/UnknownAlbum/")));
    // Exporting the same playlist again updates in place instead of duplicating.
    let req2 = ExportRequest { root: root.clone(), nodes, tracks, tag_categories: vec![] };
    run_export(req2, &|_| {}, &cancel).unwrap();
    let lib2 = read_device(&root).unwrap();
    assert_eq!(lib2.tracks.len(), 2);
    assert_eq!(lib2.nodes.len(), 2);
    std::fs::write(base.join("LAST"), root.to_string_lossy().as_bytes()).unwrap();
}
