//! The live write boundary. SQLite owns the database transaction. Separate ANLZ files
//! require a durable before/after journal, even when a batch changes only one grid.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{
    backup::{Backup, StepResult},
    types::ValueRef,
    Connection, OpenFlags, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};
use tempfile::NamedTempFile;

use crate::export::usb::sha256_file;
use crate::rbimport::{check_schema, MASTER_DB_KEY};

const MARKER: &str = ".recordbox-writeback-pending.json";
const MANIFEST: &str = "grid-recovery.json";
const LOCK: &str = ".recordbox-writeback.lock";
const MAX_MARKER: u64 = 256 * 1024;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct FileChange {
    /// Relative to the native library, including the leading share directory.
    pub relative_path: PathBuf,
    pub expected_sha256: [u8; 32],
    pub contents: Vec<u8>,
}

#[derive(Deserialize, Serialize)]
struct GridFile {
    path: PathBuf,
    before: [u8; 32],
    after: [u8; 32],
}

#[derive(Deserialize, Serialize)]
struct GridJournal {
    source: PathBuf,
    source_file: NativeFileId,
    before_db: [u8; 32],
    after_db: [u8; 32],
    files: Vec<GridFile>,
}

#[derive(Deserialize, Serialize)]
struct Pending {
    directory: PathBuf,
    manifest_sha256: [u8; 32],
}

pub fn pending(source_dir: &Path) -> bool {
    match fs::symlink_metadata(source_dir.join(MARKER)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        _ => true,
    }
}

fn app_process(command: &str) -> bool {
    let name = command
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = name.trim_end_matches(".exe");
    name == "rekordbox"
        || name.starts_with("rekordboxagent")
        || name.starts_with("rekordbox agent")
        || name.starts_with("rekordbox helper")
        || name.starts_with("rekordbox (")
        || name
            .strip_prefix("rekordbox ")
            .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

fn ensure_apps_closed() -> Result<()> {
    check_process_list(app_process)
}

fn check_process_list(blocked: impl Fn(&str) -> bool) -> Result<()> {
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        let system =
            std::env::var_os("SystemRoot").context("cannot locate the Windows process list")?;
        Command::new(PathBuf::from(system).join("System32/tasklist.exe"))
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x08000000)
            .output()
    };
    #[cfg(unix)]
    let output = Command::new("/bin/ps").args(["-A", "-o", "comm="]).output();
    let output = output.context("cannot check whether rekordbox or rekordboxAgent is running")?;
    ensure!(
        output.status.success(),
        "could not inspect running applications"
    );
    let lines = String::from_utf8_lossy(&output.stdout);
    #[cfg(windows)]
    let names = lines
        .lines()
        .filter_map(|s| s.strip_prefix('"')?.split('"').next());
    #[cfg(unix)]
    let names = lines.lines().filter(|s| !s.trim().is_empty());
    let mut listed = false;
    for name in names {
        listed = true;
        ensure!(
            !blocked(name),
            "close rekordbox and rekordboxAgent before writing or recovering this library"
        );
    }
    ensure!(
        listed,
        "could not identify any applications in the system process list"
    );
    Ok(())
}

// Network filesystems and cloud-backed library folders can admit writers outside
// the local process list. Restrict the pilot to known local filesystems.
fn local_volume(path: &Path) -> Result<()> {
    for part in path.components() {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        ensure!(
            !name.contains("dropbox")
                && !name.starts_with("onedrive")
                && name != "mobile documents"
                && name != "cloudstorage"
                && name != "google drive"
                && name != "icloud drive",
            "choose a local library and recovery folder outside cloud-synchronized directories"
        );
    }
    #[cfg(unix)]
    let local = {
        use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
        let path = CString::new(path.as_os_str().as_bytes())?;
        let mut status = MaybeUninit::<libc::statfs>::uninit();
        // statfs initializes the whole output structure on success.
        if unsafe { libc::statfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("checking the library filesystem");
        }
        let status = unsafe { status.assume_init() };
        #[cfg(target_os = "macos")]
        {
            status.f_flags & libc::MNT_LOCAL as u32 != 0
        }
        #[cfg(target_os = "linux")]
        {
            // ext, btrfs, XFS, tmpfs, overlayfs, F2FS, ZFS and native NTFS.
            matches!(
                status.f_type as u32,
                0xef53
                    | 0x9123683e
                    | 0x58465342
                    | 0x01021994
                    | 0x794c7630
                    | 0xf2f52010
                    | 0x2fc12fc1
                    | 0x5346544e
            )
        }
    };
    #[cfg(windows)]
    let local = {
        use std::path::Prefix;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        let drive = match path.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(d),
                _ => None,
            },
            _ => None,
        };
        drive.is_some_and(|d| {
            let root = [d as u16, b':' as u16, b'\\' as u16, 0];
            unsafe { GetDriveTypeW(root.as_ptr()) == 3 } // DRIVE_FIXED
        })
    };
    ensure!(
        local,
        "write-back requires a supported local filesystem for {}",
        path.display()
    );
    Ok(())
}

fn regular(path: &Path) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path).with_context(|| format!("inspecting {}", path.display()))?;
    ensure!(
        m.is_file() && !linked(&m),
        "expected a regular unlinked file at {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            m.nlink() == 1,
            "hard-linked file is outside the write-back pilot: {}",
            path.display()
        );
    }
    Ok(m)
}

fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        } // junction or other reparse point
    }
    m.file_type().is_symlink()
}

fn grid_target(source: &Path, relative: &Path) -> Result<PathBuf> {
    let mut parts = relative.components();
    ensure!(
        parts.next() == Some(Component::Normal("share".as_ref())),
        "analysis path must be inside the native share directory"
    );
    let mut target = source.join("share");
    for part in parts {
        let Component::Normal(name) = part else {
            bail!("invalid relative analysis path");
        };
        let parent = fs::symlink_metadata(&target)?;
        ensure!(
            parent.is_dir() && !linked(&parent),
            "linked or invalid analysis directory"
        );
        target.push(name);
    }
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    ensure!(
        name.starts_with("ANLZ")
            && [".DAT", ".EXT", ".2EX"]
                .iter()
                .any(|ext| name.ends_with(ext)),
        "unexpected native analysis filename"
    );
    regular(&target)?;
    local_volume(&target)?;
    Ok(target)
}

// Also bind the operation to this particular master.db. A backup restored at
// the same path or an externally replaced library needs a fresh review.
#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct NativeFileId {
    volume: u64,
    file: [u8; 16],
}

fn native_id(path: &Path) -> Result<NativeFileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Avoid opening a separate fd to the live DB: closing it can drop
        // process-owned POSIX locks that SQLite has on that inode.
        let m = regular(path)?;
        let mut file = [0u8; 16];
        file[..8].copy_from_slice(&m.ino().to_le_bytes());
        Ok(NativeFileId {
            volume: m.dev() as u64,
            file,
        })
    }
    #[cfg(windows)]
    {
        use std::{ffi::c_void, os::windows::io::AsRawHandle};
        #[repr(C)]
        struct FileIdInfo {
            volume: u64,
            file: [u8; 16],
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandleEx(
                handle: *mut c_void,
                class: i32,
                buffer: *mut c_void,
                bytes: u32,
            ) -> i32;
        }
        regular(path)?;
        let f = File::open(path)?;
        let mut info = FileIdInfo {
            volume: 0,
            file: [0; 16],
        };
        let ok = unsafe {
            GetFileInformationByHandleEx(
                f.as_raw_handle(),
                18,
                (&mut info as *mut FileIdInfo).cast(),
                std::mem::size_of::<FileIdInfo>() as u32,
            )
        }; // FileIdInfo
        ensure!(
            ok != 0,
            "cannot identify the native library file on this volume"
        );
        Ok(NativeFileId {
            volume: info.volume,
            file: info.file,
        })
    }
}

fn check_native_files(source: &Path, transaction_open: bool) -> Result<()> {
    regular(&source.join("master.db"))?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let path = source.join(format!("master.db{suffix}"));
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            result => {
                result?;
            }
        }
        let m = regular(&path)?;
        ensure!(
            transaction_open || suffix != "-journal" || m.len() == 0,
            "active rollback journal; close rekordbox cleanly and try again"
        );
    }
    Ok(())
}

fn native(source: &Path, mode: OpenFlags) -> Result<Connection> {
    let conn = Connection::open_with_flags(source, mode | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
    conn.busy_timeout(Duration::ZERO)?;
    conn.pragma_update(None, "key", MASTER_DB_KEY)?;
    conn.pragma_update(None, "trusted_schema", false)?;
    Ok(conn)
}

fn integrity(db: &Connection) -> Result<()> {
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(
        integrity == "ok",
        "library failed SQLite integrity validation: {integrity}"
    );
    let mut cipher = db.prepare("PRAGMA cipher_integrity_check")?;
    ensure!(
        cipher.query([])?.next()?.is_none(),
        "library failed SQLCipher page authentication"
    );
    check_schema(db)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    // Windows publication below uses MOVEFILE_WRITE_THROUGH.
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

fn publish(temp: NamedTempFile, target: &Path, replace: bool) -> Result<()> {
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    if replace {
        temp.persist(target)?;
    } else {
        temp.persist_noclobber(target)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32;
        }
        // All parents are canonical absolute paths (including verbatim/long paths).
        let old = temp
            .path()
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let new = target
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let flag = if replace { 1 } else { 0 };
        if unsafe { MoveFileExW(old.as_ptr(), new.as_ptr(), 8 | flag) } == 0 {
            return Err(std::io::Error::last_os_error()).context("publishing write-back file");
        }
    }
    sync_directory(
        target
            .parent()
            .context("missing parent for write-back file")?,
    )
}

fn save_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp = NamedTempFile::new_in(path.parent().context("missing recovery directory")?)?;
    temp.write_all(bytes)?;
    publish(temp, path, false)
}

fn backup_database(source: &Path, directory: &Path) -> Result<()> {
    let reader = native(&source.join("master.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let path = directory.join("master.db");
    {
        let mut copy = native(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        {
            let backup = Backup::new(&reader, &mut copy)?;
            loop {
                match backup.step(256)? {
                    StepResult::Done => break,
                    StepResult::More => continue,
                    _ => bail!("the database became busy while creating the recovery copy"),
                }
            }
        }
        let mode: String = copy.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        ensure!(
            mode.eq_ignore_ascii_case("delete"),
            "cannot consolidate recovery database"
        );
    }
    fs::OpenOptions::new().write(true).open(&path)?.sync_all()?;
    integrity(&native(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?)?;
    sync_directory(directory)
}

// A grid batch spans two different storage systems. Hash all persisted rows,
// schema and metadata of the private backup and the staged SQL transaction.
// During recovery even an unrelated subsequent database edit must stop replay.
fn logical_digest(db: &Connection) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    fn feed(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    fn rows(db: &Connection, sql: &str, hash: &mut Sha256) -> Result<()> {
        let mut query = db.prepare(sql)?;
        let cols = query.column_count();
        hash.update((cols as u64).to_le_bytes());
        let mut result = query.query([])?;
        while let Some(row) = result.next()? {
            hash.update([1]);
            for i in 0..cols {
                match row.get_ref(i)? {
                    ValueRef::Null => hash.update([0]),
                    ValueRef::Integer(v) => {
                        hash.update([1]);
                        hash.update(v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        hash.update([2]);
                        hash.update(v.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(v) => {
                        hash.update([3]);
                        feed(hash, v);
                    }
                    ValueRef::Blob(v) => {
                        hash.update([4]);
                        feed(hash, v);
                    }
                }
            }
        }
        hash.update([0]);
        Ok(())
    }
    rows(db, "PRAGMA main.user_version", &mut hash)?;
    rows(db, "PRAGMA main.application_id", &mut hash)?;
    rows(
        db,
        "SELECT type,name,tbl_name,sql FROM main.sqlite_schema ORDER BY type,name",
        &mut hash,
    )?;
    let mut stmt = db.prepare("SELECT name,type FROM pragma_table_list WHERE schema='main' AND name <> 'sqlite_schema' ORDER BY name")?;
    let tables = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for table in tables {
        let (name, kind) = table?;
        ensure!(
            kind != "virtual" && kind != "shadow",
            "virtual tables are outside the grid recovery pilot"
        );
        if kind != "table" {
            continue;
        }
        feed(&mut hash, name.as_bytes());
        // A table scan yields rowid order or primary-key order for WITHOUT ROWID.
        let quoted = name.replace('"', "\"\"");
        rows(
            db,
            &format!("SELECT * FROM main.\"{quoted}\" NOT INDEXED"),
            &mut hash,
        )?;
    }
    Ok(hash.finalize().into())
}

fn backup_parent(source: &Path, root: &Path) -> Result<PathBuf> {
    let requested = std::path::absolute(root)?;
    let existing = requested
        .ancestors()
        .find(|p| p.exists())
        .context("missing recovery parent")?
        .canonicalize()?;
    ensure!(
        !existing.starts_with(source),
        "choose a recovery folder outside the native library"
    );
    local_volume(&existing)?;
    fs::create_dir_all(&requested)?;
    let root = requested.canonicalize()?;
    ensure!(
        !root.starts_with(source),
        "recovery folder overlaps the native library"
    );
    local_volume(&root)?;
    Ok(root)
}

fn stage_grids(
    source: &Path,
    directory: &Path,
    db: &Connection,
    changes: Vec<FileChange>,
) -> Result<GridJournal> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for (i, change) in changes.into_iter().enumerate() {
        let path = grid_target(source, &change.relative_path)?;
        ensure!(
            seen.insert(path.canonicalize()?),
            "duplicate native analysis file in selection"
        );
        let before = sha256_file(&path)?;
        ensure!(
            before == change.expected_sha256,
            "native grid changed since review: {}",
            path.display()
        );
        let after: [u8; 32] = Sha256::digest(&change.contents).into();
        ensure!(before != after, "redundant analysis-file replacement");
        let copy = directory.join(format!("{i}.original"));
        {
            let mut temp = NamedTempFile::new_in(directory)?;
            std::io::copy(&mut File::open(&path)?, temp.as_file_mut())?;
            publish(temp, &copy, false)?;
        }
        ensure!(
            sha256_file(&copy)? == before,
            "analysis file changed while backing it up"
        );
        let edited = directory.join(format!("{i}.reviewed"));
        save_new(&edited, &change.contents)?;
        ensure!(
            sha256_file(&edited)? == after,
            "reviewed grid copy failed verification"
        );
        files.push(GridFile {
            path: change.relative_path,
            before,
            after,
        });
    }
    let before_db = logical_digest(&native(
        &directory.join("master.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)?;
    let after_db = logical_digest(db)?;
    ensure!(
        before_db != after_db,
        "grid update requires a corresponding native database revision"
    );
    Ok(GridJournal {
        source: source.to_owned(),
        source_file: native_id(&source.join("master.db"))?,
        before_db,
        after_db,
        files,
    })
}

fn copy_grids(journal: &GridJournal, directory: &Path, forward: bool) -> Result<()> {
    // Preflight the complete set, including recovery contents, before the first rename.
    let mut staged = Vec::new();
    for (i, entry) in journal.files.iter().enumerate() {
        let (old, want, side) = if forward {
            (entry.before, entry.after, "reviewed")
        } else {
            (entry.after, entry.before, "original")
        };
        let target = grid_target(&journal.source, &entry.path)?;
        let current = sha256_file(&target)?;
        ensure!(
            current == old || current == want,
            "analysis file has unrecognized edits: {}",
            target.display()
        );
        if current == want {
            continue;
        }
        let saved = directory.join(format!("{i}.{side}"));
        regular(&saved)?;
        let mut temp = NamedTempFile::new_in(target.parent().context("missing analysis folder")?)?;
        std::io::copy(&mut File::open(saved)?, temp.as_file_mut())?;
        ensure!(
            sha256_file(temp.path())? == want,
            "recovery grid checksum mismatch"
        );
        fs::set_permissions(temp.path(), regular(&target)?.permissions())?;
        staged.push((temp, entry, old, want));
    }
    for (temp, entry, old, want) in staged {
        let target = grid_target(&journal.source, &entry.path)?;
        ensure!(
            sha256_file(&target)? == old,
            "native analysis changed during publication"
        );
        publish(temp, &target, true)?;
        ensure!(
            sha256_file(&target)? == want,
            "published analysis failed verification"
        );
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = regular(path)?;
    ensure!(
        metadata.len() <= limit,
        "oversized recovery metadata at {}",
        path.display()
    );
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "oversized recovery metadata");
    Ok(bytes)
}

// All Recordbox profiles using this native library share the same advisory lock,
// including the interval between SQLite commit and clearing a pending file batch.
// Leave the lock inode in place even after unlocking it.
fn operation_lock(source: &Path, recovering: bool) -> Result<File> {
    ensure_apps_closed()?;
    local_volume(source)?;
    check_native_files(source, recovering)?;
    let path = source.join(LOCK);
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x00200000); // share read/write, open the reparse point itself.
    }
    let lock = options
        .open(&path)
        .context("opening the Recordbox write-back lock")?;
    let current = regular(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = lock.metadata()?;
        ensure!(
            current.ino() == opened.ino() && current.dev() == opened.dev(),
            "write-back lock file changed while opening it"
        );
    }
    #[cfg(windows)]
    let _ = current;
    lock.try_lock()
        .context("another Recordbox write-back or recovery operation is using this library")?;
    Ok(lock)
}

fn writer(source: &Path) -> Result<Connection> {
    let db = native(&source.join("master.db"), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let journal: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    ensure!(
        matches!(journal.as_str(), "wal" | "delete" | "truncate" | "persist"),
        "native library requires a recoverable SQLite journal mode"
    );
    db.pragma_update(None, "synchronous", "FULL")?;
    db.pragma_update(None, "foreign_keys", true)?;
    Ok(db)
}

/// The callback re-reads previewed values on this connection. All its writes
/// participate in this transaction and must not commit or open another writer.
pub fn apply<T>(
    source_dir: &Path,
    backup_root: &Path,
    under_lock: impl FnOnce(&Connection) -> Result<(T, Vec<FileChange>)>,
) -> Result<(T, PathBuf)> {
    let source = source_dir.canonicalize()?;
    let _guard = operation_lock(&source, false)?;
    let identity = native_id(&source.join("master.db"))?;
    let mut db = writer(&source)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context(
            "cannot acquire the exclusive SQLite writer slot; close all rekordbox library writers",
        )?;
    ensure!(
        !pending(&source),
        "previous write-back needs recovery; use Recover interrupted write-back first"
    );
    integrity(&tx)?;
    let root = backup_parent(&source, backup_root)?;
    let backup = tempfile::Builder::new()
        .prefix("rekordbox-")
        .tempdir_in(&root)?;
    backup_database(&source, backup.path())?;
    ensure!(
        native_id(&source.join("master.db"))? == identity,
        "native library file changed during backup"
    );
    let (result, grids) = under_lock(&tx)?;
    check_native_files(&source, true)?;
    ensure!(
        native_id(&source.join("master.db"))? == identity,
        "native library file changed during review"
    );
    ensure_apps_closed()?;
    // Reopening a backup after a successful database-only operation always
    // yields its committed, pre-edit state, including the original WAL records.
    if grids.is_empty() {
        let directory = backup.keep();
        sync_directory(&root)?;
        tx.commit().with_context(|| {
            format!(
                "database commit failed; verified recovery copy: {}",
                directory.display()
            )
        })?;
        return Ok((result, directory));
    }

    let journal = stage_grids(&source, backup.path(), &tx, grids)?;
    let manifest = serde_json::to_vec_pretty(&journal)?;
    ensure!(
        manifest.len() as u64 <= MAX_MANIFEST,
        "selection exceeds the recoverable grid batch size"
    );
    save_new(&backup.path().join(MANIFEST), &manifest)?;
    let directory = backup.keep();
    sync_directory(&root)?;
    let intent = Pending {
        directory: directory.clone(),
        manifest_sha256: Sha256::digest(&manifest).into(),
    };
    save_new(&source.join(MARKER), &serde_json::to_vec(&intent)?)?;
    let commit = || -> Result<()> {
        ensure_apps_closed()?;
        ensure!(
            native_id(&source.join("master.db"))? == identity,
            "native library file changed before grid publication"
        );
        copy_grids(&journal, &directory, true)?;
        ensure_apps_closed()?;
        ensure!(
            native_id(&source.join("master.db"))? == identity,
            "native library file changed before commit"
        );
        tx.commit()?;
        fs::remove_file(source.join(MARKER))?;
        sync_directory(&source)
    };
    commit().with_context(|| {
        format!(
            "Recover interrupted write-back before continuing. Verified backup: {}",
            directory.display()
        )
    })?;
    Ok((result, directory))
}

/// Complete an interrupted grid publication according to the database state
/// that actually committed. Never replace a file outside either recorded state.
pub fn recover(source_dir: &Path, backup_root: &Path) -> Result<PathBuf> {
    let source = source_dir.canonicalize()?;
    let _guard = operation_lock(&source, true)?;
    let marker = source.join(MARKER);
    let pending: Pending = serde_json::from_slice(&read_bounded(&marker, MAX_MARKER)?)?;
    let directory = pending.directory.canonicalize()?;
    let root = backup_root.canonicalize()?;
    ensure!(
        directory.parent() == Some(root.as_path()),
        "expected recovery folder under {}; recorded {}",
        root.display(),
        directory.display()
    );
    local_volume(&directory)?;
    let manifest = read_bounded(&directory.join(MANIFEST), MAX_MANIFEST)?;
    let digest: [u8; 32] = Sha256::digest(&manifest).into();
    ensure!(
        digest == pending.manifest_sha256,
        "recovery manifest checksum mismatch"
    );
    let journal: GridJournal = serde_json::from_slice(&manifest)?;
    ensure!(
        journal.source == source,
        "recovery belongs to a different library"
    );
    ensure!(
        native_id(&source.join("master.db"))? == journal.source_file,
        "the native library file was replaced; inspect the retained backup before recovery"
    );
    regular(&directory.join("master.db"))?;
    let pre = native(
        &directory.join("master.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    integrity(&pre)?;
    ensure!(
        logical_digest(&pre)? == journal.before_db,
        "recovery database has changed"
    );
    // Only a verified, pending operation may ask SQLite to resolve a hot
    // rollback journal left behind by an interrupted native transaction.
    ensure_apps_closed()?;
    let mut db = writer(&source)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("library is busy")?;
    for (i, grid) in journal.files.iter().enumerate() {
        for (name, expected) in [("original", grid.before), ("reviewed", grid.after)] {
            let file = directory.join(format!("{i}.{name}"));
            regular(&file)?;
            ensure!(
                sha256_file(&file)? == expected,
                "recovery grid checksum mismatch"
            );
        }
    }
    integrity(&tx)?;
    let state = logical_digest(&tx)?;
    ensure!(state == journal.before_db || state == journal.after_db, "native database has newer or unrecognized changes; no recovery files were applied; backup: {}", directory.display());
    ensure_apps_closed()?;
    copy_grids(&journal, &directory, state == journal.after_db)?;
    fs::remove_file(marker)?;
    sync_directory(&source)?;
    tx.commit()?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        source: PathBuf,
        backups: PathBuf,
        relative: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            let root = tempfile::tempdir()?;
            let base = root.path().canonicalize()?;
            let source = base.join("desktop");
            let relative: PathBuf = "share/PIONEER/USBANLZ/ANLZ0000.DAT".into();
            fs::create_dir_all(source.join(&relative).parent().unwrap())?;
            fs::write(source.join(&relative), b"original beats")?;
            let db = native(
                &source.join("master.db"),
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            )?;
            for (table, cols) in crate::rbimport::REQUIRED_SCHEMA {
                db.execute(&format!("CREATE TABLE {table} ({})", cols.join(", ")), [])?;
            }
            db.execute_batch("INSERT INTO djmdContent(ID,Rating) VALUES('track',1)")?;
            Ok(Self {
                _root: root,
                source,
                backups: base.join("backups"),
                relative,
            })
        }
        fn db(&self) -> Result<Connection> {
            native(
                &self.source.join("master.db"),
                OpenFlags::SQLITE_OPEN_READ_WRITE,
            )
        }
        fn bytes(&self) -> Result<Vec<u8>> {
            Ok(fs::read(self.source.join(&self.relative))?)
        }
        fn change(&self) -> FileChange {
            FileChange {
                relative_path: self.relative.clone(),
                expected_sha256: Sha256::digest(b"original beats").into(),
                contents: b"reviewed beats".to_vec(),
            }
        }
        // Simulate either side of a crash at the SQLite commit boundary, with
        // the ANLZ rename either already visible or still at its old contents.
        fn interrupted(&self, committed: bool, published: bool) -> Result<PathBuf> {
            let mut db = self.db()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let directory = self.backups.join("fixture");
            fs::create_dir_all(&directory)?;
            backup_database(&self.source, &directory)?;
            tx.execute("UPDATE djmdContent SET rating=3", [])?;
            let journal = stage_grids(&self.source, &directory, &tx, vec![self.change()])?;
            let manifest = serde_json::to_vec(&journal)?;
            save_new(&directory.join(MANIFEST), &manifest)?;
            let marker = Pending {
                directory: directory.clone(),
                manifest_sha256: Sha256::digest(&manifest).into(),
            };
            save_new(&self.source.join(MARKER), &serde_json::to_vec(&marker)?)?;
            if published {
                copy_grids(&journal, &directory, true)?;
            }
            if committed {
                tx.commit()?;
            }
            Ok(directory)
        }
    }

    #[test]
    fn recognizes_desktop_process_names_without_matching_recordbox() {
        for name in [
            "rekordbox.exe",
            "C:\\Pioneer\\rekordboxAgent.exe",
            "/Applications/rekordbox 7.app/Contents/MacOS/rekordbox",
            "rekordbox Agent",
            "rekordbox Helper (GPU)",
        ] {
            assert!(app_process(name), "{name}");
        }
        for other in [
            "Recordbox",
            "recordbox.exe",
            "rekordbox-migration-helper",
            "shell script referencing rekordbox",
        ] {
            assert!(!app_process(other), "{other}");
        }
    }

    #[test]
    fn os_process_list_detects_a_running_named_executable() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let image = if cfg!(windows) {
            "rb-wbprobe.exe"
        } else {
            "rb-wbprobe"
        };
        let exe = folder.path().join(image);
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            let system = std::env::var_os("SystemRoot").context("missing Windows system folder")?;
            fs::copy(PathBuf::from(system).join("System32/ping.exe"), &exe)?;
            Command::new(&exe)
                .args(["-n", "5", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .creation_flags(0x08000000)
                .spawn()?
        };
        #[cfg(unix)]
        let mut child = {
            fs::copy("/bin/sleep", &exe)?;
            Command::new(&exe).arg("10").spawn()?
        };
        let outcome = check_process_list(|p| p.trim().rsplit(['/', '\\']).next() == Some(image));
        let _ = child.kill();
        child.wait()?;
        let err = outcome.expect_err("OS process enumeration missed a running executable");
        assert!(err.to_string().contains("close rekordbox"), "{err}");
        Ok(())
    }

    #[test]
    fn independent_recordbox_lock_and_rollback_on_callback_error() -> Result<()> {
        let fixture = Fixture::new()?;
        let other = operation_lock(&fixture.source, false)?;
        assert!(operation_lock(&fixture.source, false).is_err());
        drop(other);
        let db = fixture.db()?;
        db.execute_batch("BEGIN IMMEDIATE")?;
        assert!(apply::<()>(&fixture.source, &fixture.backups, |_| panic!(
            "competing writer must be refused"
        ))
        .is_err());
        db.execute_batch("ROLLBACK")?;
        assert!(apply::<()>(&fixture.source, &fixture.backups, |db| {
            db.execute("UPDATE djmdContent SET Rating=5", [])?;
            bail!("simulated failure")
        })
        .is_err());
        assert_eq!(
            db.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert!(!pending(&fixture.source));
        Ok(())
    }

    #[test]
    fn backups_include_wal_and_grid_replacements() -> Result<()> {
        let fixture = Fixture::new()?;
        let other = fixture.db()?;
        other.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE); UPDATE djmdContent SET Rating=2")?;
        assert!(fs::metadata(fixture.source.join("master.db-wal"))?.len() > 0);
        let ((), backup) = apply(&fixture.source, &fixture.backups, |db| {
            db.execute("UPDATE djmdContent SET Rating=4", [])?;
            Ok(((), vec![fixture.change()]))
        })?;
        let backup_db = native(&backup.join("master.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        assert_eq!(
            backup_db.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            2
        );
        assert_eq!(
            other.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            4
        );
        assert_eq!(fs::read(backup.join("0.original"))?, b"original beats");
        assert_eq!(fixture.bytes()?, b"reviewed beats");
        assert!(!pending(&fixture.source));
        Ok(())
    }

    #[test]
    fn interrupted_grids_follow_the_state_that_committed() -> Result<()> {
        for (committed, installed) in [(false, true), (true, false), (true, true)] {
            let fixture = Fixture::new()?;
            let backup = fixture.interrupted(committed, installed)?;
            assert!(apply::<()>(&fixture.source, &fixture.backups, |_| panic!(
                "pending transaction must stop new writes"
            ))
            .is_err());
            assert_eq!(recover(&fixture.source, &fixture.backups)?, backup);
            let expected: &[u8] = if committed {
                b"reviewed beats"
            } else {
                b"original beats"
            };
            assert_eq!(fixture.bytes()?, expected);
            assert!(!pending(&fixture.source));
        }
        Ok(())
    }

    #[test]
    fn recovery_preserves_subsequent_untouched_work() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.interrupted(false, true)?;
        fs::write(fixture.source.join(&fixture.relative), b"later file")?;
        assert!(recover(&fixture.source, &fixture.backups).is_err());
        assert_eq!(fixture.bytes()?, b"later file");
        assert!(pending(&fixture.source));
        fs::write(fixture.source.join(&fixture.relative), b"reviewed beats")?;
        fixture
            .db()?
            .execute("INSERT INTO djmdContent(ID,Rating) VALUES('later',5)", [])?;
        assert!(recover(&fixture.source, &fixture.backups).is_err());
        assert_eq!(fixture.bytes()?, b"reviewed beats");
        assert!(pending(&fixture.source));
        Ok(())
    }

    #[test]
    fn refuses_traversal_and_changed_grid_before_committing_sql() -> Result<()> {
        let fixture = Fixture::new()?;
        for path in ["../ANLZ0000.DAT", "share/../master.db", "master.db"] {
            assert!(grid_target(&fixture.source, Path::new(path)).is_err());
        }
        let proposed = fixture.change();
        fs::write(fixture.source.join(&fixture.relative), b"an external edit")?;
        assert!(apply(&fixture.source, &fixture.backups, |db| {
            db.execute("UPDATE djmdContent SET Rating=4", [])?;
            Ok(((), vec![proposed]))
        })
        .is_err());
        assert_eq!(fixture.bytes()?, b"an external edit");
        assert_eq!(
            fixture
                .db()?
                .query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert!(!pending(&fixture.source));
        Ok(())
    }
}
