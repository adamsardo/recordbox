//! Apply one validated desktop snapshot to Recordbox's private collection.
use super::RbLibrary;
use crate::db::Db;
use crate::model::{NodeKind, Track};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

pub(crate) struct Summary {
    pub added: usize,
    pub updated: usize,
    pub first: bool,
}

fn setting_json<T: serde::de::DeserializeOwned + Default>(db: &Db, name: &str) -> Result<T> {
    match db.setting_checked(name)? {
        Some(json) => serde_json::from_str(&json).with_context(|| format!("reading {name}")),
        None => Ok(T::default()),
    }
}

pub(crate) fn apply(db: &Db, lib: &RbLibrary) -> Result<Summary> {
    let tx = db.conn.unchecked_transaction()?;
    let mut seen: HashMap<String, String> = setting_json(db, "rb_sync_fingerprints")?;
    let first = seen.is_empty();
    // My Tag categories and tags, matched by name.
    let mut tag_id: HashMap<(String, String), i64> = HashMap::new();
    let mut rb_tag_ids: HashSet<i64> = HashSet::new();
    let existing = db.mytags()?;
    for (cat, tags) in &lib.tag_categories {
        let cid = match existing.iter().find(|t| t.parent == 0 && &t.name == cat) {
            Some(t) => t.id,
            None => db.add_mytag(0, cat)?,
        };
        for tg in tags {
            let tid = match existing.iter().find(|t| t.parent == cid && &t.name == tg) {
                Some(t) => t.id,
                None => db.add_mytag(cid, tg)?,
            };
            tag_id.insert((cat.clone(), tg.clone()), tid);
            rb_tag_ids.insert(tid);
        }
    }
    let (mut added, mut updated) = (0, 0);
    let mut idmap: HashMap<String, i64> = HashMap::new();
    // Where rekordbox's own analysis of each audio file lives (used by the USB export).
    let mut rb_anlz: HashMap<String, String> = setting_json(db, "rb_anlz")?;
    for rt in &lib.tracks {
        if let Some(b) = &rt.anlz_base {
            rb_anlz.insert(rt.xt.track.path.clone(), b.to_string_lossy().to_string());
        } else {
            rb_anlz.remove(&rt.xt.track.path);
        }
    }
    db.set_setting_checked("rb_anlz", &serde_json::to_string(&rb_anlz)?)?;
    // Tracks the user removed from Recordbox stay removed.
    let removed: HashSet<String> = setting_json(db, "rb_removed_paths")?;
    let mut unique = HashSet::new();
    let duplicated: HashSet<_> = lib.tracks.iter().filter_map(|r| {
        let path = r.xt.track.path.as_str();
        (!unique.insert(path)).then_some(path)
    }).collect();
    for rt in &lib.tracks {
        if removed.contains(&rt.xt.track.path) {
            continue;
        }
        // known: synced before; changed: rekordbox edited it since the last sync.
        let changed = seen
            .get(&rt.rb_id)
            .is_some_and(|previous| previous != &rt.fingerprint);
        let mut existing = db.track_at_path(&rt.xt.track.path)?;
        // Libraries synced before the key's spelling was kept: fill it in once, where the
        // stored key is still rekordbox's own (not analysed over or chosen by the user).
        if let Some(t) = existing.as_mut() {
            let rb = &rt.xt.track;
            if t.key_text.is_empty() && !rb.key_text.is_empty() && !t.key_manual && t.key == rb.key
            {
                t.key_text = rb.key_text.clone();
                db.update_track(t)?;
            }
        }
        let (id, fresh) = match &existing {
            None => {
                let mut t = rt.xt.track.clone();
                if t.sample_rate == 0 || t.duration <= 0.0 {
                    let real = crate::importer::read_track(std::path::Path::new(&t.path));
                    t.sample_rate = real.sample_rate;
                    t.bit_depth = real.bit_depth;
                    if t.duration <= 0.0 {
                        t.duration = real.duration;
                    }
                    if t.artwork.is_empty() {
                        t.artwork = real.artwork;
                    }
                }
                let id = db.insert_track(&t)?;
                added += 1;
                (id, true)
            }
            Some(t) => (t.id, false),
        };
        let cur = existing;
        let locked = cur.as_ref().map(|t| t.grid_locked).unwrap_or(false);
        if changed {
            if let Some(cur) = &cur {
                let r = &rt.xt.track;
                let t = Track {
                    id,
                    analyzed: cur.analyzed,
                    grid_locked: cur.grid_locked,
                    // The desktop reader provides no values for these Recordbox fields.
                    album_artist: cur.album_artist.clone(),
                    mix_name: cur.mix_name.clone(),
                    grouping: cur.grouping.clone(),
                    message: cur.message.clone(),
                    gain_db: cur.gain_db,
                    raw_samples: cur.raw_samples,
                    missing: cur.missing,
                    artwork: if r.artwork.is_empty() {
                        cur.artwork.clone()
                    } else {
                        r.artwork.clone()
                    },
                    sample_rate: if r.sample_rate > 0 {
                        r.sample_rate
                    } else {
                        cur.sample_rate
                    },
                    bit_depth: if r.bit_depth > 0 {
                        r.bit_depth
                    } else {
                        cur.bit_depth
                    },
                    duration: if r.duration > 0.0 {
                        r.duration
                    } else {
                        cur.duration
                    },
                    // A locked grid keeps its tempo.
                    bpm: if locked { cur.bpm } else { r.bpm },
                    ..r.clone()
                };
                db.update_track(&t)?;
                updated += 1;
            }
        }
        let my_cues = if fresh { vec![] } else { db.cues(id)? };
        let my_grid = if fresh {
            None
        } else {
            db.grid_checked(id)?.filter(|g| !g.is_empty())
        };
        // Grid: new tracks and tracks without one get rekordbox's; a changed track gets it
        // unless the grid is locked.
        if let Some(g) = &rt.xt.grid {
            if fresh || my_grid.is_none() || (changed && !locked) {
                db.save_grid(id, g)?;
            }
        }
        // Cues: new tracks and tracks without cues get rekordbox's; a changed track gets
        // them only when rekordbox has some (Recordbox's are never wiped by an empty list).
        if (fresh || my_cues.is_empty() || changed)
            && !(rt.xt.cues.is_empty() && !my_cues.is_empty())
        {
            db.replace_cues(id, &rt.xt.cues)?;
        }
        // My Tags: rekordbox's are added; on a change in rekordbox, ones it removed are too.
        let mine: HashSet<i64> = rt
            .tags
            .iter()
            .filter_map(|k| tag_id.get(k).copied())
            .collect();
        for t in &rb_tag_ids {
            if mine.contains(t) {
                db.set_track_tag(id, *t, true)?;
            } else if changed {
                db.set_track_tag(id, *t, false)?;
            }
        }
        if let Some(source) = &lib.source {
            if duplicated.contains(rt.xt.track.path.as_str()) {
                db.conn.execute("DELETE FROM native_edit_baseline WHERE track_id=?1", [id])?;
            } else {
                super::writeback::record_import(db, id, rt, source)?;
            }
        }
        seen.insert(rt.rb_id.clone(), rt.fingerprint.clone());
        idmap.insert(rt.rb_id.clone(), id);
    }
    db.set_setting_checked("rb_sync_fingerprints", &serde_json::to_string(&seen)?)?;
    // Playlists: rebuild the mirrored folder.
    if let Some(old) = db.setting_checked("rb_folder_id")? {
        let old: i64 = old
            .parse()
            .context("reading the saved rekordbox mirror ID")?;
        if db.playlists()?.iter().any(|n| {
            n.id == old && n.parent == 0 && n.kind == NodeKind::Folder && n.name == "rekordbox"
        }) {
            db.delete_node(old)?;
        }
        db.conn
            .execute("DELETE FROM settings WHERE key='rb_folder_id'", [])?;
    }
    fn add_nodes(
        db: &Db,
        parent: i64,
        nodes: &[crate::importer::XmlNode],
        idmap: &HashMap<String, i64>,
        tag_id: &HashMap<(String, String), i64>,
    ) -> Result<()> {
        for n in nodes {
            if n.folder {
                let id = db.create_node(parent, &n.name, NodeKind::Folder, None)?;
                add_nodes(db, id, &n.children, idmap, tag_id)?;
            } else if let Some(rule) = &n.smart {
                // Intelligent playlist: My Tag conditions name their tag until now.
                let mut rule = rule.clone();
                let mut ok = true;
                for c in rule
                    .conditions
                    .iter_mut()
                    .filter(|c| c.field == crate::smart::Field::MyTag)
                {
                    let found = c
                        .value
                        .strip_prefix('\u{1}')
                        .and_then(|v| v.split_once('\u{1f}'))
                        .and_then(|(cat, t)| tag_id.get(&(cat.to_string(), t.to_string())));
                    match found {
                        Some(id) => c.value = id.to_string(),
                        None => ok = false,
                    }
                }
                if ok {
                    db.create_node(parent, &n.name, NodeKind::Smart, Some(&rule))?;
                }
            } else {
                let id = db.create_node(parent, &n.name, NodeKind::Playlist, None)?;
                let tr: Vec<i64> = n
                    .track_keys
                    .iter()
                    .filter_map(|k| idmap.get(k).copied())
                    .collect();
                db.set_playlist_tracks(id, &tr)?;
            }
        }
        Ok(())
    }
    if !lib.playlists.is_empty() || !lib.histories.is_empty() {
        let root = db.create_node(0, "rekordbox", NodeKind::Folder, None)?;
        db.set_setting_checked("rb_folder_id", &root.to_string())?;
        add_nodes(db, root, &lib.playlists, &idmap, &tag_id)?;
        if !lib.histories.is_empty() {
            let h = db.create_node(root, "Histories", NodeKind::Folder, None)?;
            add_nodes(db, h, &lib.histories, &idmap, &tag_id)?;
        }
    }
    tx.commit()?;
    Ok(Summary {
        added,
        updated,
        first,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::{XmlNode, XmlTrack};
    use crate::model::{BeatGrid, Cue, CueKind};
    use crate::rbimport::RbTrack;
    use std::path::Path;

    fn library(stamp: &str, title: &str) -> RbLibrary {
        let track = Track {
            path: "/fixture/track.wav".into(),
            title: title.into(),
            duration: 50.0,
            sample_rate: 44_100,
            bpm: 120.0,
            ..Default::default()
        };
        let cue = Cue {
            id: 0,
            track_id: 0,
            kind: CueKind::Memory,
            slot: -1,
            start: 12.0,
            end: None,
            color: 0,
            comment: "Source cue".into(),
            active_loop: false,
        };
        let xt = XmlTrack {
            xml_id: "7".into(),
            track,
            grid: Some(BeatGrid::constant(0.1, 120.0, 1)),
            cues: vec![cue],
            extra: Default::default(),
        };
        RbLibrary {
            source: None,
            tracks: vec![RbTrack {
                rb_id: "7".into(),
                native: None,
                fingerprint: stamp.into(),
                xt,
                tags: vec![],
                anlz_base: None,
            }],
            playlists: vec![XmlNode {
                name: title.into(),
                children: vec![],
                folder: false,
                track_keys: vec!["7".into()],
                by_location: false,
                smart: None,
            }],
            tag_categories: vec![],
            not_local: 0,
            smart_lists: 0,
            histories: vec![],
        }
    }

    #[test]
    fn desktop_sync_rolls_back_tracks_mirror_and_fingerprints_on_write_failure() -> Result<()> {
        let db = Db::open(Path::new(":memory:"))?;
        apply(&db, &library("v1", "Old"))?;
        let old_mirror = db.setting("rb_folder_id").unwrap();
        let old_lists = db.playlists()?;
        db.conn.execute_batch("CREATE TRIGGER fail_sync BEFORE INSERT ON playlist_tracks BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;")?;
        assert!(apply(&db, &library("v2", "New")).is_err());
        assert!(db.conn.is_autocommit());
        assert_eq!(db.all_tracks()?[0].title, "Old");
        assert_eq!(
            db.setting("rb_folder_id").as_deref(),
            Some(old_mirror.as_str())
        );
        let restored = db.playlists()?;
        assert_eq!(restored.len(), old_lists.len());
        assert_eq!(restored[1].name, "Old");
        assert_eq!(
            db.playlist_tracks(restored[1].id)?,
            vec![db.all_tracks()?[0].id]
        );
        db.conn.execute_batch("DROP TRIGGER fail_sync")?;
        assert_eq!(apply(&db, &library("v2", "New"))?.updated, 1);
        assert_eq!(db.all_tracks()?[0].title, "New");
        Ok(())
    }

    #[test]
    fn desktop_sync_preserves_user_edits_locks_and_empty_source_cues() -> Result<()> {
        let db = Db::open(Path::new(":memory:"))?;
        assert_eq!(apply(&db, &library("v1", "Source"))?.added, 1);
        let mut t = db.all_tracks()?.remove(0);
        t.title = "Local edit".into();
        t.grouping = "My grouping".into();
        t.grid_locked = true;
        t.bpm = 121.0;
        db.update_track(&t)?;
        db.save_grid(t.id, &BeatGrid::constant(0.25, t.bpm, 1))?;
        assert_eq!(apply(&db, &library("v1", "Source"))?.updated, 0);
        assert_eq!(db.track(t.id).unwrap().title, "Local edit");
        let mut changed = library("v2", "Changed in source");
        changed.tracks[0].xt.cues.clear();
        assert_eq!(apply(&db, &changed)?.updated, 1);
        let merged = db.track(t.id).unwrap();
        assert_eq!(
            (merged.title.as_str(), merged.grouping.as_str(), merged.bpm),
            ("Changed in source", "My grouping", 121.0)
        );
        assert_eq!(db.grid(t.id).unwrap(), BeatGrid::constant(0.25, 121.0, 1));
        assert_eq!(db.cues(t.id)?[0].comment, "Source cue");
        Ok(())
    }

    #[test]
    fn desktop_sync_keeps_a_user_folder_that_reuses_the_deleted_mirror_id() -> Result<()> {
        let db = Db::open(Path::new(":memory:"))?;
        let lib = library("v1", "Fixture");
        apply(&db, &lib)?;
        let mirror: i64 = db.setting("rb_folder_id").unwrap().parse()?;
        db.delete_node(mirror)?;
        let local = db.create_node(0, "rekordbox", NodeKind::Folder, None)?;
        assert_eq!(local, mirror, "exercise SQLite's row ID reuse");
        let child = db.create_node(local, "User playlist", NodeKind::Playlist, None)?;
        apply(&db, &lib)?;
        assert!(db
            .playlists()?
            .iter()
            .any(|node| node.id == child && node.name == "User playlist"));
        assert_ne!(db.setting("rb_folder_id").unwrap(), local.to_string());
        Ok(())
    }
}
