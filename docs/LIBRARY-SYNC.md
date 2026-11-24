# Sync the desktop rekordbox library

Open the two arrows at the bottom left of the collection browser or choose **File → Review rekordbox library sync…**. Check the paths and choose a direction.

| Direction | Source | Destination |
| --- | --- | --- |
| **rekordbox → Recordbox** | A snapshot of the installed AlphaTheta desktop `master.db` | The active Recordbox `library.db` |
| **Recordbox → rekordbox** | Selected edits in the active Recordbox collection | The installed AlphaTheta desktop library and any analysis files included in the review |

Opening the chooser lets you inspect both paths. On Windows the expected AlphaTheta database is `%APPDATA%\Pioneer\rekordbox\master.db`. On macOS it is `~/Library/Pioneer/rekordbox/master.db`. The Recordbox footer identifies its own active library. **File → Show Library Folder** opens its directory.

## Import into Recordbox

Choose **rekordbox → Recordbox**, then **Import into Recordbox** to start the import.

| What Recordbox touches | What happens |
| --- | --- |
| Desktop `master.db` and its write-ahead log | Recordbox takes a temporary private snapshot. A nonempty rollback journal requires closing rekordbox and retrying. It opens the copy read-only with SQLCipher and checks database integrity and the expected schema before importing. |
| The source’s `share` folder and referenced audio | Recordbox can read beat-grid files, check artwork and file paths, and read audio tags or embedded artwork to fill missing details. Only source tracks whose file path resolves to a file on this computer qualify. |
| Recordbox’s data folder | The snapshot is staged here. Confirming sync commits tracks, cues, grids, My Tags, imported playlists and sync state together in Recordbox’s collection. A database write failure rolls back the merge. Reading embedded artwork can also create a local artwork cache entry. |
| USB devices and cloud accounts | Device exports have their own Sync Manager. Desktop library sync makes no provider API requests. An on-demand file can still be hydrated by the operating system when an application reads it. |

### What happens to existing Recordbox work?

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

On Linux, run `PREPDECK_DATA="$(mktemp -d)" ./recordbox`. Both examples keep the test Recordbox collection in the temporary folder after Recordbox exits. **The override changes the Recordbox data folder only.** For a write-back test, use an operating-system account with a disposable AlphaTheta desktop library and check the `master.db` path in the chooser.

## Write reviewed changes to rekordbox

The write-back pilot targets desktop libraries with a recognized native schema. Start with a disposable AlphaTheta profile on the rekordbox version you use. Native application acceptance requires reopening that library in AlphaTheta rekordbox and checking the selected tracks there.

1. Import from that desktop library to establish a baseline, then edit the tracks in Recordbox.
2. Choose **Recordbox → rekordbox** and preview the changes. The comparison needs an unambiguous track in the same native library and a saved baseline. For example, if both libraries had two stars at import and you set four in Recordbox, the proposed rating is four. If rekordbox now has five, the conflict is skipped. If both already show four, there is nothing to apply.
3. Select eligible edits, choose **Review selected changes**, and check the target path and count. Quit rekordbox and its background agent before confirming. Use a local desktop library for the pilot. Cloud and network synchronization need separate acceptance testing.
4. Choose **Back up and apply** for the displayed number of changes. Recordbox checks the native library again under a SQLite writer lock and saves a verified encrypted recovery copy that includes committed changes from the write-ahead log. The result identifies the recovery folder under `native-recovery` beside your active Recordbox `library.db`.

Even a one-rating write-back needs enough free space for a complete database recovery copy. Grid edits can also touch analysis files under the native library’s `share` folder. Keep the entire recovery folder so the affected files and database can be recovered together.

If the application stops during a grid write-back, close desktop rekordbox and its agents, reopen the arrows, and select **Write back → Review recovery… → Confirm recovery**. Recovery checks the database state and affected file hashes. A verified committed change is kept. If the database did not commit, recovery can restore the corresponding original analysis files. If any recorded state has since changed unexpectedly, keep the recovery folder and investigate before attempting another sync. The small `.recordbox-writeback.lock` file can remain beside `master.db` between operations. The operating system releases its lock when the operation ends.

| Field | What the pilot can review |
| --- | --- |
| Rating | The native zero-to-five-star value for a matched track. |
| BPM | The track's numeric BPM. If that number tracks a constant-tempo native grid, review and select the corresponding grid change together with the BPM change. |
| Key | A change to an existing, unambiguous native key entry. An unknown key or ambiguous mapping is skipped. |
| Cues | Change comments and colors, reassign an existing hot cue to another pad at the same time, or remove an existing cue when its native mirrors can stay consistent. Referenced hot-cue banks and unsupported sidecar cue data can block an edit. Creating cues and changing their start or loop times require native seek information that Recordbox does not generate. |
| Beat grid | Shift a supported grid in whole milliseconds or change which beat starts the bar. A constant-tempo grid can also be retimed when its native analysis files supply the timing evidence and have no dependent cue positions. Edits that add or remove tempo segments, or depend on unrecognized analysis files, are skipped. |

The pilot draws on Library Workbench’s native backup writer for per-row update counters, the library-wide change counter and the relationship between cue rows and the serialized cue list. Workbench performs those edits in an exported backup copy. A Recordbox write-back targets the installed library identified in its own review. [Workbench native writer](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/native_export.py) · [Recordbox field checks](../app/src/rbimport/writeback/mod.rs) · [Cue edits](../app/src/rbimport/writeback/cues.rs) · [Grid edits](../app/src/rbimport/writeback/grid.rs)

## Locate separated STEMS

After Recordbox separates a track, it caches the four stereo parts in a `.stems` file. `PDSTEMS1` is the first eight bytes of that file. Open **Preferences → Extensions → STEMS** and copy the **Recordbox STEMS cache** path. The built-in manual also shows this directory in its **Stems** chapter.

| System | Default Recordbox STEMS folder |
| --- | --- |
| Windows | `%APPDATA%\Recordbox\stems` |
| macOS | `~/Library/Application Support/Recordbox/stems` |
| Linux | `$XDG_DATA_HOME/Recordbox/stems`, normally `~/.local/share/Recordbox/stems` |

Setting `PREPDECK_DATA` puts the cache in the `stems` subfolder of that directory. Filenames have the form `<24 hex characters>-<model>.stems`. Recordbox hashes the audio path, file size and modification time in seconds for the prefix. The file header stores the format and frame count. Library Workbench can inspect its structure in **Library lab → Formats & STEMS**. [Recordbox cache code](../app/src/stems/mod.rs) · [Workbench format inspector](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/FORMATS-AND-STEMS.md)

AlphaTheta’s own per-track STEMS storage and a compatible import format remain unverified. AlphaTheta documents `.rbsc` as a captured DRUMS sample. A useful native sample for whole-track separation starts with a short self-created track in a disposable AlphaTheta library. Record the rekordbox version, OS, mode and STEMS setting, inventory the library and cache before and after separation, then check whether any generated file persists after quitting and reopening rekordbox. Ratings, cues, BPM, key and beat-grid write-back can be tested independently of STEMS. [AlphaTheta sample capture in Workbench](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/DISCOVERY-ASSETS-HISTORY.md) · [Workbench STEMS evidence](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/STEMS-WORKFLOWS.md)

## Sync a USB drive

**Device Sync Manager** is a separate action for a selected drive. It can write classic device catalogs, audio, artwork and analysis files. The review lists previously managed playlists selected for removal. Depending on the settings, sync can replace cues on the device or remove tracks and corresponding files. Check the target drive, cue option and deletion settings before confirming a device operation. [Device Sync Manager](../app/src/device_ext.rs) · [USB export](../app/src/export/usb.rs)

## How the formats compare with Library Workbench

| Source or destination | Recordbox | Library Workbench |
| --- | --- | --- |
| Desktop v6/v7 `master.db` | Imports supported records from a private snapshot. The manual write-back pilot checks the native schema, identity and baseline before offering edits. | Captures the source for inspection, including missing and cloud file references. Its snapshot reader handles WAL and rollback-journal evidence. |
| Desktop v5 and earlier `.edb` | Use a collection XML export for interchange. | Detects legacy database files and inventories them. Use XML for track-level analysis. |
| Collection XML | Imports and exports tracks, playlist links, supported cues and tempo information. | Inspects XML alone, or pairs it with a native backup for review and edited export. |
| Backup ZIP | Creates and restores Recordbox’s own backups, identified by their manifest and `library.db`. | Opens AlphaTheta’s Backup Library ZIP and can prepare an edited native backup copy. |
| Classic USB Device Library | Reads and writes supported DeviceSQL records in `export.pdb` and `exportExt.pdb` and handles associated audio and ANLZ files. | Inspects catalog headers, settings and selected ANLZ evidence. Device record decoding is outside its inspector’s scope. |
| OneLibrary / Device Library Plus | Detects `exportLibrary.db`. Classic exports on a mixed drive update only the classic catalog. | Recognizes the separate OneLibrary catalog by filename. Its records remain undecoded in both applications. |

Workbench sources for this comparison were checked at commit `b10e1a4`. [Desktop analyzer](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/analyzer.py) · [Native backup exporter](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/native_export.py) · [Device-format inspector](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/asset_formats.py)

Generated desktop-sync tests read committed rows from an encrypted WAL, verify source-file preservation, reject overlapping snapshot paths and exercise repeated imports, local edits, empty source cues, locked grids and preservation of a user folder that reuses a deleted mirror ID. A forced database-write error exercises rollback of the destination tracks, mirror and sync state. Other tests cover encrypted SQLite/WAL reads and classic USB exports. The installed-library check runs separately with a local installation. Hardware acceptance on a particular player, or genuine-library compatibility with a specific rekordbox build, needs separate testing. Workbench reports v7.2.19 evidence and lists genuine v6 fixture validation as pending. [Recordbox database tests](../app/src/db/sqlcipher_tests.rs) · [USB regression tests](../app/src/export/usb_fix_test.rs) · [Workbench compatibility](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/COMPATIBILITY.md) · [Workbench format evidence](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/FORMAT-SOURCES.md)
