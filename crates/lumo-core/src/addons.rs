//! Add-ons: features that download only for the people who switch them on.
//!
//! An add-on's code is part of the app and stays asleep while it is off. The
//! switch downloads the add-on's files (data and sounds) from a pre-release
//! on the app's GitHub repository, and switching it off deletes them. Each
//! file's size and SHA-256 are built into the app, so a changed or broken
//! file is refused; new files come with a new app release (a new `tag`).
//! The app's `services::addons` does the downloading.

use crate::phonedrop::fmt_bytes;
use crate::sha256;

/// Where the files live: `<RELEASES>/<tag>/<name>`.
pub const RELEASES: &str = "https://github.com/mthlpbs/lumo/releases/download";

/// Managed Brave-engine protection data. It is enabled only from Browser,
/// with a short-lived, cancellable download rather than the add-on retry loop.
pub const WEB_BLOCKING: Addon = Addon {
    id:"web-blocking",title:"Web ad blocking",blurb:"Brave adblock-rust protection for embedded websites",
    tag:"web-blocking-2026.1113.1",
    files:&[
        AddonFile{name:"easylist.txt",size:2044083,sha256:"316356f48dd784da248e162a1c895d6e426e9145c5dcd28b3334332991d8ffd8"},
        AddonFile{name:"easyprivacy.txt",size:1505358,sha256:"e97535eea6c7825cd1a264cd33d4968bfe381845dfc748cef699eeab37ab25b5"},
        AddonFile{name:"resources.json",size:596966,sha256:"d1f5e7fb0263cb70c19aac5ae432e18a920448527d1354d1c2d2ca309500932d"},
    ],
};

/// One file an add-on downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddonFile {
    pub name: &'static str,
    pub size: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
}

impl AddonFile {
    /// True when `bytes` are exactly this file.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.size && sha256::hex(&sha256::sha256(bytes)) == self.sha256
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addon {
    /// Stable id: the key in settings.json and the folder's name.
    pub id: &'static str,
    pub title: &'static str,
    /// What it does, in one line, for the Add-ons page.
    pub blurb: &'static str,
    /// The release that holds this version of the files.
    pub tag: &'static str,
    pub files: &'static [AddonFile],
}

impl Addon {
    /// Every file together, in bytes.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn url(&self, file: &AddonFile) -> String {
        format!("{RELEASES}/{}/{}", self.tag, file.name)
    }

    pub fn find(id: &str) -> Option<&'static Addon> {
        ALL.iter().copied().find(|a| a.id == id)
    }

    /// Whether it has files to download. One without (Script reader) is
    /// already in the app: the switch only shows or hides it, and the
    /// downloader never touches it.
    pub fn downloads(&self) -> bool {
        !self.files.is_empty()
    }
}

/// Script reader: the Script page, a teleprompter under the webcam. Its
/// code is in the app (a few dozen KB), so there is nothing to download; off,
/// the island has no Script tab, no Alt+Shift+S and nothing running for it.
pub const SCRIPT_READER: Addon = Addon {
    id: "script",
    title: "Script reader",
    blurb: "Teleprompter. Hidden from screen shares.",
    tag: "",
    files: &[],
};

/// Clipboard history: the Clipboard page, text you copy kept in memory to
/// paste again. In the app since 0.1.0 and on for everyone until 0.2.7, which
/// made it an add-on, off (settings v4 switched it off once). Its switch is
/// `Settings::clipboard_history`, which the tray's item also flips; off, there
/// is no Clipboard tab and nothing watches the clipboard .
pub const CLIPBOARD_HISTORY: Addon = Addon {
    id: "clipboard",
    title: "Clipboard history",
    blurb: "Copied text stays in memory, never on disk.",
    tag: "",
    files: &[],
};

/// Optional AI companion. Disabling it hides its surfaces without removing
/// provider settings, credentials, cached models or saved conversations.
pub const AI_COMPANION: Addon = Addon {
    id: "ai",
    title: "AI companion",
    blurb: "Ask your AI provider without leaving your desktop.",
    tag: "",
    files: &[],
};

/// Every add-on, in the order the Add-ons page lists them.
pub const ALL: [&Addon; 3] = [&SCRIPT_READER, &CLIPBOARD_HISTORY, &AI_COMPANION];

/// Where an add-on stands, as the Add-ons page shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Switched off: none of its files on this PC, nothing running.
    Off,
    /// Switched on; looking at the files already on this PC.
    Checking,
    /// Bytes so far of the add-on's total.
    Downloading { done: u64, total: u64 },
    /// Switched on with no internet: the download starts by itself later.
    Waiting,
    /// Switched on, every file in place.
    Ready,
    /// A file came back different from the one the app expects, so it was
    /// thrown away. Trying again is the person's call (off, then on).
    Failed,
}

impl Status {
    /// The switch shows on for everything but `Off`.
    pub fn is_on(self) -> bool {
        self != Status::Off
    }

    /// The line under the add-on's name.
    pub fn line(self, addon: &Addon) -> String {
        let here = "this PC";
        match self {
            Status::Off => addon.blurb.to_string(),
            Status::Checking => format!("Checking its files on {here}"),
            Status::Downloading { done, total } => format!("Downloading · {} of {}", fmt_bytes(done), fmt_bytes(total)),
            Status::Waiting => "No internet. It downloads by itself once you're back online.".to_string(),
            Status::Ready if !addon.downloads() => "On".to_string(),
            Status::Ready => format!("On · {} on {here}", fmt_bytes(addon.size())),
            Status::Failed => "The download didn't come through right. Switch it off and on to try again.".to_string(),
        }
    }

    /// Download progress 0..1, while downloading.
    pub fn progress(self) -> Option<f32> {
        match self {
            Status::Downloading { done, total } if total > 0 => Some((done as f64 / total as f64).clamp(0.0, 1.0) as f32),
            Status::Downloading { .. } => Some(0.0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_supported_addons_can_be_enabled() {
        for id in ["sholat", "prayer", "ja", "health", "money"] {
            assert!(Addon::find(id).is_none());
        }
        assert_eq!(Addon::find("script"), Some(&SCRIPT_READER));
        assert!(ALL.iter().all(|a| !a.downloads()));
    }
}
