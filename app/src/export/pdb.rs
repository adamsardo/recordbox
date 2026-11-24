//! Pioneer "DeviceSQL" page database (export.pdb / exportExt.pdb) writer and reader.
//!
//! Layout rules come from app/MECHANICS.md ("USB database"): 4096-byte pages, page 0 is the
//! file header with one table pointer per table, every table starts with an index page
//! (flags 0x64) followed by data pages (flags 0x24) whose rows are addressed by row groups of
//! 16 offsets stored backwards from the end of the page.

pub const PAGE: usize = 4096;
const HEAP_START: usize = 0x28;
const GROUP_BYTES: usize = 36;
const MAX_INDEX_ENTRIES: usize = (PAGE - 0x3c - 20) / 4;

use anyhow::{ensure, Context, Result};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// Limit for descriptive text. The longest comment in the 3,886-track rekordcrate
/// fixture is 255 characters. Player paths must instead be encoded without truncation.
pub const MAX_STRING_CHARS: usize = 255;

/// Largest row a page can hold after four-byte alignment, leaving its row directory intact.
pub const MAX_ROW_BYTES: usize = (PAGE - HEAP_START - 6) & !3;

/// `s` cut to at most `max` characters (never inside a character).
pub fn clip_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Encode a string the way DeviceSQL stores it. Text beyond [`MAX_STRING_CHARS`] is cut, so
/// the 16-bit length of the long forms can never wrap.
pub fn dstr(s: &str) -> Vec<u8> {
    encode_text(clip_chars(s, MAX_STRING_CHARS))
}

/// Player paths are identities. Preserve the complete spelling, or report that
/// the encoded path exceeds one DeviceSQL page before generating any output.
pub(super) fn dstr_path(s: &str) -> Result<Vec<u8>> {
    let body = if s.is_ascii() {
        s.len()
    } else {
        2 * s.encode_utf16().take(MAX_ROW_BYTES / 2 + 1).count()
    };
    ensure!(body <= MAX_ROW_BYTES - 4, "device path is too long for one DeviceSQL row");
    Ok(encode_text(s))
}

fn encode_text(s: &str) -> Vec<u8> {
    if s.is_ascii() && s.len() <= 126 {
        let mut v = vec![(((s.len() + 1) << 1) | 1) as u8];
        v.extend_from_slice(s.as_bytes());
        v
    } else if s.is_ascii() {
        let mut v = vec![0x40u8];
        v.extend_from_slice(&((s.len() + 4) as u16).to_le_bytes());
        v.push(0);
        v.extend_from_slice(s.as_bytes());
        v
    } else {
        encode_utf16(s, MAX_ROW_BYTES / 2)
    }
}

/// Force the UTF-16LE ("0x90") long form.
pub fn dstr_ucs2(s: &str) -> Vec<u8> {
    encode_utf16(s, 2 * MAX_STRING_CHARS)
}

fn encode_utf16(s: &str, max_units: usize) -> Vec<u8> {
    let mut units: Vec<_> = s.encode_utf16().take(max_units).collect();
    if units.last().is_some_and(|u| (0xd800..0xdc00).contains(u)) {
        units.pop();
    }
    let mut v = vec![0x90u8];
    v.extend_from_slice(&((units.len() * 2 + 4) as u16).to_le_bytes());
    v.push(0);
    for u in units {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}

/// The ISRC form of the track row's first string: kind 0x90, length = text + 6, a zero pad
/// byte, the marker 0x03, the ASCII text and a terminating NUL (rekordcrate's
/// `DeviceSQLString` ISRC variant; e.g. `90 12 00 00 03 "DEOQ91710032" 00` in a real export).
/// An empty ISRC is the ordinary empty string.
pub fn dstr_isrc(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).take(64).collect();
    if s.is_empty() {
        return dstr("");
    }
    let mut v = vec![0x90u8];
    v.extend_from_slice(&((s.len() + 6) as u16).to_le_bytes());
    v.push(0);
    v.push(3);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// Whether the string at `data[pos..]` is stored in the ISRC form.
pub fn is_isrc_form(data: &[u8], pos: usize) -> bool {
    if data.get(pos) != Some(&0x90) || pos + 4 > data.len() {
        return false;
    }
    let len = u16::from_le_bytes([data[pos + 1], data[pos + 2]]) as usize;
    match data.get(pos + 4..pos + len) {
        Some(body) => isrc_body(body).is_some(),
        None => false,
    }
}

/// The text of an ISRC-form body (0x03, printable ASCII, NUL), or None for ordinary UTF-16.
fn isrc_body(body: &[u8]) -> Option<&[u8]> {
    if body.len() < 3 || body[0] != 3 || *body.last()? != 0 {
        return None;
    }
    let text = &body[1..body.len() - 1];
    text.iter().all(|b| (0x20..0x7f).contains(b)).then_some(text)
}

/// Decode a DeviceSQL string at `data[pos..]`.
pub fn read_dstr(data: &[u8], pos: usize) -> String {
    if pos >= data.len() {
        return String::new();
    }
    let h = data[pos];
    if h & 1 == 1 {
        let n = ((h >> 1) as usize).saturating_sub(1);
        let end = (pos + 1 + n).min(data.len());
        return String::from_utf8_lossy(&data[pos + 1..end]).to_string();
    }
    if pos + 4 > data.len() {
        return String::new();
    }
    let len = u16::from_le_bytes([data[pos + 1], data[pos + 2]]) as usize;
    let body_len = len.saturating_sub(4);
    let start = pos + 4;
    let end = (start + body_len).min(data.len());
    let body = &data[start..end];
    match h {
        0x40 => String::from_utf8_lossy(body).to_string(),
        0x90 => {
            // ISRC variant: 0x03, ASCII text, NUL (any length, e.g. the usual 12 characters).
            if let Some(text) = isrc_body(body) {
                return String::from_utf8_lossy(text).to_string();
            }
            let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::new(),
    }
}

/// Build a row made of a fixed part followed by an offset array (magic 3) and strings.
/// Offsets are relative to the row start. UTF-16 strings are aligned to 4 bytes.
pub fn row_with_strings(fixed: &[u8], strings: &[Vec<u8>], wide_offsets: bool) -> Vec<u8> {
    let mut row = fixed.to_vec();
    let osz = if wide_offsets { 2 } else { 1 };
    let table_pos = row.len();
    row.resize(table_pos + osz * (strings.len() + 1), 0);
    let mut offs = Vec::with_capacity(strings.len());
    for s in strings {
        // Both long forms start on a 4-byte boundary in real exports (num_rows: all 1,225
        // UTF-16 and all 50 long-ASCII strings of the track table).
        if matches!(s.first(), Some(&0x90) | Some(&0x40)) {
            while row.len() % 4 != 0 {
                row.push(0);
            }
        }
        offs.push(row.len());
        row.extend_from_slice(s);
    }
    if wide_offsets {
        row[table_pos..table_pos + 2].copy_from_slice(&3u16.to_le_bytes());
        for (i, o) in offs.iter().enumerate() {
            let p = table_pos + 2 + i * 2;
            row[p..p + 2].copy_from_slice(&(*o as u16).to_le_bytes());
        }
    } else {
        row[table_pos] = 3;
        for (i, o) in offs.iter().enumerate() {
            row[table_pos + 1 + i] = (*o).min(255) as u8;
        }
    }
    row
}

/// Whether a row built with 8-bit offsets from these parts keeps every string offset in a byte.
pub fn fits_narrow(fixed_len: usize, strings: &[Vec<u8>]) -> bool {
    let mut at = fixed_len + strings.len() + 1;
    for s in strings {
        if matches!(s.first(), Some(&0x90) | Some(&0x40)) {
            at = (at + 3) & !3;
        }
        if at > 255 {
            return false;
        }
        at += s.len();
    }
    true
}

/// One table to be written: its page type and its rows (already encoded).
#[derive(Default)]
pub struct TableSpec {
    pub page_type: u32,
    pub rows: Vec<Vec<u8>>,
    /// Write index entries pointing at the data pages (rekordbox does this for tracks).
    pub index_entries: bool,
    /// Patch "index_shift" (bytes 2..4) to 0x20 * row index within the page.
    pub index_shift: bool,
    /// Complete data pages written as they are after the rows' pages (only the page number
    /// and the link to the next page are set): tables Recordbox does not manage keep their
    /// rows byte for byte, deleted rows and flags included.
    pub raw_pages: Vec<Vec<u8>>,
}

fn groups_bytes(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let full = (n - 1) / 16;
    let last = n - full * 16;
    full * GROUP_BYTES + 4 + 2 * last
}

/// Split rows into pages respecting the heap capacity.
fn paginate(rows: &[Vec<u8>]) -> Vec<Vec<usize>> {
    let mut pages: Vec<Vec<usize>> = vec![];
    let mut cur: Vec<usize> = vec![];
    let mut used = 0usize;
    for (i, r) in rows.iter().enumerate() {
        // write_database bounds every row by MAX_ROW_BYTES before pagination.
        let len = (r.len() + 3) & !3;
        if !cur.is_empty() && HEAP_START + used + len + groups_bytes(cur.len() + 1) > PAGE {
            pages.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(i);
        used += len;
    }
    if !cur.is_empty() {
        pages.push(cur);
    }
    pages
}

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Serialize a whole database. Pages are allocated table by table: index page, data pages,
/// then an empty "candidate" page, all written to the file (blank pages are zero-filled).
pub fn write_database(tables: &[TableSpec], header_unknown: u32, sequence: u32) -> anyhow::Result<Vec<u8>> {
    ensure!(
        tables.len() <= 64,
        "too many DeviceSQL table descriptors for the supported layout"
    );
    let mut types = HashSet::new();
    for t in tables {
        ensure!(
            types.insert(t.page_type),
            "duplicate DeviceSQL table {}",
            t.page_type
        );
        ensure!(
            !t.rows.iter().any(Vec::is_empty),
            "empty encoded row in table {}",
            t.page_type
        );
        if let Some(r) = t.rows.iter().find(|r| r.len() > MAX_ROW_BYTES) {
            anyhow::bail!(
                "a row of table {} is {} bytes, more than a database page holds ({MAX_ROW_BYTES})",
                t.page_type,
                r.len()
            );
        }
        for page in &t.raw_pages {
            ensure!(
                page.len() == PAGE && matches!(page[27], 0x24 | 0x34),
                "invalid retained DeviceSQL data page in table {}",
                t.page_type
            );
            row_directory(page)
                .with_context(|| format!("checking retained page in table {}", t.page_type))?;
        }
    }
    struct Plan {
        index: u32,
        data: Vec<u32>,
        empty: u32,
        groups: Vec<Vec<usize>>,
    }
    let mut next = 1u32;
    let mut plans = vec![];
    for t in tables {
        let groups = paginate(&t.rows);
        let count = groups
            .len()
            .checked_add(t.raw_pages.len())
            .context("DeviceSQL page count overflow")?;
        ensure!(!t.index_entries || count <= MAX_INDEX_ENTRIES,
            "table {} needs {count} indexed pages; the supported DeviceSQL index holds {MAX_INDEX_ENTRIES}", t.page_type);
        let count = u32::try_from(count).context("too many DeviceSQL pages")?;
        let index = next;
        let empty = index
            .checked_add(1)
            .and_then(|n| n.checked_add(count))
            .context("DeviceSQL page numbers exhausted")?;
        let data = (index + 1..empty).collect();
        next = empty
            .checked_add(1)
            .context("DeviceSQL page numbers exhausted")?;
        plans.push(Plan {
            index,
            data,
            empty,
            groups,
        });
    }
    let total_pages = next as usize;
    let mut out = vec![
        0u8;
        total_pages
            .checked_mul(PAGE)
            .context("DeviceSQL output is too large")?
    ];

    // File header.
    put_u32(&mut out, 4, PAGE as u32);
    put_u32(&mut out, 8, tables.len() as u32);
    put_u32(&mut out, 12, next);
    put_u32(&mut out, 16, header_unknown);
    put_u32(&mut out, 20, sequence);
    for (i, (t, p)) in tables.iter().zip(&plans).enumerate() {
        let at = 28 + i * 16;
        put_u32(&mut out, at, t.page_type);
        put_u32(&mut out, at + 4, p.empty);
        put_u32(&mut out, at + 8, p.index);
        put_u32(&mut out, at + 12, *p.data.last().unwrap_or(&p.index));
    }

    let mut seq = 2u32;
    for (t, p) in tables.iter().zip(&plans) {
        // Index page.
        let base = p.index as usize * PAGE;
        let pg = &mut out[base..base + PAGE];
        put_u32(pg, 4, p.index);
        put_u32(pg, 8, t.page_type);
        put_u32(pg, 12, *p.data.first().unwrap_or(&p.empty));
        put_u32(pg, 16, 1);
        pg[27] = 0x64;
        let ix = 0x20;
        let entries: Vec<u32> = if t.index_entries { p.data.iter().map(|d| d << 3).collect() } else { vec![] };
        put_u16(pg, ix, 0x1fff);
        put_u16(pg, ix + 2, 0x1fff);
        put_u16(pg, ix + 4, 0x03ec);
        put_u16(pg, ix + 6, entries.len() as u16);
        put_u32(pg, ix + 8, p.index);
        put_u32(pg, ix + 12, *p.data.first().unwrap_or(&0x03ff_ffff));
        put_u32(pg, ix + 16, 0x03ff_ffff);
        put_u32(pg, ix + 20, 0);
        put_u16(pg, ix + 24, entries.len() as u16);
        put_u16(pg, ix + 26, 0x1fff);
        let mut e = ix + 28;
        for v in &entries {
            put_u32(pg, e, *v);
            e += 4;
        }
        while e + 4 <= PAGE - 20 {
            put_u32(pg, e, 0x1fff_fff8);
            e += 4;
        }

        // Data pages.
        for (k, rows_idx) in p.groups.iter().enumerate() {
            let pi = p.data[k];
            let base = pi as usize * PAGE;
            let pg = &mut out[base..base + PAGE];
            let n = rows_idx.len();
            let available = (PAGE - HEAP_START).checked_sub(groups_bytes(n))
                .ok_or_else(|| anyhow::anyhow!("row directory of table {} exceeds a database page", t.page_type))?;
            put_u32(pg, 4, pi);
            put_u32(pg, 8, t.page_type);
            let nextp = if k + 1 < p.data.len() { p.data[k + 1] } else { p.empty };
            put_u32(pg, 12, nextp);
            put_u32(pg, 16, seq);
            seq += 1;
            let packed = (n as u32 & 0x1fff) | ((n as u32 & 0x7ff) << 13);
            pg[24] = packed as u8;
            pg[25] = (packed >> 8) as u8;
            pg[26] = (packed >> 16) as u8;
            pg[27] = 0x24;
            let mut used = 0usize;
            let mut offsets = vec![];
            for (ri, &r) in rows_idx.iter().enumerate() {
                let mut row = tables_row(t, r, ri);
                let len = (row.len() + 3) & !3;
                row.resize(len, 0);
                let end = used.checked_add(len).filter(|&end| end <= available)
                    .ok_or_else(|| anyhow::anyhow!("rows of table {} exceed a database page", t.page_type))?;
                pg[HEAP_START + used..HEAP_START + end].copy_from_slice(&row);
                offsets.push(used as u16);
                used = end;
            }
            let free = available - used;
            put_u16(pg, 28, free as u16);
            put_u16(pg, 30, used as u16);
            put_u16(pg, 32, n as u16);
            put_u16(pg, 34, 0);
            for g in 0..(n + 15) / 16 {
                let gbase = PAGE - g * GROUP_BYTES;
                let mut flags = 0u16;
                for i in 0..16 {
                    let ri = g * 16 + i;
                    if ri >= n {
                        break;
                    }
                    flags |= 1 << i;
                    put_u16(pg, gbase - 6 - 2 * i, offsets[ri]);
                }
                put_u16(pg, gbase - 4, flags);
                put_u16(pg, gbase - 2, flags);
            }
        }
        // Kept pages, as they were, linked into this table's chain.
        for (k, raw) in t.raw_pages.iter().enumerate() {
            let k = p.groups.len() + k;
            let pi = p.data[k];
            let base = pi as usize * PAGE;
            let pg = &mut out[base..base + PAGE];
            pg.copy_from_slice(raw);
            put_u32(pg, 4, pi);
            put_u32(pg, 8, t.page_type);
            put_u32(pg, 12, if k + 1 < p.data.len() { p.data[k + 1] } else { p.empty });
        }
    }
    Ok(out)
}

/// Rows of one data page with their extents: (offset in heap, length, present). A row runs to
/// the next row's offset (or the end of the used heap), which is how much the page allots it.
pub fn page_rows(pg: &[u8]) -> Vec<(usize, usize, bool)> {
    if pg.len() < PAGE || pg[27] & 0x40 != 0 {
        return vec![];
    }
    row_directory(pg).unwrap_or_default()
}

/// Flag every present row of a data page deleted (bytes and offsets stay), as rekordbox leaves
/// superseded rows. A page that had present rows takes the 0x34 flags.
pub fn retire_rows(pg: &mut [u8]) {
    if pg.len() != PAGE || pg[27] & 0x40 != 0 {
        return;
    }
    let packed = pg[24] as usize | (pg[25] as usize) << 8 | (pg[26] as usize) << 16;
    let n = packed & 0x1fff;
    if packed >> 13 == 0 || row_directory(pg).is_err() {
        return;
    }
    for g in 0..(n + 15) / 16 {
        let gbase = PAGE - g * GROUP_BYTES;
        let was = get_u16(pg, gbase - 4);
        put_u16(pg, gbase - 4, 0);
        put_u16(pg, gbase - 2, was);
    }
    pg[25] &= 0x1f;
    pg[26] = 0;
    pg[27] = 0x34;
}

/// A fresh data page (flags 0x24) holding nothing yet.
pub fn blank_page(page_type: u32) -> Vec<u8> {
    let mut pg = vec![0u8; PAGE];
    put_u32(&mut pg, 8, page_type);
    put_u32(&mut pg, 16, 1);
    pg[27] = 0x24;
    put_u16(&mut pg, 28, (PAGE - HEAP_START) as u16);
    pg
}

/// Add `row` to a data page the way rekordbox adds a row to its history table at each
/// synchronisation: the row goes at the end of the used heap with the next row offset, and with
/// `retire_others` the rows present before are flagged deleted (their bytes stay) and the page
/// takes the 0x34 flags real pages with deleted rows carry. The second flag word of a group
/// lists the rows this change touched (observed: old and new row). Returns false, leaving the
/// page untouched, when the row does not fit.
pub fn append_row(pg: &mut [u8], row: &[u8], retire_others: bool) -> bool {
    if pg.len() != PAGE || pg[27] & 0x40 != 0 || row.is_empty() || row.len() > MAX_ROW_BYTES || row_directory(pg).is_err() {
        return false;
    }
    let packed = pg[24] as usize | (pg[25] as usize) << 8 | (pg[26] as usize) << 16;
    let n = packed & 0x1fff;
    let mut present = packed >> 13;
    let used = get_u16(pg, 30) as usize;
    let len = (row.len() + 3) & !3;
    if HEAP_START + used + len + groups_bytes(n + 1) > PAGE || n + 1 > 0x1fff {
        return false;
    }
    let mut touched = vec![0u16; (n + 16) / 16];
    if retire_others {
        for g in 0..(n + 15) / 16 {
            let gbase = PAGE - g * GROUP_BYTES;
            touched[g] = get_u16(pg, gbase - 4);
            put_u16(pg, gbase - 4, 0);
            put_u16(pg, gbase - 2, touched[g]);
        }
        if present > 0 {
            pg[27] = 0x34;
        }
        present = 0;
    }
    pg[HEAP_START + used..HEAP_START + used + len].fill(0);
    pg[HEAP_START + used..HEAP_START + used + row.len()].copy_from_slice(row);
    let (g, k) = (n / 16, n % 16);
    let gbase = PAGE - g * GROUP_BYTES;
    if k == 0 {
        put_u16(pg, gbase - 4, 0);
        put_u16(pg, gbase - 2, 0);
    }
    put_u16(pg, gbase - 6 - 2 * k, used as u16);
    let flags = get_u16(pg, gbase - 4) | (1 << k);
    put_u16(pg, gbase - 4, flags);
    put_u16(pg, gbase - 2, if retire_others { touched[g] | (1 << k) } else { get_u16(pg, gbase - 2) | (1 << k) });
    let (n, present) = (n + 1, present + 1);
    let packed = (n & 0x1fff) | ((present & 0x7ff) << 13);
    pg[24] = packed as u8;
    pg[25] = (packed >> 8) as u8;
    pg[26] = (packed >> 16) as u8;
    put_u16(pg, 30, (used + len) as u16);
    put_u16(pg, 28, (PAGE - HEAP_START - used - len - groups_bytes(n)) as u16);
    true
}

fn tables_row(t: &TableSpec, r: usize, index_in_page: usize) -> Vec<u8> {
    let mut row = t.rows[r].clone();
    if t.index_shift && row.len() >= 4 {
        let v = (index_in_page as u16).wrapping_mul(0x20);
        row[2..4].copy_from_slice(&v.to_le_bytes());
    }
    row
}

// ---------------------------------------------------------------------------------------------
// Reader (used to merge an existing device library and to verify our own output)
// ---------------------------------------------------------------------------------------------

pub struct RawRow<'a> {
    pub data: &'a [u8],
    /// Byte slice from the row start to the end of the page heap (strings live here).
    pub page_heap: &'a [u8],
    pub offset_in_heap: usize,
}

struct TablePages {
    data: Vec<usize>,          // complete data pages retained by the passthrough writer
    rows: Vec<(usize, usize)>, // (absolute page heap, row offset within heap)
}

pub struct Database {
    pub bytes: Vec<u8>,
    pub page_size: usize,
    pub tables: Vec<(u32, u32, u32, u32)>, // type, empty, first, last
    directory: HashMap<u32, TablePages>,
    ends: HashMap<usize, usize>, // absolute start → exclusive end of a live row
}

impl Database {
    pub fn parse(bytes: Vec<u8>) -> Result<Database> {
        ensure!(
            bytes.len() >= 28 && get_u32(&bytes, 0) == 0,
            "incomplete or unsupported DeviceSQL header"
        );
        let page_size = get_u32(&bytes, 4) as usize;
        ensure!(
            page_size.is_power_of_two() && (512..=65536).contains(&page_size),
            "bad DeviceSQL page size {page_size}"
        );
        ensure!(
            bytes.len().is_multiple_of(page_size),
            "truncated DeviceSQL page"
        );
        let n = get_u32(&bytes, 8) as usize;
        ensure!(
            n <= 64 && 28 + n * 16 <= page_size,
            "invalid DeviceSQL table count {n}"
        );
        let mut tables = Vec::with_capacity(n);
        let mut directory = HashMap::with_capacity(n);
        let mut ends = HashMap::new();
        let mut claimed = HashSet::new();
        for descriptor in bytes[28..28 + n * 16].chunks_exact(16) {
            let t = get_u32(descriptor, 0);
            let empty = get_u32(descriptor, 4);
            let first = get_u32(descriptor, 8);
            let last = get_u32(descriptor, 12);
            ensure!(!directory.contains_key(&t), "duplicate DeviceSQL table {t}");
            tables.push((t, empty, first, last));
            let mut table = TablePages {
                data: vec![],
                rows: vec![],
            };
            let mut index = first;
            let mut index_pages = Vec::new();
            loop {
                ensure!(
                    index != 0 && (index as usize) < bytes.len() / page_size,
                    "table {t} has an unavailable page {index} before its declared end {last}"
                );
                ensure!(
                    claimed.insert(index),
                    "table {t} revisits a page or shares it with another table"
                );
                let base = index as usize * page_size;
                let pg = &bytes[base..base + page_size];
                ensure!(
                    get_u32(pg, 0) == 0 && get_u32(pg, 4) == index && get_u32(pg, 8) == t,
                    "DeviceSQL table {t} points to an unrelated page at {index}"
                );
                // DeviceSQL data pages with live/deleted slots and its two observed index flags.
                ensure!(
                    matches!(pg[27], 0x24 | 0x34 | 0x44 | 0x64),
                    "unsupported DeviceSQL flags on table {t}, page {index}"
                );
                ensure!(
                    index != first || pg[27] & 0x40 != 0,
                    "table {t} has no initial index page"
                );
                if pg[27] & 0x40 == 0 {
                    let count = packed_counts(pg) & 0x1fff;
                    if count > 0 {
                        table.data.push(base);
                    }
                    let rows = row_directory(pg)
                        .with_context(|| format!("reading DeviceSQL table {t}, page {index}"))?;
                    for (offset, len, present) in rows {
                        if present {
                            let heap = base + HEAP_START;
                            table.rows.push((heap, offset));
                            ends.insert(heap + offset, heap + offset + len);
                        }
                    }
                } else {
                    index_pages.push(index);
                }
                if index == last {
                    ensure!(
                        get_u32(pg, 12) == empty,
                        "inconsistent last page in DeviceSQL table {t}"
                    );
                    break;
                }
                index = get_u32(pg, 12);
            }
            for index_page in index_pages {
                let base = index_page as usize * page_size;
                let page = &bytes[base..base + page_size];
                let documented_target = |target: u32, index_expected: bool| {
                    if target == 0x03ff_ffff {
                        return true;
                    }
                    if !claimed.contains(&target) {
                        return false;
                    }
                    let at = target as usize * page_size;
                    get_u32(&bytes, at + 8) == t && (bytes[at + 27] & 0x40 != 0) == index_expected
                };
                let count = get_u16(page, 38) as usize;
                ensure!(
                    get_u32(page, 40) == index_page
                        && documented_target(get_u32(page, 44), false)
                        && documented_target(get_u32(page, 48), true)
                        && get_u16(page, 56) as usize <= count
                        && count <= get_u16(page, 36) as usize
                        && count <= (page_size - 80) / 4,
                    "unsupported index header in DeviceSQL table {t}, page {index_page}"
                );
                for at in (60..60 + 4 * count).step_by(4) {
                    // The low three bits are native index flags.
                    ensure!(documented_target(get_u32(page, at) >> 3, false),
                        "index references an unavailable data page in DeviceSQL table {t}");
                }
            }
            directory.insert(t, table);
        }
        ensure!(
            tables.iter().all(|t| !claimed.contains(&t.1)),
            "DeviceSQL spare page overlaps an active table"
        );
        for (index, page) in bytes.chunks_exact(page_size).enumerate().skip(1) {
            if !claimed.contains(&(index as u32)) {
                ensure!(
                    page[27] & 0x40 != 0 || packed_counts(page) >> 13 == 0,
                    "DeviceSQL page {index} contains live rows outside its table chain"
                );
            }
        }
        Ok(Self {
            bytes,
            page_size,
            tables,
            directory,
            ends,
        })
    }

    /// Only rebuild the classic schemas the writer can reproduce completely.
    pub fn validate_table_set(&self, expected: Range<u32>) -> Result<()> {
        ensure!(
            self.page_size == PAGE,
            "unsupported DeviceSQL page size for rewriting"
        );
        ensure!(
            expected.len() == self.tables.len()
                && expected.clone().all(|t| self.directory.contains_key(&t)),
            "unsupported DeviceSQL table layout (expected types {} through {})",
            expected.start,
            expected.end.saturating_sub(1)
        );
        Ok(())
    }

    fn records(&self, page_type: u32) -> impl Iterator<Item = &[u8]> {
        self.directory
            .get(&page_type)
            .into_iter()
            .flat_map(|table| {
                table
                    .rows
                    .iter()
                    .map(|&(heap, offset)| self.row_bytes(heap, offset))
            })
    }

    /// Check the rows the classic main-catalog reader interprets before it can
    /// normalize a missing lookup, silently drop a playlist link or join a
    /// catalog-supplied path to a mounted drive.
    pub fn validate_track_rows(&self) -> Result<()> {
        let mut names: HashMap<u32, HashSet<u32>> = HashMap::new();
        for (table, id_at, name_at, indexed) in [
            (1, 0, 4, false),
            (2, 4, 8, true),
            (3, 12, 20, true),
            (4, 0, 4, false),
            (5, 0, 8, false),
            (11, 0, 4, false),
            (13, 0, 4, false),
        ] {
            let ids = names.entry(table).or_default();
            for row in self.records(table) {
                let label = if indexed {
                    string_slots(row, name_at, 1)?[0]
                } else {
                    name_at
                };
                checked_dstr_len(row, label)
                    .with_context(|| format!("invalid name in DeviceSQL table {table}"))?;
                let id = get_u32(row, id_at);
                ensure!(
                    id != 0 && ids.insert(id),
                    "ambiguous ID in DeviceSQL table {table}"
                );
                if table == 13 {
                    let path = read_dstr(row, label);
                    ensure!(
                        path.is_empty() || device_relative_path(&path),
                        "invalid device artwork path"
                    );
                }
            }
        }
        for album in self.records(3) {
            let artist = get_u32(album, 8);
            ensure!(
                artist == 0 || names[&2].contains(&artist),
                "album references an unavailable device artist"
            );
        }
        let mut colors = HashSet::new();
        for color in self.records(6) {
            checked_dstr_len(color, 8)?;
            ensure!(
                (1..=8).contains(&color[5]) && colors.insert(color[5]),
                "unsupported device colour directory"
            );
        }
        ensure!(
            colors.is_empty() || colors.len() == 8,
            "incomplete device colour directory"
        );
        let mut tracks = HashSet::new();
        for row in self.records(0) {
            let slots = string_slots(row, 0x5c, 21)?;
            let id = get_u32(row, 0x48);
            ensure!(
                id != 0 && tracks.insert(id),
                "ambiguous DeviceSQL track ID {id}"
            );
            let audio = read_dstr(row, slots[20]);
            ensure!(
                device_relative_path(&audio),
                "invalid device audio path on track {id}"
            );
            let anlz = read_dstr(row, slots[14]);
            if !anlz.is_empty() {
                let upper = anlz.to_ascii_uppercase();
                ensure!(
                    device_relative_path(&anlz)
                        && upper.ends_with(".DAT")
                        && (upper.starts_with("/PIONEER/USBANLZ/")
                            || upper.starts_with("/.PIONEER/USBANLZ/")),
                    "invalid device analysis path on track {id}"
                );
            }
            for (table, at) in [
                (1, 0x3c),
                (2, 0x0c),
                (2, 0x24),
                (2, 0x2c),
                (2, 0x44),
                (3, 0x40),
                (4, 0x28),
                (5, 0x20),
                (13, 0x1c),
            ] {
                let foreign = get_u32(row, at);
                ensure!(
                    foreign == 0 || names[&table].contains(&foreign),
                    "track {id} has an unresolved reference in DeviceSQL table {table}"
                );
            }
        }
        let mut nodes = HashMap::new();
        for row in self.records(7) {
            checked_dstr_len(row, 20)?;
            let id = get_u32(row, 12);
            let kind = get_u32(row, 16);
            ensure!(
                kind <= 1 && id != 0 && nodes.insert(id, (get_u32(row, 0), kind)).is_none(),
                "invalid or duplicate device playlist node"
            );
        }
        let mut reached = HashSet::from([0]);
        for &id in nodes.keys() {
            let mut path = HashSet::new();
            let mut here = id;
            while !reached.contains(&here) {
                ensure!(path.insert(here), "cycle in device playlist tree");
                let parent = nodes[&here].0;
                ensure!(
                    parent == 0 || nodes.get(&parent).is_some_and(|n| n.1 == 1),
                    "missing parent folder for device playlist {here}"
                );
                here = parent;
            }
            reached.extend(path);
        }
        for row in self.records(8) {
            ensure!(
                row.len() >= 12
                    && tracks.contains(&get_u32(row, 4))
                    && nodes.get(&get_u32(row, 8)).is_some_and(|n| n.1 == 0),
                "device playlist entry references an unavailable track or playlist"
            );
        }
        for row in self.records(12) {
            ensure!(
                row.len() >= 12 && names[&11].contains(&get_u32(row, 4)),
                "device history entry references an unavailable history"
            );
        }
        Ok(())
    }

    pub fn validate_ext_rows(&self) -> Result<()> {
        let mut cats = HashSet::new();
        let mut ids = HashSet::new();
        let mut labels = HashSet::new();
        let mut tags = HashMap::new();
        for row in self.records(3) {
            let slots = string_slots(row, 0x1c, 1)?;
            let parent = get_u32(row, 12);
            let id = get_u32(row, 20);
            let name = read_dstr(row, slots[0]);
            ensure!(
                id != 0
                    && ids.insert(id)
                    && !name.contains('\u{1f}')
                    && labels.insert((parent, name)),
                "ambiguous DeviceSQL My Tag identity or name"
            );
            match row[27] {
                1 if parent == 0 => {
                    cats.insert(id);
                }
                0 => {
                    tags.insert(id, parent);
                }
                _ => anyhow::bail!("unsupported DeviceSQL My Tag row kind"),
            }
        }
        ensure!(
            tags.values().all(|p| cats.contains(p)),
            "device tag references an unavailable category"
        );
        for row in self.records(4) {
            ensure!(
                row.len() >= 12 && tags.contains_key(&get_u32(row, 8)),
                "device track tag references an unavailable tag"
            );
        }
        Ok(())
    }

    pub fn rows(&self, page_type: u32) -> Vec<(usize, usize)> {
        self.directory
            .get(&page_type)
            .map(|t| t.rows.clone())
            .unwrap_or_default()
    }

    pub fn data_pages(&self, page_type: u32) -> Vec<Vec<u8>> {
        self.directory
            .get(&page_type)
            .into_iter()
            .flat_map(|t| t.data.iter())
            .map(|&at| self.bytes[at..at + self.page_size].to_vec())
            .collect()
    }

    /// Strings and fixed columns belong to this row, even when later rows share its page.
    pub fn row_bytes(&self, heap: usize, off: usize) -> &[u8] {
        let Some(start) = heap.checked_add(off) else {
            return &[];
        };
        self.ends
            .get(&start)
            .map(|&end| &self.bytes[start..end])
            .unwrap_or(&[])
    }
}

fn packed_counts(pg: &[u8]) -> usize {
    pg[24] as usize | (pg[25] as usize) << 8 | (pg[26] as usize) << 16
}

/// Bound the heap by its own row directory. Deleted slots delimit surviving rows
/// as well, so a stale string offset can never borrow bytes from its neighbour.
fn row_directory(pg: &[u8]) -> Result<Vec<(usize, usize, bool)>> {
    let count = packed_counts(pg);
    let n = count & 0x1fff;
    let used = get_u16(pg, 30) as usize;
    ensure!(
        HEAP_START + used + groups_bytes(n) <= pg.len(),
        "row heap and directory overlap"
    );
    if !n.is_multiple_of(16) {
        let end = pg.len() - (n / 16) * GROUP_BYTES;
        ensure!(
            get_u16(pg, end - 4) >> (n % 16) == 0,
            "page marks unused row slots as present"
        );
    }
    let mut slots = Vec::with_capacity(n);
    for i in 0..n {
        let k = i % 16;
        let end = pg.len() - (i / 16) * GROUP_BYTES;
        let off = get_u16(pg, end - 6 - 2 * k) as usize;
        let present = get_u16(pg, end - 4) & (1 << k) != 0;
        ensure!(
            off <= used && (!present || off < used),
            "row offset is outside its heap"
        );
        slots.push((off, present));
    }
    ensure!(
        slots.iter().filter(|s| s.1).count() == count >> 13,
        "live row count disagrees with directory flags"
    );
    let mut offsets: Vec<_> = slots.iter().map(|s| s.0).collect();
    offsets.push(used);
    offsets.sort_unstable();
    let mut present_offsets = HashSet::new();
    slots
        .into_iter()
        .map(|(off, present)| {
            ensure!(
                !present || present_offsets.insert(off),
                "duplicate live row offset"
            );
            let end = offsets
                .get(offsets.partition_point(|&o| o <= off))
                .copied()
                .unwrap_or(used);
            Ok((off, end - off, present))
        })
        .collect()
}

/// Validate slash-rooted device notation independently of the host's path syntax.
pub(super) fn device_relative_path(s: &str) -> bool {
    s.strip_prefix('/').is_some_and(|path| {
        path.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with(['.', ' '])
                && !p.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
                && !p.bytes().any(|b| b < 32)
        })
    })
}

/// A string's declared size must fit the row, including its prefix. Lossy
/// decoding when rewriting a catalog would silently replace the original text.
fn checked_dstr_len(bytes: &[u8], at: usize) -> Result<usize> {
    let s = bytes
        .get(at..)
        .context("DeviceSQL string points outside its row")?;
    let h = *s.first().context("missing DeviceSQL string header")?;
    if h & 1 != 0 {
        let n = (h >> 1) as usize;
        ensure!(n > 0 && n <= s.len(), "truncated short DeviceSQL string");
        std::str::from_utf8(&s[1..n]).context("unknown short DeviceSQL string encoding")?;
        return Ok(n);
    }
    ensure!(s.len() >= 4, "missing long DeviceSQL string length");
    let n = get_u16(s, 1) as usize;
    let body = s.get(4..n).context("truncated long DeviceSQL string")?;
    match h {
        0x40 => {
            std::str::from_utf8(body).context("unknown long DeviceSQL string encoding")?;
        }
        0x90 if isrc_body(body).is_some() => {}
        0x90 => {
            ensure!(
                body.len().is_multiple_of(2)
                    && char::decode_utf16(
                        body.chunks_exact(2)
                            .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    )
                    .all(|c| c.is_ok()),
                "invalid DeviceSQL UTF-16"
            );
        }
        _ => anyhow::bail!("unsupported DeviceSQL string encoding"),
    }
    Ok(n)
}

/// Return validated string offsets. Some row types have additional fields
/// the reader doesn't interpret, so accept a gap before the first string.
fn string_slots(row: &[u8], header: usize, count: usize) -> Result<Vec<usize>> {
    ensure!(
        row.len() >= header && header >= 2,
        "truncated DeviceSQL fixed row"
    );
    let wide = get_u16(row, 0) & 4 != 0;
    let size = if wide { 2 } else { 1 };
    let min = header + (count + 1) * size;
    ensure!(min <= row.len(), "truncated DeviceSQL string directory");
    let at = |i| {
        if wide {
            get_u16(row, header + i * 2) as usize
        } else {
            row[header + i] as usize
        }
    };
    ensure!(at(0) == 3, "unsupported DeviceSQL string directory");
    let slots: Vec<_> = (1..=count).map(at).collect();
    let mut limits = slots.clone();
    limits.push(row.len());
    limits.sort_unstable();
    for &off in &slots {
        ensure!(
            off >= min && off < row.len(),
            "DeviceSQL string overlaps its header or extends beyond the row"
        );
        let n = checked_dstr_len(row, off)?;
        let end = limits[limits.partition_point(|&other| other <= off)];
        ensure!(n <= end - off, "overlapping DeviceSQL strings");
    }
    Ok(slots)
}

/// Read a string referenced by an offset array entry.
pub fn row_string(row: &[u8], table_at: usize, index: usize, wide: bool) -> String {
    let off = if wide {
        let p = table_at + 2 + index * 2;
        if p + 2 > row.len() {
            return String::new();
        }
        get_u16(row, p) as usize
    } else {
        let p = table_at + 1 + index;
        if p >= row.len() {
            return String::new();
        }
        row[p] as usize
    };
    read_dstr(row, off)
}

pub fn rd_u16(b: &[u8], at: usize) -> u16 {
    if at + 2 > b.len() {
        0
    } else {
        get_u16(b, at)
    }
}
pub fn rd_u32(b: &[u8], at: usize) -> u32 {
    if at + 4 > b.len() {
        0
    } else {
        get_u32(b, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strings_roundtrip() {
        for s in ["", "Pink", "Ünïcødé ✓", &"x".repeat(200), &"x".repeat(126), &"x".repeat(127), &"x".repeat(255)] {
            let e = dstr(s);
            assert_eq!(read_dstr(&e, 0), s);
        }
        assert_eq!(dstr("Pink"), vec![0x0b, b'P', b'i', b'n', b'k']);
    }
    #[test]
    fn database_roundtrip() {
        let rows: Vec<Vec<u8>> = (0..40u32).map(|i| {
            let mut fixed = vec![0x60, 0, 0, 0];
            fixed.extend_from_slice(&(i + 1).to_le_bytes());
            row_with_strings(&fixed, &[dstr(&format!("Artist {i}"))], false)
        }).collect();
        let db = write_database(&[TableSpec { page_type: 2, rows, index_entries: false, index_shift: true, ..Default::default() }], 5, 3).unwrap();
        let parsed = Database::parse(db).unwrap();
        let rows = parsed.rows(2);
        assert_eq!(rows.len(), 40);
        let r = parsed.row_bytes(rows[39].0, rows[39].1);
        assert_eq!(rd_u32(r, 4), 40);
        assert_eq!(row_string(r, 8, 0, false), "Artist 39");
    }
    fn test_track(id: u32, audio: &str, analysis: &str) -> Vec<u8> {
        let mut header = vec![0; 0x5c];
        put_u16(&mut header, 0, 0x24);
        put_u32(&mut header, 0x48, id);
        let mut fields = vec![dstr(""); 21];
        fields[14] = dstr(analysis);
        fields[20] = dstr(audio);
        row_with_strings(&header, &fields, true)
    }

    fn test_table(kind: u32, rows: Vec<Vec<u8>>) -> Vec<u8> {
        write_database(
            &[TableSpec {
                page_type: kind,
                rows,
                ..Default::default()
            }],
            5,
            3,
        )
        .unwrap()
    }

    #[test]
    fn invalid_chain_or_row_counts_cannot_become_a_partial_catalog() {
        let original = test_table(0, vec![vec![0; 3000], vec![1; 3000]]);
        let first = get_u32(&original, 36) as usize;
        let first_data = get_u32(&original, first * PAGE + 12) as usize;
        let mut truncated = original.clone();
        truncated.pop();
        assert!(Database::parse(truncated).is_err());

        let mut wrong_table = original.clone();
        put_u32(&mut wrong_table, first_data * PAGE + 8, 20);
        assert!(Database::parse(wrong_table).is_err());

        let mut bad_chain = original.clone();
        put_u32(&mut bad_chain, first_data * PAGE + 12, first_data as u32);
        assert!(Database::parse(bad_chain).is_err());

        let mut short_chain = original.clone();
        let spare = get_u32(&short_chain, 32);
        put_u32(&mut short_chain, first_data * PAGE + 12, spare);
        put_u32(&mut short_chain, 40, first_data as u32);
        assert!(Database::parse(short_chain).is_err());

        let mut stale_index = original.clone();
        put_u32(&mut stale_index, first * PAGE + 44, first as u32);
        assert!(Database::parse(stale_index).is_err());
        let mut stale_count = original;
        put_u16(&mut stale_count, (first_data + 1) * PAGE - 4, 0);
        assert!(Database::parse(stale_count).is_err());
    }

    #[test]
    fn audio_and_anlz_references_stay_within_the_track_and_device() {
        let path = "/PIONEER/USBANLZ/P00A/00004D8C/ANLZ0000.DAT";
        // U+0081 occurs in a genuine rekordcrate export filename.
        let row = test_track(1, "/Contents/Artist/Mix\u{81}.mp3", path);
        let other = test_track(2, "/Contents/第二曲.mp3", "");
        let parsed = Database::parse(test_table(0, vec![row.clone(), other.clone()])).unwrap();
        assert!(parsed.validate_track_rows().is_ok());
        let (heap, off) = parsed.rows(0)[0];
        assert_eq!(
            parsed.row_bytes(heap, off).len(),
            row.len().next_multiple_of(4)
        );
        let mut crossed = row.clone();
        let wrong = (row.len().next_multiple_of(4) + 4) as u16;
        put_u16(&mut crossed, 0x5c + 2 * (1 + 14), wrong);
        assert!(Database::parse(test_table(0, vec![crossed, other]))
            .unwrap()
            .validate_track_rows()
            .is_err());
        for (audio, analysis) in [
            ("/Contents/../track.mp3", path),
            ("C:/Music/track.mp3", path),
            ("/Contents/track.mp3", "/PIONEER/USBANLZ/../Unrelated.DAT"),
            ("/Contents/track.mp3", "/Contents/Unrelated.DAT"),
        ] {
            let db = Database::parse(test_table(0, vec![test_track(1, audio, analysis)])).unwrap();
            assert!(db.validate_track_rows().is_err());
        }
    }

    #[test]
    fn rewrite_requires_known_tables_and_resolvable_rows() {
        let db = Database::parse(test_table(25, vec![])).unwrap();
        assert!(db.validate_table_set(0..20).is_err());
        let mut track = test_track(1, "/Contents/track.mp3", "");
        put_u32(&mut track, 0x44, 9);
        let db = Database::parse(test_table(0, vec![track])).unwrap();
        assert!(db.validate_track_rows().is_err());
        let mut entry = vec![0; 12];
        put_u32(&mut entry, 8, 71);
        let db = Database::parse(test_table(4, vec![entry])).unwrap();
        assert!(db.validate_ext_rows().is_err());
    }

    #[test]
    fn device_paths_keep_full_ascii_and_unicode_spelling() {
        for part in ["ab", "龍🎵"] {
            let path = format!("/Contents/{}/track.wav", part.repeat(180));
            let encoded = dstr_path(&path).unwrap();
            assert_eq!(read_dstr(&encoded, 0), path);
        }
        assert!(dstr_path(&"x".repeat(MAX_ROW_BYTES)).is_err());
        assert!(dstr_path(&"🎵".repeat(MAX_ROW_BYTES / 4 + 1)).is_err());
    }

    #[test]
    fn indexed_table_capacity_is_checked_before_writing() {
        let table = TableSpec {
            page_type: 0,
            rows: vec![vec![0; 3000]; MAX_INDEX_ENTRIES + 1],
            index_entries: true,
            ..Default::default()
        };
        assert!(write_database(&[table], 5, 3)
            .unwrap_err()
            .to_string()
            .contains("indexed pages"));
    }
}

#[cfg(test)]
mod table_census {
    /// Rows per table type in real rekordbox exports (fixtures). Run with --ignored.
    #[test]
    #[ignore]
    fn row_counts() {
        let root = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/format-oracles");
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().map(|n| n == "export.pdb" || n == "exportExt.pdb").unwrap_or(false) {
                    let Ok(db) = super::Database::parse(std::fs::read(&p).unwrap()) else { continue };
                    let counts: Vec<String> = (0..30).filter_map(|t| { let n = db.rows(t).len(); (n > 0).then(|| format!("{t}:{n}")) }).collect();
                    for (h, o) in db.rows(19) {
                        let r = db.row_bytes(h, o);
                        println!("  table19 head {:?}", &r[..32.min(r.len())]);
                    }
                    println!("{} -> {}", p.display().to_string().split("format-oracles/").nth(1).unwrap_or(""), counts.join(" "));
                }
            }
        }
    }
}
