# Sync the desktop rekordbox library

The two arrows at the bottom left of the collection browser and **File → Review rekordbox library sync…** open a review of a **one-way import into Recordbox**. Check the paths before continuing. **Source** is AlphaTheta rekordbox’s desktop `master.db`. **Destination** is the `library.db` Recordbox actually has open. The Recordbox footer also shows the active library. **File → Show Library Folder** opens its directory. You can open the review and close it after checking both paths. **Import into Recordbox** starts the manual import.

| What Recordbox touches | What happens |
| --- | --- |
| Desktop `master.db` and its write-ahead log | Recordbox takes a temporary private snapshot. A nonempty rollback journal requires closing rekordbox and retrying. It opens the copy read-only with SQLCipher and checks database integrity and the expected schema before importing. |
| The source’s `share` folder and referenced audio | Recordbox can read beat-grid files, check artwork and file paths, and read audio tags or embedded artwork to fill missing details. Only source tracks whose file path resolves to a file on this computer qualify. |
| Recordbox’s data folder | The snapshot is staged here. Confirming sync commits tracks, cues, grids, My Tags, imported playlists and sync state together in Recordbox’s collection. A database write failure rolls back the merge. Reading embedded artwork can also create a local artwork cache entry. |
| USB devices and cloud accounts | Device exports have their own Sync Manager. Desktop library sync makes no provider API requests. An on-demand file can still be hydrated by the operating system when an application reads it. |

## What happens to existing work?

- **Tracks** are matched by their recorded file path. Recordbox remembers the source content ID and a fingerprint of source changes. New local tracks are added. A changed source track can replace local metadata, including a rating. An unchanged source generally preserves local edits, while absent cues or a missing grid can be filled in. A path previously removed from Recordbox is skipped.
- **Cues and grids** on a changed track can be replaced by source values. An empty source cue list preserves existing cues. A locked local grid keeps its tempo and timing. Changes to source My Tag assignments can update tags in Recordbox.
- **Playlists and histories** are rebuilt in the imported `rekordbox` folder on each sync. Local edits inside that mirror can be lost. Create your own playlists outside it. Unsupported intelligent-playlist rules are skipped.
- **Analysis** is a separate preference. Adding tracks can queue automatic analysis of locally available tracks that still need it.

For a first test, check the displayed destination and use **File → Library → Backup Library** if it contains work you want to keep. Startup sync is opt-in. To enable it, use **Preferences → Advanced → Others → Import the installed rekordbox library at startup**. Close AlphaTheta rekordbox before taking a snapshot of a busy library. [Desktop reader](../app/src/rbimport.rs) · [Collection merge](../app/src/rbimport/merge.rs) · [Recordbox backup and restore](../app/src/backup.rs)

To try a full import in a fresh Recordbox collection on Windows, open a new PowerShell window in the extracted application folder and run the following. The destination shown in the review should be under the new test directory. Close that PowerShell window after the test to discard the environment override.

```powershell
$env:PREPDECK_DATA = Join-Path $env:TEMP ("Recordbox-test-" + [guid]::NewGuid())
.\recordbox.exe
```

On Linux, run `PREPDECK_DATA="$(mktemp -d)" ./recordbox`. Both examples keep the test collection in the temporary folder after Recordbox exits.

**Device Sync Manager** is a separate action for a selected drive. It can write classic device catalogs, audio, artwork and analysis files. The review lists previously managed playlists selected for removal. Depending on the settings, sync can replace cues on the device or remove tracks and corresponding files. Check the target drive, cue option and deletion settings before confirming a device operation. [Device Sync Manager](../app/src/device_ext.rs) · [USB export](../app/src/export/usb.rs)

## How the formats compare with Library Workbench

| Source or destination | Recordbox | Library Workbench |
| --- | --- | --- |
| Desktop v6/v7 `master.db` | Imports supported records from a private snapshot into a local DJ collection. | Captures the source for inspection, including missing and cloud file references. Its snapshot reader handles WAL and rollback-journal evidence. |
| Desktop v5 and earlier `.edb` | Use a collection XML export for interchange. | Detects legacy database files and inventories them. Use XML for track-level analysis. |
| Collection XML | Imports and exports tracks, playlist links, supported cues and tempo information. | Inspects XML alone, or pairs it with a native backup for review and edited export. |
| Backup ZIP | Creates and restores Recordbox’s own backups, identified by their manifest and `library.db`. | Opens AlphaTheta’s Backup Library ZIP and can prepare an edited native backup copy. |
| Classic USB Device Library | Reads and writes supported DeviceSQL records in `export.pdb` and `exportExt.pdb` and handles associated audio and ANLZ files. | Inspects catalog headers, settings and selected ANLZ evidence. Device record decoding is outside its inspector’s scope. |
| OneLibrary / Device Library Plus | Detects `exportLibrary.db`. Classic exports on a mixed drive update only the classic catalog. | Recognizes the separate OneLibrary catalog by filename. Its records remain undecoded in both applications. |

Workbench sources for this comparison were checked at commit `b10e1a4`. [Desktop analyzer](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/analyzer.py) · [Native backup exporter](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/native_export.py) · [Device-format inspector](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/asset_formats.py)

Generated desktop-sync tests read committed rows from an encrypted WAL, verify source-file preservation, reject overlapping snapshot paths and exercise repeated imports, local edits, empty source cues, locked grids and preservation of a user folder that reuses a deleted mirror ID. A forced database-write error exercises rollback of the destination tracks, mirror and sync state. Other tests cover encrypted SQLite/WAL reads and classic USB exports. The installed-library check runs separately with a local installation. Hardware acceptance on a particular player, or genuine-library compatibility with a specific rekordbox build, needs separate testing. Workbench reports v7.2.19 evidence and lists genuine v6 fixture validation as pending. [Recordbox database tests](../app/src/db/sqlcipher_tests.rs) · [USB regression tests](../app/src/export/usb_fix_test.rs) · [Workbench compatibility](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/COMPATIBILITY.md) · [Workbench format evidence](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/FORMAT-SOURCES.md)
