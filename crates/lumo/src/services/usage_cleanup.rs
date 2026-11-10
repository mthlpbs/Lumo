//! Migration only: restore status-line handlers installed by the retired
//! usage add-on. No quota readers, network access or background work.
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX: u64 = 1024 * 1024;
fn read(path: &Path) -> Option<serde_json::Value> {
    let f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > MAX {
        return None;
    }
    serde_json::from_reader(f.take(MAX)).ok()
}
fn restore(dir: &Path) -> std::io::Result<()> {
    let Some(backup) = read(&dir.join("lumo-statusline-backup.json")) else {
        return Ok(());
    };
    let path = dir.join("settings.json");
    let Some(mut settings) = read(&path) else {
        return Ok(());
    };
    let Some(installed) = backup["installed"]
        .as_str()
        .filter(|s| s.contains("--usage-statusline"))
    else {
        return Ok(());
    };
    if settings["statusLine"]["command"].as_str() != Some(installed) {
        return Ok(());
    }
    let Some(obj) = settings.as_object_mut() else {
        return Ok(());
    };
    if backup["previous"].is_null() {
        obj.remove("statusLine");
    } else {
        obj.insert("statusLine".into(), backup["previous"].clone());
    }
    lumo_core::store::save_json_atomic(&path, &settings)
}
pub fn restore_all() -> i32 {
    let pending = crate::paths::cache_dir().join("retired-usage-hooks.json");
    let mut dirs: Vec<PathBuf> = read(&pending)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if let Some(s) = read(&crate::paths::settings_path()) {
        if let Some(accounts) = s["ai_usage"]["accounts"].as_array() {
            for a in accounts
                .iter()
                .take(32)
                .filter(|a| a["provider"] == "claude")
            {
                let dir = a["directory"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                            .join(".claude")
                    });
                if dir.is_absolute() && !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }
    if dirs.is_empty() {
        return 0;
    }
    dirs.retain(|dir| restore(dir).is_err());
    let _ = lumo_core::store::write_json_atomic(&pending, &dirs);
    if dirs.is_empty() {
        0
    } else {
        1
    }
}
/// A provider can still invoke an old installed command before Lumo next
/// launches. Restore its handler and forward stdin once; never collect usage.
pub fn retired_statusline(backup: &str) -> i32 {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let path = Path::new(backup);
    if let Some(dir) = path.parent() {
        let _ = restore(dir);
    }
    let Some(data) = read(path) else {
        return 0;
    };
    let Some(command) = data["previous"]["command"].as_str() else {
        return 0;
    };
    let mut bytes = vec![];
    if std::io::stdin()
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX as usize
    {
        return 1;
    }
    let Ok(mut child) = Command::new("cmd.exe")
        .args(["/d", "/s", "/c", command])
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::null())
        .spawn()
    else {
        return 1;
    };
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(&bytes);
    }
    child.wait().map_or(1, |s| s.code().unwrap_or(1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_handler_restore_preserves_unrelated_settings_and_user_edits() {
        let dir = std::env::temp_dir().join(format!("lumo-retired-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let installed = "lumo.exe --usage-statusline snapshot backup";
        let original = serde_json::json!({"type":"command","command":"echo existing"});
        lumo_core::store::write_json_atomic(
            &dir.join("lumo-statusline-backup.json"),
            &serde_json::json!({"installed":installed,"previous":original}),
        )
        .unwrap();
        lumo_core::store::write_json_atomic(
            &path,
            &serde_json::json!({"statusLine":{"command":installed},"keep":42}),
        )
        .unwrap();
        restore(&dir).unwrap();
        assert_eq!(
            read(&path).unwrap(),
            serde_json::json!({"statusLine":original,"keep":42})
        );
        let edited = serde_json::json!({"statusLine":{"command":"echo changed"},"keep":42});
        lumo_core::store::write_json_atomic(&path, &edited).unwrap();
        restore(&dir).unwrap();
        assert_eq!(read(&path).unwrap(), edited);
    }
}
