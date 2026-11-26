//! Guard outbound writes when a former default master.db remains after relocating
//! a library. Read only this profile's settings and never follow a configured
//! destination. Missing optional configuration is normal on a copied profile.
use anyhow::{bail, ensure, Context, Result};
use quick_xml::{events::Event, Reader, XmlVersion};
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};

const MAX_SETTINGS: u64 = 2 * 1024 * 1024;

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn read_optional(vendor: &Path, relative: &str) -> Result<Option<String>> {
    let path = vendor.join(relative);
    let mut directory = vendor.to_owned();
    for next in Path::new(relative).components() {
        match fs::symlink_metadata(&directory) {
            Ok(metadata) => ensure!(
                !is_link(&metadata) && metadata.is_dir(),
                "Linked or invalid settings directory"
            ),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        directory.push(next);
    }
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        meta.is_file() && !is_link(&meta) && meta.len() <= MAX_SETTINGS,
        "Settings file is linked, oversized or irregular"
    );
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path)?.take(MAX_SETTINGS + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == meta.len(),
        "Settings file changed during inspection"
    );
    let text = String::from_utf8(bytes).context("Unreadable settings encoding")?;
    Ok(Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()))
}

fn xml_directories(text: &str) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut root = false;
    loop {
        let event = reader.read_event()?;
        let elem = match &event {
            Event::Empty(e) | Event::Start(e) => e,
            Event::End(_) => {
                depth = depth.checked_sub(1).context("Unexpected closing XML tag")?;
                continue;
            }
            Event::Eof => {
                ensure!(root && depth == 0, "Incomplete rekordbox settings XML");
                break;
            }
            Event::DocType(_) => bail!("Unexpected DTD in rekordbox settings"),
            Event::Text(t) if depth == 0 => {
                ensure!(
                    t.iter().all(u8::is_ascii_whitespace),
                    "Unexpected text outside settings"
                );
                continue;
            }
            _ => continue,
        };
        if depth == 0 {
            ensure!(
                !root && elem.name().as_ref() == b"PROPERTIES",
                "Unexpected rekordbox settings root"
            );
            root = true;
        }
        if elem.name().as_ref() == b"VALUE" {
            let mut setting = None;
            let mut value = None;
            for at in elem.attributes() {
                let at = at?;
                match at.key.as_ref() {
                    b"name" => setting = Some(at.normalized_value(XmlVersion::Explicit1_0)?.into_owned()),
                    b"val" => value = Some(at.normalized_value(XmlVersion::Explicit1_0)?.into_owned()),
                    _ => {}
                }
            }
            if setting
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("masterDbDirectory"))
            {
                paths.push(value.context("masterDbDirectory has no path value")?);
            }
        }
        if matches!(event, Event::Start(_)) {
            depth += 1;
        }
    }
    Ok(paths)
}

#[derive(Clone, Copy)]
enum Setting {
    DatabaseDirectory,
    DatabaseFile,
    AnalysisRoot,
}

fn agent_paths(text: &str) -> Result<Vec<(Setting, String)>> {
    let root: serde_json::Value = serde_json::from_str(text)?;
    let options = root.as_object().context("Expected a rekordboxAgent options object")?;
    let mut paths = Vec::new();
    let mut push = |key: &str, value: &serde_json::Value| -> Result<()> {
        let kind = if key == "db-path" {
            Setting::DatabaseFile
        } else {
            Setting::AnalysisRoot
        };
        paths.push((
            kind,
            value
                .as_str()
                .with_context(|| format!("Agent {key} is not a string"))?
                .to_owned(),
        ));
        Ok(())
    };
    match options.get("options").unwrap_or(&root) {
        serde_json::Value::Array(rows) => {
            for row in rows {
                let row = row.as_array().context("Malformed agent option")?;
                let key = row
                    .first()
                    .and_then(|v| v.as_str())
                    .context("Missing agent option name")?;
                if matches!(key, "db-path" | "analysis-data-root-path") {
                    ensure!(row.len() == 2, "Malformed agent {key}");
                    push(key, &row[1])?;
                }
            }
        }
        serde_json::Value::Object(rows) => {
            for key in ["db-path", "analysis-data-root-path"] {
                if let Some(value) = rows.get(key) {
                    push(key, value)?;
                }
            }
        }
        _ => bail!("Unsupported rekordboxAgent options structure"),
    }
    Ok(paths)
}

fn plain(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(path) = path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        return Path::new(path).components().collect();
    }
    path.components().collect()
}

fn verify_path(value: &str, kind: Setting, selected_db: &Path, actual_db: &Path) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    } // unset preference
    let path = Path::new(value);
    ensure!(
        !value.chars().any(char::is_control)
            && !value.starts_with("//")
            && !value.starts_with(r"\\")
            && path.is_absolute()
            && !path.components().any(|c| c == Component::ParentDir),
        "Library or analysis preference must be an absolute local path"
    );
    let candidate = match kind {
        Setting::DatabaseDirectory => path.join("master.db"),
        Setting::DatabaseFile | Setting::AnalysisRoot => path.to_owned(),
    };
    let selected_share;
    let actual_share;
    let (selected, actual) = if matches!(kind, Setting::AnalysisRoot) {
        selected_share = selected_db
            .parent()
            .context("Missing database directory")?
            .join("share");
        actual_share = actual_db.parent().context("Missing database directory")?.join("share");
        (selected_share.as_path(), actual_share.as_path())
    } else {
        (selected_db, actual_db)
    };
    let candidate = plain(&candidate);
    if candidate == selected || candidate == actual {
        return Ok(());
    }
    // Accept case-only spelling differences on case-insensitive filesystems only
    // after confirming they resolve to the selected file. A different drive or
    // an offline configured folder must never be probed or used as a fallback.
    if [selected, actual]
        .iter()
        .any(|p| p.as_os_str().eq_ignore_ascii_case(candidate.as_os_str()))
        && candidate.canonicalize().is_ok_and(|p| plain(&p) == actual)
    {
        return Ok(());
    }
    if matches!(kind, Setting::AnalysisRoot) {
        bail!(
            "rekordboxAgent uses analysis files at {}. This write-back pilot requires analysis files at {}",
            candidate.display(),
            selected.display()
        );
    }
    bail!("rekordbox is configured for {}. The selected file is {}. Review the database location in rekordbox before writing", candidate.display(), selected.display())
}

pub(super) fn verify(source_dir: &Path) -> Result<()> {
    let parent = source_dir.parent().context("Missing library parent directory")?;
    let mut vendors = vec![parent.to_owned()];
    // In macOS the default database and application settings often live under
    // Library/Pioneer and Library/Application Support/Pioneer respectively.
    if let (Some(root), Some(vendor)) = (parent.parent(), parent.file_name()) {
        if root.file_name().is_some_and(|s| s == "Library") {
            vendors.push(root.join("Application Support").join(vendor));
        } else if root.file_name().is_some_and(|s| s == "Application Support")
            && root.parent().and_then(Path::file_name).is_some_and(|s| s == "Library")
        {
            vendors.push(root.parent().unwrap().join(vendor));
        }
    }
    let selected = plain(&std::path::absolute(source_dir.join("master.db"))?);
    let actual = plain(&selected.canonicalize()?);
    // Workbench also discovers newer AlphaTheta-branded profile folders. Only look
    // alongside the selected profile; never consult another account's HOME.
    if let Some(name) = parent.file_name() {
        let other = if name.eq_ignore_ascii_case("Pioneer") {
            Some("AlphaTheta")
        } else if name.eq_ignore_ascii_case("AlphaTheta") {
            Some("Pioneer")
        } else {
            None
        };
        if let Some(other) = other {
            let aliases = vendors
                .iter()
                .filter_map(|p| p.parent().map(|base| base.join(other)))
                .collect::<Vec<_>>();
            vendors.extend(aliases);
        }
    }
    for vendor in vendors {
        for (file, xml) in [
            ("rekordbox6/rekordbox3.settings", true),
            ("rekordbox7/rekordbox3.settings", true),
            ("rekordboxAgent/storage/options.json", false),
        ] {
            let path = vendor.join(file);
            (|| -> Result<()> {
                let Some(text) = read_optional(&vendor, file)? else {
                    return Ok(());
                };
                let locations = if xml {
                    xml_directories(&text)?
                        .into_iter()
                        .map(|v| (Setting::DatabaseDirectory, v))
                        .collect()
                } else {
                    agent_paths(&text)?
                };
                for (setting, value) in locations {
                    verify_path(&value, setting, &selected, &actual)?;
                }
                Ok(())
            })()
            .with_context(|| format!("Verifying the active desktop library using {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_default_and_disagreeing_agent_are_blocked_without_following_either() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("Pioneer/rekordbox");
        let settings = temp.path().join("Pioneer/rekordbox6/rekordbox3.settings");
        let agent = temp.path().join("Pioneer/rekordboxAgent/storage/options.json");
        fs::create_dir_all(&source)?;
        fs::write(source.join("master.db"), b"original")?;
        verify(&source)?;
        fs::create_dir_all(settings.parent().unwrap())?;
        fs::create_dir_all(agent.parent().unwrap())?;
        let xml = |path: &Path| {
            format!(
                "<PROPERTIES><VALUE name=\"masterDbDirectory\" val=\"{}\"/></PROPERTIES>",
                quick_xml::escape::escape(path.to_string_lossy())
            )
        };
        let elsewhere = temp.path().join("disconnected-drive");
        fs::write(&settings, xml(&elsewhere))?;
        assert!(format!("{:#}", verify(&source).unwrap_err()).contains("configured for"));
        fs::write(&settings, xml(&source))?;
        fs::write(
            &agent,
            serde_json::to_vec(&serde_json::json!({"options":[["db-path",elsewhere.join("master.db")]]}))?,
        )?;
        assert!(verify(&source).is_err());
        fs::write(
            &agent,
            serde_json::to_vec(&serde_json::json!({"options":[["db-path",source.join("master.db")]]}))?,
        )?;
        verify(&source)?;
        assert_eq!(fs::read(source.join("master.db"))?, b"original");
        assert!(!elsewhere.exists());
        Ok(())
    }

    #[test]
    fn xml_and_json_require_unambiguous_path_values() -> Result<()> {
        assert_eq!(
            xml_directories("<PROPERTIES><VALUE name='masterDbDirectory' val='/D&amp;J'/></PROPERTIES>")?,
            ["/D&J"]
        );
        for malformed in [
            "<PROPERTIES>",
            "<wrong/>",
            "<PROPERTIES><VALUE name='masterDbDirectory'/></PROPERTIES>",
            "<PROPERTIES/><PROPERTIES/>",
        ] {
            assert!(xml_directories(malformed).is_err());
        }
        for malformed in [
            r#"{"options":[["db-path"]]}"#,
            r#"{"options":{"db-path":7}}"#,
            r#"{"options":null}"#,
        ] {
            assert!(agent_paths(malformed).is_err());
        }
        assert!(xml_directories("<PROPERTIES><VALUE name='other' val='x'/></PROPERTIES>")?.is_empty());
        assert!(agent_paths(r#"{"options":[["unrelated",true]]}"#)?.is_empty());
        Ok(())
    }

    #[test]
    fn relocated_analysis_root_blocks_writes_even_for_a_matching_database() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("Pioneer/rekordbox");
        let agent = temp.path().join("Pioneer/rekordboxAgent/storage/options.json");
        fs::create_dir_all(&source)?;
        fs::create_dir_all(agent.parent().unwrap())?;
        let db = source.join("master.db");
        fs::write(&db, b"original")?;
        let share = source.join("share");
        let relocated = temp.path().join("external-analysis");
        fs::write(
            &agent,
            serde_json::to_vec(&serde_json::json!({"options":[["db-path",db], ["analysis-data-root-path",share]]}))?,
        )?;
        verify(&source)?;
        fs::write(
            &agent,
            serde_json::to_vec(&serde_json::json!({"options":{"db-path":db, "analysis-data-root-path":relocated}}))?,
        )?;
        assert!(format!("{:#}", verify(&source).unwrap_err()).contains("requires analysis files"));
        assert!(!relocated.exists());
        assert!(!share.exists());
        assert_eq!(fs::read(db)?, b"original");
        Ok(())
    }

    #[test]
    fn mac_library_checks_its_sibling_application_support_configuration() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let library = temp.path().join("Library");
        let source = library.join("Pioneer/rekordbox");
        let agent = library.join("Application Support/Pioneer/rekordboxAgent/storage/options.json");
        fs::create_dir_all(&source)?;
        fs::write(source.join("master.db"), b"old")?;
        fs::create_dir_all(agent.parent().unwrap())?;
        fs::write(agent, br#"{"options":[["db-path","relative-location/master.db"]]}"#)?;
        assert!(format!("{:#}", verify(&source).unwrap_err()).contains("absolute local path"));
        Ok(())
    }
}
