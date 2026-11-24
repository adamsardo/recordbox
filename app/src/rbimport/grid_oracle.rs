//! Replay a desktop user's single rightward 1 ms nudge on copies of their ANLZ files.
use super::writeback::grid;
use anyhow::{ensure, Context, Result};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "set RECORDBOX_TEST_GRID_NUDGE to a folder with before/, after/ and content-id.txt"]
fn whole_grid_nudge_matches_four_native_files() -> Result<()> {
    let captures = PathBuf::from(
        std::env::var_os("RECORDBOX_TEST_GRID_NUDGE").context("Missing grid-nudge captures")?,
    );
    let owner = fs::read_to_string(captures.join("content-id.txt"))?;
    let scratch = tempfile::tempdir()?;
    let share = scratch.path().join("share");
    let analysis = "PIONEER/USBANLZ/P001/ANLZ0000.DAT";
    let dat = share.join(analysis);
    fs::create_dir_all(dat.parent().unwrap())?;
    let extensions = ["DAT", "EXT", "2EX", "3EX"];
    for extension in extensions {
        let source = captures
            .join("before")
            .join("ANLZ0000")
            .with_extension(extension);
        fs::copy(source, dat.with_extension(extension))?;
    }
    let initial = grid::inspect(&share, analysis)?;
    ensure!(
        !initial.grid.is_empty(),
        "Capture needs a populated beat grid"
    );
    let mut desired = initial.grid.clone();
    for anchor in &mut desired.anchors {
        anchor.time += 0.001;
    }
    let changes = grid::plan(&share, analysis, &initial, &desired, owner.trim(), None)?;
    ensure!(!changes.is_empty(), "No grid changes planned");
    for change in changes {
        let path = scratch.path().join(change.relative_path);
        ensure!(
            extensions.iter().any(|ext| path == dat.with_extension(ext)),
            "Planned change outside the captured file set"
        );
        fs::write(path, change.contents)?;
    }
    for extension in extensions {
        let expected = captures
            .join("after")
            .join("ANLZ0000")
            .with_extension(extension);
        ensure!(
            fs::read(dat.with_extension(extension))? == fs::read(expected)?,
            "Edited {extension} bytes differ from the native capture"
        );
    }
    Ok(())
}
