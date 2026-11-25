//! Review the active Recordbox collection. Filesystem observations never remove audio.
pub(crate) mod paths;
pub(crate) mod store;
pub(crate) mod ui;

use crate::db::Db;
use crate::model::Track;
use anyhow::{ensure, Context, Result};
use paths::{PathFinding, PathInspector, PathStatus};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

const HASH_BUDGET: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Candidate {
    Missing,
    Duplicate { keeper: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub id: i64,
    pub title: String,
    pub path: String,
    pub os: &'static str,
    pub volume: String,
    pub status: PathStatus,
    pub native: bool,
    pub protected_metadata: bool,
    pub candidate: Option<Candidate>,
    /// A matching local file was found, although each distinct set of track metadata needs a keeper.
    pub same_file_group: bool,
    metadata: [u8; 32],
    canonical: Option<PathBuf>,
    bytes: Option<u64>,
    modified: Option<SystemTime>,
    digest: Option<[u8; 32]>,
}

impl Row {
    pub(crate) fn protected(&self) -> bool {
        self.native
            || self.protected_metadata
            || !matches!(self.status, PathStatus::Present | PathStatus::Missing)
    }
    pub(crate) fn explanation(&self) -> &'static str {
        if self.native {
            "Imported or unconfirmed source; kept"
        } else if self.protected_metadata {
            "Saved library data or relationships; kept"
        } else {
            match self.candidate {
                Some(Candidate::Missing) => "File absent in a verified local Music folder",
                Some(Candidate::Duplicate { .. }) if self.digest.is_some() => {
                    "Matching complete file bytes and catalog metadata"
                }
                Some(Candidate::Duplicate { .. }) => {
                    "Same resolved local file and catalog metadata"
                }
                None if self.same_file_group => {
                    "Kept to preserve a reference and its catalog metadata"
                }
                None => self.status.label(),
            }
        }
    }

    fn valid_file(&self) -> bool {
        self.status == PathStatus::Present
            && !self.native
            && self.bytes.is_some_and(|n| n > 0)
            && self.modified.is_some()
            && self.canonical.is_some()
    }

    fn same_reviewed_record(&self, other: &Self) -> bool {
        self.id == other.id
            && self.path == other.path
            && self.metadata == other.metadata
            && self.candidate == other.candidate
            && self.status == other.status
            && self.native == other.native
            && self.protected_metadata == other.protected_metadata
            && self.canonical == other.canonical
            && self.bytes == other.bytes
            && self.modified == other.modified
            && self.digest == other.digest
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct VolumeSummary {
    pub os: &'static str,
    pub volume: String,
    pub total: usize,
    pub present: usize,
    pub missing: usize,
    pub protected: usize,
    pub candidates: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Report {
    pub library: PathBuf,
    pub can_apply: bool,
    pub rows: Vec<Row>,
    pub volumes: Vec<VolumeSummary>,
    pub statuses: BTreeMap<PathStatus, usize>,
    pub compare_bytes: bool,
    pub hashed_bytes: u64,
    pub hashed_files: usize,
    pub unverified_files: usize,
}

impl Report {
    pub(crate) fn verify_selection(&self, current: &Report, selected: &[i64]) -> Result<()> {
        ensure!(
            self.can_apply && current.can_apply,
            "Cleanup is report-only on this host"
        );
        ensure!(
            self.library == current.library && self.compare_bytes == current.compare_bytes,
            "The reviewed collection changed"
        );
        ensure!(
            !selected.is_empty(),
            "Select references from the cleanup review"
        );
        let old: HashMap<_, _> = self.rows.iter().map(|r| (r.id, r)).collect();
        let new: HashMap<_, _> = current.rows.iter().map(|r| (r.id, r)).collect();
        let unique: HashSet<_> = selected.iter().copied().collect();
        ensure!(unique.len() == selected.len(), "Duplicate selection");
        for id in selected {
            let check = |key: &i64| -> Result<&Row> {
                let before = old.get(key).context("Track absent from review")?;
                let now = new.get(key).context("Track disappeared since review")?;
                ensure!(
                    before.same_reviewed_record(now),
                    "Track {key} changed. Review the collection again."
                );
                Ok(*now)
            };
            let row = check(id)?;
            ensure!(!row.protected(), "Track {id} is protected");
            if let Candidate::Duplicate { keeper } = row
                .candidate
                .context("Selected track is ineligible for cleanup")?
            {
                let retained = check(&keeper)?;
                ensure!(
                    !unique.contains(&keeper)
                        && retained.valid_file()
                        && retained.candidate.is_none(),
                    "A verified duplicate keeper must remain in the collection"
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn review_path_cancellable(
    path: &Path,
    compare_bytes: bool,
    cancelled: &AtomicBool,
) -> Result<Report> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    scan(&Db { conn }, compare_bytes, None, cancelled)
}

pub(super) fn review_selected(db: &Db, reviewed: &Report, selected: &[i64]) -> Result<Report> {
    let mut scope: BTreeSet<i64> = selected.iter().copied().collect();
    for row in &reviewed.rows {
        if scope.contains(&row.id) {
            if let Some(Candidate::Duplicate { keeper }) = row.candidate {
                scope.insert(keeper);
            }
        }
    }
    scan(
        db,
        reviewed.compare_bytes,
        Some(&scope),
        &AtomicBool::new(false),
    )
}

fn scan(
    db: &Db,
    compare_bytes: bool,
    scope: Option<&BTreeSet<i64>>,
    cancelled: &AtomicBool,
) -> Result<Report> {
    let library = std::fs::canonicalize(db.active_library_path())
        .context("Locate the active Recordbox collection")?;
    let inspector = PathInspector::current();
    // Revalidation may already run under the store's IMMEDIATE transaction.
    let snapshot = db
        .conn
        .is_autocommit()
        .then(|| db.conn.unchecked_transaction())
        .transpose()?;
    let protected = store::protections(db)?;
    let tracks = match scope {
        Some(ids) => ids
            .iter()
            .map(|id| {
                db.track(*id)
                    .with_context(|| format!("Could not reread selected track {id} or its keeper"))
            })
            .collect::<Result<Vec<_>>>()?,
        None => db.all_tracks()?,
    };
    let mut rows = Vec::with_capacity(tracks.len());
    for track in tracks {
        ensure!(!cancelled.load(Ordering::Relaxed), "Scan cancelled");
        let native = protected.native.contains(&track.id);
        let prepared = protected.is_prepared(&track);
        let finding = inspector.inspect(&track.path, false);
        rows.push(row_from(track, finding, native, prepared)?);
    }
    if let Some(snapshot) = snapshot {
        snapshot.commit()?;
    }
    let can_apply = cfg!(any(windows, target_os = "macos"));
    let mut report = Report {
        library,
        can_apply,
        rows,
        volumes: vec![],
        statuses: BTreeMap::new(),
        compare_bytes,
        hashed_bytes: 0,
        hashed_files: 0,
        unverified_files: 0,
    };
    if compare_bytes && can_apply {
        let mut buffer = vec![0; 256 * 1024];
        hash_matches(
            &mut report,
            scope,
            HASH_BUDGET,
            cancelled,
            |row, remaining, used| {
                hash_local(
                    |path| inspector.inspect(path, false),
                    row,
                    &mut buffer,
                    remaining,
                    used,
                    cancelled,
                )
            },
        )?;
    }
    assign_candidates(&mut report);
    Ok(report)
}

fn row_from(mut track: Track, finding: PathFinding, native: bool, prepared: bool) -> Result<Row> {
    let id = std::mem::take(&mut track.id);
    let path = std::mem::take(&mut track.path);
    // Exclude only the reference identity and location. Distinct tags, dates, ratings or other
    // stored metadata are valid reasons for keeping separate records of identical file bytes.
    let metadata = Sha256::digest(serde_json::to_vec(&track)?).into();
    Ok(Row {
        id,
        path,
        title: track.title,
        os: finding.os,
        volume: finding.volume,
        status: finding.status,
        native,
        protected_metadata: prepared,
        candidate: None,
        same_file_group: false,
        metadata,
        canonical: finding.local_identity,
        bytes: finding.bytes,
        modified: finding.modified,
        digest: None,
    })
}

fn hash_matches<F>(
    report: &mut Report,
    scope: Option<&BTreeSet<i64>>,
    budget: u64,
    cancelled: &AtomicBool,
    mut hash: F,
) -> Result<()>
where
    F: FnMut(&Row, u64, &mut u64) -> Result<[u8; 32]>,
{
    let mut sizes = BTreeMap::<u64, Vec<usize>>::new();
    for (index, row) in report.rows.iter().enumerate() {
        if row.valid_file() && scope.is_none_or(|ids| ids.contains(&row.id)) {
            sizes.entry(row.bytes.unwrap()).or_default().push(index);
        }
    }
    for (bytes, members) in sizes {
        if members.len() < 2 {
            continue;
        }
        if budget.saturating_sub(report.hashed_bytes) / bytes < 2 {
            report.unverified_files += members.len();
            continue;
        }
        for index in members {
            ensure!(!cancelled.load(Ordering::Relaxed), "Scan cancelled");
            let left = budget.saturating_sub(report.hashed_bytes);
            if bytes > left {
                report.unverified_files += 1;
                continue;
            }
            let row = &mut report.rows[index];
            let mut used = 0;
            match hash(row, left, &mut used) {
                Ok(digest) => {
                    row.digest = Some(digest);
                    report.hashed_files += 1;
                }
                Err(_) => {
                    report.unverified_files += 1;
                }
            }
            report.hashed_bytes += used;
        }
    }
    ensure!(!cancelled.load(Ordering::Relaxed), "Scan cancelled");
    Ok(())
}

fn hash_local(
    inspect: impl Fn(&str) -> PathFinding,
    row: &Row,
    buf: &mut [u8],
    remaining: u64,
    used: &mut u64,
    cancelled: &AtomicBool,
) -> Result<[u8; 32]> {
    let path = Path::new(&row.path);
    let matches = || {
        let f = inspect(&row.path);
        f.status == PathStatus::Present
            && f.local_identity == row.canonical
            && f.bytes == row.bytes
            && f.modified == row.modified
    };
    ensure!(matches(), "File changed before comparison");
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        options.custom_flags(FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    let expected = row.bytes.context("Unknown local file size")?;
    let stable = |f: &File| -> Result<bool> {
        let m = f.metadata()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
                FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
            };
            if m.file_attributes()
                & (FILE_ATTRIBUTE_OFFLINE
                    | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
                    | FILE_ATTRIBUTE_RECALL_ON_OPEN
                    | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
            {
                return Ok(false);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let named = std::fs::symlink_metadata(path)?;
            if named.ino() != m.ino() || named.dev() != m.dev() {
                return Ok(false);
            }
        }
        Ok(m.is_file() && m.len() == expected && m.modified().ok() == row.modified)
    };
    ensure!(
        expected <= remaining && stable(&file)? && matches(),
        "File changed while opening it"
    );
    let mut digest = Sha256::new();
    while *used < expected {
        ensure!(!cancelled.load(Ordering::Relaxed), "Scan cancelled");
        let cap = (expected - *used).min(buf.len() as u64) as usize;
        let count = file.read(&mut buf[..cap])?;
        ensure!(count > 0, "File shortened during comparison");
        *used += count as u64;
        digest.update(&buf[..count]);
    }
    ensure!(
        stable(&file)? && matches(),
        "File changed during comparison"
    );
    Ok(digest.finalize().into())
}

#[derive(PartialOrd, Ord, PartialEq, Eq)]
enum FileMatch<'a> {
    Path(&'a Path),
    Bytes(u64, [u8; 32]),
}

fn assign_candidates(report: &mut Report) {
    let mut groups = BTreeMap::<FileMatch<'_>, Vec<usize>>::new();
    for (index, row) in report.rows.iter().enumerate() {
        if !row.valid_file() {
            continue;
        }
        let key = if report.compare_bytes {
            let Some(digest) = row.digest else { continue };
            FileMatch::Bytes(row.bytes.unwrap(), digest)
        } else {
            FileMatch::Path(row.canonical.as_deref().unwrap())
        };
        groups.entry(key).or_default().push(index);
    }
    let groups: Vec<_> = groups.into_values().filter(|g| g.len() > 1).collect();
    for group in groups {
        // Prefer an already protected keeper. Each distinct catalog description keeps its own row.
        let mut keepers = HashMap::<[u8; 32], i64>::new();
        let mut group = group;
        group.sort_by_key(|&i| (!report.rows[i].protected_metadata, report.rows[i].id));
        for i in group {
            let row = &mut report.rows[i];
            row.same_file_group = true;
            if let Some(&keeper) = keepers.get(&row.metadata) {
                if report.can_apply && !row.protected() {
                    row.candidate = Some(Candidate::Duplicate { keeper });
                }
            } else {
                keepers.insert(row.metadata, row.id);
            }
        }
    }
    let mut volumes = BTreeMap::<(&str, String), VolumeSummary>::new();
    report.statuses.clear();
    for row in &mut report.rows {
        *report.statuses.entry(row.status).or_default() += 1;
        if report.can_apply && row.status == PathStatus::Missing && !row.protected() {
            row.candidate = Some(Candidate::Missing);
        }
        let v = volumes
            .entry((row.os, row.volume.clone()))
            .or_insert_with(|| VolumeSummary {
                os: row.os,
                volume: row.volume.clone(),
                ..Default::default()
            });
        v.total += 1;
        v.present += usize::from(row.status == PathStatus::Present);
        v.missing += usize::from(row.status == PathStatus::Missing);
        v.protected += usize::from(row.protected());
        v.candidates += usize::from(row.candidate.is_some());
    }
    report.volumes = volumes.into_values().collect();
}

#[cfg(test)]
mod tests;
