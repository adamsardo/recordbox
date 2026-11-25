//! Interpret recorded paths using the conventions of their source OS. Only probe
//! ordinary audio under this user's startup-disk Music directory on the matching OS.
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

mod provider_roots;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PathStatus {
    Present,
    Missing,
    Cloud,
    External,
    Foreign,
    Outside,
    Unclassified,
    Unavailable,
}

impl PathStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Present => "Present on this computer",
            Self::Missing => "File absent from a reachable local folder",
            Self::Cloud => "Cloud, streaming or online-only",
            Self::External => "External, other drive or network",
            Self::Foreign => "Recorded on another operating system",
            Self::Outside => "Outside verified local Music folder",
            Self::Unclassified => "Unclassified path",
            Self::Unavailable => "Unavailable or could not verify",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PathFinding {
    pub os: &'static str,
    pub volume: String,
    pub status: PathStatus,
    pub local_identity: Option<PathBuf>,
    pub bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Os {
    Windows,
    Mac,
    Other,
}

impl Os {
    fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Mac => "macOS",
            Self::Other => "Unclassified",
        }
    }
}

/// Keep network addresses, other volumes and opaque URI forms out of host filesystem calls.
fn origin(path: &str) -> (Os, String, Option<PathStatus>) {
    if let Some(unc) = path.strip_prefix("//") {
        let mut parts = unc.split('/');
        return match (parts.next(), parts.next()) {
            (Some("?") | Some("."), _) => (
                Os::Windows,
                "Device namespace".into(),
                Some(PathStatus::Unclassified),
            ),
            (Some(server), Some(share)) if !server.is_empty() && !share.is_empty() => (
                Os::Windows,
                format!("//{server}/{share}").to_ascii_lowercase(),
                Some(PathStatus::External),
            ),
            _ => (
                Os::Windows,
                "Network share".into(),
                Some(PathStatus::External),
            ),
        };
    }
    if let [letter, b':', tail @ ..] = path.as_bytes() {
        if letter.is_ascii_alphabetic() {
            let root = format!("{}:", char::from(letter.to_ascii_uppercase()));
            let held = (!tail.starts_with(b"/")).then_some(PathStatus::Unclassified);
            return (Os::Windows, root, held);
        }
    }
    if path == "/Volumes" || path.starts_with("/Volumes/") {
        let volume = path.split('/').nth(2).unwrap_or("");
        return (
            Os::Mac,
            format!("/Volumes/{volume}"),
            Some(PathStatus::External),
        );
    }
    if path.starts_with("/Network/") {
        return (Os::Mac, "/Network".into(), Some(PathStatus::External));
    }
    if ["/Users/", "/System/", "/Applications/", "/Library/"]
        .iter()
        .any(|s| path.starts_with(s))
    {
        return (Os::Mac, "/".into(), None);
    }
    if let Some((scheme, _)) = path.split_once(':') {
        if !scheme.is_empty()
            && scheme
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        {
            let status = match scheme.to_ascii_lowercase().as_str() {
                "smb" | "afp" | "nfs" => PathStatus::External,
                "http" | "https" | "spotify" | "tidal" | "soundcloud" | "beatport"
                | "beatsource" | "dropbox" | "googledrive" | "icloud" => PathStatus::Cloud,
                _ => PathStatus::Unclassified,
            };
            return (
                Os::Other,
                format!("{}:", scheme.to_ascii_lowercase()),
                Some(status),
            );
        }
    }
    (
        Os::Other,
        "Unclassified".into(),
        Some(PathStatus::Unclassified),
    )
}

fn provider_component(component: &str) -> bool {
    let c = component.to_ascii_lowercase();
    c.contains("dropbox")
        || c.contains("onedrive")
        || c.contains("google drive")
        || c.contains("googledrive")
        || c.contains("icloud")
        || matches!(
            c.as_str(),
            "cloudstorage"
                | "mobile documents"
                | "my drive"
                | "shared drives"
                | "box sync"
                | "box drive"
        )
        || c.strip_prefix("contents_")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

fn within(path: &str, root: &str, os: Os) -> bool {
    if root == "/" {
        return path.starts_with('/');
    }
    let root = root.trim_end_matches('/');
    !root.is_empty()
        && path.get(..root.len()).is_some_and(|s| {
            if os == Os::Windows {
                s.eq_ignore_ascii_case(root)
            } else {
                s == root
            }
        })
        && matches!(path.as_bytes().get(root.len()), None | Some(b'/'))
}

// Case-insensitive APFS/HFS and Windows paths can use different Unicode spellings.
// Fold only the cloud exclusion key. Local-file identities keep their exact spelling.
fn cloud_key(path: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    path.strip_prefix("//?/")
        .unwrap_or(path)
        .nfkd()
        .flat_map(char::to_uppercase)
        .collect()
}

fn ambiguous_component(s: &str, os: Os) -> bool {
    if s.is_empty() || matches!(s, "." | "..") || s.chars().any(char::is_control) {
        return true;
    }
    if os != Os::Windows {
        return false;
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    s.ends_with([' ', '.'])
        || s.contains([':', '*', '?', '"', '<', '>', '|'])
        || matches!(
            stem.as_str(),
            "CON" | "AUX" | "NUL" | "PRN" | "CONIN$" | "CONOUT$"
        )
        || ["LPT", "COM"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|n| {
                matches!(
                    n,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

fn slash_path(path: &Path, os: Os) -> Option<String> {
    let raw = path.to_str()?;
    if os != Os::Windows {
        return Some(raw.to_owned());
    }
    // Known folders and canonicalize can return an extended-length DOS path.
    // Use the same comparison key as the drive path recorded by the library.
    let normalized = raw.replace('\\', "/");
    if let Some(drive) = normalized.strip_prefix("//?/") {
        if matches!(drive.as_bytes(), [d, b':', b'/', ..] if d.is_ascii_alphabetic()) {
            return Some(drive.to_owned());
        }
    }
    Some(normalized)
}

pub(crate) struct PathInspector {
    os: Os,
    music: String,
    startup_drive: String,
    cloud_roots: Vec<String>,
    roots_known: bool,
}

impl PathInspector {
    fn cloud_path(&self, path: &str) -> bool {
        path.split('/').any(provider_component)
            || (!self.cloud_roots.is_empty() && {
                let key = cloud_key(path);
                self.cloud_roots
                    .iter()
                    .any(|root| within(&key, root, Os::Other))
            })
    }

    pub(crate) fn current() -> Self {
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Other
        };
        let home = dirs::home_dir();
        // Windows Known Folders ignore the isolated HOME of the native integration test.
        #[cfg(all(windows, test))]
        let home = std::env::var_os("RECORDBOX_TEST_CLEANUP_HOME")
            .map(PathBuf::from)
            .or(home);
        let home = home.filter(|p| p.is_absolute() && os != Os::Other);
        let providers = home.as_ref().map(|dir| provider_roots::discover(dir));
        let music = home
            .and_then(|dir| slash_path(&dir.join("Music"), os))
            .unwrap_or_default();
        Self {
            os,
            music,
            startup_drive: startup_drive(),
            roots_known: providers.as_ref().is_some_and(|r| r.complete),
            cloud_roots: providers
                .into_iter()
                .flat_map(|r| r.paths)
                .filter_map(|path| slash_path(&path, os).map(|s| cloud_key(&s)))
                .collect(),
        }
    }

    pub(crate) fn inspect(&self, path: &str, cloud_hint: bool) -> PathFinding {
        self.inspect_using(path, cloud_hint, |raw| self.probe(raw))
    }

    fn inspect_using(
        &self,
        raw: &str,
        cloud_hint: bool,
        probe: impl FnOnce(&str) -> (PathStatus, Option<(PathBuf, fs::Metadata)>),
    ) -> PathFinding {
        // An XML import normally decodes file: URLs. Preserve source-drive evidence
        // for residual URLs, without assuming an unescaped local filesystem path.
        let file_uri = raw
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("file://"));
        let mut place = raw;
        if file_uri {
            place = &raw[7..];
            if place
                .get(..10)
                .is_some_and(|s| s.eq_ignore_ascii_case("localhost/"))
            {
                place = &place[9..];
            }
        }
        let remote_uri = file_uri && !place.starts_with('/');
        let mut normalized = if place.starts_with(r"\\")
            || matches!(place.as_bytes(), [a,b':',..] if a.is_ascii_alphabetic())
        {
            place.replace('\\', "/")
        } else {
            place.to_owned()
        };
        if file_uri
            && matches!(normalized.as_bytes(), [b'/',d,b':',b'/',..] if d.is_ascii_alphabetic())
        {
            normalized.remove(0);
        }
        if remote_uri {
            normalized.insert_str(0, "//")
        }
        let (os, volume, held) = origin(&normalized);
        let mut finding = PathFinding {
            os: os.label(),
            volume,
            status: PathStatus::Unclassified,
            local_identity: None,
            bytes: None,
            modified: None,
        };
        let status = if cloud_hint || self.cloud_path(&normalized) {
            PathStatus::Cloud
        } else if let Some(reason) = held {
            reason
        } else if file_uri {
            PathStatus::Unclassified
        } else if os != self.os {
            PathStatus::Foreign
        } else if os == Os::Windows && self.startup_drive != finding.volume {
            if self.startup_drive.is_empty() {
                PathStatus::Unavailable
            } else {
                PathStatus::External
            }
        } else if !within(&normalized, &self.music, os) || normalized.len() <= self.music.len() {
            PathStatus::Outside
        } else if !self.roots_known {
            PathStatus::Unavailable
        } else {
            let below = &normalized[self.music.len() + 1..];
            if below
                .split('/')
                .any(|s| ambiguous_component(s, os) || s.starts_with('.'))
                || !crate::audio::has_audio_extension(Path::new(below))
            {
                PathStatus::Unclassified
            } else {
                let (result, info) = probe(&normalized);
                if let Some((identity, metadata)) = info {
                    finding.local_identity = Some(identity);
                    finding.bytes = Some(metadata.len());
                    finding.modified = metadata.modified().ok();
                }
                result
            }
        };
        finding.status = status;
        finding
    }

    fn probe(&self, raw: &str) -> (PathStatus, Option<(PathBuf, fs::Metadata)>) {
        self.probe_volume(raw, startup_volume)
    }

    fn probe_volume(
        &self,
        raw: &str,
        on_startup_volume: impl Fn(&Path) -> bool,
    ) -> (PathStatus, Option<(PathBuf, fs::Metadata)>) {
        let path = Path::new(raw);
        let Some(parent) = path.parent() else {
            return (PathStatus::Unavailable, None);
        };
        // Inspect ancestors root first. Never follow a junction, package placeholder,
        // custom mount or symlink into a different storage domain.
        let dirs: Vec<_> = parent.ancestors().collect();
        for dir in dirs.into_iter().rev() {
            let Ok(metadata) = fs::symlink_metadata(dir) else {
                return (PathStatus::Unavailable, None);
            };
            if let Some(status) = held_metadata(&metadata) {
                return (status, None);
            }
            if !metadata.is_dir() || !on_startup_volume(dir) {
                return (PathStatus::Unavailable, None);
            }
        }
        // Confirm the resolved ancestry too, including the case-sensitive directories
        // that can be enabled on Windows.
        let (Ok(music), Ok(actual_parent)) =
            (fs::canonicalize(&self.music), fs::canonicalize(parent))
        else {
            return (PathStatus::Unavailable, None);
        };
        if !actual_parent.starts_with(&music) {
            return (PathStatus::Outside, None);
        }
        // Windows can resolve short (8.3) directory aliases to a registered sync root.
        let Some(resolved_parent) = slash_path(&actual_parent, self.os) else {
            return (PathStatus::Unavailable, None);
        };
        if self.cloud_path(&resolved_parent) {
            return (PathStatus::Cloud, None);
        }
        match fs::symlink_metadata(path) {
            Ok(meta) => {
                if let Some(status) = held_metadata(&meta) {
                    return (status, None);
                }
                if !meta.is_file() {
                    return (PathStatus::Unavailable, None);
                }
                if let Ok(id) = fs::canonicalize(path) {
                    if id.parent() == Some(actual_parent.as_path()) {
                        return (PathStatus::Present, Some((id, meta)));
                    }
                }
                (PathStatus::Unavailable, None)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                #[cfg(target_os = "macos")]
                {
                    let stub = parent.join(format!(
                        ".{}.icloud",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    match fs::symlink_metadata(stub) {
                        Ok(_) => return (PathStatus::Cloud, None),
                        Err(e) if e.kind() == ErrorKind::NotFound => {}
                        Err(_) => return (PathStatus::Unavailable, None),
                    }
                }
                // A parent unplugged or replaced between the two lookups also yields ENOENT.
                if fs::canonicalize(parent).ok().as_deref() != Some(actual_parent.as_path())
                    || !on_startup_volume(parent)
                {
                    return (PathStatus::Unavailable, None);
                }
                (PathStatus::Missing, None)
            }
            Err(_) => (PathStatus::Unavailable, None),
        }
    }
}

#[cfg(windows)]
fn startup_drive() -> String {
    use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    let mut buffer = [0u16; 260];
    // On success the returned path includes a DOS drive prefix.
    let n =
        unsafe { GetSystemWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if n > 2 && n < buffer.len() && buffer[1] == b':' as u16 && buffer[0] <= 0x7f {
        return format!("{}:", (buffer[0] as u8 as char).to_ascii_uppercase());
    }
    String::new()
}
#[cfg(not(windows))]
fn startup_drive() -> String {
    String::new()
}

fn held_metadata(meta: &fs::Metadata) -> Option<PathStatus> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem as fsys;
        let attrs = meta.file_attributes();
        if attrs
            & (fsys::FILE_ATTRIBUTE_OFFLINE
                | fsys::FILE_ATTRIBUTE_RECALL_ON_OPEN
                | fsys::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
        {
            return Some(PathStatus::Cloud);
        }
        if attrs & fsys::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Some(PathStatus::Unavailable);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const SF_DATALESS: u32 = 0x4000_0000;
        if meta.st_flags() & SF_DATALESS != 0 {
            return Some(PathStatus::Cloud);
        }
    }
    meta.file_type()
        .is_symlink()
        .then_some(PathStatus::Unavailable)
}

#[cfg(target_os = "macos")]
fn startup_volume(dir: &Path) -> bool {
    use std::{
        ffi::{CStr, CString},
        mem::MaybeUninit,
        os::unix::ffi::OsStrExt,
    };
    let Ok(path) = CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    let mut status = MaybeUninit::<libc::statfs>::uninit();
    // statfs initializes its output on success; f_mntonname is a NUL-terminated mount point.
    if unsafe { libc::statfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return false;
    }
    let status = unsafe { status.assume_init() };
    let mount = unsafe { CStr::from_ptr(status.f_mntonname.as_ptr()) }.to_bytes();
    status.f_flags & libc::MNT_LOCAL as u32 != 0 && matches!(mount, b"/" | b"/System/Volumes/Data")
}
#[cfg(windows)]
fn startup_volume(dir: &Path) -> bool {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let drive = match dir.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
            _ => return false,
        },
        _ => return false,
    };
    let root = [drive as u16, b':' as u16, b'\\' as u16, 0];
    // A directory mount point under this root was already rejected as a reparse point.
    unsafe { GetDriveTypeW(root.as_ptr()) == 3 }
}
#[cfg(not(any(target_os = "macos", windows)))]
fn startup_volume(_: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host(os: Os) -> PathInspector {
        PathInspector {
            os,
            music: match os {
                Os::Mac => "/Users/DJ/Music",
                _ => "C:/Users/DJ/Music",
            }
            .to_owned(),
            startup_drive: "C:".to_owned(),
            roots_known: true,
            cloud_roots: vec![],
        }
    }

    #[test]
    fn portable_path_census_never_probes_media_on_linux() {
        for (path, os, volume, status) in [
            (r"d:\Archive\Song.wav", "Windows", "D:", PathStatus::Foreign),
            (
                r"\\NaS\DJ\Song.wav",
                "Windows",
                "//nas/dj",
                PathStatus::External,
            ),
            (r"C:relative.wav", "Windows", "C:", PathStatus::Unclassified),
            (
                "file://localhost/C:/Users/DJ/Music/Test%20A.wav",
                "Windows",
                "C:",
                PathStatus::Unclassified,
            ),
            (
                "file://NAS/Mixes/a.wav",
                "Windows",
                "//nas/mixes",
                PathStatus::External,
            ),
            (
                "/Volumes/Music Archive/sets.wav",
                "macOS",
                "/Volumes/Music Archive",
                PathStatus::External,
            ),
            (
                "/Users/old/Music/sets.wav",
                "macOS",
                "/",
                PathStatus::Foreign,
            ),
            (
                "/Users/old/Library/CloudStorage/Provider/sets.wav",
                "macOS",
                "/",
                PathStatus::Cloud,
            ),
            (
                "/home/dj/music.wav",
                "Unclassified",
                "Unclassified",
                PathStatus::Unclassified,
            ),
            (
                "/mnt/c/Users/DJ/Music.wav",
                "Unclassified",
                "Unclassified",
                PathStatus::Unclassified,
            ),
            (
                "https://audio.example/sets",
                "Unclassified",
                "https:",
                PathStatus::Cloud,
            ),
        ] {
            let result = host(Os::Other)
                .inspect_using(path, false, |_| panic!("unexpected probe of {path}"));
            assert_eq!(
                (result.os, result.volume.as_str(), result.status),
                (os, volume, status),
                "{path}"
            );
        }
    }

    #[test]
    fn user_music_on_startup_disk_uses_only_unambiguous_audio_and_cloud_evidence() {
        let mut inspector = host(Os::Windows);
        inspector.music = slash_path(Path::new(r"\\?\C:\Users\DJ\Music"), Os::Windows).unwrap();
        inspector
            .cloud_roots
            .push(cloud_key("C:/Users/DJ/Music/Studio"));
        for (path, cloud, status) in [
            (r"D:\Users\DJ\Music\song.wav", false, PathStatus::External),
            ("C:/Users/Guest/Music/song.wav", false, PathStatus::Outside),
            ("C:/Users/DJ/Musical/song.wav", false, PathStatus::Outside),
            ("C:/Users/DJ/Music/file.wav", true, PathStatus::Cloud),
            (
                "C:/Users/DJ/Music/Cloud Option DJ Team Dropbox/a.wav",
                false,
                PathStatus::Cloud,
            ),
            (
                "C:/Users/DJ/Music/contents_42/a.wav",
                false,
                PathStatus::Cloud,
            ),
            ("C:/Users/DJ/Music/studio/a.wav", false, PathStatus::Cloud),
            (
                "C:/Users/DJ/Music/../Other/a.wav",
                false,
                PathStatus::Unclassified,
            ),
            ("C:/Users/DJ/Music/CON.wav", false, PathStatus::Unclassified),
            (
                "C:/Users/DJ/Music/foo:other.wav",
                false,
                PathStatus::Unclassified,
            ),
            (
                "C:/Users/DJ/Music/._test.wav",
                false,
                PathStatus::Unclassified,
            ),
            (
                "C:/Users/DJ/Music/ANLZ0000.DAT",
                false,
                PathStatus::Unclassified,
            ),
        ] {
            let result =
                inspector.inspect_using(path, cloud, |_| panic!("unexpected probe of {path}"));
            assert_eq!(result.status, status, "{path}");
        }
        assert_eq!(
            inspector
                .inspect_using(r"c:\Users\DJ\Music\Studio2\a.wav", false, |_| (
                    PathStatus::Missing,
                    None
                ))
                .status,
            PathStatus::Missing
        );
        inspector.roots_known = false;
        assert_eq!(
            inspector
                .inspect_using("C:/Users/DJ/Music/B.wav", false, |_| panic!())
                .status,
            PathStatus::Unavailable
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_drive_recognizes_ordinary_and_verbatim_dos_roots() {
        let drive = startup_drive();
        assert!(!drive.is_empty());
        assert!(startup_volume(Path::new(&format!(r"{drive}\"))));
        assert!(startup_volume(Path::new(&format!(r"\\?\{drive}\"))));
        assert!(!startup_volume(Path::new(r"\\server\offline")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_startup_root_and_users_data_volume_are_both_local() {
        // Modern macOS presents /Users through a firmlink to the APFS Data volume.
        assert!(startup_volume(Path::new("/")));
        assert!(startup_volume(Path::new("/Users")));
    }

    #[test]
    fn custom_sync_root_uses_unicode_and_component_boundaries_for_protection() {
        let mut inspector = host(Os::Mac);
        inspector
            .cloud_roots
            .push(cloud_key("/Users/DJ/Music/Café"));
        assert_eq!(
            inspector
                .inspect_using(
                    "/Users/DJ/Music/cafe\u{301}/Offline.wav",
                    false,
                    |_| panic!("custom cloud root must be protected")
                )
                .status,
            PathStatus::Cloud
        );
        assert_eq!(
            inspector
                .inspect_using("/Users/DJ/Music/Caféteria/File.wav", false, |_| (
                    PathStatus::Missing,
                    None
                ))
                .status,
            PathStatus::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_absent_leaf_under_reachable_unlinked_folder_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let music = root.join("Music");
        fs::create_dir(&music).unwrap();
        let mut inspector = host(Os::Mac);
        inspector.music = music.to_str().unwrap().to_owned();
        let check = |p: &Path| inspector.probe_volume(p.to_str().unwrap(), |_| true).0;
        let song = music.join("Song.wav");
        assert_eq!(check(&song), PathStatus::Missing);
        assert_eq!(
            check(&music.join("OldFolder/Gone.wav")),
            PathStatus::Unavailable
        );
        fs::write(&song, b"test audio fixture").unwrap();
        let (status, info) = inspector.probe_volume(song.to_str().unwrap(), |_| true);
        assert_eq!(status, PathStatus::Present);
        assert_eq!(info.unwrap().0, song);
        let linked = music.join("mounted");
        std::os::unix::fs::symlink(&root, &linked).unwrap();
        assert_eq!(check(&linked.join("Gone.wav")), PathStatus::Unavailable);
        std::os::unix::fs::symlink(root.join("unplugged"), music.join("Dangling.wav")).unwrap();
        assert_eq!(check(&music.join("Dangling.wav")), PathStatus::Unavailable);
        assert_eq!(
            inspector
                .probe_volume(song.to_str().unwrap(), |p| p != music)
                .0,
            PathStatus::Unavailable
        );
        #[cfg(target_os = "macos")]
        {
            let remote = music.join("Remote.wav");
            fs::write(music.join(".Remote.wav.icloud"), b"stub").unwrap();
            assert_eq!(check(&remote), PathStatus::Cloud);
        }
    }
}
