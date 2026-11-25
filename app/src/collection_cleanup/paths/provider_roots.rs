//! Local provider registrations protect custom sync folders even when all media are offline.
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

const MAX_CONFIG: u64 = 256 * 1024;
const MAX_ROOTS: usize = 256;

pub(super) struct Roots {
    pub(super) paths: Vec<PathBuf>,
    pub(super) complete: bool,
}

impl Roots {
    fn add(&mut self, path: PathBuf) {
        if !path.is_absolute()
            || path.components().any(|p| p == Component::ParentDir)
            || path.to_str().is_none_or(|s| s.contains('\0'))
        {
            self.complete = false;
        } else if !self.paths.contains(&path) {
            if self.paths.len() < MAX_ROOTS {
                self.paths.push(path);
            } else {
                self.complete = false;
            }
        }
    }

    fn dropbox(&mut self, config: &Path) {
        match read_config(config) {
            Ok(Some(data)) => {
                let parsed = serde_json::from_slice::<serde_json::Value>(
                    data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data),
                );
                let Ok(serde_json::Value::Object(accounts)) = parsed else {
                    self.complete = false;
                    return;
                };
                for account in accounts.values() {
                    if let Some(path) = account.get("path").and_then(|v| v.as_str()) {
                        self.add(PathBuf::from(path));
                    } else {
                        self.complete = false;
                    }
                }
            }
            Ok(None) => {}
            Err(_) => self.complete = false,
        }
    }
}

fn read_config(path: &Path) -> io::Result<Option<Vec<u8>>> {
    // Never follow an alternate/network path or hydrate a provider placeholder.
    #[cfg(windows)]
    if !super::startup_volume(path) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if !path.is_absolute() || path.components().any(|p| p == Component::ParentDir) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let ancestors: Vec<_> = path.ancestors().collect();
    for &entry in ancestors.iter().rev() {
        let metadata = match fs::symlink_metadata(entry) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if super::held_metadata(&metadata).is_some()
            || (entry != path && !metadata.is_dir())
            || (entry == path && (!metadata.is_file() || metadata.len() > MAX_CONFIG))
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        #[cfg(target_os = "macos")]
        if !super::startup_volume(entry) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
    }
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(MAX_CONFIG + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_CONFIG {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(Some(data))
}

pub(super) fn discover(home: &Path) -> Roots {
    let mut roots = Roots {
        paths: Vec::new(),
        complete: true,
    };
    #[cfg(windows)]
    {
        use std::env;
        for name in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(location) = env::var_os(name) {
                roots.dropbox(&PathBuf::from(location).join("Dropbox/info.json"));
            } else {
                roots.complete = false;
            }
        }
        for name in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(path) = env::var_os(name) {
                roots.add(path.into());
            }
        }
        if registry::read_roots(&mut roots).is_err() {
            roots.complete = false;
        }
    }
    #[cfg(not(windows))]
    {
        roots.dropbox(&home.join(".dropbox/info.json"));
        roots.dropbox(&home.join("Library/Application Support/Dropbox/info.json"));
        roots.add(home.join("Library/CloudStorage"));
        roots.add(home.join("Library/Mobile Documents"));
    }
    #[cfg(windows)]
    let _ = home;
    roots
}

#[cfg(windows)]
mod registry {
    use super::Roots;
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        },
        System::Registry::*,
    };

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn open(parent: HKEY, name: &[u16]) -> Result<Option<Key>, ()> {
        let mut key = ptr::null_mut();
        match unsafe { RegOpenKeyExW(parent, name.as_ptr(), 0, KEY_READ, &mut key) } {
            ERROR_SUCCESS => Ok(Some(Key(key))),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
            _ => Err(()),
        }
    }

    pub(super) fn read_roots(roots: &mut Roots) -> Result<(), ()> {
        let branch: Vec<_> = r"Software\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager"
            .encode_utf16()
            .chain([0])
            .collect();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let Some(manager) = open(hive, &branch)? else {
                continue;
            };
            for i in 0..=256 {
                let mut name = [0u16; 256];
                let mut n = name.len() as u32;
                match unsafe {
                    RegEnumKeyExW(
                        manager.0,
                        i,
                        name.as_mut_ptr(),
                        &mut n,
                        ptr::null(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    )
                } {
                    ERROR_NO_MORE_ITEMS => break,
                    ERROR_SUCCESS if i < 256 => {}
                    _ => return Err(()),
                }
                let mut table = name[..n as usize].to_vec();
                table.extend(r"\UserSyncRoots".encode_utf16().chain([0]));
                if let Some(key) = open(manager.0, &table)? {
                    read_entries(&key, roots)?;
                }
            }
        }
        Ok(())
    }

    fn read_entries(key: &Key, roots: &mut Roots) -> Result<(), ()> {
        // Registry registrations are lexical protection only. Never open their target folders.
        let mut data = vec![0u16; 32768];
        for i in 0..=64 {
            let mut sid = [0u16; 256];
            let mut len = sid.len() as u32;
            match unsafe {
                RegEnumValueW(
                    key.0,
                    i,
                    sid.as_mut_ptr(),
                    &mut len,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            } {
                ERROR_NO_MORE_ITEMS => return Ok(()),
                ERROR_SUCCESS if i < 64 => {}
                _ => return Err(()),
            }
            // RegGetValue also expands REG_EXPAND_SZ and provides a terminated result.
            let mut bytes = (data.len() * 2) as u32;
            let mut kind = REG_NONE;
            let status = unsafe {
                RegGetValueW(
                    key.0,
                    ptr::null(),
                    sid.as_ptr(),
                    RRF_RT_ANY,
                    &mut kind,
                    data.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            if status != ERROR_SUCCESS || kind != REG_SZ || bytes < 2 || bytes % 2 != 0 {
                return Err(());
            }
            let path = &data[..bytes as usize / 2 - 1];
            if path.contains(&0) || path.contains(&(b'%' as u16)) {
                return Err(());
            }
            roots.add(OsString::from_wide(path).into());
        }
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_custom_paths_remain_protected_and_incomplete_data_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(not(windows))]
        let root = dir.path().canonicalize().unwrap();
        #[cfg(windows)]
        let root = dir.path().to_path_buf();
        let config = root.join("info.json");
        let offline = root.join("custom location");
        let mut roots = Roots {
            paths: Vec::new(),
            complete: true,
        };
        roots.dropbox(&config);
        assert!(roots.complete && roots.paths.is_empty());
        fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({"business": {"path": offline}})).unwrap(),
        )
        .unwrap();
        roots.dropbox(&config);
        assert!(roots.complete);
        assert_eq!(roots.paths, [offline]);
        fs::write(&config, b"{invalid").unwrap();
        roots.dropbox(&config);
        assert!(!roots.complete);
    }
}
