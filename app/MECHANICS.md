# PrepDeck mechanics sheet

Rules, constants and formulas the app implements, with where each came from. "Estimated" means the
value was looked for (decompiler, prior research, real files) and not found, so a reasoned value is
used instead.

Sources used:
- **REA**: the earlier read-only investigation of the installed rekordbox 7.2.19 binary at
  `~/Developer/rekordbox-rea` (instruction-level traces with function addresses).
- **Fixtures**: real rekordbox-made exports kept as open-source test data (rekordcrate
  `demo_tracks` and `empty`, pyrekordbox sample export, rekordbox-pdb `one-song`), read byte by byte.
- **Skin/Resources**: `/Applications/rekordbox 7/rekordbox.app/Contents/Resources` and the
  person's screenshots in `reference/`.

## USB drive layout

| Item | Rule | Source |
|---|---|---|
| Database | `/PIONEER/rekordbox/export.pdb`, plus `exportExt.pdb` (My Tag) | literals `0x104fd0b1a`, `0x104fb1239` (REA library-formats) |
| Analysis folder | `/PIONEER/USBANLZ/P%03X/%08X/ANLZ0000.{DAT,EXT,2EX}` | REA usb-export-paths |
| Folder hash | over UTF-16 units of the media path: `h = ((h*23497 + c)*37813 + c) mod 2^32`; `n = h mod 200003`; P = bits 0,2,6,7,9,13,16 of n | REA usb-export-paths (151 emulated cases); reproduced exactly on 3 real exports (P016/0000875E, P053/0001D21F, P011/00005719) |
| Media path | `/Contents/<artist>/<album>/<file>` | REA usb-export-paths; fixtures |
| Forbidden characters | `/ \ : ? * " < > \| ~` → `_`; trailing `.` → `_` | REA usb-export-paths |
| Empty artist / album | `UnknownArtist` / `UnknownAlbum` | REA; fixture `/Contents/Loopmasters/UnknownAlbum/…` |
| Component length | artist, album ≤ 48 chars; file name stem shortened so name+ext ≤ 48 | REA usb-export-paths |
| Name collision | `stem-1.ext`, `stem-2.ext`, … | REA usb-export-paths |
| Artwork | `/PIONEER/Artwork/00001/a<id>.jpg` (80×80) and `a<id>_m.jpg` (240×240) | fixture path `/PIONEER/Artwork/00001/a1.jpg`; sizes estimated |
| Hidden `.PIONEER` variant | not used; `PIONEER` is the folder players read from USB | REA (selected by two flags, conditions unresolved) |
| Settings files (MYSETTING*.DAT, DEVSETTING.DAT) | not written (players keep their own settings) | estimated as optional |

## Device database (DeviceSQL pages)

- 4096-byte pages; page 0 header: `0, page_size, num_tables, next_unused_page, unknown(5), sequence, 0`, then 16-byte table pointers `type, empty_candidate, first_page, last_page`. (fixtures)
- Each table: index page (flags `0x64`, index header `1fff 1fff 03ec next_off page next 03ffffff 0 count 1fff`, entries `0x1ffffff8`, last 20 bytes zero), data pages (flags `0x24`), then an empty candidate page. Empty table: index `next` = empty candidate, index content next = `0x03ffffff`. (fixtures)
- Page header 0x28 bytes; packed row counts 13+11 bits; free/used sizes; data header `u5 = rows written in the batch` (fixture pattern u5+u6 = rows).
- Row groups: 16 offsets stored backwards from the page end, 36 bytes per group; group *g* base = 4096 − 36g; flags at base−4, mirror at base−2; row *i* offset at base−6−2i; rows 4-byte aligned. (fixtures + rekordcrate)
- Strings: short ASCII `((len+1)<<1)|1`; long ASCII `0x40, u16 len+4, 0`; UTF-16LE `0x90, u16 len+4, 0`; UTF-16 aligned to 4. (fixtures)
- Tables written in order 0–19: tracks, genres, artists, albums, labels, keys, colours, playlist tree, playlist entries, 9, 10, history playlists, history entries, artwork, 14, 15, columns (27 static rows), menu (22 static rows), table 18 (17 static rows), sync history (1 row: date, "1000"). Static rows copied from real exports.
- Track row: 0x5C fixed bytes (subtype 0x24, index_shift 0x20·row, bitmask 0x000c0700, sample rate, composer, size, …, tempo = BPM×100, ids, disc, play count, year, bit depth, seconds, 0x29, colour, rating 0–5, file type) + 21 string offsets: `"" lyricist "3" "3" "" message "" autoload("ON") "" "" date_added release mix "" analyze_path analyze_date comment title "" filename path`. (fixtures)
- File type codes: MP3 1, M4A 4, FLAC 5, WAV 11, AIFF 12. (rekordcrate)
- Colours table: Pink, Red, Orange, Yellow, Green, Aqua, Blue, Purple (ids 1–8). (fixtures)
- Keys table: names in classic notation, ids assigned by first use. (fixtures)
- exportExt.pdb: 9 tables; table 3 tag rows (subtype 0x0680, parent, position, id, category flag, name), table 4 tag↔track rows (`0, track, tag, 3`), table 7 one static row. (fixtures)
- Existing device libraries are read and merged; history playlists written by players are kept.

## Analysis files (ANLZ, big-endian)

| File | Sections (in order) |
|---|---|
| .DAT | PPTH, PVBR (400 zero entries then the decoded sample count, as in the person's own rekordbox files), PQTZ, PWAV (400), PWV2 (100), PCOB hot, PCOB memory |
| .EXT | PPTH, PWV3 (150/s), PCOB ×2 empty, PCO2 hot, PCO2 memory, PWV5 (150/s), PWV4 (1200×6) |
| .2EX | PPTH, PWV7 (150/s ×3), PWV6 (1200×3), PWVC |

- PMAI header 28 bytes `…00000001 00010000 00010000 00000000`. (fixtures)
- PQTZ: header `0, 0x00080000, count`; beat = `u16 beat-in-bar, u16 BPM×100, u32 ms`. (fixtures)
- PCPT (56 bytes): hot cue number (A=1), status 4 for active loop, `0x00100000`, prev/next order, type 1 point / 2 loop, `0x03e8`, start ms, end ms or `0xffffffff`. (rekordcrate)
- PCP2 (88 bytes + comment): hot cue no., type, `0x03e8`, times, memory colour id, loop size 0/0, UTF-16BE comment, hot cue colour index + RGB. (public populated record)
- Waveform value ranges matched to real files: PWAV height ≤31, PWV2 ≤15, PWV3/PWV5 height ≤31, PWV6 bands ≤43, PWV7 ≤127. (fixture statistics)
- Waveform values come from linear models of peak and RMS per band, fitted against rekordbox's own analysis files for ~100 of the person's tracks (read-only); frame counts match exactly (ceil(samples ÷ (rate/150)) for detail, fixed 400/100/1200 for previews). Shapes correlate well with the original for the full/low bands; PWV7 mid/high correlate only about 0.6. (measured)
- Not written: PQT2 (extended grid), PSSI (phrases), PVDI, the .3EX file and MYSETTING files. (absent from export)
- PWVC values `0x005b 0x008a 0x018c` copied from a real file (meaning unknown).

## Cue / XML behaviour (for import and export)

- XML `POSITION_MARK`: Start/End seconds with 3 decimals; Type 0 point, 4 loop; Num 0–7 hot cues, −1 memory. (`addPositionMarkNode` 0x1010574b0, REA)
- XML `TEMPO`: Inizio, Bpm (2 dp), Metro 4/4, Battito. (`addTempoNode` 0x101057074, REA)
- 8 hot cues A–H in export mode. (binary string "8 Hot Cues can be set to A to H in EXPORT mode")
- Hot cue palette: 62 colours from `skins/hotCueImageAlpha.png`; picker uses 16 of them; default new hot cue colour index 43 (#FF376F, hot cue A in the screenshot; other defaults estimated).
- Memory cue / track colours: Pink #F870F8, Red #F80000, Orange #F8A030, Yellow #F8E330, Green #00E000, Aqua #00C0F8, Blue #0050F8, Purple #9808F8. (skin PNGs)
- Quantize: cue points, hot cues and loops snap to the nearest beat when Q is on. (behaviour of the original; snap target estimated as nearest beat)

## Track analysis (original DSP; the original uses trained models that are not reused)

| Value | Setting | Source |
|---|---|---|
| BPM range options | 70-180 (default), 48-95, 58-115, 68-135, 78-155, 88-175, 98-195, 108-215, 118-235, 128-255 | binary strings / Preferences screenshot |
| BPM limits for edits | 40 – 499 | binary message "BPM value should be between 40 and 499." |
| Onset envelope | 1024-pt FFT, 200 frames/s, log spectral flux to 8 kHz + low band to 160 Hz | estimated |
| Tempo | autocorrelation over the range, then phase-aligned comb on a 0.01 BPM grid; whole BPM preferred within 3% score | estimated |
| Beat phase | kick-band check of half-beat, first-kick anchor, then median attack start (85% of steepest rise, ±40 ms, 1 ms steps) | calibrated against 105 rekordbox-made grids in the person's local analysis files |
| MP3 start alignment | LAME-tagged: keep encoder lead-in; ffmpeg-tagged (Lavc/Lavf): remove delay from the tag (1105 samples typical); no tag: drop 1152 samples | measured: these three classes were offset by −27 / 0 / +26 ms against rekordbox's own grids before this rule |
| Benchmark | 105 tracks: tempo identical on 104 (one half-tempo miss); beat offset mean 0.5 ms, median 6 ms, 90% within 32 ms; bar start agrees on 81 | reference grids from the original's analysis files and collection XML (read-only) |
| Downbeat | low-band attack per beat position + chroma novelty at bar starts + bonus 3.0 for the bar position of the first beat of music | calibrated (66 → 81 of 105 bar starts) |
| Key | chroma 55–1760 Hz from 8192-pt FFT at ~11 kHz, spectral peaks only, bass weight 0.45, diatonic-triad templates, minor bias 0.15 | calibrated: 69 of 100 identical to rekordbox's key in the XML, 11 more a neighbour (relative/fifth); 28 of 37 against screenshot keys |
| Key names | Classic: Abm Ebm Bbm Fm Cm Gm Dm Am Em Bm Gbm Dbm / B Gb Db Ab Eb Bb F C G D A E; Alphanumeric 1A–12A / 1B–12B | binary key table + screenshot spellings |
| Waveform | 150 frames/s; peak and RMS of full, low (<200 Hz), mid (200–2500 Hz), high (>2500 Hz) | 150/s from PWV3 header `0x0096` (fixtures); band edges and model fitted to the original's files |
| Waveform colours (3Band) | low #0055E1, mid #FFA202, high #F4EAD5 | screenshot |
| Loudness | BS.1770 K-weighting, 400 ms blocks, −70 LUFS absolute and −10 LU relative gates | estimated (prior research has loudness cases; target not established) |
| Auto gain reference | −8 LUFS | estimated |

## Library

- Private SQLite library in `~/Library/Application Support/PrepDeck` (tracks, cues, grids, waveforms, playlists, intelligent playlists, My Tag, settings). Audio files are only ever read; edits never touch tags in files; rekordbox's own library is never opened (import is via its exported XML).
- Intelligent playlists: match all / any; text ops is, is not, contains, does not contain, starts with, ends with; numbers is, is not, greater, less, in the range; dates also in the last / not in the last (days, months); limit 128 lists. (dialog strings from the binary; "is"/"is greater than" wording estimated)
- Playlist limit 10000 tracks; device limit 1000 lists and folders. (binary messages)
- Default My Tag: Genre (Acid House, Deep House, Techno, Nu Disco, Electro House, Bass Music, Trap), Components (Synth, Vocal, Beat, Sub Bass, Percussion, Piano, Dark, Upper), Situation (Main Floor, Second Floor, Lounge, Mid Night, Morning, Build up, Peak Time, Build down), Untitled Column (My Comment). (real exportExt.pdb rows)

## Player and grid editing

- Beat jump sizes 1/4 … 64 beats, default 4. Auto beat loop 1/64 … 512 beats, default 4. (strings)
- Grid shift step 5 ms (Shift: 1 ms) — estimated. Wide/Narrow ±0.01 BPM — estimated.
- Tap tempo: average of the last taps (≥4), reset after 2 s pause — estimated.
- Detail waveform zoom spans 4, 8, 16 (default), 32, 64, 128 s — estimated from the screenshot.
