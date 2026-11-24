//! Existing desktop cues retain their native identity, timing and decoder-specific seek data.
use crate::model::{Cue, CueKind};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, types::ValueRef, Connection, Row};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use super::super::{cue_slot, desktop_cue_color};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct Position {
    slot: i32,
    start: i64,
    end: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Label {
    comment: String,
    color: u8,
    active_loop: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CueView(Vec<(Position, Label)>);

impl CueView {
    pub(crate) fn summary(&self) -> String {
        if self.0.is_empty() {
            return "No cues".into();
        }
        self.0
            .iter()
            .map(|(p, label)| {
                let kind = if p.slot < 0 {
                    "Memory".into()
                } else {
                    format!("Hot {}", (b'A' + p.slot as u8) as char)
                };
                format!(
                    "{kind} at {:.3}s · {} · color {}",
                    p.start as f64 / 1000.0,
                    label.comment,
                    label.color
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn finish(mut entries: Vec<(Position, Label)>) -> Result<CueView> {
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    ensure!(
        !entries.windows(2).any(|w| w[0].0 == w[1].0),
        "Two cues share the same slot and times; their identities are ambiguous"
    );
    let mut pads = HashSet::new();
    ensure!(
        entries
            .iter()
            .all(|(p, _)| p.slot < 0 || pads.insert(p.slot)),
        "Multiple hot cues occupy the same pad"
    );
    Ok(CueView(entries))
}

pub(crate) fn local(cues: &[Cue]) -> Result<CueView> {
    fn millis(sec: f64) -> Result<i64> {
        let ms = sec * 1000.0;
        ensure!(ms.is_finite() && (0.0..u32::MAX as f64).contains(&ms) && (ms.round() - ms).abs() < 0.00001,
            "Cue timing needs native seek data; only existing cue labels, colors and removals can be written");
        Ok(ms.round() as i64)
    }
    let mut entries = Vec::with_capacity(cues.len());
    for cue in cues {
        let slot = if cue.kind == CueKind::Memory {
            -1
        } else {
            ensure!((0..16).contains(&cue.slot), "Unknown hot cue slot");
            cue.slot
        };
        let start = millis(cue.start)?;
        let end = cue.end.map(millis).transpose()?;
        ensure!(end.is_none_or(|e| e > start), "Invalid local loop end");
        entries.push((
            Position { slot, start, end },
            Label {
                comment: cue.comment.clone(),
                color: cue.color,
                active_loop: cue.active_loop,
            },
        ));
    }
    finish(entries)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NativeCue {
    id: String,
    position: Position,
    label: Label,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Mirror {
    id: String,
    stamp: String,
    entries: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NativeCues {
    identity: String,
    cues: Vec<NativeCue>,
    mirror: Option<Mirror>,
    view: CueView,
}

impl NativeCues {
    pub(crate) fn view(&self) -> CueView {
        self.view.clone()
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
}

// Length and SQLite storage class delimit each value. Hashing full rows also detects
// a changed UUID, a different seek record or an edit which did not update its USN.
fn hash_row(hash: &mut Sha256, row: &Row<'_>) -> Result<()> {
    for i in 0..row.as_ref().column_count() {
        let value = row.get_ref(i)?;
        let num;
        let (tag, bytes): (u8, &[u8]) = match value {
            ValueRef::Null => (0, &[]),
            ValueRef::Integer(n) => {
                num = n.to_be_bytes();
                (1, &num)
            }
            ValueRef::Real(n) => {
                num = n.to_be_bytes();
                (2, &num)
            }
            ValueRef::Text(s) => (3, s),
            ValueRef::Blob(b) => (4, b),
        };
        hash.update([tag]);
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    Ok(())
}

pub(crate) fn identities(c: &Connection) -> Result<HashMap<String, String>> {
    // Import scans the native table once. There is no need to open per-track
    // analysis files or deserialize every native JSON cache for an inbound sync.
    let mut q =
        c.prepare("SELECT * FROM djmdCue WHERE rb_local_deleted=0 ORDER BY ContentID, ID")?;
    let mut rows = q.query([])?;
    let mut hashes: HashMap<String, Sha256> = HashMap::new();
    while let Some(row) = rows.next()? {
        hash_row(hashes.entry(row.get("ContentID")?).or_default(), row)?;
    }
    Ok(hashes
        .into_iter()
        .map(|(id, hash)| (id, format!("{:x}", hash.finalize())))
        .collect())
}

fn json_column(row: &Row<'_>, name: &str) -> Result<Value> {
    Ok(match row.get_ref(name)? {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => n.into(),
        ValueRef::Text(b) => std::str::from_utf8(b)?.into(),
        _ => bail!("Unsupported native cue value for {name}"),
    })
}

fn check_hot_cue_banks(c: &Connection, id: &str) -> Result<()> {
    let exists = |table: &str| -> rusqlite::Result<bool> {
        c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=? COLLATE NOCASE)", [table], |r| r.get(0))
    };
    if exists("djmdSongHotCueBanklist")? {
        let linked: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM djmdSongHotCueBanklist WHERE rb_local_deleted=0 AND
             (ContentID=?1 OR CueID IN (SELECT ID FROM djmdCue WHERE ContentID=?1 AND rb_local_deleted=0)))",
            [id], |r| r.get(0))?;
        ensure!(
            !linked,
            "A native hot cue bank uses this track; its linked cues need a coordinated update"
        );
    }
    if exists("hotCueBanklistCue")? {
        let linked: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM hotCueBanklistCue b, json_each(b.Cues) j WHERE b.rb_local_deleted=0 AND
              (json_type(b.Cues)!='array' OR j.type!='object'
               OR (json_extract(j.value,'$.ContentID') IS NULL AND json_extract(j.value,'$.CueID') IS NULL)
               OR CAST(json_extract(j.value,'$.ContentID') AS TEXT)=?1
               OR CAST(json_extract(j.value,'$.CueID') AS TEXT) IN
                    (SELECT ID FROM djmdCue WHERE ContentID=?1 AND rb_local_deleted=0)))", [id], |r| r.get(0))?;
        ensure!(
            !linked,
            "A hot cue bank mirror requires review before editing this track"
        );
    }
    Ok(())
}

pub(crate) fn read(c: &Connection, id: &str) -> Result<NativeCues> {
    check_hot_cue_banks(c, id)?;
    let mut caches =
        c.prepare("SELECT * FROM contentCue WHERE ContentID=? AND rb_local_deleted=0")?;
    let mut cache_rows = caches.query([id])?;
    let mirror = if let Some(row) = cache_rows.next()? {
        let entries: Vec<Value> = serde_json::from_str(&row.get::<_, String>("Cues")?)
            .context("Unsupported native contentCue JSON")?;
        ensure!(
            row.get::<_, i64>("rb_cue_count")? == entries.len() as i64,
            "Native cue mirror count differs"
        );
        let mut stamp = Sha256::new();
        hash_row(&mut stamp, row)?;
        let cache = Mirror {
            id: row.get("ID")?,
            stamp: format!("{:x}", stamp.finalize()),
            entries,
        };
        ensure!(
            cache_rows.next()?.is_none(),
            "Multiple active contentCue mirrors for the track"
        );
        Some(cache)
    } else {
        None
    };
    let mut cached = HashMap::new();
    if let Some(m) = &mirror {
        for entry in &m.entries {
            let cue_id = entry
                .get("ID")
                .and_then(Value::as_str)
                .context("Unrecognized cue mirror identity")?;
            ensure!(
                cached.insert(cue_id, entry).is_none(),
                "Duplicate cue identity in native mirror"
            );
        }
    }
    let mut q =
        c.prepare("SELECT * FROM djmdCue WHERE ContentID=? AND rb_local_deleted=0 ORDER BY ID")?;
    let mut rows = q.query([id])?;
    let mut cues = Vec::new();
    let mut digest = Sha256::new();
    while let Some(row) = rows.next()? {
        hash_row(&mut digest, row)?;
        let cue_id: String = row.get("ID")?;
        let entry = cached
            .remove(cue_id.as_str())
            .context("Native cue has no matching active contentCue mirror entry")?;
        // Keep mirror-only data and serialized order intact. Reject divergent native
        // identities/geometry and metadata instead of regenerating an unfamiliar mirror.
        for field in [
            "UUID",
            "ContentID",
            "Kind",
            "InMsec",
            "OutMsec",
            "ActiveLoop",
            "Comment",
            "Color",
            "ColorTableIndex",
        ] {
            ensure!(
                entry.get(field) == Some(&json_column(row, field)?),
                "Cue mirror differs for {field}"
            );
        }
        for field in [
            "ContentUUID",
            "InFrame",
            "OutFrame",
            "InMpegFrame",
            "OutMpegFrame",
            "InMpegAbs",
            "OutMpegAbs",
            "InPointSeekInfo",
            "OutPointSeekInfo",
            "BeatLoopSize",
            "CueMicrosec",
        ] {
            if let Some(value) = entry.get(field) {
                ensure!(
                    value == &json_column(row, field)?,
                    "Cue mirror has different {field}"
                );
            }
        }
        let uuid: String = row.get("UUID")?;
        ensure!(
            !cue_id.is_empty() && !uuid.is_empty(),
            "Missing native cue identity"
        );
        let kind: i64 = row.get("Kind")?;
        ensure!(
            (0..=17).contains(&kind),
            "Unrecognized native cue kind {kind}"
        );
        let slot = cue_slot(kind);
        let start: i64 = row.get("InMsec")?;
        ensure!(
            (0..u32::MAX as i64).contains(&start),
            "Invalid native cue position"
        );
        let end = row.get::<_, Option<i64>>("OutMsec")?.filter(|e| *e > start);
        let active = row.get::<_, Option<i64>>("ActiveLoop")?.unwrap_or(0);
        ensure!(
            (0..=1).contains(&active),
            "Unsupported native active-loop flag"
        );
        let color = desktop_cue_color(
            slot,
            end.is_some(),
            row.get("Color")?,
            row.get("ColorTableIndex")?,
            true,
        );
        cues.push(NativeCue {
            id: cue_id,
            position: Position {
                slot: slot.unwrap_or(-1),
                start,
                end,
            },
            label: Label {
                comment: row.get::<_, Option<String>>("Comment")?.unwrap_or_default(),
                color,
                active_loop: active != 0,
            },
        });
    }
    ensure!(
        cached.is_empty(),
        "The native mirror contains additional cues"
    );
    let view = finish(
        cues.iter()
            .map(|cue| (cue.position.clone(), cue.label.clone()))
            .collect(),
    )?;
    Ok(NativeCues {
        identity: format!("{:x}", digest.finalize()),
        cues,
        mirror,
        view,
    })
}

#[derive(Debug)]
struct Edit {
    id: String,
    comment: Option<String>,
    memory_color: Option<i64>,
    hot_palette: Option<i64>,
    kind: Option<i64>,
    remove: bool,
}

#[derive(Debug)]
pub(crate) struct CuePatch {
    edits: Vec<Edit>,
    desired: CueView,
}

pub(crate) fn plan(current: &NativeCues, desired: &CueView) -> Result<CuePatch> {
    let desired_labels: BTreeMap<_, _> = desired.0.iter().map(|(p, label)| (p, label)).collect();
    let existing: HashSet<_> = current.cues.iter().map(|cue| &cue.position).collect();
    let mut unmatched: BTreeMap<_, _> = desired_labels
        .iter()
        .filter(|(p, _)| !existing.contains(**p))
        .map(|(&p, &l)| (p, l))
        .collect();
    let mut edits = Vec::new();
    for cue in &current.cues {
        let mut edit = Edit {
            id: cue.id.clone(),
            comment: None,
            memory_color: None,
            hot_palette: None,
            kind: None,
            remove: false,
        };
        let mut target = desired_labels.get(&cue.position).copied();
        if target.is_none() && cue.position.slot >= 0 {
            // Moving a pad without retiming it preserves its exact native seek fields.
            // Require a unique source and destination at this loop/point geometry.
            let same_geometry = |p: &Position| {
                p.slot >= 0 && p.start == cue.position.start && p.end == cue.position.end
            };
            let mut destinations = unmatched.keys().copied().filter(|p| same_geometry(p));
            if let Some(to) = destinations.next() {
                ensure!(
                    destinations.next().is_none()
                        && current
                            .cues
                            .iter()
                            .filter(|c| !desired_labels.contains_key(&c.position)
                                && same_geometry(&c.position))
                            .count()
                            == 1,
                    "Multiple hot cues share the proposed pad's position"
                );
                edit.kind = Some(i64::from(if to.slot < 3 {
                    to.slot + 1
                } else {
                    to.slot + 2
                }));
                target = unmatched.remove(to);
            }
        }
        if let Some(label) = target {
            if label == &cue.label && edit.kind.is_none() {
                continue;
            }
            ensure!(
                label.active_loop == cue.label.active_loop,
                "Changing an active loop needs native timing metadata"
            );
            if label.comment != cue.label.comment {
                ensure!(
                    !label.comment.contains('\0') && label.comment.encode_utf16().count() <= 255,
                    "Cue comments must fit in 255 UTF-16 units and contain no NUL"
                );
                edit.comment = Some(label.comment.clone());
            }
            if label.color != cue.label.color || edit.kind.is_some() {
                if cue.position.slot < 0 {
                    ensure!(label.color <= 8, "Unknown memory cue color");
                    edit.memory_color = Some(if label.color == 0 {
                        255
                    } else {
                        i64::from(label.color - 1)
                    });
                } else {
                    ensure!(
                        (1..=64).contains(&label.color),
                        "Unknown hot cue palette index"
                    );
                    edit.hot_palette = Some(i64::from(label.color));
                }
            }
        } else {
            edit.remove = true;
        }
        edits.push(edit);
    }
    ensure!(unmatched.is_empty(), "Creating or retiming cues needs native seek data; this pilot supports labels, colors, hot pad reassignment and removals");
    Ok(CuePatch {
        edits,
        desired: desired.clone(),
    })
}

pub(crate) fn check_sidecars(share: &Path, analysis_path: &str, patch: &CuePatch) -> Result<()> {
    if patch.edits.is_empty() || analysis_path.is_empty() {
        return Ok(());
    }
    let position_changes = patch
        .edits
        .iter()
        .any(|edit| edit.remove || edit.kind.is_some());
    for ext in ["DAT", "EXT", "2EX"] {
        let Some(data) = super::grid::read_sidecar(share, analysis_path, ext)? else {
            ensure!(
                ext != "DAT",
                "The registered desktop analysis file is missing"
            );
            continue;
        };
        // read_sidecar has validated the PMAI container and all section extents.
        let mut pos = 28;
        while pos < data.len() {
            let section = &data[pos..];
            let header = u32::from_be_bytes(section[4..8].try_into()?) as usize;
            let total = u32::from_be_bytes(section[8..12].try_into()?) as usize;
            match &section[..4] {
                b"PCO2" => {
                    ensure!(header == 20 && total == header && section[16..18] == [0,0],
                        "An analysis file contains extended cue data; this pilot cannot synchronize its cue mirror");
                }
                b"PCOB" if position_changes => {
                    ensure!(header == 24 && total == header && section[18..20] == [0,0],
                        "An analysis file contains cue positions; this change would leave an inconsistent mirror");
                }
                _ => {}
            }
            pos += total;
        }
    }
    Ok(())
}

fn next(usn: &mut i64) -> Result<i64> {
    *usn = usn
        .checked_add(1)
        .context("Native update counter overflow")?;
    Ok(*usn)
}

pub(crate) fn apply(
    c: &Connection,
    id: &str,
    baseline: &NativeCues,
    patch: &CuePatch,
    usn: &mut i64,
    timestamp: &str,
) -> Result<()> {
    ensure!(
        !c.is_autocommit(),
        "Cue writes require the native write transaction"
    );
    ensure!(
        &read(c, id)? == baseline,
        "Native cues changed since preview"
    );
    if patch.edits.is_empty() {
        return Ok(());
    }
    let mirror = baseline.mirror.as_ref().context("No native cue mirror")?;
    let mut entries = mirror.entries.clone();
    for edit in &patch.edits {
        let revision = next(usn)?;
        let n = c.execute("UPDATE djmdCue SET Comment=coalesce(?1,Comment), Color=coalesce(?2,Color),
            ColorTableIndex=coalesce(?3,ColorTableIndex), Kind=coalesce(?4,Kind), rb_local_deleted=?5, rb_local_usn=?6, updated_at=?7, rb_local_synced=0
            WHERE ContentID=?8 AND ID=?9 AND rb_local_deleted=0",
            params![edit.comment, edit.memory_color, edit.hot_palette, edit.kind, edit.remove, revision, timestamp, id, edit.id])?;
        ensure!(n == 1, "Native cue identity disappeared");
        let i = entries
            .iter()
            .position(|e| e["ID"].as_str() == Some(edit.id.as_str()))
            .context("Native cue mirror identity disappeared")?;
        if edit.remove {
            entries.remove(i);
        } else {
            let obj: &mut Map<String, Value> =
                entries[i].as_object_mut().context("Invalid cue mirror")?;
            if let Some(v) = &edit.comment {
                obj.insert("Comment".into(), v.as_str().into());
            }
            if let Some(v) = edit.memory_color {
                obj.insert("Color".into(), v.into());
            }
            if let Some(v) = edit.kind {
                obj.insert("Kind".into(), v.into());
            }
            if let Some(v) = edit.hot_palette {
                obj.insert("ColorTableIndex".into(), v.into());
            }
            for (field, value) in [
                ("rb_local_usn", revision.into()),
                ("updated_at", timestamp.into()),
                ("rb_local_synced", 0.into()),
            ] {
                if obj.contains_key(field) {
                    obj.insert(field.into(), value);
                }
            }
        }
    }
    let n = c.execute("UPDATE contentCue SET Cues=?, rb_cue_count=?, rb_local_usn=?, updated_at=?, rb_local_synced=0
        WHERE ID=? AND ContentID=? AND rb_local_deleted=0",
        params![serde_json::to_string(&entries)?, i64::try_from(entries.len())?, next(usn)?, timestamp, mirror.id, id])?;
    ensure!(n == 1, "Native cue mirror disappeared");
    let n = c.execute("UPDATE djmdContent SET CueUpdated=1, rb_local_usn=?, updated_at=?, rb_local_synced=0 WHERE ID=? AND rb_local_deleted=0",
        params![next(usn)?, timestamp, id])?;
    ensure!(n == 1, "Cue owner disappeared");
    ensure!(
        read(c, id)?.view == patch.desired,
        "Native cue verification failed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Result<Connection> {
        let c = Connection::open_in_memory()?;
        c.execute_batch("CREATE TABLE djmdContent(ID TEXT PRIMARY KEY, CueUpdated, rb_local_usn, updated_at, rb_local_synced, rb_local_deleted);
            INSERT INTO djmdContent VALUES ('song',0,1,'old',1,0);
            CREATE TABLE djmdCue(ID TEXT PRIMARY KEY, UUID TEXT, ContentID TEXT, Kind INTEGER, InMsec INTEGER, OutMsec INTEGER,
                InPointSeekInfo TEXT, ActiveLoop INTEGER, Comment TEXT, Color INTEGER, ColorTableIndex INTEGER,
                rb_local_deleted INTEGER, rb_local_usn INTEGER, updated_at TEXT, rb_local_synced INTEGER);
            INSERT INTO djmdCue VALUES('pad','native-pad','song',1,1250,-1,'original-seek',0,'Start',-1,62,0,3,'old',1);
            INSERT INTO djmdCue VALUES('mem','native-mem','song',0,8000,11000,'loop-seek',0,'Break',0,0,0,4,'old',1);
            CREATE TABLE contentCue(ID TEXT PRIMARY KEY, ContentID TEXT, Cues TEXT, rb_cue_count INTEGER, rb_local_deleted INTEGER,
                rb_local_usn INTEGER, updated_at TEXT, rb_local_synced INTEGER);")?;
        let entry = |id: &str,
                     kind: i64,
                     start: i64,
                     end: i64,
                     name: &str,
                     color: i64,
                     palette: i64| {
            json!({
                "ID":id, "UUID":format!("native-{id}"), "ContentID":"song", "Kind":kind, "InMsec":start, "OutMsec":end,
                "ActiveLoop":0, "Comment":name, "Color":color, "ColorTableIndex":palette,
                "unknown":{"fields":[1,null]}, "rb_local_synced":1
            })
        };
        let entries = vec![
            entry("pad", 1, 1250, -1, "Start", -1, 62),
            entry("mem", 0, 8000, 11000, "Break", 0, 0),
        ];
        c.execute(
            "INSERT INTO contentCue VALUES ('cache','song',?,2,0,5,'old',1)",
            [serde_json::to_string(&entries)?],
        )?;
        Ok(c)
    }

    #[test]
    fn reviewed_metadata_and_pad_reassignment_preserve_ids_and_native_data() -> Result<()> {
        let c = fixture()?;
        let base = read(&c, "song")?;
        assert_eq!(identities(&c)?["song"], base.identity());
        let mut target = base.view();
        let (mem, hot) = target.0.split_at_mut(1);
        assert_eq!(mem[0].1.color, 1); // native 0 is pink
        assert_eq!(hot[0].1.color, 62); // hot pads use the separate palette
        mem[0].1.color = 0;
        hot[0].0.slot = 3;
        hot[0].1.color = 5;
        hot[0].1.comment = "Vocal ♫".into();
        let patch = plan(&base, &target)?;
        c.execute_batch("BEGIN IMMEDIATE")?;
        let mut usn = 5;
        apply(&c, "song", &base, &patch, &mut usn, "now")?;
        assert_eq!(usn, 9);
        let attrs: (i64, i64, String, String) = c.query_row(
            "SELECT Kind,ColorTableIndex,InPointSeekInfo,UUID FROM djmdCue WHERE ID='pad'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        assert_eq!(
            attrs,
            (5, 5, "original-seek".to_owned(), "native-pad".to_owned())
        );
        assert_eq!(
            c.query_row("SELECT Color FROM djmdCue WHERE ID='mem'", [], |r| r
                .get::<_, i64>(0))?,
            255
        );
        let updated = read(&c, "song")?;
        assert_eq!(updated.view(), target);
        assert!(plan(&updated, &target)?.edits.is_empty());
        let mirror = updated.mirror.unwrap();
        assert_eq!(mirror.entries[0]["unknown"], json!({"fields":[1,null]}));
        assert_eq!(mirror.entries[0]["Comment"], "Vocal ♫");
        assert_eq!(mirror.entries[0]["Kind"], 5);
        assert_eq!(mirror.entries[0]["rb_local_synced"], 0);
        c.execute_batch("ROLLBACK")?;
        assert_eq!(read(&c, "song")?, base);
        Ok(())
    }

    #[test]
    fn deletion_tombstones_only_reviewed_cue_and_keeps_mirror_in_sync() -> Result<()> {
        let c = fixture()?;
        let base = read(&c, "song")?;
        let mut target = base.view();
        target.0.remove(0);
        let patch = plan(&base, &target)?;
        c.execute_batch("BEGIN IMMEDIATE")?;
        apply(&c, "song", &base, &patch, &mut 10, "now")?;
        assert_eq!(
            c.query_row(
                "SELECT rb_local_deleted FROM djmdCue WHERE ID='mem'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        assert_eq!(
            c.query_row("SELECT rb_local_usn FROM djmdCue WHERE ID='pad'", [], |r| r
                .get::<_, i64>(0))?,
            3
        );
        assert_eq!(read(&c, "song")?.mirror.unwrap().entries.len(), 1);
        Ok(())
    }

    #[test]
    fn refuses_new_geometry_changed_seek_or_disagreeing_json_mirror() -> Result<()> {
        let c = fixture()?;
        let base = read(&c, "song")?;
        let mut target = base.view();
        target.0[1].0.start += 1;
        assert!(plan(&base, &target).is_err());
        target = base.view();
        target.0[1].1.comment = "New".into();
        let patch = plan(&base, &target)?;
        c.execute_batch(
            "BEGIN IMMEDIATE; UPDATE djmdCue SET InPointSeekInfo='other' WHERE ID='pad'",
        )?;
        let mut usn = 10;
        assert!(apply(&c, "song", &base, &patch, &mut usn, "now").is_err());
        assert_eq!(usn, 10);
        c.execute(
            "UPDATE contentCue SET Cues=replace(Cues,'Start','Moved')",
            [],
        )?;
        assert!(read(&c, "song").is_err());
        Ok(())
    }

    #[test]
    fn a_bank_or_an_orphan_bank_mirror_blocks_writeback_to_linked_cues() -> Result<()> {
        let c = fixture()?;
        c.execute_batch("CREATE TABLE djmdSongHotCueBanklist(ContentID TEXT, CueID TEXT, rb_local_deleted INTEGER);
            INSERT INTO djmdSongHotCueBanklist VALUES (NULL,'pad',0);")?;
        assert!(read(&c, "song").is_err());
        c.execute_batch(
            "UPDATE djmdSongHotCueBanklist SET rb_local_deleted=1;
            CREATE TABLE hotCueBanklistCue(Cues TEXT, rb_local_deleted INTEGER);",
        )?;
        c.execute(
            "INSERT INTO hotCueBanklistCue VALUES (?,0)",
            [r#"[{"CueID":"mem"}]"#],
        )?;
        assert!(read(&c, "song").is_err());
        c.execute(
            "UPDATE hotCueBanklistCue SET Cues=?",
            [r#"[{"ContentID":"other"}]"#],
        )?;
        read(&c, "song")?;
        Ok(())
    }

    #[test]
    fn two_way_encrypted_cue_pilot_checks_preview_and_reopens_recovery_copy() -> Result<()> {
        use crate::rbimport::writeback::{self, integration_tests::Libraries, Field};
        use crate::{db::Db, rbimport};
        let fixture = Libraries::new()?;
        let master = fixture.source.join("master.db");
        let native = |p: &Path| -> Result<Connection> {
            let db = Connection::open(p)?;
            db.pragma_update(None, "key", rbimport::MASTER_DB_KEY)?;
            Ok(db)
        };
        let owner = "f0214d3b-d66c-46a1-8927-04de39519c39";
        let initial = json!([{"ID":"deckcue","UUID":"source-cue","ContentID":"10","ContentUUID":owner,
            "Kind":1,"InMsec":500,"OutMsec":-1,"Color":-1,"ColorTableIndex":19,"ActiveLoop":0,
            "Comment":"First","InPointSeekInfo":"opaque","vendor":{"data":[1,2]}}]);
        {
            let c = native(&master)?;
            c.execute_batch(
                "ALTER TABLE djmdCue ADD COLUMN ColorTableIndex INTEGER;
                ALTER TABLE djmdCue ADD COLUMN InPointSeekInfo TEXT;",
            )?;
            c.execute("INSERT INTO djmdCue(ID,UUID,ContentID,ContentUUID,InMsec,OutMsec,Kind,Color,ActiveLoop,
                Comment,rb_local_synced,rb_local_usn,updated_at,rb_local_deleted,ColorTableIndex,InPointSeekInfo)
                VALUES('deckcue','source-cue','10',?,500,-1,1,-1,0,'First',1,1,'next',0,19,'opaque')",[owner])?;
            c.execute("INSERT INTO contentCue(ID,UUID,ContentID,Cues,rb_cue_count,rb_local_deleted,updated_at,rb_local_synced,rb_local_usn)
                VALUES('cache','mirror-uuid','10',?,1,0,'next',1,1)",[initial.to_string()])?;
        }
        let staged = rbimport::load_from(&fixture.source, fixture.local.parent().unwrap())?;
        {
            let local = Db::open(&fixture.local)?;
            rbimport::merge::apply(&local, &staged)?;
            let track = local.track_at_path(&fixture.audio)?.unwrap();
            let mut edited = local.cues(track.id)?;
            assert_eq!(edited.len(), 1);
            assert_eq!(edited[0].color, 19);
            edited[0].comment = "Refrain".into();
            edited[0].color = 41;
            local.replace_cues(track.id, &edited)?;
        }
        let select = |plan: &writeback::Plan| -> Vec<usize> {
            plan.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.field == Field::Cues && r.reason.is_none())
                .map(|(i, _)| i)
                .collect()
        };
        let first = writeback::preview(&fixture.local, &fixture.source)?;
        assert_eq!(select(&first).len(), 1);
        let mut revised_mirror = initial;
        revised_mirror[0]["vendor"]["data"] = json!([3]);
        native(&master)?.execute("UPDATE contentCue SET Cues=?", [revised_mirror.to_string()])?;
        assert!(
            writeback::apply(&fixture.local, &fixture.source, &first, &select(&first)).is_err()
        );
        let reviewed = writeback::preview(&fixture.local, &fixture.source)?;
        assert_eq!(select(&reviewed).len(), 1);
        let receipt = writeback::apply(
            &fixture.local,
            &fixture.source,
            &reviewed,
            &select(&reviewed),
        )?;
        let copy = read(&native(&receipt.backup.join("master.db"))?, "10")?;
        assert_eq!(copy.view.0[0].1.comment, "First");
        let written = read(&native(&master)?, "10")?;
        assert_eq!(written.view.0[0].1.comment, "Refrain");
        assert_eq!(written.view.0[0].1.color, 41);
        assert_eq!(
            written.mirror.unwrap().entries[0]["vendor"]["data"],
            json!([3])
        );
        assert!(select(&writeback::preview(&fixture.local, &fixture.source)?).is_empty());
        Ok(())
    }

    #[test]
    fn cue_sidecar_checks_refuse_inconsistent_device_mirrors() -> Result<()> {
        let c = fixture()?;
        let root = tempfile::tempdir()?;
        let source = "PIONEER/USBANLZ/ANLZ0000.DAT";
        let dat = root.path().join(source);
        std::fs::create_dir_all(dat.parent().unwrap())?;
        let mut pmai = vec![0; 28];
        pmai[..4].copy_from_slice(b"PMAI");
        pmai[4..8].copy_from_slice(&28u32.to_be_bytes());
        pmai[8..12].copy_from_slice(&28u32.to_be_bytes());
        let base = read(&c, "song")?;
        let mut desired = base.view();
        desired.0[1].1.color = 42;
        let patch = plan(&base, &desired)?;
        assert!(check_sidecars(root.path(), source, &patch).is_err());
        std::fs::write(&dat, &pmai)?;
        check_sidecars(root.path(), source, &patch)?;
        let mut ext = pmai;
        ext.extend_from_slice(b"PCO2");
        ext.extend_from_slice(&20u32.to_be_bytes());
        ext.extend_from_slice(&20u32.to_be_bytes());
        ext.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 0]);
        ext[8..12].copy_from_slice(&48u32.to_be_bytes());
        std::fs::write(dat.with_extension("EXT"), ext)?;
        assert!(check_sidecars(root.path(), source, &patch).is_err());
        Ok(())
    }
}
