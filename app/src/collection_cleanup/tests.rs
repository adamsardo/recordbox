use super::*;
use std::time::UNIX_EPOCH;

fn row(id: i64, status: PathStatus, volume: &str) -> Row {
    let present = status == PathStatus::Present;
    let path = format!("{volume}/Music/{id}.flac");
    row_from(
        Track {
            id,
            path: path.clone(),
            title: "House mix".to_owned(),
            ..Track::default()
        },
        PathFinding {
            os: "Windows",
            volume: volume.to_owned(),
            status,
            local_identity: present.then(|| path.into()),
            bytes: present.then_some(32),
            modified: present.then_some(UNIX_EPOCH),
        },
        false,
        false,
    )
    .unwrap()
}

fn report(rows: Vec<Row>, compare_bytes: bool, can_apply: bool) -> Report {
    Report {
        library: PathBuf::from("library.db"),
        can_apply,
        rows,
        volumes: vec![],
        statuses: BTreeMap::new(),
        compare_bytes,
        hashed_bytes: 0,
        hashed_files: 0,
        unverified_files: 0,
    }
}

#[test]
fn review_counts_foreign_cloud_external_and_uncertain_separately_from_confirmed_missing() {
    let mut protected_missing = row(3, PathStatus::Missing, "C:");
    protected_missing.protected_metadata = true;
    let mut mac = row(6, PathStatus::Foreign, "/");
    mac.os = "macOS";
    let mut r = report(
        vec![
            row(1, PathStatus::Present, "C:"),
            row(2, PathStatus::Missing, "C:"),
            protected_missing,
            row(4, PathStatus::External, "E:"),
            row(5, PathStatus::Cloud, "C:"),
            mac,
            row(7, PathStatus::Unavailable, "C:"),
        ],
        false,
        true,
    );
    assign_candidates(&mut r);
    assert_eq!(r.rows[1].candidate, Some(Candidate::Missing));
    assert_eq!(r.rows.iter().filter(|r| r.candidate.is_some()).count(), 1);
    let c = r.volumes.iter().find(|v| v.volume == "C:").unwrap();
    assert_eq!(
        (c.total, c.present, c.missing, c.protected, c.candidates),
        (5, 1, 2, 3, 1)
    );
    assert_eq!(r.volumes.iter().map(|v| v.total).sum::<usize>(), 7);
    assert_eq!(r.statuses[&PathStatus::Cloud], 1);
    assert_eq!(r.statuses[&PathStatus::Missing], 2);
    let e = r.volumes.iter().find(|v| v.volume == "E:").unwrap();
    assert_eq!((e.total, e.missing, e.protected), (1, 0, 1));
}

#[test]
fn default_scan_requires_same_resolved_file_and_keeps_all_distinct_catalog_metadata() {
    let a = row(1, PathStatus::Present, "C:");
    let mut b = row(2, PathStatus::Present, "C:");
    let mut labeled = row(3, PathStatus::Present, "C:");
    b.canonical.clone_from(&a.canonical);
    labeled.canonical.clone_from(&a.canonical);
    labeled.metadata = [0x55; 32]; // separate catalog tags
    let mut kept = row(4, PathStatus::Present, "C:");
    kept.canonical.clone_from(&a.canonical);
    kept.protected_metadata = true; // playlist or cue, keep even when its ID sorts later
    let mut r = report(
        vec![a, b, labeled, kept, row(5, PathStatus::Present, "C:")],
        false,
        true,
    );
    assign_candidates(&mut r);
    assert_eq!(
        r.rows.iter().map(|row| row.candidate).collect::<Vec<_>>(),
        vec![
            Some(Candidate::Duplicate { keeper: 4 }),
            Some(Candidate::Duplicate { keeper: 4 }),
            None,
            None,
            None
        ]
    );
    assert!(r.rows[2].same_file_group);
    assert!(!r.rows[4].same_file_group);
    assert_eq!(r.hashed_bytes, 0);
}

#[test]
fn byte_comparison_skips_protected_locations_zero_unique_sizes_and_limits_actual_reads() {
    let mut rows = (1..=8)
        .map(|id| row(id, PathStatus::Present, "C:"))
        .collect::<Vec<_>>();
    rows[3].bytes = Some(0);
    rows[4].bytes = Some(123);
    rows[5].native = true;
    rows[6].status = PathStatus::Cloud;
    rows[7].status = PathStatus::External;
    let mut r = report(rows, true, true);
    let mut visited = Vec::new();
    hash_matches(&mut r, None, 64, &false.into(), |row, _, used| {
        visited.push(row.id);
        *used = row.bytes.unwrap();
        Ok([1; 32])
    })
    .unwrap();
    assert_eq!(visited, [1, 2]);
    assert_eq!(
        (r.hashed_bytes, r.hashed_files, r.unverified_files),
        (64, 2, 1)
    );
    assign_candidates(&mut r);
    assert_eq!(
        r.rows[1].candidate,
        Some(Candidate::Duplicate { keeper: 1 })
    );
    assert_eq!(r.rows.iter().filter(|r| r.candidate.is_some()).count(), 1);
}

#[test]
fn incomplete_or_different_file_hash_cannot_confirm_a_duplicate() {
    let mut r = report(
        (1..=4).map(|i| row(i, PathStatus::Present, "C:")).collect(),
        true,
        true,
    );
    hash_matches(&mut r, None, 128, &false.into(), |row, _, used| {
        if row.id == 3 {
            *used = 9;
            anyhow::bail!("changed while reading")
        }
        *used = 32;
        Ok([if row.id == 2 { 2 } else { 1 }; 32])
    })
    .unwrap();
    assign_candidates(&mut r);
    assert_eq!(
        (r.hashed_bytes, r.hashed_files, r.unverified_files),
        (105, 3, 1)
    );
    assert_eq!(
        r.rows[3].candidate,
        Some(Candidate::Duplicate { keeper: 1 })
    );
    assert!(r.rows[1].candidate.is_none() && r.rows[2].candidate.is_none());
    assert!(
        hash_matches(&mut r, None, 128, &true.into(), |_, _, _| panic!(
            "cancelled scan read audio"
        ))
        .is_err()
    );
}

#[test]
fn streams_entire_file_and_refuses_changed_or_cloud_source_before_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("local.flac");
    let content = b"the full local file, including its ending";
    std::fs::write(&path, content).unwrap();
    let meta = path.metadata().unwrap();
    let finding = PathFinding {
        os: "macOS",
        volume: "/".into(),
        status: PathStatus::Present,
        local_identity: Some(path.canonicalize().unwrap()),
        bytes: Some(meta.len()),
        modified: Some(meta.modified().unwrap()),
    };
    let row = row_from(
        Track {
            id: 9,
            path: path.to_str().unwrap().into(),
            ..Track::default()
        },
        finding.clone(),
        false,
        false,
    )
    .unwrap();
    let mut bytes = 0;
    let digest = hash_local(
        |_| finding.clone(),
        &row,
        &mut [0; 3],
        128,
        &mut bytes,
        &false.into(),
    )
    .unwrap();
    assert_eq!(digest, <[u8; 32]>::from(Sha256::digest(content)));
    assert_eq!(bytes, content.len() as u64);
    bytes = 0;
    let mut cloud = finding.clone();
    cloud.status = PathStatus::Cloud;
    assert!(hash_local(
        |_| cloud.clone(),
        &row,
        &mut [0; 3],
        128,
        &mut bytes,
        &false.into()
    )
    .is_err());
    assert_eq!(bytes, 0);
    std::fs::write(&path, b"some different data").unwrap();
    assert!(hash_local(
        |_| finding.clone(),
        &row,
        &mut [0; 3],
        128,
        &mut bytes,
        &false.into()
    )
    .is_err());
    assert_eq!(bytes, 0);
}

#[test]
fn rechecks_selected_record_keeper_hash_host_and_database_identity() {
    let mut r = report(
        vec![
            row(1, PathStatus::Present, "C:"),
            row(2, PathStatus::Present, "C:"),
            row(3, PathStatus::Missing, "C:"),
        ],
        true,
        true,
    );
    r.rows[0].digest = Some([99; 32]);
    r.rows[1].digest = Some([99; 32]);
    assign_candidates(&mut r);
    assert!(r.verify_selection(&r, &[2, 3]).is_ok());
    for ids in [vec![1, 2], vec![2, 2], vec![], vec![12]] {
        assert!(r.verify_selection(&r, &ids).is_err());
    }
    let mut changed = r.clone();
    changed.rows[0].digest = Some([9; 32]);
    assert!(r.verify_selection(&changed, &[2]).is_err());
    let mut different_db = r.clone();
    different_db.library = "replaced.db".into();
    assert!(r.verify_selection(&different_db, &[2]).is_err());
    let mut cloud = r.clone();
    cloud.rows[2].status = PathStatus::Cloud;
    assert!(r.verify_selection(&cloud, &[3]).is_err());
    let mut new_cue = r.clone();
    new_cue.rows[2].protected_metadata = true;
    assert!(r.verify_selection(&new_cue, &[3]).is_err());
    let mut linux = r.clone();
    linux.can_apply = false;
    assert!(r.verify_selection(&linux, &[2]).is_err());
}

#[test]
#[cfg(target_os = "linux")]
fn linux_reviews_real_mixed_collection_without_probing_foreign_volumes_or_authorizing_cleanup() {
    let profile = tempfile::tempdir().unwrap();
    let library = profile.path().join("recordbox.db");
    let db = Db::open(&library).unwrap();
    for path in [
        r"C:\Users\DJ\Music\Track.flac",
        r"E:\Audio\other.wav",
        "/Volumes/Scratch/other.wav",
        "/Users/DJ/Dropbox/Set.wav",
        "/home/dj/Music/Local.wav",
    ] {
        db.insert_track(&Track {
            path: path.to_string(),
            title: "Fixture".into(),
            ..Track::default()
        })
        .unwrap();
    }
    drop(db);
    let report = review_path_cancellable(&library, true, &false.into()).unwrap();
    assert_eq!(report.rows.len(), 5);
    assert_eq!(report.volumes.iter().map(|v| v.missing).sum::<usize>(), 0);
    assert_eq!(report.rows.iter().filter(|r| r.os == "Windows").count(), 2);
    assert_eq!(report.rows.iter().filter(|r| r.os == "macOS").count(), 2);
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.os == "Unclassified")
            .count(),
        1
    );
    assert_eq!(report.hashed_bytes, 0);
    assert!(
        !report.can_apply
            && report
                .rows
                .iter()
                .all(|r| r.candidate.is_none() && r.protected())
    );
    assert!(report.verify_selection(&report, &[1]).is_err());
    // Even a fabricated UI report must not bypass the actual host gate.
    let backup_parent = library.parent().unwrap();
    let before: Vec<_> = std::fs::read_dir(backup_parent)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let mut forged = report;
    forged.can_apply = true;
    forged.rows[0].candidate = Some(Candidate::Missing);
    assert!(
        store::apply(&library, &forged, &BTreeSet::from([1]), true, "MOVE 1")
            .unwrap_err()
            .to_string()
            .contains("review only")
    );
    let after: Vec<_> = std::fs::read_dir(backup_parent)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(before, after);
}
