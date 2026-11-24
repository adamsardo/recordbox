//! Local desktop libraries can register each ANLZ file in contentFile. Verify
//! the existing native MD5 and size before replacing either value. SHA-256
//! independently binds the reviewed file to the recovery operation.
use super::storage::FileChange;
use anyhow::{ensure, Context, Result};
use md5::Md5;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;

struct Registration {
    id: String,
    hash: String,
    size: i64,
}

fn ensure_idle(db: &Connection, id: &str, path: &str) -> Result<()> {
    let idle: bool = db.query_row(
            "SELECT rb_local_synced=0 AND coalesce(rb_file_hash_dirty,0)=0 AND coalesce(rb_file_size_dirty,0)=0
             AND coalesce(rb_in_progress,0)=0 AND coalesce(rb_local_file_status,0)=0 AND coalesce(rb_process_type,0)=0
             AND coalesce(rb_temp_path,'')='' AND coalesce(rb_insync_hash,'')='' AND coalesce(rb_insync_local_usn,0)=0
             AND coalesce(rb_data_status,0)=0 AND coalesce(rb_local_data_status,0)=0 AND coalesce(usn,0)=0
             FROM contentFile WHERE ID=? AND rb_local_deleted=0",
            [id], |r| r.get(0),
        ).context("Unrecognized or incomplete native file-registration state")?;
    ensure!(
        idle,
        "{path} has cloud, dirty or pending file-registration state"
    );
    Ok(())
}

/// A missing optional disk sibling is meaningful only if the native file registry
/// also has no record of it. Check the entire registered set before editing native
/// cues or a grid. Never open a path supplied by the registration.
pub(super) fn check_inventory(
    db: &Connection,
    owner: &str,
    share: &Path,
    analysis_path: &str,
) -> Result<()> {
    if !super::registered_files(db, owner)? {
        return Ok(());
    }
    let relative = super::grid::analysis_relative_path(analysis_path)?;
    let base = relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    let mut stmt = db.prepare(
        "SELECT ID,Path,Hash,Size FROM contentFile WHERE ContentID=? AND rb_local_deleted=0",
    )?;
    let mut records = stmt.query([owner])?;
    let mut extensions = std::collections::HashSet::new();
    let mut checked = 0usize;
    while let Some(r) = records.next()? {
        let id: String = r.get(0)?;
        let recorded: String = r.get(1)?;
        let normalized = recorded.replace('\\', "/");
        let (stem, ext) = normalized
            .strip_prefix('/')
            .unwrap_or(&normalized)
            .rsplit_once('.')
            .context("Registered analysis path has no extension")?;
        ensure!(
            stem == base,
            "Native file registration {recorded} lies outside this track's known analysis set"
        );
        ensure!(
            extensions.insert(ext.to_ascii_uppercase()),
            "Duplicate native analysis registration for {recorded}"
        );
        ensure!(
            extensions.len() <= 64,
            "Too many registered analysis companions"
        );
        ensure_idle(db, &id, &recorded)?;
        let hash: String = r.get(2)?;
        let size: i64 = r.get(3)?;
        ensure!(
            hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Unsupported native file checksum for {recorded}"
        );
        let bytes = super::grid::read_companion(share, analysis_path, ext)?
            .with_context(|| format!("Registered analysis companion {recorded} is missing"))?;
        checked = checked
            .checked_add(bytes.len())
            .context("Analysis size overflow")?;
        ensure!(
            checked <= 256 * 1024 * 1024,
            "Registered analysis set exceeds the read limit"
        );
        ensure!(
            i64::try_from(bytes.len())? == size
                && hash.eq_ignore_ascii_case(&format!("{:x}", Md5::digest(&bytes))),
            "Native registration size or checksum is stale for {recorded}"
        );
    }
    Ok(())
}

fn registrations(
    db: &Connection,
    owner: &str,
    source: &Path,
    files: &[FileChange],
) -> Result<Vec<Registration>> {
    if files.is_empty() {
        return Ok(vec![]);
    }
    let table: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='contentFile' COLLATE NOCASE)",
        [], |r| r.get(0),
    )?;
    if !table {
        return Ok(vec![]);
    }
    let has_files: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM contentFile WHERE ContentID=? AND rb_local_deleted=0)",
        [owner],
        |r| r.get(0),
    )?;
    let mut result = Vec::new();
    for file in files {
        let relative = file.relative_path.strip_prefix("share")?;
        let path = format!(
            "/{}",
            relative
                .to_str()
                .context("Non-Unicode ANLZ registration path")?
                .replace('\\', "/")
        );
        let mut matches = db.prepare(
            "SELECT ID,ContentID,Path,Hash,Size,rb_local_path FROM contentFile
             WHERE replace(Path,char(92),'/')=? COLLATE NOCASE AND rb_local_deleted=0",
        )?;
        let mut candidates = matches.query([&path])?;
        let Some(row) = candidates.next()? else {
            ensure!(!has_files, "The desktop registration for {path} is missing");
            continue;
        };
        let id: String = row.get(0)?;
        let registered_owner: String = row.get(1)?;
        let registered_path: String = row.get(2)?;
        let hash: String = row.get(3)?;
        let size: i64 = row.get(4)?;
        let local: Option<String> = row.get(5)?;
        ensure!(
            candidates.next()?.is_none()
                && registered_owner == owner
                && registered_path.replace('\\', "/") == path,
            "Ambiguous native file registration for {path}"
        );
        ensure_idle(db, &id, &path)?;
        let target = source.join(&file.relative_path);
        if let Some(registered) = local.filter(|s| !s.is_empty()) {
            ensure!(
                Path::new(&registered).canonicalize().ok().as_ref()
                    == Some(&target.canonicalize()?),
                "The registered local path for {path} points to a different file"
            );
        }
        let (stem, extension) = path
            .rsplit_once('.')
            .context("Missing analysis file extension")?;
        let old =
            super::grid::read_sidecar(&source.join("share"), &format!("{stem}.DAT"), extension)?
                .context("Registered analysis file is missing")?;
        let before_sha: [u8; 32] = Sha256::digest(&old).into();
        ensure!(
            before_sha == file.expected_sha256
                && i64::try_from(old.len())? == size
                && hash.eq_ignore_ascii_case(&format!("{:x}", Md5::digest(&old))),
            "Native registration size or checksum is stale for {path}"
        );
        let new_hash = format!("{:x}", Md5::digest(&file.contents));
        result.push(Registration {
            id,
            hash: if hash.bytes().any(|b| b.is_ascii_uppercase()) {
                new_hash.to_ascii_uppercase()
            } else {
                new_hash
            },
            size: i64::try_from(file.contents.len())?,
        });
    }
    Ok(result)
}

pub(super) fn check(
    db: &Connection,
    owner: &str,
    source: &Path,
    files: &[FileChange],
) -> Result<()> {
    registrations(db, owner, source, files).map(|_| ())
}

pub(super) fn update(
    db: &Connection,
    owner: &str,
    source: &Path,
    files: &[FileChange],
    usn: &mut i64,
    timestamp: &str,
) -> Result<()> {
    for registration in registrations(db, owner, source, files)? {
        *usn = usn
            .checked_add(1)
            .context("Desktop update counter overflow")?;
        ensure!(db.execute(
            "UPDATE contentFile SET Hash=?1,Size=?2,rb_local_usn=?3,updated_at=?4,rb_local_synced=0
             WHERE ID=?5 AND ContentID=?6 AND rb_local_deleted=0",
            params![registration.hash,registration.size,*usn,timestamp,registration.id,owner],
        )? == 1, "Native analysis registration changed during write-back");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mismatched_owner_digest_or_cloud_registration_blocks_the_grid() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let rel = Path::new("share/PIONEER/USBANLZ/P001/ANLZ0000.DAT");
        let target = dir.path().join(rel);
        std::fs::create_dir_all(target.parent().unwrap())?;
        let mut original = [0u8; 28];
        original[..4].copy_from_slice(b"PMAI");
        original[4..8].copy_from_slice(&28u32.to_be_bytes());
        original[8..12].copy_from_slice(&28u32.to_be_bytes());
        std::fs::write(&target, original)?;
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE contentFile(ID PRIMARY KEY, ContentID, Path, Hash, Size, rb_local_path,
           rb_insync_hash, rb_insync_local_usn, rb_file_hash_dirty, rb_local_file_status, rb_in_progress,
           rb_process_type, rb_temp_path, rb_file_size_dirty, rb_data_status, rb_local_data_status,
           rb_local_deleted, rb_local_synced, usn, rb_local_usn, updated_at);")?;
        db.execute(
            "INSERT INTO contentFile(ID,ContentID,Path,Hash,Size,rb_local_deleted,rb_local_synced)
                    VALUES('f','1','/PIONEER/USBANLZ/P001/ANLZ0000.DAT',?1,?2,0,0)",
            params![
                format!("{:x}", Md5::digest(original)),
                original.len() as i64
            ],
        )?;
        let source = "/PIONEER/USBANLZ/P001/ANLZ0000.DAT";
        let share = dir.path().join("share");
        check_inventory(&db, "1", &share, source)?;
        db.execute("INSERT INTO contentFile(ID,ContentID,Path,Hash,Size,rb_local_deleted,rb_local_synced)
              SELECT 'missing',ContentID,replace(Path,'.DAT','.3EX'),Hash,Size,0,0 FROM contentFile WHERE ID='f'", [])?;
        assert!(check_inventory(&db, "1", &share, source)
            .unwrap_err()
            .to_string()
            .contains("missing"));
        db.execute(
            "UPDATE contentFile SET Path=replace(Path,'ANLZ0000','ANLZ0001') WHERE ID='missing'",
            [],
        )?;
        assert!(check_inventory(&db, "1", &share, source).is_err());
        db.execute(
            "UPDATE contentFile SET rb_local_deleted=1 WHERE ID='missing'",
            [],
        )?;
        let changes = [FileChange {
            relative_path: rel.into(),
            expected_sha256: Sha256::digest(original).into(),
            contents: b"changed".to_vec(),
        }];
        check(&db, "1", dir.path(), &changes)?;
        assert!(check(&db, "other", dir.path(), &changes).is_err());
        db.execute("UPDATE contentFile SET rb_in_progress=1", [])?;
        assert!(check(&db, "1", dir.path(), &changes).is_err());
        db.execute(
            "UPDATE contentFile SET rb_in_progress=0, Hash='outdated'",
            [],
        )?;
        let mut usn = 1;
        assert!(update(&db, "1", dir.path(), &changes, &mut usn, "now").is_err());
        assert_eq!(usn, 1);
        assert_eq!(std::fs::read(&target)?, original);
        Ok(())
    }
}
