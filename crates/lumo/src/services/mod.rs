//! Background services. Each one reports to the UI thread through an
//! [`EventSink`]; a failing service is logged and skipped, never fatal.

pub mod addons;
pub mod ai;
pub mod ai_shortcut;
pub mod applemusic;
pub mod applemusic_ui;
pub mod audio;
pub mod bluetooth;
pub mod clipboard;
pub mod frames;
pub mod fullscreen;
pub mod http;
pub mod imaging;
pub mod lyrics;
pub mod media;
pub mod music_files;
pub mod search;
pub mod search_integrations;
pub mod pairdrop;
pub mod phonedrop;
pub mod power;
pub mod psu;
pub mod recorder;
pub mod romanize;
pub mod secret;
pub mod sound;
pub mod sysmon;
pub mod update;

use crate::commands::MediaCmd;
use crate::events::EventSink;
use crate::state::MediaSession;
use lumo_core::geom::Color;
use lumo_core::settings::Settings;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};

pub struct Services {
    media: Option<media::MediaService>,
    lyrics: Option<lyrics::LyricsService>,
    _audio: Option<audio::AudioService>,
    _bluetooth: Option<bluetooth::BluetoothService>,
    fullscreen: Option<fullscreen::FullscreenWatcher>,
    clip: Option<clipboard::ClipboardHistory>,
    pairdrop: Option<pairdrop::PairDropService>,
    addons: Option<addons::AddonService>,
}

fn started<T, E: std::fmt::Display>(name: &str, r: Result<T, E>) -> Option<T> {
    match r {
        Ok(v) => {
            crate::pinfo!("service {name}: started");
            Some(v)
        }
        Err(e) => {
            crate::pwarn!("service {name}: unavailable ({e})");
            None
        }
    }
}

impl Services {
    pub fn start(sink: &EventSink, _island: HWND, helper: HWND, settings: &Settings) -> Self {
        let media = started("media", media::MediaService::start(sink.clone()));
        let lyrics = started("lyrics", lyrics::LyricsService::start(sink.clone()));
        let audio = started("audio", audio::AudioService::start(sink.clone()));
        let bluetooth = started("bluetooth", bluetooth::BluetoothService::start(sink.clone()));
        let fullscreen = started("fullscreen", fullscreen::FullscreenWatcher::start(helper, sink.clone()));
        let clip = started("clipboard history", clipboard::ClipboardHistory::start(sink.clone(), settings.clipboard_history));
        // Asks nothing until a phone is paired.
        let pairdrop = started("pair drop", pairdrop::PairDropService::start(sink.clone()));
        // Built-in add-ons need no worker. Updates use a short-lived worker
        // when requested, so there is no updater thread or HTTP client at rest.
        let addons = if lumo_core::addons::ALL.iter().any(|a| a.downloads() && settings.addon_on(a.id)) {
            let on = lumo_core::addons::ALL.iter().filter(|a| settings.addon_on(a.id)).map(|a| a.id).collect();
            started("add-ons", addons::AddonService::start(sink.clone(), on))
        } else {
            None
        };
        Self { media, lyrics, _audio: audio, _bluetooth: bluetooth, fullscreen, clip, pairdrop, addons }
    }

    pub fn stop(self, _helper: HWND) {
        if let Some(c) = self.clip {
            c.stop();
        }
        if let Some(f) = self.fullscreen {
            f.stop();
        }
    }

    /// Run a one-off download on the add-on thread (the update file).
    /// False when that thread couldn't start.
    pub fn run_job(&self, job: addons::Job) -> bool {
        if let Some(a) = self.addons.as_ref() { a.run(job); return true; }
        // No permanent updater worker or WinRT client at startup. A requested
        // check/download owns its client only until that one job has finished.
        std::thread::Builder::new().name("lumo-update".into()).spawn(move || {
            let _com = http::MtaGuard::new();
            let client = http::Client::new(concat!("Lumo/", env!("CARGO_PKG_VERSION")));
            let error = client.as_ref().err().map(ToString::to_string);
            job(client.as_ref().map_err(|_| error.as_deref().unwrap_or("HTTP unavailable")));
        }).is_ok()
    }

    /// An add-on was switched on (download its files) or off (delete them).
    pub fn set_addon(&self, id: &'static str, on: bool) {
        if let Some(a) = &self.addons {
            a.set(id, on);
        }
    }

    /// Turn clipboard history capture on or off (the setting changed).
    pub fn set_clipboard_history(&self, on: bool) {
        if let Some(c) = &self.clip {
            c.set_enabled(on);
        }
    }

    /// Step aside on (the band along the top edge of the island's monitor,
    /// screen pixels) or off: the full-screen watcher reports what reaches it.
    pub fn watch_top(&self, band: Option<RECT>) {
        if let Some(f) = &self.fullscreen {
            f.watch_top(band);
        }
    }

    /// Explorer restarted: AppBar registrations are lost and must be redone.
    pub fn on_taskbar_created(&self) {
        if let Some(f) = &self.fullscreen {
            f.on_taskbar_created();
        }
    }

    pub fn media_cmd(&self, c: MediaCmd) {
        if let Some(m) = &self.media {
            m.send(c);
        }
    }

    pub fn request_lyrics(&self, s: &MediaSession, settings: &Settings) -> u64 {
        if s.music_source.is_none() { return 0; }
        let dir = (settings.local_lyrics || !settings.music_directory.trim().is_empty()).then(|| crate::paths::music_dir(settings));
        self.lyrics.as_ref().map_or(0, |l| l.request(s, dir, crate::paths::lyrics_download_dir(settings), settings.lyrics_auto_save, settings.local_lyrics))
    }

    pub fn cancel_lyrics(&self) {
        if let Some(l) = &self.lyrics { l.cancel(); }
    }

    /// Show a pairing QR code: the link, or a sentence for the panel.
    pub fn pair_begin(&self) -> Result<String, String> {
        match &self.pairdrop {
            Some(p) => p.begin_pairing(),
            None => Err("Pairing couldn't start on this PC.".into()),
        }
    }

    pub fn pair_cancel(&self) {
        if let Some(p) = &self.pairdrop {
            p.cancel_pairing();
        }
    }

    pub fn pair_remove(&self, room: &str) {
        if let Some(p) = &self.pairdrop {
            p.remove(room);
        }
    }

    /// The Shelf's files, which connected paired phones may save.
    pub fn pair_shelf(&self, paths: Vec<std::path::PathBuf>) {
        if let Some(p) = &self.pairdrop {
            p.set_shelf(paths);
        }
    }

    pub fn pairs(&self) -> Vec<lumo_core::pairdrop::Pair> {
        self.pairdrop.as_ref().map(|p| p.pairs()).unwrap_or_default()
    }

    /// The Apple Music token on disk was saved or removed.
    pub fn apple_music_token_changed(&self) {
        if let Some(l) = &self.lyrics {
            l.apple_token_changed();
        }
    }

    /// Handle the AppBar callback message; `Some(true)` when a full-screen
    /// app covers the island's monitor.
    pub fn on_appbar_message(&self, wp: WPARAM, lp: LPARAM, monitor: isize) -> Option<bool> {
        self.fullscreen.as_ref().and_then(|f| f.on_appbar_message(wp, lp, monitor))
    }
}

/// The Windows accent colour, lightened for use on the black island.
pub fn system_accent() -> Option<Color> {
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    let s = UISettings::new().ok()?;
    let c = s.GetColorValue(UIColorType::AccentLight2).ok()?;
    Some(Color::rgba(c.R as f32 / 255.0, c.G as f32 / 255.0, c.B as f32 / 255.0, 1.0).for_island_accent())
}
