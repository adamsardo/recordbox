//! Player settings files written to /PIONEER on a USB (MYSETTING.DAT, MYSETTING2.DAT,
//! DEVSETTING.DAT, DJMMYSETTING.DAT).
//!
//! Layout (rekordbox-rea reports/native-usb-settings.md, rekordcrate src/setting.rs):
//! u32 LE header length 96, three 32-byte NUL-padded strings (vendor, product, version),
//! u32 LE payload length, the payload, then a u32 LE CRC-16/XMODEM of the payload only.
//! Most option bytes use the high bit as "set" and the low seven bits as the value.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingFile {
    My,
    My2,
    Dev,
    Djm,
}

pub struct Field {
    pub file: SettingFile,
    /// Offset in the payload.
    pub offset: usize,
    pub name: &'static str,
    /// (label as rekordbox shows it, stored byte).
    pub options: &'static [(&'static str, u8)],
}

/// rekordbox 7.2.19's default payloads (MySettingData::setDefaultContent and
/// DeviceSettingData::setDefaultContent, rekordbox-rea US-04). MYSETTING.DAT is identical to a
/// real rekordbox export; MYSETTING2/DEVSETTING carry the fields added since rekordbox 6.
pub const DEFAULT_MY: [u8; 40] = hex40(b"78563412020000008183818881018281810101018280808180818000008100008181818081800000");
pub const DEFAULT_MY2: [u8; 40] = hex40(b"81808383818080000000808185000000000000000000000000000000000000000000000000000000");
pub const DEFAULT_DEV: [u8; 32] = hex32(b"7856341201000000010101010101010101000000000000000000000000000000");
/// DJM mixer settings as rekordbox writes them by default (rekordcrate demo export).
pub const DEFAULT_DJM: [u8; 52] = hex52(b"7856341201000000200000008182808081818081808085828000000000000000000000000000000000000000000000000000000000");

const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => c - b'A' + 10,
    }
}
macro_rules! hexn {
    ($name:ident, $n:expr) => {
        const fn $name(s: &[u8]) -> [u8; $n] {
            let mut out = [0u8; $n];
            let mut i = 0;
            while i < $n {
                out[i] = nib(s[2 * i]) << 4 | nib(s[2 * i + 1]);
                i += 1;
            }
            out
        }
    };
}
hexn!(hex40, 40);
hexn!(hex32, 32);
hexn!(hex52, 52);

/// CRC-16/XMODEM: polynomial 0x1021, initial 0, no reflection, no final XOR.
pub fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn padded(s: &str) -> [u8; 32] {
    let mut b = [0u8; 32];
    let n = s.len().min(31);
    b[..n].copy_from_slice(&s.as_bytes()[..n]);
    b
}

/// The settings PrepDeck writes to a USB, as raw payloads (so bytes PrepDeck does not interpret
/// are kept as they are).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbSettings {
    pub my: Vec<u8>,
    pub my2: Vec<u8>,
    pub dev: Vec<u8>,
    pub djm: Vec<u8>,
}

impl Default for UsbSettings {
    fn default() -> Self {
        UsbSettings { my: DEFAULT_MY.to_vec(), my2: DEFAULT_MY2.to_vec(), dev: DEFAULT_DEV.to_vec(), djm: DEFAULT_DJM.to_vec() }
    }
}

/// Waveform colour shown on the players (DEVSETTING payload 10): Blue 1, RGB 3, 3Band 4.
pub const DEV_WAVEFORM_COLOR: usize = 10;
/// Waveform current position on the players (DEVSETTING payload 13): Center 1, Left 2.
pub const DEV_WAVEFORM_POSITION: usize = 13;

impl UsbSettings {
    fn payload_mut(&mut self, f: SettingFile) -> &mut Vec<u8> {
        match f {
            SettingFile::My => &mut self.my,
            SettingFile::My2 => &mut self.my2,
            SettingFile::Dev => &mut self.dev,
            SettingFile::Djm => &mut self.djm,
        }
    }
    pub fn get(&self, f: SettingFile, offset: usize) -> u8 {
        let p = match f {
            SettingFile::My => &self.my,
            SettingFile::My2 => &self.my2,
            SettingFile::Dev => &self.dev,
            SettingFile::Djm => &self.djm,
        };
        p.get(offset).copied().unwrap_or(0)
    }
    pub fn set(&mut self, f: SettingFile, offset: usize, v: u8) {
        let p = self.payload_mut(f);
        if offset < p.len() {
            p[offset] = v;
        }
    }
    /// Repair payloads of the wrong length (e.g. from an older preferences file).
    pub fn normalized(mut self) -> Self {
        let d = UsbSettings::default();
        if self.my.len() != 40 {
            self.my = d.my.clone();
        }
        if self.my2.len() != 40 {
            self.my2 = d.my2.clone();
        }
        if self.dev.len() != 32 {
            self.dev = d.dev.clone();
        }
        if self.djm.len() != 52 {
            self.djm = d.djm;
        }
        self
    }

    /// The settings to write to a drive: where the drive already has a valid file (right
    /// length and checksum), its bytes are kept and only the fields PrepDeck lets you set
    /// (Preferences > DJ System) are written over them, so values set elsewhere survive.
    pub fn merged_onto_drive(&self, pioneer_dir: &std::path::Path) -> UsbSettings {
        let mut out = self.clone().normalized();
        let read = |name: &str, len: usize, whole: bool| -> Option<Vec<u8>> {
            let b = std::fs::read(pioneer_dir.join(name)).ok()?;
            if b.len() != 108 + len || u32::from_le_bytes(b.get(100..104)?.try_into().ok()?) as usize != len {
                return None;
            }
            let crc = u32::from_le_bytes(b[104 + len..108 + len].try_into().ok()?) as u16;
            let covered = if whole { &b[..104 + len] } else { &b[104..104 + len] };
            (crc == crc16_xmodem(covered)).then(|| b[104..104 + len].to_vec())
        };
        let managed = |f: SettingFile| -> Vec<usize> {
            let mut v: Vec<usize> = crate::export::settings_fields::MY_SETTINGS.iter().filter(|x| x.file == f).map(|x| x.offset).collect();
            if f == SettingFile::Dev {
                v.extend([DEV_WAVEFORM_COLOR, DEV_WAVEFORM_POSITION]);
            }
            v
        };
        for (f, name, len, whole) in [(SettingFile::My, "MYSETTING.DAT", 40, false), (SettingFile::My2, "MYSETTING2.DAT", 40, false), (SettingFile::Dev, "DEVSETTING.DAT", 32, false), (SettingFile::Djm, "DJMMYSETTING.DAT", 52, true)] {
            if let Some(mut base) = read(name, len, whole) {
                let mine = out.payload_mut(f).clone();
                for o in managed(f) {
                    if o < len {
                        base[o] = mine[o];
                    }
                }
                *out.payload_mut(f) = base;
            }
        }
        out
    }

    /// (file name, bytes) for each of the four files.
    pub fn files(&self) -> Vec<(&'static str, Vec<u8>)> {
        let s = self.clone().normalized();
        vec![
            ("MYSETTING.DAT", build("PIONEER", "rekordbox", "0.001", &s.my)),
            ("MYSETTING2.DAT", build("PIONEER", "rekordbox", "0.001", &s.my2)),
            ("DEVSETTING.DAT", build("PIONEER DJ", "rekordbox", "7.2.19", &s.dev)),
            ("DJMMYSETTING.DAT", build_whole_crc("PioneerDJ", "rekordbox", "1.000", &s.djm)),
        ]
    }
}

pub fn build(vendor: &str, product: &str, version: &str, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(108 + payload.len());
    b.extend_from_slice(&96u32.to_le_bytes());
    b.extend_from_slice(&padded(vendor));
    b.extend_from_slice(&padded(product));
    b.extend_from_slice(&padded(version));
    b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    b.extend_from_slice(payload);
    b.extend_from_slice(&(crc16_xmodem(payload) as u32).to_le_bytes());
    b
}

/// DJMMYSETTING.DAT's checksum covers every byte before it (header and strings too), unlike the
/// other three files (rekordcrate src/setting.rs).
pub fn build_whole_crc(vendor: &str, product: &str, version: &str, payload: &[u8]) -> Vec<u8> {
    let mut b = build(vendor, product, version, payload);
    let n = b.len() - 4;
    let crc = crc16_xmodem(&b[..n]) as u32;
    b[n..].copy_from_slice(&crc.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_bytes_kept_except_managed_fields() {
        let dir = std::env::temp_dir().join(format!("prepdeck_settings_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The drive's DEVSETTING has an unmanaged byte (20) set by something else, and a
        // waveform colour that differs from the user's preference.
        let mut drive = UsbSettings::default();
        drive.dev[20] = 0x42;
        drive.dev[DEV_WAVEFORM_COLOR] = 1;
        for (n, b) in drive.files() {
            std::fs::write(dir.join(n), b).unwrap();
        }
        let mut mine = UsbSettings::default();
        mine.dev[DEV_WAVEFORM_COLOR] = 4;
        let merged = mine.merged_onto_drive(&dir);
        assert_eq!(merged.dev[20], 0x42);
        assert_eq!(merged.dev[DEV_WAVEFORM_COLOR], 4);
        // A damaged file (bad checksum) is not used as the base.
        let mut bad = std::fs::read(dir.join("DEVSETTING.DAT")).unwrap();
        let n = bad.len();
        bad[n - 4] ^= 0xff;
        std::fs::write(dir.join("DEVSETTING.DAT"), bad).unwrap();
        assert_eq!(mine.merged_onto_drive(&dir).dev[20], UsbSettings::default().dev[20]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31c3);
        assert_eq!(crc16_xmodem(&DEFAULT_MY), 0x7a48);
        assert_eq!(crc16_xmodem(&DEFAULT_MY2), 0x9c7a);
        assert_eq!(crc16_xmodem(&DEFAULT_DEV), 0x98e1);
    }

    /// The default MYSETTING.DAT and DJMMYSETTING.DAT are byte-identical to files written by
    /// rekordbox (rekordcrate's real exports), when that checkout is present.
    #[test]
    fn defaults_match_real_exports() {
        let base = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/format-oracles/Holzhaus__rekordcrate/data/complete_export/demo_tracks/PIONEER");
        if !base.is_dir() {
            return;
        }
        let files = UsbSettings::default().files();
        for name in ["MYSETTING.DAT", "DJMMYSETTING.DAT"] {
            let real = std::fs::read(base.join(name)).unwrap();
            let mine = &files.iter().find(|f| f.0 == name).unwrap().1;
            assert_eq!(mine, &real, "{name}");
        }
        // The other two differ only in the payload fields added after that export's version.
        for name in ["MYSETTING2.DAT", "DEVSETTING.DAT"] {
            let real = std::fs::read(base.join(name)).unwrap();
            let mine = &files.iter().find(|f| f.0 == name).unwrap().1;
            assert_eq!(mine.len(), real.len(), "{name}");
            assert_eq!(&mine[..68], &real[..68], "{name} header");
        }
    }
}
