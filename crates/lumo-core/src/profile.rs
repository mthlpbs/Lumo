//! One-time import of older profiles into the canonical Windows data folder.
//! Old folders are read only; every subsequent save goes to Lumo.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FOLDER: &str = "Lumo";
const MARKER: &str = ".migration-complete";

pub fn dir(base: &Path) -> PathBuf { base.join(FOLDER) }

pub fn prepare(base: &Path) -> io::Result<PathBuf> {
    let target = dir(base);
    if target.exists() && redirected(&fs::symlink_metadata(&target)?) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "profile destination is redirected"));
    }
    if target.join(MARKER).is_file() { return Ok(target); }
    // Prefer the most recently used profile if both historical names exist.
    let mut sources: Vec<_> = ["Lemo", "Poni"].iter().map(|name| base.join(name)).filter(|p| p.is_dir()).collect();
    sources.sort_by_key(|p| std::cmp::Reverse(["settings.json", "data.json", "lumo.log"].iter()
        .filter_map(|name| fs::metadata(p.join(name)).ok()?.modified().ok()).max()));
    fs::create_dir_all(&target)?;
    for source in sources { copy_missing(&source, &target)?; }
    for name in ["settings.json", "settings.bak.json"] { normalize_settings(&target.join(name))?; }
    fs::write(target.join(MARKER), b"1\n")?;
    Ok(target)
}

fn obsolete(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == MARKER || name == "addons" || name == "updates" || name == "update"
        || name == "update.json" || name == "update.bak.json"
        || name.ends_with(".log") || name.starts_with("poni.log")
        || name.starts_with("lumo.log") || name.ends_with(".tmp")
}

fn redirected(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() { return true; }
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

fn copy_missing(source: &Path, target: &Path) -> io::Result<()> {
    if redirected(&fs::symlink_metadata(source)?) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "legacy profile is redirected"));
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if obsolete(&entry.file_name().to_string_lossy()) { continue; }
        let metadata = fs::symlink_metadata(entry.path())?;
        if redirected(&metadata) { continue; }
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_missing(&entry.path(), &destination)?;
        } else if metadata.is_file() && !destination.exists() {
            let pending = destination.with_extension("migration.tmp");
            fs::copy(entry.path(), &pending)?;
            fs::rename(pending, destination)?;
        }
    }
    Ok(())
}

fn normalize_settings(path: &Path) -> io::Result<()> {
    if !path.is_file() { return Ok(()); }
    let original = fs::read(path)?;
    // Corrupt files retain their original bytes for the normal recovery path.
    let Ok(settings) = serde_json::from_slice::<crate::settings::Settings>(&original) else { return Ok(()); };
    let normalized = serde_json::to_vec_pretty(&settings.sanitized()).map_err(io::Error::other)?;
    if original != normalized {
        let pending = path.with_extension("migration.tmp");
        fs::write(&pending, normalized)?;
        fs::rename(pending, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let root = std::env::temp_dir().join(format!("lumo-profile-{}-{stamp}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            if self.0.exists() {
                assert_eq!(self.0.canonicalize().unwrap().parent(), Some(std::env::temp_dir().canonicalize().unwrap().as_path()));
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn imports_existing_data_and_normalizes_retired_settings_without_altering_source() {
        let temp = Temp::new();
        let old = temp.0.join("Poni");
        fs::create_dir_all(old.join("lyrics")).unwrap();
        let settings = br#"{"theme":"classic","application":"Poni","addons":{"script":true}}"#;
        fs::write(old.join("settings.json"), settings).unwrap();
        fs::write(old.join("settings.bak.json"), settings).unwrap();
        fs::write(old.join("data.json"), br#"{"tasks":{"items":[]},"user_text":"Poni is a personal file name"}"#).unwrap();
        fs::write(old.join("paired-phones.bin"), b"sealed-pairing").unwrap();
        fs::write(old.join("lyrics/song.json"), b"cached-song").unwrap();
        fs::write(old.join("poni.log"), b"old branding").unwrap();
        fs::write(old.join("update.json"), b"old owner release").unwrap();
        let target = prepare(&temp.0).unwrap();
        assert_eq!(target, temp.0.join("Lumo"));
        for name in ["settings.json", "settings.bak.json"] {
            let clean: serde_json::Value = serde_json::from_slice(&fs::read(target.join(name)).unwrap()).unwrap();
            assert_eq!(clean["theme"], "classic");
            assert!(clean.get("application").is_none());
        }
        for name in ["data.json", "paired-phones.bin", "lyrics/song.json"] {
            assert_eq!(fs::read(target.join(name)).unwrap(), fs::read(old.join(name)).unwrap());
        }
        assert!(!target.join("poni.log").exists());
        assert!(!target.join("update.json").exists());
        assert_eq!(fs::read(old.join("settings.json")).unwrap(), settings);
    }

    #[test]
    fn existing_lumo_files_win_and_completed_import_never_repeats() {
        let temp = Temp::new();
        let old = temp.0.join("Lemo");
        let target = dir(&temp.0);
        fs::create_dir_all(&old).unwrap(); fs::create_dir_all(&target).unwrap();
        fs::write(old.join("data.json"), b"old data").unwrap();
        fs::write(target.join("data.json"), b"current data").unwrap();
        prepare(&temp.0).unwrap();
        assert_eq!(fs::read(target.join("data.json")).unwrap(), b"current data");
        fs::write(old.join("new-old-file.json"), b"later").unwrap();
        prepare(&temp.0).unwrap();
        assert!(!target.join("new-old-file.json").exists());
    }

    #[test]
    fn damaged_settings_are_preserved_for_backup_recovery() {
        let temp = Temp::new();
        let old = temp.0.join("Poni"); fs::create_dir(&old).unwrap();
        fs::write(old.join("settings.json"), b"broken JSON").unwrap();
        fs::write(old.join("settings.bak.json"), br#"{"theme":"classic"}"#).unwrap();
        let target = prepare(&temp.0).unwrap();
        assert_eq!(fs::read(target.join("settings.json")).unwrap(), b"broken JSON");
        let backup: crate::settings::Settings = serde_json::from_slice(&fs::read(target.join("settings.bak.json")).unwrap()).unwrap();
        assert_eq!(backup.theme, crate::settings::Theme::Classic);
    }
}
