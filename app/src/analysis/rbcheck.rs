//! Checks of recordbox's decoding and analysis against what the installed rekordbox stored for
//! the user's library (read from a snapshot copy of its database and from its analysis files;
//! nothing of rekordbox is changed). All tests here are opt-in probes (`--ignored`).

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    /// Local tracks of the installed rekordbox library that have analysis files:
    /// (audio path, ANLZ base without extension, rekordbox's key index or -1).
    pub(crate) fn library() -> Vec<(String, String, i32)> {
        let dest = std::env::temp_dir().join(format!("recordbox_rbcheck_{}", std::process::id()));
        let Ok(Some(lib)) = crate::rbimport::load_installed(&dest) else { return vec![] };
        let _ = std::fs::remove_dir_all(&dest);
        let mut v: Vec<(String, String, i32)> = lib
            .tracks
            .iter()
            .filter_map(|t| Some((t.xt.track.path.clone(), t.anlz_base.as_ref()?.to_string_lossy().to_string(), t.xt.track.key)))
            .filter(|r| Path::new(&r.0).exists())
            .collect();
        v.sort();
        if let Ok(f) = std::env::var("PREPDECK_RBCHECK_FILTER") {
            v.retain(|r| r.0.contains(&f));
        }
        v
    }

    /// The number of PVDI levels rekordbox's vocal detector makes for `frames` sample frames at
    /// `sr` (`VocalDetector::setData`: (int)(n · 22050 / sr) per block of 2·sr frames, then
    /// ceil(total / 1024)).
    pub(crate) fn pvdi_len(frames: usize, sr: u32) -> usize {
        let block = 2 * sr as usize;
        let mut out = 0usize;
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(block);
            out += (n as f64 * 22050.0 / sr as f64) as usize;
            done += n;
        }
        out.div_ceil(1024)
    }

    fn section_count(sec: &std::collections::HashMap<String, Vec<u8>>, tag: &str) -> Option<usize> {
        let s = sec.get(tag)?;
        let hlen = u32::from_be_bytes(s.get(4..8)?.try_into().ok()?) as usize;
        let _ = hlen;
        // PVDI: u32 count at 20; PWV3 / PWV5: u32 count at 16 (after a u32 entry size at 12).
        match tag {
            "PVDI" => Some(u32::from_be_bytes(s.get(20..24)?.try_into().ok()?) as usize),
            _ => Some(u32::from_be_bytes(s.get(16..20)?.try_into().ok()?) as usize),
        }
    }

    /// recordbox's decode of PREPDECK_AUDIO, as the 16-bit values rekordbox's reader hands to the
    /// analysis (× 32768, clamped, truncated), against a reference s16le stereo file
    /// (PREPDECK_REF, e.g. `flac -d --force-raw-format --endian=little --sign=signed` or
    /// `ffmpeg -f s16le`). Prints lengths, the best lag within ±PREPDECK_LAG frames (default
    /// 4096) and how many samples are identical at that lag. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn compare_with_pcm() {
        let (Ok(path), Ok(refp)) = (std::env::var("PREPDECK_AUDIO"), std::env::var("PREPDECK_REF")) else { return };
        let max_lag: i64 = std::env::var("PREPDECK_LAG").ok().and_then(|v| v.parse().ok()).unwrap_or(4096);
        let d = crate::audio::decode_file(&path).unwrap();
        let mine: Vec<i16> = d.samples.iter().map(|v| crate::analysis::rb_beat1::reader_sample(*v) as i16).collect();
        let theirs: Vec<i16> = std::fs::read(&refp).unwrap().chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        let (nm, nt) = (mine.len() as i64 / 2, theirs.len() as i64 / 2);
        // Lag by matching a window of loud audio from the middle.
        let mid = (nm / 2).max(0);
        let win = 4096i64.min(nm - mid);
        let mut best = (i64::MAX, 0i64);
        for lag in -max_lag..=max_lag {
            let mut err = 0i64;
            for k in 0..win {
                let (a, b) = (mid + k, mid + k + lag);
                if b < 0 || b >= nt {
                    err = i64::MAX;
                    break;
                }
                err += (mine[2 * a as usize] as i64 - theirs[2 * b as usize] as i64).abs();
            }
            if err < best.0 {
                best = (err, lag);
            }
        }
        let lag = best.1;
        let (mut same, mut n, mut maxd) = (0usize, 0usize, 0i32);
        for a in 0..nm {
            let b = a + lag;
            if b < 0 || b >= nt {
                continue;
            }
            for c in 0..2 {
                let (x, y) = (mine[(2 * a) as usize + c], theirs[(2 * b) as usize + c]);
                n += 1;
                same += (x == y) as usize;
                maxd = maxd.max((x as i32 - y as i32).abs());
            }
        }
        eprintln!("PCM frames mine {nm} reference {nt}; lag {lag} (reference = mine shifted by {lag}); identical {same}/{n}, max difference {maxd}");
    }

    /// Decoded length of every local library track against the lengths rekordbox stored: the
    /// vocal levels (PVDI, exact to 1024 samples at 22,050 Hz) and the detail waveform (PWV5,
    /// 150 entries per second). Prints one line per track and a summary per format.
    /// PREPDECK_RBCHECK_FILTER = path part. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn library_lengths() {
        let rows = library();
        let mut by_ext: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
        for (path, base, _) in &rows {
            let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let ex2 = crate::export::anlz::read_sections(Path::new(&format!("{base}.2EX")));
            let ext_s = crate::export::anlz::read_sections(Path::new(&format!("{base}.EXT")));
            let pv = section_count(&ex2, "PVDI");
            let w5 = section_count(&ext_s, "PWV5");
            let dec = match crate::audio::decode_file(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("{ext}\tDECODE-ERROR {e}\t{path}");
                    continue;
                }
            };
            let frames = dec.samples.len() / 2;
            let mine = pvdi_len(frames, dec.sr);
            let w5_mine = (frames as f64 / dec.sr as f64 * 150.0).ceil() as usize;
            let e = by_ext.entry(ext.clone()).or_default();
            e.0 += 1;
            if let Some(p) = pv {
                e.1 += (p == mine) as usize;
            }
            e.2 += pv.is_none() as usize;
            eprintln!(
                "{ext}\tsr {}\tframes {frames}\tpvdi rb {} mine {mine}{}\tpwv5 rb {} est {w5_mine}\t{}",
                dec.sr,
                pv.map(|v| v.to_string()).unwrap_or("-".into()),
                if pv.is_some() && pv != Some(mine) { " DIFF" } else { "" },
                w5.map(|v| v.to_string()).unwrap_or("-".into()),
                path
            );
        }
        for (ext, (n, same, none)) in by_ext {
            eprintln!("LENGTHS {ext}: {n} tracks, PVDI length equal {same}/{} (no PVDI: {none})", n - none);
        }
    }
}

#[cfg(test)]
mod mp3probe {
    /// PREPDECK_AUDIO = MP3: frames from rekordbox's mpg123 (vocal path and clean-only path) and
    /// from decode_file.
    #[test]
    #[ignore]
    fn mp3_paths() {
        let Ok(p) = std::env::var("PREPDECK_AUDIO") else { return };
        for p in p.split('|') {
            let v = crate::vocal_rb_mp3::decode(p).map(|x| x.0.len() / 2);
            let c = crate::vocal_rb_mp3::decode_clean(p).map(|x| x.0.len() / 2);
            let d = crate::audio::decode_file(p).map(|x| x.samples.len() / 2).ok();
            eprintln!("MP3PATH vocal {v:?} clean {c:?} decode_file {d:?} :: {p}");
        }
    }
}

#[cfg(test)]
mod listprobe {
    /// Prints the library's audio path and ANLZ base for paths containing PREPDECK_RBCHECK_FILTER.
    #[test]
    #[ignore]
    fn list_library() {
        for (p, b, k) in super::tests::library() {
            eprintln!("LIB {p}\t{b}\t{k}");
        }
    }
}

#[cfg(test)]
mod aacprobe {
    /// Best lag (b = a shifted by lag) of two 16-bit stereo streams around their middle, and the
    /// share of identical samples / largest difference at that lag.
    pub(crate) fn align(a: &[i16], b: &[i16], max_lag: i64) -> (i64, usize, usize, i32) {
        let (na, nb) = (a.len() as i64 / 2, b.len() as i64 / 2);
        let mid = na / 3;
        let win = 8192i64.min(na - mid);
        let mut best = (i64::MAX, 0i64);
        for lag in -max_lag..=max_lag {
            let mut err = 0i64;
            for k in 0..win {
                let (x, y) = (mid + k, mid + k + lag);
                if y < 0 || y >= nb {
                    err = i64::MAX;
                    break;
                }
                err += (a[2 * x as usize] as i64 - b[2 * y as usize] as i64).abs();
            }
            if err < best.0 {
                best = (err, lag);
            }
        }
        let lag = best.1;
        let (mut same, mut n, mut maxd) = (0usize, 0usize, 0i32);
        for x in 0..na {
            let y = x + lag;
            if y < 0 || y >= nb {
                continue;
            }
            for c in 0..2 {
                let (p, q) = (a[(2 * x) as usize + c], b[(2 * y) as usize + c]);
                n += 1;
                same += (p == q) as usize;
                maxd = maxd.max((p as i32 - q as i32).abs());
            }
        }
        (lag, same, n, maxd)
    }

    /// PREPDECK_AUDIO = M4A files ('|'-separated): AudioToolbox decode as rekordbox does it
    /// against this application's decode (symphonia), as 16-bit reader values.
    #[test]
    #[ignore]
    fn aac_paths() {
        let Ok(list) = std::env::var("PREPDECK_AUDIO") else { return };
        for p in list.split('|') {
            let apple = crate::analysis::rb_reader::apple_aac::decode(p);
            let mine = crate::audio::decode_file(p).ok().map(|d| (d.samples.iter().map(|v| crate::analysis::rb_beat1::reader_sample(*v) as i16).collect::<Vec<i16>>(), d.sr));
            match (apple, mine) {
                (Some((a, ra)), Some((m, rm))) => {
                    let (lag, same, n, maxd) = align(&m, &a, 4096);
                    eprintln!("AAC apple frames {} ({ra} Hz), decode_file frames {} ({rm} Hz); apple = decode_file shifted by {lag}; identical {same}/{n}, max difference {maxd} :: {p}", a.len() / 2, m.len() / 2);
                }
                (a, m) => eprintln!("AAC apple {:?} decode_file {:?} :: {p}", a.map(|x| x.0.len() / 2), m.map(|x| x.0.len() / 2)),
            }
        }
    }
}

#[cfg(test)]
mod dbprobe {
    /// Runs PREPDECK_SQL on a snapshot copy of rekordbox's database and prints the rows.
    #[test]
    #[ignore]
    fn rb_sql() {
        let Ok(sql) = std::env::var("PREPDECK_SQL") else { return };
        let src = crate::rbimport::library_dir().unwrap();
        let dest = std::env::temp_dir().join(format!("recordbox_rbsql_{}", std::process::id()));
        let db = crate::rbimport::snapshot(&src, &dest).unwrap();
        let c = crate::rbimport::open_snapshot(&db).unwrap();
        let mut st = c.prepare(&sql).unwrap();
        let n = st.column_count();
        let names: Vec<String> = (0..n).map(|i| st.column_name(i).unwrap().to_string()).collect();
        eprintln!("SQL {}", names.join("\t"));
        for r in st.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join("\t"))).unwrap().flatten() {
            eprintln!("SQL {r}");
        }
        drop(st);
        drop(c);
        let _ = std::fs::remove_dir_all(&dest);
    }
}

#[cfg(test)]
mod gainprobe {
    /// Auto Gain and sample peak against what rekordbox stored (djmdMixerParam: the gain and the
    /// peak as float32 bits split into two 16-bit halves). Prints the loudness rekordbox's gain
    /// implies (-10 - 20·log10 gain), ours, and the difference, and a summary.
    /// PREPDECK_RBCHECK_FILTER = path part. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn autogain_vs_rekordbox() {
        let src = crate::rbimport::library_dir().unwrap();
        let dest = std::env::temp_dir().join(format!("recordbox_rbgain_{}", std::process::id()));
        let db = crate::rbimport::snapshot(&src, &dest).unwrap();
        let c = crate::rbimport::open_snapshot(&db).unwrap();
        let mut st = c.prepare("SELECT c.FolderPath, m.GainHigh, m.GainLow, m.PeakHigh, m.PeakLow FROM djmdMixerParam m JOIN djmdContent c ON c.ID = m.ContentID WHERE c.rb_local_deleted = 0 AND m.rb_local_deleted = 0").unwrap();
        let rows: Vec<(String, f32, f32)> = st
            .query_map([], |r| {
                let g = ((r.get::<_, i64>(1)? as u32) << 16) | (r.get::<_, i64>(2)? as u32 & 0xffff);
                let p = ((r.get::<_, i64>(3)? as u32) << 16) | (r.get::<_, i64>(4)? as u32 & 0xffff);
                Ok((r.get::<_, String>(0)?, f32::from_bits(g), f32::from_bits(p)))
            })
            .unwrap()
            .flatten()
            .filter(|r| std::path::Path::new(&r.0).exists())
            .filter(|r| std::env::var("PREPDECK_RBCHECK_FILTER").map(|f| r.0.contains(&f)).unwrap_or(true))
            .collect();
        drop(st);
        drop(c);
        let _ = std::fs::remove_dir_all(&dest);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((path, g, p)) = rows.get(i) else { break };
                    let Ok(d) = crate::audio::decode_file(path) else { continue };
                    let (l, pk) = crate::analysis::loudness_and_peak(&d);
                    res.lock().unwrap().push((path.clone(), -10.0 - 20.0 * (*g as f64).log10(), l, *p, pk));
                });
            }
        });
        let mut r = res.into_inner().unwrap();
        r.sort_by(|a, b| a.0.cmp(&b.0));
        let (mut n, mut within, mut exact_gain, mut peak_same, mut sum) = (0, 0, 0, 0, 0f64);
        let mut worst = 0f64;
        for (path, lr, lm, pr, pm) in &r {
            if !lr.is_finite() || !lm.is_finite() {
                eprintln!("GAIN no value: rekordbox {lr} ours {lm} :: {path}");
                continue;
            }
            let d = lm - lr;
            n += 1;
            sum += d;
            worst = worst.max(d.abs());
            within += (d.abs() < 0.01) as usize;
            let gm = 10f64.powf((-10.0 - lm) * 0.05) as f32;
            let gr = 10f64.powf((-10.0 - lr) * 0.05) as f32;
            exact_gain += (gm == gr) as usize;
            peak_same += (*pr == *pm) as usize;
            if d.abs() >= 0.01 || pr != pm {
                eprintln!("GAIN rekordbox {lr:.4} LUFS ours {lm:.4} (diff {d:+.4}); peak rekordbox {pr} ours {pm} :: {}", path.rsplit('/').next().unwrap_or(path));
            }
        }
        eprintln!("GAIN tracks {n}: loudness within 0.01 dB {within}, gain float identical {exact_gain}, peak identical {peak_same}; mean difference {:+.4} dB, largest {worst:.4} dB", sum / n.max(1) as f64);
    }
}

#[cfg(test)]
mod gridprobe {
    /// For library tracks matching PREPDECK_RBCHECK_FILTER: the first beats of rekordbox's stored
    /// grid (PQTZ) and of the grid this application makes (PREPDECK_BEAT / PREPDECK_HP_MODE).
    #[test]
    #[ignore]
    fn grid_vs_stored() {
        for (path, base, _) in super::tests::library() {
            let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
            let Some(q) = dat.get("PQTZ") else { continue };
            let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
            let rb: Vec<String> = (0..n.min(6)).map(|k| { let e = &q[24 + 8 * k..32 + 8 * k]; format!("{}@{}ms/{}", u16::from_be_bytes([e[0], e[1]]), u32::from_be_bytes([e[4], e[5], e[6], e[7]]), u16::from_be_bytes([e[2], e[3]])) }).collect();
            let Ok(d) = crate::audio::decode_file(&path) else { continue };
            let mono = d.mono();
            let (bpm, g) = crate::analysis::tempo_and_grid_for(&d, &mono, &crate::analysis::AnalysisSettings::default());
            let ours: Vec<String> = g.map(|g| g.beats(d.duration()).iter().filter(|b| b.time >= 0.0).take(6).map(|b| format!("{}@{}ms", b.beat, (b.time * 1000.0).round())).collect()).unwrap_or_default();
            eprintln!("GRID rekordbox {} beats {:?}\nGRID ours bpm {bpm} {:?} :: {}", n, rb, ours, path.rsplit('/').next().unwrap());
        }
    }
}
