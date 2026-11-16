//! Comparison of this app's analysis files with the original's own files for the same audio
//! (read-only). PREPDECK_ANLZ_PAIRS = tsv "audio path\toriginal ANLZ base path (without ext)".

use std::collections::BTreeMap;

pub fn sections(d: &[u8]) -> Vec<([u8; 4], Vec<u8>, Vec<u8>)> {
    let mut out = vec![];
    if d.len() < 12 {
        return out;
    }
    let mut o = u32::from_be_bytes([d[4], d[5], d[6], d[7]]) as usize;
    while o + 12 <= d.len() {
        let tag = [d[o], d[o + 1], d[o + 2], d[o + 3]];
        let hl = u32::from_be_bytes([d[o + 4], d[o + 5], d[o + 6], d[o + 7]]) as usize;
        let tl = u32::from_be_bytes([d[o + 8], d[o + 9], d[o + 10], d[o + 11]]) as usize;
        if tl < 12 || o + tl > d.len() || hl > tl {
            break;
        }
        out.push((tag, d[o + 12..o + hl].to_vec(), d[o + hl..o + tl].to_vec()));
        o += tl;
    }
    out
}

pub fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 2 {
        return 0.0;
    }
    let ma = a[..n].iter().sum::<f64>() / n as f64;
    let mb = b[..n].iter().sum::<f64>() / n as f64;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..n {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    num / (da * db).sqrt().max(1e-12)
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

#[derive(Default)]
pub struct Stats {
    pub vals: BTreeMap<String, Vec<f64>>,
    pub notes: BTreeMap<String, usize>,
}

impl Stats {
    fn add(&mut self, k: &str, v: f64) {
        if v.is_finite() {
            self.vals.entry(k.to_string()).or_default().push(v);
        }
    }
    fn note(&mut self, k: String) {
        *self.notes.entry(k).or_default() += 1;
    }
}

/// Values of one channel of a waveform body.
fn chan(body: &[u8], stride: usize, k: usize, f: impl Fn(u32) -> u32) -> Vec<f64> {
    body.chunks_exact(stride).map(|c| f(c[k] as u32) as f64).collect()
}

pub fn compare_pair(mine: &[(&str, Vec<u8>)], theirs: &[(&str, Vec<u8>)], st: &mut Stats) {
    for ((ext, m), (_, t)) in mine.iter().zip(theirs) {
        let ms = sections(m);
        let ts = sections(t);
        let mt: Vec<String> = ms.iter().map(|s| String::from_utf8_lossy(&s.0).to_string()).collect();
        let tt: Vec<String> = ts.iter().map(|s| String::from_utf8_lossy(&s.0).to_string()).collect();
        for t in &tt {
            if !mt.contains(t) {
                st.note(format!("{ext}: section {t} written by the original, not by this app"));
            }
        }
        for t in &mt {
            if !tt.contains(t) {
                st.note(format!("{ext}: section {t} written by this app, not by the original"));
            }
        }
        let find = |v: &Vec<([u8; 4], Vec<u8>, Vec<u8>)>, tag: &[u8; 4]| v.iter().find(|s| &s.0 == tag).cloned();
        for tag in [b"PQTZ", b"PWAV", b"PWV2", b"PWV3", b"PWV4", b"PWV5", b"PWV6", b"PWV7", b"PVBR"] {
            let (Some(a), Some(b)) = (find(&ms, tag), find(&ts, tag)) else { continue };
            let name = String::from_utf8_lossy(tag).to_string();
            if a.1 != b.1 && tag != b"PQTZ" && tag != b"PWV3" && tag != b"PWV5" && tag != b"PWV7" {
                st.note(format!("{name}: header differs ({} vs {})", hex(&a.1), hex(&b.1)));
            }
            match tag {
                b"PQTZ" => {
                    let beats = |x: &[u8]| -> Vec<(u16, u16, u32)> {
                        x.chunks_exact(8).map(|c| (u16::from_be_bytes([c[0], c[1]]), u16::from_be_bytes([c[2], c[3]]), u32::from_be_bytes([c[4], c[5], c[6], c[7]]))).collect()
                    };
                    let (ba, bb) = (beats(&a.2), beats(&b.2));
                    st.add("PQTZ beat count difference", ba.len() as f64 - bb.len() as f64);
                    // align by nearest beat time
                    let mut diffs = vec![];
                    let mut same_bar = 0;
                    let mut j = 0;
                    for x in &bb {
                        while j + 1 < ba.len() && (ba[j + 1].2 as i64 - x.2 as i64).abs() < (ba[j].2 as i64 - x.2 as i64).abs() {
                            j += 1;
                        }
                        if let Some(y) = ba.get(j) {
                            diffs.push((y.2 as f64 - x.2 as f64).abs());
                            if y.0 == x.0 {
                                same_bar += 1;
                            }
                        }
                    }
                    diffs.sort_by(|p, q| p.partial_cmp(q).unwrap());
                    st.add("PQTZ median beat time difference (ms)", diffs.get(diffs.len() / 2).copied().unwrap_or(0.0));
                    st.add("PQTZ share of beats with the same beat-in-bar", same_bar as f64 / bb.len().max(1) as f64);
                    st.add("PQTZ same tempo field", if ba.first().map(|x| x.1) == bb.first().map(|x| x.1) { 1.0 } else { 0.0 });
                }
                b"PVBR" => {
                    st.add("PVBR original non-zero", if b.2.iter().any(|x| *x != 0) { 1.0 } else { 0.0 });
                }
                b"PWAV" | b"PWV3" => {
                    let h = |v: u32| v & 31;
                    let w = |v: u32| v >> 5;
                    let (ha, hb) = (chan(&a.2, 1, 0, h), chan(&b.2, 1, 0, h));
                    st.add(&format!("{name} entry count difference"), ha.len() as f64 - hb.len() as f64);
                    st.add(&format!("{name} height correlation"), pearson(&ha, &hb));
                    st.add(&format!("{name} height mean ratio (app/original)"), mean(&ha) / mean(&hb).max(1e-9));
                    st.add(&format!("{name} whiteness correlation"), pearson(&chan(&a.2, 1, 0, w), &chan(&b.2, 1, 0, w)));
                    st.add(&format!("{name} whiteness mean ratio"), mean(&chan(&a.2, 1, 0, w)) / mean(&chan(&b.2, 1, 0, w)).max(1e-9));
                }
                b"PWV2" => {
                    let (ha, hb) = (chan(&a.2, 1, 0, |v| v & 15), chan(&b.2, 1, 0, |v| v & 15));
                    st.add("PWV2 height correlation", pearson(&ha, &hb));
                    st.add("PWV2 height mean ratio", mean(&ha) / mean(&hb).max(1e-9));
                }
                b"PWV4" => {
                    for k in 0..6 {
                        let (x, y) = (chan(&a.2, 6, k, |v| v), chan(&b.2, 6, k, |v| v));
                        st.add(&format!("PWV4 byte {k} correlation"), pearson(&x, &y));
                        st.add(&format!("PWV4 byte {k} mean ratio"), mean(&x) / mean(&y).max(1e-9));
                    }
                }
                b"PWV5" => {
                    let word = |x: &[u8]| -> Vec<u32> { x.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]) as u32).collect() };
                    let (wa, wb) = (word(&a.2), word(&b.2));
                    st.add("PWV5 entry count difference", wa.len() as f64 - wb.len() as f64);
                    for (nm, sh, mask) in [("height", 2, 31), ("red", 13, 7), ("green", 10, 7), ("blue", 7, 7)] {
                        let x: Vec<f64> = wa.iter().map(|w| ((w >> sh) & mask) as f64).collect();
                        let y: Vec<f64> = wb.iter().map(|w| ((w >> sh) & mask) as f64).collect();
                        st.add(&format!("PWV5 {nm} correlation"), pearson(&x, &y));
                        st.add(&format!("PWV5 {nm} mean ratio"), mean(&x) / mean(&y).max(1e-9));
                    }
                }
                b"PWV6" | b"PWV7" => {
                    if tag == b"PWV7" {
                        st.add("PWV7 entry count difference", (a.2.len() / 3) as f64 - (b.2.len() / 3) as f64);
                    }
                    for (k, band) in ["low", "mid", "high"].iter().enumerate() {
                        let (x, y) = (chan(&a.2, 3, k, |v| v), chan(&b.2, 3, k, |v| v));
                        st.add(&format!("{name} {band} correlation"), pearson(&x, &y));
                        st.add(&format!("{name} {band} mean ratio"), mean(&x) / mean(&y).max(1e-9));
                    }
                }
                _ => {}
            }
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore]
    fn compare_with_original_files() {
        let Ok(f) = std::env::var("PREPDECK_ANLZ_PAIRS") else { return };
        let pairs: Vec<(String, String)> = std::fs::read_to_string(f).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
        let stats = std::sync::Mutex::new(Stats::default());
        std::thread::scope(|s| {
            for ch in pairs.chunks((pairs.len() / 8).max(1)) {
                let stats = &stats;
                s.spawn(move || {
                    for (audio, base) in ch {
                        let Ok(r) = crate::analysis::analyze_file(audio, &Default::default()) else { continue };
                        let beats = r.grid.as_ref().map(|g| g.beats(r.duration)).unwrap_or_default();
                        let name = std::path::Path::new(audio).file_name().unwrap().to_string_lossy().to_string();
                        let beats = r.grid.as_ref().map(|g| g.beats(r.raw_samples as f64 / r.sample_rate.max(1) as f64)).unwrap_or_default();
                        let files = super::super::anlz::build(&format!("/Contents/A/B/{name}"), &beats, &r.wave, &[], r.raw_samples, r.sample_rate);
                        let read = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
                        let mine = vec![("DAT", files.dat), ("EXT", files.ext), ("2EX", files.ex2)];
                        let theirs = vec![("DAT", read("DAT")), ("EXT", read("EXT")), ("2EX", read("2EX"))];
                        let mut local = Stats::default();
                        compare_pair(&mine, &theirs, &mut local);
                        let mut g = stats.lock().unwrap();
                        for (k, v) in local.vals {
                            g.vals.entry(k).or_default().extend(v);
                        }
                        for (k, v) in local.notes {
                            *g.notes.entry(k).or_default() += v;
                        }
                    }
                });
            }
        });
        let st = stats.into_inner().unwrap();
        for (k, v) in &st.vals {
            let mut s = v.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            eprintln!("{:<52} n={:3} mean {:8.3} median {:8.3}", k, v.len(), mean(v), s[s.len() / 2]);
        }
        for (k, n) in &st.notes {
            eprintln!("NOTE x{n}: {k}");
        }
    }
}
