//! Pioneer "DeviceSQL" page database (export.pdb / exportExt.pdb) writer and reader.
//!
//! Layout rules come from app/MECHANICS.md ("USB database"): 4096-byte pages, page 0 is the
//! file header with one table pointer per table, every table starts with an index page
//! (flags 0x64) followed by data pages (flags 0x24) whose rows are addressed by row groups of
//! 16 offsets stored backwards from the end of the page.

pub const PAGE: usize = 4096;
const HEAP_START: usize = 0x28;
const GROUP_BYTES: usize = 36;

/// Longest string (in characters) Recordbox writes into a database row. The longest text in
/// 3,886 real track rows (rekordcrate's `num_rows` export) is a 255-character comment; no
/// longer string has been seen in a rekordbox-made export, so everything is cut there.
pub const MAX_STRING_CHARS: usize = 255;

/// Largest row a page can hold (heap minus one row group).
pub const MAX_ROW_BYTES: usize = PAGE - HEAP_START - 6;

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
    let s = clip_chars(s, MAX_STRING_CHARS);
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
        dstr_ucs2(s)
    }
}

/// Force the UTF-16LE ("0x90") long form.
pub fn dstr_ucs2(s: &str) -> Vec<u8> {
    let mut units: Vec<u16> = s.encode_utf16().collect();
    // Two units per character at most: 255 characters always fit; anything longer handed in
    // directly is cut at a character boundary below the 16-bit length.
    if units.len() > 2 * MAX_STRING_CHARS {
        units.truncate(2 * MAX_STRING_CHARS);
        if units.last().map(|u| (0xd800..0xdc00).contains(u)).unwrap_or(false) {
            units.pop();
        }
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
    for t in tables {
        if let Some(r) = t.rows.iter().find(|r| r.len() > MAX_ROW_BYTES) {
            anyhow::bail!("a row of table {} is {} bytes, more than a database page holds ({MAX_ROW_BYTES})", t.page_type, r.len());
        }
        if t.raw_pages.iter().any(|p| p.len() != PAGE) {
            anyhow::bail!("a kept page of table {} is not {PAGE} bytes", t.page_type);
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
        let index = next;
        next += 1;
        let data: Vec<u32> = (0..(groups.len() + t.raw_pages.len()) as u32).map(|k| next + k).collect();
        next += (groups.len() + t.raw_pages.len()) as u32;
        let empty = next;
        next += 1;
        plans.push(Plan { index, data, empty, groups });
    }
    let total_pages = next as usize;
    let mut out = vec![0u8; total_pages * PAGE];

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
                pg[HEAP_START + used..HEAP_START + used + len].copy_from_slice(&row);
                offsets.push(used as u16);
                used += len;
            }
            let free = PAGE - HEAP_START - used - groups_bytes(n);
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
    let packed = pg[24] as usize | (pg[25] as usize) << 8 | (pg[26] as usize) << 16;
    let n = packed & 0x1fff;
    let used = get_u16(pg, 30) as usize;
    let mut offs = vec![];
    for i in 0..n {
        let (g, k) = (i / 16, i % 16);
        let gbase = PAGE - g * GROUP_BYTES;
        if gbase < HEAP_START + 6 + 2 * k {
            break;
        }
        offs.push((get_u16(pg, gbase - 6 - 2 * k) as usize, get_u16(pg, gbase - 4) & (1 << k) != 0));
    }
    let mut sorted: Vec<usize> = offs.iter().map(|o| o.0).collect();
    sorted.push(used);
    sorted.sort_unstable();
    offs.iter().map(|&(o, present)| (o, sorted.iter().find(|&&x| x > o).copied().unwrap_or(used).saturating_sub(o), present)).collect()
}

/// Flag every present row of a data page deleted (bytes and offsets stay), as rekordbox leaves
/// superseded rows. A page that had present rows takes the 0x34 flags.
pub fn retire_rows(pg: &mut [u8]) {
    if pg.len() != PAGE || pg[27] & 0x40 != 0 {
        return;
    }
    let packed = pg[24] as usize | (pg[25] as usize) << 8 | (pg[26] as usize) << 16;
    let n = packed & 0x1fff;
    if packed >> 13 == 0 {
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
    if pg.len() != PAGE || pg[27] & 0x40 != 0 {
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

pub struct Database {
    pub bytes: Vec<u8>,
    pub page_size: usize,
    pub tables: Vec<(u32, u32, u32, u32)>, // type, empty, first, last
}

impl Database {
    pub fn parse(bytes: Vec<u8>) -> anyhow::Result<Database> {
        if bytes.len() < 28 {
            anyhow::bail!("file too small");
        }
        let page_size = get_u32(&bytes, 4) as usize;
        if page_size < 512 || page_size > 65536 {
            anyhow::bail!("bad page size {page_size}");
        }
        let n = get_u32(&bytes, 8) as usize;
        if n > 64 || 28 + n * 16 > bytes.len() {
            anyhow::bail!("bad table count");
        }
        let tables = (0..n)
            .map(|i| {
                let at = 28 + i * 16;
                (get_u32(&bytes, at), get_u32(&bytes, at + 4), get_u32(&bytes, at + 8), get_u32(&bytes, at + 12))
            })
            .collect();
        Ok(Database { bytes, page_size, tables })
    }

    /// Present rows of a table, in page/row order. Returns (page heap slice, row offset) pairs.
    pub fn rows(&self, page_type: u32) -> Vec<(usize, usize)> {
        let mut out = vec![];
        let Some(&(_, _, first, last)) = self.tables.iter().find(|t| t.0 == page_type) else {
            return out;
        };
        let ps = self.page_size;
        let mut page = first;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            let base = page as usize * ps;
            if base + ps > self.bytes.len() {
                break;
            }
            let pg = &self.bytes[base..base + ps];
            let flags = pg[27];
            let ptype = get_u32(pg, 8);
            if ptype == page_type && flags & 0x40 == 0 {
                let packed = pg[24] as u32 | (pg[25] as u32) << 8 | (pg[26] as u32) << 16;
                let nrows = (packed & 0x1fff) as usize;
                for g in 0..(nrows + 15) / 16 {
                    let gbase = ps - g * GROUP_BYTES;
                    if gbase < 6 {
                        break;
                    }
                    let flags = get_u16(pg, gbase - 4);
                    for i in 0..16 {
                        if g * 16 + i >= nrows {
                            break;
                        }
                        if flags & (1 << i) != 0 {
                            let off = get_u16(pg, gbase - 6 - 2 * i) as usize;
                            out.push((base + HEAP_START, off));
                        }
                    }
                }
            }
            if page == last {
                break;
            }
            let next = get_u32(pg, 12);
            if next == page || next as usize * ps >= self.bytes.len() {
                break;
            }
            page = next;
        }
        out
    }

    /// The data pages of a table that hold row offsets, in chain order, as whole pages.
    pub fn data_pages(&self, page_type: u32) -> Vec<Vec<u8>> {
        let mut out = vec![];
        let Some(&(_, _, first, last)) = self.tables.iter().find(|t| t.0 == page_type) else {
            return out;
        };
        let ps = self.page_size;
        if ps != PAGE {
            return out;
        }
        let mut page = first;
        for _ in 0..100_000 {
            let base = page as usize * ps;
            if base + ps > self.bytes.len() {
                break;
            }
            let pg = &self.bytes[base..base + ps];
            let packed = pg[24] as u32 | (pg[25] as u32) << 8 | (pg[26] as u32) << 16;
            if get_u32(pg, 8) == page_type && pg[27] & 0x40 == 0 && packed & 0x1fff != 0 {
                out.push(pg.to_vec());
            }
            if page == last {
                break;
            }
            let next = get_u32(pg, 12);
            if next == page || next as usize * ps >= self.bytes.len() {
                break;
            }
            page = next;
        }
        out
    }

    pub fn row_bytes(&self, heap: usize, off: usize) -> &[u8] {
        let start = (heap + off).min(self.bytes.len());
        let page_end = ((heap - HEAP_START) + self.page_size).min(self.bytes.len());
        &self.bytes[start..page_end.max(start)]
    }
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
        for s in ["", "Pink", "Ünïcødé ✓", &"x".repeat(300)] {
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
