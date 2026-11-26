//! Synthetic compatibility checks for the bundled SQLite/SQLCipher engine.
use rusqlite::{backup::Backup, Connection, OpenFlags};
use std::time::Duration;

const TEST_KEY: &str = "generated test data, never a user credential";

fn key(conn: &Connection, value: &str) -> rusqlite::Result<()> {
    conn.pragma_update(None, "key", value)?;
    conn.pragma_update(None, "cipher_compatibility", 4)
}

#[test]
fn bundled_engine_contains_the_sqlite_wal_fix_and_sqlcipher() -> rusqlite::Result<()> {
    // RUSTSEC-2026-0459: the bundled-sqlcipher feature needs its own fixed engine,
    // even when cargo-audit considers the libsqlite3-sys package version patched.
    assert!(rusqlite::version_number() >= 3_054_000);
    let conn = Connection::open_in_memory()?;
    let cipher_version: String = conn.query_row("PRAGMA cipher_version", [], |row| row.get(0))?;
    assert!(!cipher_version.is_empty());
    Ok(())
}

#[test]
fn encrypted_wal_reads_backup_reopen_and_wrong_key() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("encrypted.db");
    let writer = Connection::open(&path)?;
    key(&writer, TEST_KEY)?;
    writer.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
         CREATE TABLE tracks(id INTEGER PRIMARY KEY, title TEXT NOT NULL);
         INSERT INTO tracks VALUES(1, 'Synthetic \u{266b} track');",
    )?;
    let journal: String = writer.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    assert_eq!(journal, "wal");
    assert!(std::fs::metadata(dir.path().join("encrypted.db-wal"))?.len() > 0);
    assert_ne!(&std::fs::read(&path)?[..16], b"SQLite format 3\0");

    // A second connection must see committed rows that still reside in the WAL.
    let reader = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    key(&reader, TEST_KEY)?;
    let title: String =
        reader.query_row("SELECT title FROM tracks WHERE id=1", [], |row| row.get(0))?;
    assert_eq!(title, "Synthetic \u{266b} track");
    assert!(Connection::open(&path)?
        .query_row("SELECT title FROM tracks", [], |row| row
            .get::<_, String>(0))
        .is_err());
    let wrong_key = Connection::open(&path)?;
    key(&wrong_key, "a different generated key")?;
    assert!(wrong_key
        .query_row("SELECT title FROM tracks", [], |row| row
            .get::<_, String>(0))
        .is_err());
    drop(wrong_key);

    let backup_path = dir.path().join("encrypted-backup.db");
    let mut destination = Connection::open(&backup_path)?;
    key(&destination, TEST_KEY)?;
    {
        let backup = Backup::new(&reader, &mut destination)?;
        backup.run_to_completion(5, Duration::from_millis(1), None)?;
    }
    drop(destination);
    drop(reader);
    writer.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(writer);
    for file in [&path, &backup_path] {
        let reopened = Connection::open(file)?;
        key(&reopened, TEST_KEY)?;
        assert_eq!(
            reopened.query_row("SELECT title FROM tracks WHERE id=1", [], |row| row
                .get::<_, String>(0))?,
            title
        );
        assert_eq!(
            reopened.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?,
            "ok"
        );
    }
    Ok(())
}
