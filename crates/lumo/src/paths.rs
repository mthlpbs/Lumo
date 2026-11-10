//! Where Lumo keeps its files.
//!
//! - Settings + data: `%APPDATA%\Lumo\` (roams with the user profile)
//! - Cache + log:     `%LOCALAPPDATA%\Lumo\`
//!
//! `LUMO_HOME` overrides both (used by `--selftest` to stay isolated).

use std::path::PathBuf;

fn home_override() -> Option<PathBuf> {
    std::env::var_os("LUMO_HOME").map(PathBuf::from)
}

/// Runs after the single-instance check, before the first file is opened.
pub fn prepare_profile() -> std::io::Result<()> {
    if home_override().is_some() { return Ok(()); }
    for variable in ["APPDATA", "LOCALAPPDATA"] {
        lumo_core::profile::prepare(&env_dir(variable))?;
    }
    Ok(())
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    home_override().unwrap_or_else(|| lumo_core::profile::dir(&env_dir("APPDATA")))
}

pub fn cache_dir() -> PathBuf {
    home_override().map(|h| h.join("cache")).unwrap_or_else(|| lumo_core::profile::dir(&env_dir("LOCALAPPDATA")))
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn store_path() -> PathBuf {
    data_dir().join("data.json")
}

pub fn log_path() -> PathBuf {
    cache_dir().join("lumo.log")
}

pub fn lyrics_cache_dir() -> PathBuf {
    cache_dir().join("lyrics")
}

/// The selected local-lyrics library. An empty preference follows Windows'
/// Music known folder, so redirected and OneDrive Music folders work too.
pub fn music_dir(settings: &lumo_core::settings::Settings) -> PathBuf {
    if settings.music_directory.trim().is_empty() {
        default_music_dir()
    } else {
        PathBuf::from(&settings.music_directory)
    }
}

/// Use the library shown in Settings after local lyrics are enabled, or an
/// explicitly chosen folder. Global/streaming lyrics alone enable no writes.
pub fn lyrics_download_dir(settings: &lumo_core::settings::Settings) -> Option<PathBuf> {
    let custom=settings.lyrics_download_directory.trim();
    let music=settings.music_directory.trim();
    if !custom.is_empty() {Some(PathBuf::from(custom))}
    else if !music.is_empty() || settings.local_lyrics {Some(music_dir(settings))}
    else {None}
}

pub fn default_music_dir() -> PathBuf {
    if let Some(home) = home_override() {
        return home.join("Music");
    }
    static MUSIC: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    MUSIC.get_or_init(|| {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{FOLDERID_Music, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
        let known = unsafe {
            SHGetKnownFolderPath(&FOLDERID_Music, KF_FLAG_DEFAULT, HANDLE::default()).ok().and_then(|p| {
                let text = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const _));
                text.filter(|s| !s.is_empty()).map(PathBuf::from)
            })
        };
        known.unwrap_or_else(|| env_dir("USERPROFILE").join("Music"))
    }).clone()
}

/// The user's Apple Music token, sealed with DPAPI (never roams; uninstalling
/// removes it with the cache).
pub fn apple_music_token_path() -> PathBuf {
    cache_dir().join("apple-music-token.bin")
}

/// The Apple Music web player's (public) developer token, cached until it expires.
pub fn apple_music_web_token_path() -> PathBuf {
    cache_dir().join("apple-music-web.json")
}

/// New files go straight to Downloads, or an explicitly selected folder.
/// Existing received files and their Shelf references are left in place.
pub fn phone_inbox_dir(settings: &lumo_core::settings::Settings) -> PathBuf {
    if !settings.phone_download_directory.trim().is_empty() {
        return PathBuf::from(&settings.phone_download_directory);
    }
    if let Some(h) = home_override() {
        return h.join("Downloads");
    }
    downloads_dir().unwrap_or_else(|| env_dir("USERPROFILE").join("Downloads"))
}

fn downloads_dir() -> Option<PathBuf> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, HANDLE::default()).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty()).map(PathBuf::from)
    }
}

/// Add-on files, one folder per add-on (`services::addons`). Local, never
/// roaming: a PC that hasn't them downloads them again. Switching an add-on
/// off or uninstalling removes them.
pub fn addons_dir() -> PathBuf {
    cache_dir().join("addons")
}

/// Paired phones (their pairing secrets and names), sealed with DPAPI.
/// Removing every phone deletes the file.
pub fn paired_phones_path() -> PathBuf {
    cache_dir().join("paired-phones.bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_files_use_downloads_directly_and_accept_an_explicit_folder() {
        let mut settings = lumo_core::settings::Settings::default();
        let expected = home_override().map(|h| h.join("Downloads")).unwrap_or_else(|| downloads_dir().unwrap_or_else(|| env_dir("USERPROFILE").join("Downloads")));
        assert_eq!(phone_inbox_dir(&settings), expected);
        settings.phone_download_directory = "D:\\Received files".into();
        assert_eq!(phone_inbox_dir(&settings), PathBuf::from("D:\\Received files"));
        settings.phone_download_directory.clear();
        assert_eq!(phone_inbox_dir(&settings), expected);
    }

    #[test]
    fn lyric_downloads_require_a_configured_folder_and_custom_paths_override_music() {
        let mut settings=lumo_core::settings::Settings::default();
        assert!(lyrics_download_dir(&settings).is_none());
        settings.local_lyrics=true;
        assert_eq!(lyrics_download_dir(&settings),Some(music_dir(&settings)));
        settings.local_lyrics=false;
        settings.music_directory="D:\\Music".into();
        assert_eq!(lyrics_download_dir(&settings),Some(PathBuf::from("D:\\Music")));
        settings.lyrics_download_directory="E:\\Lyrics".into();
        assert_eq!(lyrics_download_dir(&settings),Some(PathBuf::from("E:\\Lyrics")));
        settings.lyrics_download_directory.clear();
        assert_eq!(lyrics_download_dir(&settings),Some(PathBuf::from("D:\\Music")));
    }

    #[test]
    fn canonical_config_and_cache_folder_is_lumo() {
        let base = PathBuf::from("profile-root");
        assert_eq!(lumo_core::profile::dir(&base), base.join("Lumo"));
    }
}
