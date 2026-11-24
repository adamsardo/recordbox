//! Exercise the classic catalog and its ANLZ references together on pinned public exports.
//! Both tests copy only metadata and write exclusively into a temporary device directory.
use super::{anlz, pdb::Database, usb};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn files(root: &Path) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    let mut hashes = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let relative = entry.path().strip_prefix(root)?.to_owned();
            hashes.insert(relative, Sha256::digest(fs::read(entry.path())?).into());
        }
    }
    Ok(hashes)
}

fn track_references(lib: &usb::DeviceLibrary) -> BTreeMap<u32, (&str, &str, &str)> {
    lib.tracks
        .iter()
        .map(|(&id, track)| {
            (
                id,
                (
                    track.title.as_str(),
                    track.file_path.as_str(),
                    track.analyze_path.as_str(),
                ),
            )
        })
        .collect()
}

fn catalog_tables(root: &Path, file: &str) -> Result<BTreeSet<u32>> {
    let db = Database::parse(fs::read(root.join("PIONEER/rekordbox").join(file))?)?;
    Ok(db.tables.iter().map(|table| table.0).collect())
}

fn verify_fixture(
    checkout: &str,
    export: &str,
    tracks: usize,
    cues: usize,
    mixed: bool,
) -> Result<()> {
    let library = PathBuf::from(
        std::env::var_os(checkout)
            .with_context(|| format!("Set {checkout} to the fixture checkout"))?,
    )
    .join(export);
    let pioneer = library.join("PIONEER");
    let before = files(&pioneer)?;
    let device = tempfile::tempdir()?;
    let root = device.path();
    let copy = root.join("PIONEER");
    for relative in before.keys() {
        let target = copy.join(relative);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(pioneer.join(relative), target)?;
    }

    assert_eq!(usb::has_onelibrary(root), mixed);
    assert_eq!(catalog_tables(root, "export.pdb")?, (0..20).collect());
    assert_eq!(catalog_tables(root, "exportExt.pdb")?, (0..9).collect());
    let read = usb::read_device(root)?;
    assert_eq!(read.tracks.len(), tracks);
    let mut beats = Vec::new();
    let mut cue_count = 0;
    for track in read.tracks.values() {
        let base = usb::anlz_base(&track.analyze_path).context("missing device analysis path")?;
        let base = root.join(base.trim_start_matches('/'));
        for extension in ["DAT", "EXT", "2EX"] {
            let bytes = fs::read(base.with_extension(extension))?;
            anlz::validate(&bytes).map_err(anyhow::Error::msg)?;
            let parts = anlz::section_list(&bytes);
            let path = parts
                .iter()
                .find(|(tag, _)| tag == "PPTH")
                .and_then(|(_, p)| anlz::parse_ppth(p));
            assert_eq!(path.as_deref(), Some(track.file_path.as_str()));
        }
        let dat = anlz::read_valid_section_list(&base.with_extension("DAT"));
        let ext = anlz::read_valid_section_list(&base.with_extension("EXT"));
        cue_count += anlz::parse_cues(&ext, &dat).len();
        if let Some((_, beatgrid)) = dat.iter().find(|(tag, _)| tag == "PQTZ") {
            let count = anlz::pqtz_entries(beatgrid).len();
            if count > 0 {
                beats.push(count);
            }
        }
    }
    beats.sort_unstable();
    assert_eq!(beats, [257, 368]);
    assert_eq!(cue_count, cues);

    let (&id, example) = read
        .tracks
        .first_key_value()
        .context("missing fixture track")?;
    let new_rating = (example.rating + 1) % 6;
    usb::change_device(root, |lib| {
        lib.tracks
            .get_mut(&id)
            .context("missing track on review")?
            .rating = new_rating;
        Ok(vec![])
    })?;
    let updated = usb::read_device(root)?;
    assert_eq!(updated.tracks[&id].rating, new_rating);
    assert_eq!(track_references(&updated), track_references(&read));
    let playlists = |lib: &usb::DeviceLibrary| {
        lib.nodes
            .iter()
            .map(|n| (n.id, n.parent, n.entries.clone()))
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(playlists(&read), playlists(&updated));
    assert_eq!(catalog_tables(root, "export.pdb")?, (0..20).collect());
    assert_eq!(catalog_tables(root, "exportExt.pdb")?, (0..9).collect());
    let mut untouched = before.clone();
    let mut after = files(&copy)?;
    for name in ["rekordbox/export.pdb", "rekordbox/exportExt.pdb"] {
        ensure!(
            untouched.remove(Path::new(name)).is_some(),
            "missing original {name}"
        );
        ensure!(
            after.remove(Path::new(name)).is_some(),
            "missing edited {name}"
        );
    }
    assert_eq!(
        after, untouched,
        "USB analysis, OneLibrary and player settings must survive a classic catalog edit"
    );
    assert_eq!(files(&pioneer)?, before);
    Ok(())
}

#[test]
#[ignore = "set RECORDBOX_TEST_PYREKORDBOX to the pinned public fixture checkout"]
fn classic_catalog_and_anlz_remain_linked_after_a_rating_edit() -> Result<()> {
    verify_fixture(
        "RECORDBOX_TEST_PYREKORDBOX",
        ".testdata/export",
        6,
        0,
        false,
    )
}

#[test]
#[ignore = "set RECORDBOX_TEST_RBOX to the pinned public fixture checkout"]
fn mixed_export_keeps_onelibrary_and_shared_anlz_intact() -> Result<()> {
    verify_fixture(
        "RECORDBOX_TEST_RBOX",
        ".testdata/RBv7/demo/export",
        2,
        6,
        true,
    )
}
