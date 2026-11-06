//! User settings, stored as JSON next to the data store.

use crate::pomodoro::PomodoroConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleStyle {
    /// Small pill with a clock and battery ring.
    Clock,
    /// Small blank pill.
    Minimal,
    /// A friendly pair of blinking eyes.
    Eyes,
    /// Nothing visible until the cursor reaches the top edge.
    Hidden,
    /// A wider pill like a status bar: the date and time, the volume and
    /// the battery (the Pill theme; the other themes show the clock).
    Status,
}

/// How the island looks. Pages and live activities work the same in every theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// The plain black island.
    #[default]
    #[serde(alias = "oyen", alias = "glass")]
    Classic,
    /// Pill: the island floats a little below the top edge as a black
    /// capsule, with its own layouts: the time and battery at rest, the song
    /// while music plays, a rounded card when it opens.
    Pill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorChoice {
    Primary,
    /// The monitor under the cursor when Lumo starts.
    Cursor,
}

/// Bumped when defaults change in a way existing files should pick up.
pub const SETTINGS_VERSION: u32 = 8;

fn version_zero() -> u32 {
    0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaximizedRule {
    Show,
    Hide,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ai: crate::ai::Config,
    /// Missing in files written before versioning (treated as 0).
    #[serde(default = "version_zero")]
    pub settings_version: u32,
    pub theme: Theme,
    pub idle_style: IdleStyle,
    pub use_24h: bool,
    /// Click is the default; hover expansion is explicitly enabled in Settings.
    pub open_on_hover: bool,
    /// Delay before hover expands the island.
    pub hover_delay_ms: u32,
    /// Grace period after the cursor leaves before collapsing (0 = the
    /// island starts closing on the next frame).
    pub collapse_delay_ms: u32,
    /// Width of the expanded island in DIPs (bigger = roomier pages).
    pub expanded_width: f32,
    pub hide_in_fullscreen: bool,
    /// Keep the top edge clear over maximized windows. Individual apps can override this.
    pub hide_over_maximized: bool,
    /// Lowercase executable paths, so rules survive app/window title changes.
    pub maximized_apps: BTreeMap<String, MaximizedRule>,
    /// Step aside over maximized windows: the resting island turned into the
    /// Hidden look's invisible strip while an app window reached the top of
    /// the screen under it. 0.2.4 and 0.2.5 did it by default and it read as
    /// the island vanishing, so it's gone from the menu and Settings and
    /// always off (`sanitized`); the Hidden idle look is the way to keep the
    /// top edge clear. The key stays so files from those versions still load.
    pub step_aside: bool,
    pub monitor: MonitorChoice,
    pub show_media_activity: bool,
    pub lyrics: bool,
    /// Only the closed island; the Music page still shows lyrics.
    pub island_lyrics: bool,
    /// Show the pop-out lyrics button when the current song has lyrics.
    pub lyrics_popup_button: bool,
    /// Last manually resized lyrics window size, in DIPs.
    pub lyrics_popup_width: f32,
    pub lyrics_popup_height: f32,
    /// Local music tries sidecars, then LRCLIB; music services use LRCLIB directly.
    pub local_lyrics: bool,
    /// Empty uses Windows' Music known folder, including a redirected location.
    pub music_directory: String,
    /// Save LRCLIB results only after a Music or download folder is configured.
    pub lyrics_auto_save: bool,
    /// Empty saves beside music in the configured Music folder.
    pub lyrics_download_directory: String,
    /// Romanized pronunciation under Korean and Chinese lyrics.
    pub lyrics_pronunciation: bool,
    /// Shift synced lyrics: positive shows each line earlier (ms).
    pub lyrics_offset_ms: i32,
    pub hud_volume: bool,
    pub hud_power: bool,
    pub hud_bluetooth: bool,
    pub clipboard_history: bool,
    pub sounds: bool,
    /// Page shown first when expanding ("media", "focus", "shelf", "tasks", "clipboard", "today").
    pub start_page: String,
    /// Remember the last page instead of `start_page`.
    pub remember_page: bool,
    pub pomodoro: PomodoroConfig,
    /// Keep the System page's charts going while it's closed (a light reading
    /// of the kernel counters once a second), so they're full when it opens.
    pub system_history: bool,
    /// Add-ons switched on, by id (`lumo_core::addons`). Ids this version
    /// doesn't know are kept, so a newer Lumo's choices survive a save.
    pub addons: BTreeMap<String, bool>,
    /// Show over games: while a full-screen app is in front, the island
    /// shows its frame rate, the GPU's load and temperature and the CPU's
    /// load. Turning it on also keeps the island on top of full-screen apps.
    pub game_stats: bool,
    /// Show over games, readout only for a few seconds (when the game comes
    /// to the front, when the frame rate drops sharply, on Alt+Shift+F),
    /// then the island hides and Windows sends the game's frames straight to
    /// the screen. Off (the default), the readout stays while the game is in
    /// front. Its own name: the corner test build wrote `game_readout`
    /// ("peek" as its default) into the files of the people who tried it.
    pub game_peek: bool,
    /// Tell me about new versions: read the update file a couple of minutes
    /// after start and then twice a day (docs/updates.md), and say on the
    /// island once when a new version is out.
    pub update_check: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_version: SETTINGS_VERSION,
            ai: crate::ai::Config::default(),
            theme: Theme::Classic,
            idle_style: IdleStyle::Clock,
            use_24h: true,
            open_on_hover: false,
            hover_delay_ms: 220,
            collapse_delay_ms: 0,
            expanded_width: 620.0,
            hide_in_fullscreen: true,
            hide_over_maximized: true,
            maximized_apps: BTreeMap::new(),
            step_aside: false,
            monitor: MonitorChoice::Primary,
            show_media_activity: true,
            lyrics: true,
            island_lyrics: true,
            lyrics_popup_button: true,
            lyrics_popup_width: 430.0,
            lyrics_popup_height: 640.0,
            local_lyrics: false,
            music_directory: String::new(),
            lyrics_auto_save: true,
            lyrics_download_directory: String::new(),
            lyrics_pronunciation: true,
            lyrics_offset_ms: 0,
            hud_volume: true,
            hud_power: true,
            hud_bluetooth: true,
            clipboard_history: false,
            sounds: true,
            start_page: "media".into(),
            remember_page: true,
            pomodoro: PomodoroConfig::default(),
            system_history: true,
            addons: BTreeMap::new(),
            game_stats: false,
            game_peek: false,
            update_check: false,
        }
    }
}

impl Settings {
    pub fn keeps_app_visible(&self, executable: &str) -> bool {
        self.maximized_apps.get(executable) == Some(&MaximizedRule::Show)
    }

    pub fn keep_app_visible(&mut self, executable: String, keep: bool) {
        if keep {
            self.maximized_apps.insert(executable, MaximizedRule::Show);
        } else {
            self.maximized_apps.remove(&executable);
        }
    }

    pub fn hide_for_maximized_app(&self, executable: &str) -> bool {
        match self.maximized_apps.get(executable) {
            Some(MaximizedRule::Show) => false,
            Some(MaximizedRule::Hide) => true,
            None => self.hide_over_maximized,
        }
    }
}

impl Settings {
    /// Show over games needs the island on top of full-screen apps, so the
    /// two follow each other: turning the readout on picks Stay on top, and
    /// picking Hide afterwards turns the readout off. `before` is the state
    /// the change started from.
    pub fn pair_game_stats(&mut self, before: &Settings) {
        if !(self.game_stats && self.hide_in_fullscreen) {
            return;
        }
        if before.game_stats && !before.hide_in_fullscreen {
            self.game_stats = false;
        } else {
            self.hide_in_fullscreen = false;
        }
    }

    /// Is the add-on `id` switched on?
    /// Clipboard history's switch is `clipboard_history`, which the tray flips too.
    pub fn addon_on(&self, id: &str) -> bool {
        if id == crate::addons::CLIPBOARD_HISTORY.id {
            return self.clipboard_history;
        }
        self.addons.get(id).copied().unwrap_or(false)
    }

    pub fn set_addon(&mut self, id: &str, on: bool) {
        if id == crate::addons::CLIPBOARD_HISTORY.id {
            self.clipboard_history = on;
            return;
        }
        self.addons.insert(id.to_string(), on);
    }

    /// Whether the person has never set this add-on preference.
    pub fn addon_unset(&self, id: &str) -> bool {
        !self.addons.contains_key(id)
    }

    /// Clamp values a user may have hand-edited into nonsense.
    pub fn sanitized(mut self) -> Self {
        // Retired add-ons must never restart from an older settings file.
        self.addons.retain(|id, _| !matches!(id.as_str(), "sholat" | "ja" | "health" | "money" | "oyenza"));
        // The former master switch now controls the closed island only.
        self.island_lyrics &= self.lyrics;
        self.lyrics = true;
        if self.idle_style == IdleStyle::Eyes {
            self.idle_style = IdleStyle::Clock;
        }
        // Only untouched old defaults move.
        // v2: hover opens after a 220 ms dwell (140 ms caught pointers on
        // their way to browser tabs).
        if self.settings_version < 2 && self.hover_delay_ms == 140 {
            self.hover_delay_ms = 220;
        }
        // v3: the island closes as soon as the pointer leaves it (the old
        // 380 ms grace read as lag; coming back while it closes reopens it).
        if self.settings_version < 3 && self.collapse_delay_ms == 380 {
            self.collapse_delay_ms = 0;
        }
        // v4: Clipboard history became an add-on, off by default, and is
        // switched off once for everyone (it had been on by default); Add-ons
        // or the tray bring it back. A Clipboard start page opens Music.
        if self.settings_version < 4 {
            self.clipboard_history = false;
            if self.start_page == "clipboard" {
                self.start_page = "media".into();
            }
        }
        // v8: over maximized apps, reveal compact content on hover and open
        // it only on click. Normal and snapped windows stay visible.
        if self.settings_version < 8 {
            self.hide_over_maximized = true;
        }
        self.settings_version = self.settings_version.max(SETTINGS_VERSION);
        // Legacy overlap-based Step aside is replaced by maximized-app rules.
        self.step_aside = false;
        // Show over games needs the island on top of full-screen apps.
        if self.game_stats {
            self.hide_in_fullscreen = false;
        }
        self.hover_delay_ms = self.hover_delay_ms.min(2000);
        self.lyrics_offset_ms = self.lyrics_offset_ms.clamp(-5000, 5000);
        self.collapse_delay_ms = self.collapse_delay_ms.min(5000);
        if !self.expanded_width.is_finite() {
            self.expanded_width = 620.0;
        }
        self.expanded_width = self.expanded_width.clamp(480.0, 900.0);
        if !self.lyrics_popup_width.is_finite() { self.lyrics_popup_width = 430.0; }
        if !self.lyrics_popup_height.is_finite() { self.lyrics_popup_height = 640.0; }
        self.lyrics_popup_width = self.lyrics_popup_width.clamp(340.0, 1000.0);
        self.lyrics_popup_height = self.lyrics_popup_height.clamp(420.0, 1100.0);
        let p = &mut self.pomodoro;
        p.focus_min = p.focus_min.clamp(1, 180);
        p.short_break_min = p.short_break_min.clamp(1, 60);
        p.long_break_min = p.long_break_min.clamp(1, 120);
        p.long_every = p.long_every.min(12);
        self
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lyrics_window_preferences_survive_saving_and_reject_invalid_sizes() {
        let mut settings = Settings { lyrics_popup_button: false, lyrics_popup_width: 720.0, lyrics_popup_height: 540.0, ..Settings::default() };
        let saved: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        let saved = saved.sanitized();
        assert!(!saved.lyrics_popup_button);
        assert_eq!((saved.lyrics_popup_width, saved.lyrics_popup_height), (720.0, 540.0));
        settings.lyrics_popup_width = f32::NAN;
        settings.lyrics_popup_height = f32::INFINITY;
        let clean = settings.sanitized();
        assert_eq!((clean.lyrics_popup_width, clean.lyrics_popup_height), (430.0, 640.0));
    }
    #[test]
    fn maximized_hover_reveal_migrates_and_remains_configurable() {
        let defaults = Settings::default();
        assert!(!defaults.open_on_hover && defaults.hide_over_maximized);
        let mut old: Settings = serde_json::from_str(r#"{"settings_version":7,"hide_over_maximized":false,"hover_delay_ms":400,"maximized_apps":{"editor.exe":"show"}}"#).unwrap();
        old = old.sanitized();
        assert!(!old.open_on_hover && old.hide_over_maximized);
        assert_eq!(old.hover_delay_ms, 400);
        assert!(!old.hide_for_maximized_app("editor.exe"));
        assert_eq!(old.settings_version, SETTINGS_VERSION);
        old.open_on_hover = true;
        old.hide_over_maximized = false;
        let saved: Settings = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
        let saved = saved.sanitized();
        assert!(saved.open_on_hover && !saved.hide_over_maximized, "explicit choices must survive the next launch");
    }

    #[test]
    fn maximized_app_rules_override_the_default_and_survive_saving() {
        let mut s = Settings { hide_over_maximized: true, ..Settings::default() };
        assert!(s.hide_for_maximized_app("browser.exe"));
        s.maximized_apps.insert("music.exe".into(), MaximizedRule::Show);
        s.maximized_apps.insert("editor.exe".into(), MaximizedRule::Hide);
        s.hide_over_maximized = false;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.hide_for_maximized_app("browser.exe"));
        assert!(!back.hide_for_maximized_app("music.exe"));
        assert!(back.hide_for_maximized_app("editor.exe"));
        s.hide_over_maximized = true;
        s.keep_app_visible("browser.exe".into(), true);
        let mut back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(back.keeps_app_visible("browser.exe"));
        assert!(!back.hide_for_maximized_app("browser.exe"));
        assert!(back.hide_for_maximized_app("other.exe"));
        back.keep_app_visible("browser.exe".into(), false);
        assert!(!back.keeps_app_visible("browser.exe"));
        assert!(back.hide_for_maximized_app("browser.exe"));
    }

    #[test]
    fn legacy_lyrics_and_eyes_preferences_migrate_to_the_new_controls() {
        let old: Settings = serde_json::from_str(r#"{"lyrics":false,"idle_style":"eyes"}"#).unwrap();
        let s = old.sanitized();
        assert!(!s.island_lyrics);
        assert!(s.lyrics, "the expanded Music page keeps its lyrics");
        assert_eq!(s.idle_style, IdleStyle::Clock);
        assert!(!s.sanitized().island_lyrics, "a second load keeps the choice");
    }
    use super::*;

    #[test]
    fn partial_json_fills_defaults() {
        // An old file's "water" block (a retired page) is ignored.
        let s: Settings = serde_json::from_str(r#"{"idle_style":"eyes","water":{"goal_ml":1500}}"#).unwrap();
        assert_eq!(s.idle_style, IdleStyle::Eyes);
        assert_eq!(s.pomodoro.focus_min, 25);
        assert!(!s.local_lyrics);
        assert!(s.music_directory.is_empty());
        assert!(s.lyrics_auto_save);
        assert!(s.lyrics_download_directory.is_empty());
    }

    #[test]
    fn addons_are_off_until_switched_on_and_unknown_ones_survive() {
        let mut s = Settings::default();
        assert!(!s.addon_on("script"));
        s.set_addon("script", true);
        assert!(s.addon_on("script"));
        // A newer Lumo's add-on stays in the file when this one saves it.
        let s: Settings = serde_json::from_str(r#"{"addons":{"script":true,"future":true}}"#).unwrap();
        assert!(s.addon_on("script"));
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(back.addon_on("future"));
        // Files from before add-ons have none on.
        let old: Settings = serde_json::from_str(r#"{"settings_version":3}"#).unwrap();
        assert!(old.addons.is_empty());
    }

    #[test]
    fn an_addon_switched_off_by_hand_is_no_longer_unset() {
        let mut s = Settings::default();
        assert!(s.addon_unset("script"));
        s.set_addon("script", true);
        assert!(!s.addon_unset("script"));
        s.set_addon("script", false);
        assert!(!s.addon_unset("script"), "off by hand: the person's word, kept");
        assert!(!s.addon_on("script"));
        // It survives a save and a load.
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.addon_unset("script"));
    }

    #[test]
    fn retired_addons_are_dropped_when_old_settings_are_loaded() {
        let s: Settings = serde_json::from_str(r#"{"addons":{"sholat":true,"ja":true,"health":true,"money":true,"script":true,"future":true},"prayer":{"place":"jakarta"}}"#).unwrap();
        let s = s.sanitized();
        assert!(s.addon_on("script") && s.addon_on("future"));
        for id in ["sholat", "ja", "health", "money"] {
            assert!(!s.addons.contains_key(id));
        }
        assert!(!Settings::default().update_check);
    }

    #[test]
    fn theme_defaults_to_classic() {
        // Files written before themes existed keep the black island.
        let s: Settings = serde_json::from_str(r#"{"idle_style":"eyes"}"#).unwrap();
        assert_eq!(s.theme, Theme::Classic);
        assert_eq!(serde_json::to_value(&s).unwrap()["theme"], "classic");
        let s: Settings = serde_json::from_str(r#"{"theme":"pill"}"#).unwrap();
        assert_eq!(s.theme, Theme::Pill);
    }

    #[test]
    fn sanitize_clamps() {
        let s = Settings { expanded_width: 5000.0, hover_delay_ms: 99999, ..Default::default() }.sanitized();
        assert_eq!(s.expanded_width, 900.0);
        assert_eq!(s.hover_delay_ms, 2000);
    }

    #[test]
    fn migrates_old_hover_default_once() {
        let s: Settings = serde_json::from_str(r#"{"hover_delay_ms":140}"#).unwrap();
        assert_eq!(s.settings_version, 0);
        let s = s.sanitized();
        assert_eq!((s.hover_delay_ms, s.settings_version), (220, SETTINGS_VERSION));
        // A user's own choice is kept, and a current file isn't touched.
        let s: Settings = serde_json::from_str(r#"{"hover_delay_ms":90}"#).unwrap();
        assert_eq!(s.sanitized().hover_delay_ms, 90);
        let s: Settings = serde_json::from_str(r#"{"settings_version":2,"hover_delay_ms":140}"#).unwrap();
        assert_eq!(s.sanitized().hover_delay_ms, 140);
    }

    #[test]
    fn clipboard_history_goes_off_once_and_lives_in_add_ons() {
        assert!(!Settings::default().clipboard_history);
        // Every file from before 0.2.7 had it on: off once, and a Clipboard start page opens Music.
        let s = serde_json::from_str::<Settings>(r#"{"settings_version":3,"clipboard_history":true,"start_page":"clipboard"}"#).unwrap().sanitized();
        assert_eq!((s.clipboard_history, s.start_page.as_str(), s.settings_version), (false, "media", SETTINGS_VERSION));
        // Switched back on afterwards, it stays on.
        let s = serde_json::from_str::<Settings>(r#"{"settings_version":4,"clipboard_history":true}"#).unwrap().sanitized();
        assert!(s.clipboard_history);
        // The Add-ons switch and the tray's item are the same switch.
        let mut s = Settings::default();
        s.set_addon(crate::addons::CLIPBOARD_HISTORY.id, true);
        assert!(s.clipboard_history && s.addon_on("clipboard") && !s.addons.contains_key("clipboard"));
        s.clipboard_history = false;
        assert!(!s.addon_on("clipboard"));
    }

    #[test]
    fn migrates_old_collapse_grace_once() {
        // Files from 0.1.7 and earlier hold the old 380 ms grace.
        let s = serde_json::from_str::<Settings>(r#"{"settings_version":2,"collapse_delay_ms":380}"#).unwrap().sanitized();
        assert_eq!((s.collapse_delay_ms, s.settings_version), (0, SETTINGS_VERSION));
        let s = serde_json::from_str::<Settings>(r#"{"collapse_delay_ms":380,"hover_delay_ms":140}"#).unwrap().sanitized();
        assert_eq!((s.collapse_delay_ms, s.hover_delay_ms), (0, 220));
        // A grace the user chose is kept, a current file isn't touched, and 0 stays 0.
        let s: Settings = serde_json::from_str(r#"{"settings_version":2,"collapse_delay_ms":600}"#).unwrap();
        assert_eq!(s.sanitized().collapse_delay_ms, 600);
        let s: Settings = serde_json::from_str(r#"{"settings_version":3,"collapse_delay_ms":380}"#).unwrap();
        assert_eq!(s.sanitized().collapse_delay_ms, 380);
        assert_eq!(Settings::default().sanitized().collapse_delay_ms, 0);
        // A file from a newer Lumo keeps its version.
        let s: Settings = serde_json::from_str(r#"{"settings_version":9}"#).unwrap();
        assert_eq!(s.sanitized().settings_version, 9);
    }

    #[test]
    fn game_stats_and_full_screen_choice_follow_each_other() {
        // The default hides the island over full-screen apps; turning the
        // readout on keeps it on top instead.
        let before = Settings::default();
        let mut s = Settings { game_stats: true, ..before.clone() };
        s.pair_game_stats(&before);
        assert!(s.game_stats && !s.hide_in_fullscreen);
        // Picking Hide afterwards turns the readout off.
        let before = s.clone();
        let mut s = Settings { hide_in_fullscreen: true, ..before.clone() };
        s.pair_game_stats(&before);
        assert!(!s.game_stats && s.hide_in_fullscreen);
        // Stay on top alone, and the readout off, change nothing.
        let before = Settings { hide_in_fullscreen: false, ..Settings::default() };
        let mut s = before.clone();
        s.pair_game_stats(&before);
        assert_eq!(s, before);
        // A hand-edited file with both on keeps the readout, when changed
        // and when loaded.
        let mut s = Settings { game_stats: true, ..Settings::default() };
        s.pair_game_stats(&Settings::default());
        assert!(s.game_stats && !s.hide_in_fullscreen);
        let s: Settings = serde_json::from_str(r#"{"game_stats":true,"hide_in_fullscreen":true}"#).unwrap();
        let s = s.sanitized();
        assert!(s.game_stats && !s.hide_in_fullscreen);
    }

    #[test]
    fn the_readout_stays_unless_peeks_are_picked() {
        // Files from 0.2.0 and before, and from the corner test build (its
        // "peek" was its default, not a choice), get the readout that stays.
        for old in [r#"{"game_stats":true}"#, r#"{"game_stats":true,"game_readout":"peek"}"#, r#"{"game_readout":"corner"}"#] {
            let s: Settings = serde_json::from_str(old).unwrap();
            assert!(!s.game_peek, "{old}");
        }
        let s: Settings = serde_json::from_str(r#"{"game_stats":true,"game_peek":true}"#).unwrap();
        assert!(s.game_peek);
    }

    #[test]
    fn the_island_never_steps_aside() {
        assert!(!Settings::default().step_aside);
        // 0.2.4 and 0.2.5 saved Step aside, their default: the island stays now.
        let old: Settings = serde_json::from_str(r#"{"settings_version":3,"step_aside":true}"#).unwrap();
        assert!(!old.sanitized().step_aside);
    }

    #[test]
    fn roundtrip() {
        let s = Settings::default();
        let j = serde_json::to_string_pretty(&s).unwrap();
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
