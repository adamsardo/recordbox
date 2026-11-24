# Review collection cleanup

In the sidebar, right-click **All** under **Collection** and choose **Review collection cleanup…**. On macOS the command is also in the **File** menu. Begin with the drive summary. A DJ who used two computers might see hundreds of entries under `D:` and `/Volumes/DJ-SSD`. Those counts help identify which disk to reconnect.

The report classifies recorded paths as Windows, macOS or unclassified. A drive letter, UNC share or macOS volume is a recorded location. The computer running the review can be different from the computer that recorded it. On Linux the entire operation is review-only, including when a Windows partition is mounted and readable.

| Evidence | How to interpret it |
| --- | --- |
| Known cloud or streaming location, online-only placeholder, or redirected folder | Protected. An audio file may still be in the provider’s account. Check the provider or its sync client before considering manual cleanup. |
| External disk, network share, another operating system’s path, unavailable parent or access error | Protected. Reconnect the disk or review from the computer that owns the path. |
| File absent from a reachable parent in the current user’s local Music folder | A possible dead reference. Only a supported host can confirm absence. The catalog record must also pass the origin and preparation checks below. |
| Two records with the same filename, title, duration or file size | These clues can also describe different edits of a track. They are insufficient to authorize a move. |
| Two eligible local references proven to point to the same file, or to files with identical verified contents | Review the proposed keeper and both paths. Cleanup moves the selected redundant catalog reference. Audio files remain on disk. |

### What stays protected

Recordbox looks for evidence that the reference was added by direct file import. Older records without that evidence, and records associated with other libraries, stay available for inspection. Preparation and library relationships such as cues, beat grids, ratings, playlists and play history also protect a record. This matters even when two files contain identical audio.

Cloud software can sync folders with ordinary names. Confirm that the Music folder is always local and outside any custom sync setup before requesting a content comparison or applying a selection. Start with the default review if you are unsure. A content comparison reads eligible files of matching size, up to 8 GiB per scan. Files left unverified by the scan cannot qualify based on their contents.

### Review, apply and recover

1. Review the drive counts and protection reasons. If a disk or provider is missing, use **Relocate** or reopen the collection on its original computer.
2. Select individual candidates. The selection starts empty. Review the exact count, paths and duplicate keepers. Each operation accepts at most 25 references.
3. Confirm you are reviewing the computer and local Music folder used for these imports and type the displayed confirmation. Recordbox rechecks the selection and creates a verified backup of its library before moving any selected reference. A stale selection or failed backup stops the operation.
4. To undo a move within 30 days, right-click **All** under **Collection** and choose **Restore Removed Tracks…**. For a full rollback, open **Preferences → Advanced → Database → Restore Library** and select the backup shown by the cleanup operation. Restoring a complete backup also rolls back subsequent library edits.

Cleanup changes the current Recordbox collection. AlphaTheta Rekordbox’s native library, USB exports, provider accounts and audio files remain outside this operation.
