//! The app's own private library database (SQLite in ~/Library/Application Support/PrepDeck).
//! It never opens or writes rekordbox's library, and never writes to audio files.

use crate::model::*;
use crate::smart::SmartRule;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("PREPDECK_DATA") {
        let p = PathBuf::from(d);
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let p = base.join("PrepDeck");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn artwork_dir() -> PathBuf {
    let p = data_dir().join("artwork");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub struct Db {
    pub conn: Connection,
}

const TRACK_COLS: &str = "id,path,title,artist,album,album_artist,genre,label,remixer,original_artist,composer,lyricist,mix_name,comment,grouping,message,year,track_no,disc_no,release_date,duration,sample_rate,bitrate,bit_depth,file_size,file_type,bpm,key,rating,color,date_added,date_created,play_count,analyzed,grid_locked,autoload_hotcue,gain_db,artwork,raw_samples";

impl Db {
    pub fn open(path: &std::path::Path) -> Result<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS tracks(
                id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, title TEXT DEFAULT '', artist TEXT DEFAULT '',
                album TEXT DEFAULT '', album_artist TEXT DEFAULT '', genre TEXT DEFAULT '', label TEXT DEFAULT '',
                remixer TEXT DEFAULT '', original_artist TEXT DEFAULT '', composer TEXT DEFAULT '', lyricist TEXT DEFAULT '',
                mix_name TEXT DEFAULT '', comment TEXT DEFAULT '', grouping TEXT DEFAULT '', message TEXT DEFAULT '',
                year INTEGER DEFAULT 0, track_no INTEGER DEFAULT 0, disc_no INTEGER DEFAULT 0, release_date TEXT DEFAULT '',
                duration REAL DEFAULT 0, sample_rate INTEGER DEFAULT 0, bitrate INTEGER DEFAULT 0, bit_depth INTEGER DEFAULT 0,
                file_size INTEGER DEFAULT 0, file_type TEXT DEFAULT '', bpm REAL DEFAULT 0, key INTEGER DEFAULT -1,
                rating INTEGER DEFAULT 0, color INTEGER DEFAULT 0, date_added TEXT DEFAULT '', date_created TEXT DEFAULT '',
                play_count INTEGER DEFAULT 0, analyzed INTEGER DEFAULT 0, grid_locked INTEGER DEFAULT 0,
                autoload_hotcue INTEGER DEFAULT 1, gain_db REAL DEFAULT 0, artwork TEXT DEFAULT '');
            CREATE TABLE IF NOT EXISTS cues(
                id INTEGER PRIMARY KEY, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                kind INTEGER NOT NULL, slot INTEGER NOT NULL, start REAL NOT NULL, end_ REAL,
                color INTEGER DEFAULT 0, comment TEXT DEFAULT '', active_loop INTEGER DEFAULT 0);
            CREATE INDEX IF NOT EXISTS cues_track ON cues(track_id);
            CREATE TABLE IF NOT EXISTS grids(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS waves(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, data BLOB NOT NULL, preview BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS playlists(id INTEGER PRIMARY KEY, parent INTEGER NOT NULL, name TEXT NOT NULL,
                kind INTEGER NOT NULL, seq INTEGER NOT NULL, smart TEXT);
            CREATE TABLE IF NOT EXISTS playlist_tracks(playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, seq INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS pt_pl ON playlist_tracks(playlist_id, seq);
            CREATE TABLE IF NOT EXISTS mytags(id INTEGER PRIMARY KEY, parent INTEGER NOT NULL, name TEXT NOT NULL, seq INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS track_tags(track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                tag_id INTEGER NOT NULL REFERENCES mytags(id) ON DELETE CASCADE, PRIMARY KEY(track_id, tag_id));
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        // Columns added after the first release.
        let has_raw: bool = conn.prepare("SELECT raw_samples FROM tracks LIMIT 0").is_ok();
        if !has_raw {
            conn.execute_batch("ALTER TABLE tracks ADD COLUMN raw_samples INTEGER DEFAULT 0;")?;
        }
        let db = Db { conn };
        db.seed_mytags()?;
        Ok(db)
    }

    fn seed_mytags(&self) -> Result<()> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM mytags", [], |r| r.get(0))?;
        if n > 0 {
            return Ok(());
        }
        let cats: [(&str, &[&str]); 4] = [
            ("Genre", &["Acid House", "Deep House", "Techno", "Nu Disco", "Electro House", "Bass Music", "Trap"]),
            ("Components", &["Synth", "Vocal", "Beat", "Sub Bass", "Percussion", "Piano", "Dark", "Upper"]),
            ("Situation", &["Main Floor", "Second Floor", "Lounge", "Mid Night", "Morning", "Build up", "Peak Time", "Build down"]),
            ("Untitled Column", &["My Comment"]),
        ];
        for (ci, (c, tags)) in cats.iter().enumerate() {
            self.conn.execute("INSERT INTO mytags(parent,name,seq) VALUES(0,?1,?2)", params![c, ci as i64])?;
            let cid = self.conn.last_insert_rowid();
            for (ti, t) in tags.iter().enumerate() {
                self.conn.execute("INSERT INTO mytags(parent,name,seq) VALUES(?1,?2,?3)", params![cid, t, ti as i64])?;
            }
        }
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional().ok().flatten()
    }
    pub fn set_setting(&self, key: &str, value: &str) {
        let _ = self.conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)", params![key, value]);
    }

    fn row_to_track(r: &rusqlite::Row) -> rusqlite::Result<Track> {
        Ok(Track {
            id: r.get(0)?,
            path: r.get(1)?,
            title: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            album_artist: r.get(5)?,
            genre: r.get(6)?,
            label: r.get(7)?,
            remixer: r.get(8)?,
            original_artist: r.get(9)?,
            composer: r.get(10)?,
            lyricist: r.get(11)?,
            mix_name: r.get(12)?,
            comment: r.get(13)?,
            grouping: r.get(14)?,
            message: r.get(15)?,
            year: r.get(16)?,
            track_no: r.get(17)?,
            disc_no: r.get(18)?,
            release_date: r.get(19)?,
            duration: r.get(20)?,
            sample_rate: r.get(21)?,
            bitrate: r.get(22)?,
            bit_depth: r.get(23)?,
            file_size: r.get::<_, i64>(24)? as u64,
            file_type: r.get(25)?,
            bpm: r.get(26)?,
            key: r.get(27)?,
            rating: r.get(28)?,
            color: r.get(29)?,
            date_added: r.get(30)?,
            date_created: r.get(31)?,
            play_count: r.get(32)?,
            analyzed: r.get(33)?,
            grid_locked: r.get::<_, i64>(34)? != 0,
            autoload_hotcue: r.get::<_, i64>(35)? != 0,
            gain_db: r.get(36)?,
            artwork: r.get(37)?,
            raw_samples: r.get::<_, i64>(38)? as u64,
            missing: false,
        })
    }

    pub fn all_tracks(&self) -> Result<Vec<Track>> {
        let mut st = self.conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks ORDER BY id"))?;
        let v = st.query_map([], Self::row_to_track)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn track_by_path(&self, path: &str) -> Option<i64> {
        self.conn.query_row("SELECT id FROM tracks WHERE path=?1", [path], |r| r.get(0)).optional().ok().flatten()
    }

    /// Insert a track; returns its new id.
    pub fn insert_track(&self, t: &Track) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO tracks(path,title,artist,album,album_artist,genre,label,remixer,original_artist,composer,lyricist,mix_name,comment,grouping,message,year,track_no,disc_no,release_date,duration,sample_rate,bitrate,bit_depth,file_size,file_type,bpm,key,rating,color,date_added,date_created,play_count,analyzed,grid_locked,autoload_hotcue,gain_db,artwork,raw_samples)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38)",
            params![
                t.path, t.title, t.artist, t.album, t.album_artist, t.genre, t.label, t.remixer, t.original_artist, t.composer,
                t.lyricist, t.mix_name, t.comment, t.grouping, t.message, t.year, t.track_no, t.disc_no, t.release_date,
                t.duration, t.sample_rate, t.bitrate, t.bit_depth, t.file_size as i64, t.file_type, t.bpm, t.key, t.rating,
                t.color, t.date_added, t.date_created, t.play_count, t.analyzed, t.grid_locked as i64,
                t.autoload_hotcue as i64, t.gain_db, t.artwork, t.raw_samples as i64
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_track(&self, t: &Track) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET title=?2,artist=?3,album=?4,album_artist=?5,genre=?6,label=?7,remixer=?8,original_artist=?9,composer=?10,lyricist=?11,mix_name=?12,comment=?13,grouping=?14,message=?15,year=?16,track_no=?17,disc_no=?18,release_date=?19,duration=?20,sample_rate=?21,bitrate=?22,bit_depth=?23,file_size=?24,file_type=?25,bpm=?26,key=?27,rating=?28,color=?29,date_added=?30,date_created=?31,play_count=?32,analyzed=?33,grid_locked=?34,autoload_hotcue=?35,gain_db=?36,artwork=?37,path=?38,raw_samples=?39 WHERE id=?1",
            params![
                t.id, t.title, t.artist, t.album, t.album_artist, t.genre, t.label, t.remixer, t.original_artist, t.composer,
                t.lyricist, t.mix_name, t.comment, t.grouping, t.message, t.year, t.track_no, t.disc_no, t.release_date,
                t.duration, t.sample_rate, t.bitrate, t.bit_depth, t.file_size as i64, t.file_type, t.bpm, t.key, t.rating,
                t.color, t.date_added, t.date_created, t.play_count, t.analyzed, t.grid_locked as i64,
                t.autoload_hotcue as i64, t.gain_db, t.artwork, t.path, t.raw_samples as i64
            ],
        )?;
        Ok(())
    }

    pub fn delete_track(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM tracks WHERE id=?1", [id])?;
        Ok(())
    }

    // ---- cues
    pub fn cues(&self, track: i64) -> Result<Vec<Cue>> {
        let mut st = self.conn.prepare("SELECT id,track_id,kind,slot,start,end_,color,comment,active_loop FROM cues WHERE track_id=?1 ORDER BY kind, slot, start")?;
        let v = st
            .query_map([track], |r| {
                Ok(Cue {
                    id: r.get(0)?,
                    track_id: r.get(1)?,
                    kind: if r.get::<_, i64>(2)? == 1 { CueKind::Hot } else { CueKind::Memory },
                    slot: r.get(3)?,
                    start: r.get(4)?,
                    end: r.get(5)?,
                    color: r.get(6)?,
                    comment: r.get(7)?,
                    active_loop: r.get::<_, i64>(8)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    pub fn tracks_with_cues(&self) -> Result<std::collections::HashMap<i64, (usize, usize)>> {
        let mut st = self.conn.prepare("SELECT track_id, SUM(kind=1), SUM(kind=0) FROM cues GROUP BY track_id")?;
        let mut m = std::collections::HashMap::new();
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        for r in rows.flatten() {
            m.insert(r.0, (r.1 as usize, r.2 as usize));
        }
        Ok(m)
    }
    /// Hot cue (start, slot, colour) per track, for the track list previews.
    pub fn hot_positions(&self) -> Result<std::collections::HashMap<i64, Vec<(f64, i32, u8)>>> {
        let mut st = self.conn.prepare("SELECT track_id, start, slot, color FROM cues WHERE kind=1 ORDER BY track_id, slot")?;
        let mut m: std::collections::HashMap<i64, Vec<(f64, i32, u8)>> = std::collections::HashMap::new();
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i32>(2)?, r.get::<_, u8>(3)?)))?;
        for r in rows.flatten() {
            m.entry(r.0).or_default().push((r.1, r.2, r.3));
        }
        Ok(m)
    }
    pub fn save_cue(&self, c: &Cue) -> Result<i64> {
        let kind = if c.kind == CueKind::Hot { 1 } else { 0 };
        if c.id > 0 {
            self.conn.execute(
                "UPDATE cues SET kind=?2,slot=?3,start=?4,end_=?5,color=?6,comment=?7,active_loop=?8 WHERE id=?1",
                params![c.id, kind, c.slot, c.start, c.end, c.color, c.comment, c.active_loop as i64],
            )?;
            Ok(c.id)
        } else {
            self.conn.execute(
                "INSERT INTO cues(track_id,kind,slot,start,end_,color,comment,active_loop) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![c.track_id, kind, c.slot, c.start, c.end, c.color, c.comment, c.active_loop as i64],
            )?;
            Ok(self.conn.last_insert_rowid())
        }
    }
    pub fn delete_cue(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM cues WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn replace_cues(&self, track: i64, cues: &[Cue]) -> Result<()> {
        self.conn.execute("DELETE FROM cues WHERE track_id=?1", [track])?;
        for c in cues {
            let mut c = c.clone();
            c.id = 0;
            c.track_id = track;
            self.save_cue(&c)?;
        }
        Ok(())
    }

    // ---- grid & waveform
    pub fn grid(&self, track: i64) -> Option<BeatGrid> {
        let s: Option<String> = self.conn.query_row("SELECT json FROM grids WHERE track_id=?1", [track], |r| r.get(0)).optional().ok().flatten();
        s.and_then(|s| serde_json::from_str(&s).ok())
    }
    pub fn save_grid(&self, track: i64, g: &BeatGrid) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO grids(track_id,json) VALUES(?1,?2)", params![track, serde_json::to_string(g)?])?;
        Ok(())
    }
    pub fn wave(&self, track: i64) -> Option<Waveform> {
        let b: Option<Vec<u8>> = self.conn.query_row("SELECT data FROM waves WHERE track_id=?1", [track], |r| r.get(0)).optional().ok().flatten();
        b.map(|b| Waveform::from_bytes(&b))
    }
    pub fn save_wave(&self, track: i64, w: &Waveform, preview: &[u8]) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO waves(track_id,data,preview) VALUES(?1,?2,?3)", params![track, w.to_bytes(), preview])?;
        Ok(())
    }
    pub fn previews(&self) -> Result<std::collections::HashMap<i64, Vec<u8>>> {
        let mut st = self.conn.prepare("SELECT track_id, preview FROM waves")?;
        let mut m = std::collections::HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?.flatten() {
            m.insert(r.0, r.1);
        }
        Ok(m)
    }

    // ---- playlists
    pub fn playlists(&self) -> Result<Vec<PlaylistNode>> {
        let mut st = self.conn.prepare("SELECT id,parent,name,kind,seq,smart FROM playlists ORDER BY parent, seq, id")?;
        let v = st
            .query_map([], |r| {
                let kind: i64 = r.get(3)?;
                let smart: Option<String> = r.get(5)?;
                Ok(PlaylistNode {
                    id: r.get(0)?,
                    parent: r.get(1)?,
                    name: r.get(2)?,
                    kind: match kind {
                        1 => NodeKind::Folder,
                        2 => NodeKind::Smart,
                        _ => NodeKind::Playlist,
                    },
                    seq: r.get(4)?,
                    smart: smart.and_then(|s| serde_json::from_str::<SmartRule>(&s).ok()),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    pub fn create_node(&self, parent: i64, name: &str, kind: NodeKind, smart: Option<&SmartRule>) -> Result<i64> {
        let seq: i64 = self.conn.query_row("SELECT COALESCE(MAX(seq)+1,0) FROM playlists WHERE parent=?1", [parent], |r| r.get(0))?;
        let k = match kind {
            NodeKind::Playlist => 0,
            NodeKind::Folder => 1,
            NodeKind::Smart => 2,
        };
        let sj = smart.map(|s| serde_json::to_string(s).unwrap_or_default());
        self.conn.execute("INSERT INTO playlists(parent,name,kind,seq,smart) VALUES(?1,?2,?3,?4,?5)", params![parent, name, k, seq, sj])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn rename_node(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE playlists SET name=?2 WHERE id=?1", params![id, name])?;
        Ok(())
    }
    pub fn move_node(&self, id: i64, parent: i64, seq: i64) -> Result<()> {
        self.conn.execute("UPDATE playlists SET seq=seq+1 WHERE parent=?1 AND seq>=?2", params![parent, seq])?;
        self.conn.execute("UPDATE playlists SET parent=?2, seq=?3 WHERE id=?1", params![id, parent, seq])?;
        Ok(())
    }
    pub fn set_smart(&self, id: i64, s: &SmartRule) -> Result<()> {
        self.conn.execute("UPDATE playlists SET smart=?2 WHERE id=?1", params![id, serde_json::to_string(s)?])?;
        Ok(())
    }
    pub fn delete_node(&self, id: i64) -> Result<()> {
        let kids: Vec<i64> = {
            let mut st = self.conn.prepare("SELECT id FROM playlists WHERE parent=?1")?;
            let v = st.query_map([id], |r| r.get(0))?.flatten().collect();
            v
        };
        for k in kids {
            self.delete_node(k)?;
        }
        self.conn.execute("DELETE FROM playlists WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn playlist_tracks(&self, id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id=?1 ORDER BY seq")?;
        let v = st.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }
    pub fn set_playlist_tracks(&self, id: i64, tracks: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM playlist_tracks WHERE playlist_id=?1", [id])?;
        {
            let mut st = tx.prepare("INSERT INTO playlist_tracks(playlist_id,track_id,seq) VALUES(?1,?2,?3)")?;
            for (i, t) in tracks.iter().enumerate() {
                st.execute(params![id, t, i as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn playlist_counts(&self) -> Result<std::collections::HashMap<i64, usize>> {
        let mut st = self.conn.prepare("SELECT playlist_id, COUNT(*) FROM playlist_tracks GROUP BY playlist_id")?;
        let mut m = std::collections::HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.flatten() {
            m.insert(r.0, r.1 as usize);
        }
        Ok(m)
    }
    pub fn playlists_containing(&self, track: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT DISTINCT playlist_id FROM playlist_tracks WHERE track_id=?1")?;
        let v = st.query_map([track], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }

    // ---- my tags
    pub fn mytags(&self) -> Result<Vec<MyTag>> {
        let mut st = self.conn.prepare("SELECT id,parent,name,seq FROM mytags ORDER BY parent, seq, id")?;
        let v = st
            .query_map([], |r| Ok(MyTag { id: r.get(0)?, parent: r.get(1)?, name: r.get(2)?, seq: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    pub fn add_mytag(&self, parent: i64, name: &str) -> Result<i64> {
        let seq: i64 = self.conn.query_row("SELECT COALESCE(MAX(seq)+1,0) FROM mytags WHERE parent=?1", [parent], |r| r.get(0))?;
        self.conn.execute("INSERT INTO mytags(parent,name,seq) VALUES(?1,?2,?3)", params![parent, name, seq])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn rename_mytag(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE mytags SET name=?2 WHERE id=?1", params![id, name])?;
        Ok(())
    }
    pub fn delete_mytag(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM mytags WHERE parent=?1", [id])?;
        self.conn.execute("DELETE FROM mytags WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn track_tags(&self) -> Result<std::collections::HashMap<i64, std::collections::HashSet<i64>>> {
        let mut st = self.conn.prepare("SELECT track_id, tag_id FROM track_tags")?;
        let mut m: std::collections::HashMap<i64, std::collections::HashSet<i64>> = std::collections::HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.flatten() {
            m.entry(r.0).or_default().insert(r.1);
        }
        Ok(m)
    }
    pub fn set_track_tag(&self, track: i64, tag: i64, on: bool) -> Result<()> {
        if on {
            self.conn.execute("INSERT OR IGNORE INTO track_tags(track_id,tag_id) VALUES(?1,?2)", params![track, tag])?;
        } else {
            self.conn.execute("DELETE FROM track_tags WHERE track_id=?1 AND tag_id=?2", params![track, tag])?;
        }
        Ok(())
    }
}
