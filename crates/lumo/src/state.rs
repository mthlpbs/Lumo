//! All state shared by pages, activities and the app loop.
//!
//! Owned by the UI thread. Services never touch it directly; they send
//! [`crate::events::AppEvent`]s that the app applies here.

use crate::commands::{Command, MediaCmd, Sound};
use crate::gfx::Image;
use crate::icons::Icon;
use crate::pages::PageKind;
use lumo_core::addons::Status as AddonStatus;
use lumo_core::cliphist::ClipHistory;
use lumo_core::datetime::LocalDateTime;
use lumo_core::geom::{palette, Color};
use lumo_core::lrc::Lyrics;
use lumo_core::phonedrop::QrMatrix;
use lumo_core::pomodoro::Pomodoro;
use lumo_core::romanize::Ruby;
use lumo_core::settings::Settings;
use lumo_core::store::Store;
use lumo_core::timeline::Timeline;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub struct MediaSession {
    /// AppUserModelId of the player (e.g. `Spotify.exe`, `Chrome`).
    pub app_id: String,
    /// Friendly player name ("Spotify", "Chrome", …).
    pub app_name: String,
    /// Only local music and identified music services offer lyrics.
    pub music_source: Option<lumo_core::nowplaying::MusicSource>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: Option<Image>,
    /// Accent derived from the art (for progress bars / the activity glow).
    pub art_color: Option<Color>,
    pub timeline: Timeline,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_play_pause: bool,
    pub can_seek: bool,
    /// Identity of the track: `title\u{1}artist\u{1}album`.
    pub track_key: String,
}

impl MediaSession {
    pub fn make_track_key(title: &str, artist: &str, album: &str) -> String {
        format!("{title}\u{1}{artist}\u{1}{album}")
    }
    pub fn is_playing(&self) -> bool {
        self.timeline.playing
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LyricsStatus {
    Idle,
    Loading,
    Found,
    Instrumental,
    NotFound,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct LyricsState {
    pub track_key: String,
    pub status: LyricsStatus,
    /// Identifies the latest lookup, including source/folder changes for the same song.
    pub request_id: u64,
    pub source_file: Option<String>,
    pub lyrics: Option<Lyrics>,
    pub source: String,
    /// Each line's pieces with their readings in Latin letters (Japanese,
    /// Korean, Chinese); empty for lines that need none.
    pub ruby: Vec<Vec<Ruby>>,
}

impl Default for LyricsState {
    fn default() -> Self {
        Self { track_key: String::new(), status: LyricsStatus::Idle, request_id: 0, source_file: None, lyrics: None, source: String::new(), ruby: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeState {
    pub level: f32,
    pub muted: bool,
    pub known: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PowerState {
    pub has_battery: bool,
    /// 0..=100 (meaningful only with a battery).
    pub percent: u8,
    pub charging: bool,
    pub on_ac: bool,
    pub saver: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HudKind {
    Volume { level: f32, muted: bool },
    Charging { percent: u8, plugged: bool },
    Bluetooth { name: String, connected: bool, battery: Option<u8>, audio: bool },
    LowBattery { percent: u8 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hud {
    pub kind: HudKind,
    /// Monotonic time when it disappears.
    pub until: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NoticeAction {
    OpenPage(PageKind),
    /// Open the exact completed chat, independent of the current activity.
    OpenAiReply(u64),
    OpenAiSite(crate::ai_sites::Site),
    /// Open Focus and start the (idle) timer.
    StartFocus,
    /// Open the Update sheet in Settings (a new version's notice).
    OpenUpdate,
}

/// A one-click action shown as a small pill inside a notice (doesn't expand).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineAction {
    /// Start the idle Pomodoro phase.
    StartFocus,
    /// Put the notice away now (a new version's: the menu and Settings
    /// keep offering it).
    Hide,
    /// Open this version's release notes ("Updated to").
    WhatsNew,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub icon: Icon,
    pub color: Color,
    pub title: String,
    /// Dimmer words after the title, on its line ("0.2.5" after "Update
    /// available"); empty for most notices.
    pub title_dim: String,
    pub subtitle: String,
    pub until: f64,
    pub action: Option<NoticeAction>,
    pub inline: Option<InlineAction>,
}
impl Notice {
    /// An unread reply waits for an explicit open/dismiss, without a timer.
    pub fn is_ai_reply(&self) -> bool {
        matches!(self.action,Some(NoticeAction::OpenAiReply(_)|NoticeAction::OpenAiSite(_)))
    }
}

/// What the app knows about new versions.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Failed(String),
    Installing,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateView {
    pub status: UpdateStatus,
    pub checked: u64,
    pub managed: bool,
    /// A later release than this copy, with a download for this system
    /// (only while the checks are on).
    pub offer: Option<lumo_core::update::Release>,
    /// Settings shows the Update sheet in place of its rows (until the
    /// island closes or the page changes).
    pub sheet: bool,
    /// Its Setup is downloaded and checked (Windows): the sheet offers
    /// Install now and Install on next start.
    pub ready: bool,
    /// Install on next start was picked for it.
    pub next_start: bool,
}

/// A phone wants to send these files; the Shelf asks the person at the PC.
#[derive(Clone, Debug, PartialEq)]
pub struct PhoneAsk {
    pub id: u64,
    /// "iPhone", "Android phone", …
    pub from: String,
    /// (name, size) in the order the phone sends them.
    pub files: Vec<(String, u64)>,
    pub total: u64,
}

/// Files coming in from the phone.
#[derive(Clone, Debug, PartialEq)]
pub struct PhoneTransfer {
    pub id: u64,
    pub from: String,
    /// The file being sent (0-based) and how many there are.
    pub file: usize,
    pub files: usize,
    pub done: u64,
    pub total: u64,
}

/// What the phone panel shows on its left and right.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PhoneView {
    /// Today's same-Wi-Fi QR code.
    #[default]
    Lan,
    /// A pairing QR code (`services::pairdrop`): the link, its code and
    /// until when (app clock) it works.
    Pairing { url: String, qr: Option<QrMatrix>, until: f64 },
    /// The paired phones, each with Remove.
    Paired,
}

/// A paired phone as the panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedPhone {
    /// Its room on the meeting server (what Remove names).
    pub room: String,
    pub name: String,
    /// Unix seconds it last connected (0 = not yet).
    pub seen: i64,
}

/// Phone drop while the Shelf's phone panel is open: the link, its QR code
/// and what the phone is doing (`services::phonedrop` runs the server).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhoneDrop {
    pub view: PhoneView,
    /// A second line under `outcome` (None = the usual hint).
    pub outcome_line: Option<String>,
    /// The link the QR code carries (empty while starting or after an error).
    pub url: String,
    pub qr: Option<QrMatrix>,
    pub pc_name: String,
    /// Why it couldn't start, as a sentence for the island.
    pub error: Option<String>,
    /// The kind of phone that last opened the page ("iPhone").
    pub visitor: Option<String>,
    pub ask: Option<PhoneAsk>,
    pub transfer: Option<PhoneTransfer>,
    /// The last result, whether it's good news, and until when it shows.
    pub outcome: Option<(String, bool, f64)>,
}

impl PhoneDrop {
    /// The island stays open while a QR code waits for its first phone and
    /// while a phone waits for an answer. Once the panel has shown a result
    /// (a phone paired, files arrived), or on the list of paired phones,
    /// nothing waits: the island closes when the pointer leaves.
    pub fn holds_island(&self) -> bool {
        let waiting = !matches!(self.view, PhoneView::Paired) && self.outcome.is_none();
        self.ask.is_some() || (waiting && self.error.is_none() && self.visitor.is_none() && self.transfer.is_none())
    }
}

/// Transient per-page UI state (scroll offsets, text being typed, …).
#[derive(Clone, Debug, Default)]
pub struct PageUiState {
    pub web: crate::web_settings::Editor,
    pub choosing_music_dir: bool,
    pub choosing_lyrics_dir: bool,
    pub choosing_phone_dir: bool,
    pub task_input: String,
    pub task_scroll: f32,
    pub clip_scroll: f32,
    pub shelf_scroll: f32,
    /// Smoothly animated lyric scroll position (line index as float).
    pub lyric_pos: f32,
    pub lyric_track: String,
    /// Clipboard row that just got copied (index, mono time) for a "Copied" flash.
    pub clip_flash: Option<(usize, f64)>,
    /// Volume HUD bar position, gliding toward the real level.
    pub hud_level: f32,
    /// Collapsed media pill shows the track title until this mono time.
    pub peek_until: f64,
    /// Header tab under the pointer and since when (for hover labels).
    pub tab_hover: Option<(usize, f64)>,
    /// The module the System page shows under its strip.
    pub system_module: lumo_core::sysmon::Module,
    /// The last System reading had a GPU (keeps its chip while the next one is on its way).
    pub system_gpu: bool,
    /// The last full System reading had the whole PC's power (keeps its chip
    /// while the page's first, light reading shows).
    pub system_power: bool,
    /// The page the Settings button was pressed on (pressing it again goes back).
    pub before_settings: Option<PageKind>,
    /// Start with Windows, as last read off the UI thread (None until Settings first opens).
    pub autostart: Option<bool>,
}

/// Whether Windows lets Lumo read a game's frames. Its event tracing
/// is open to admins and to the Performance Log Users group; the one-time
/// OK adds the account to that group, which counts from the next sign-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum FpsAccess {
    /// Not asked yet.
    #[default]
    Unknown,
    /// Being checked.
    Checking,
    /// Frames can be read.
    Ready,
    /// The one-time OK is needed.
    NeedsOk,
    /// Windows' prompt for it is open.
    Asking,
    /// The account is in the group; Windows applies it at the next sign-in.
    SignOut,
    /// The OK didn't go through (why, in a few words).
    Failed(String),
}

/// Show over games: the readout in the island over a full-screen app
/// (`activities::Activity::Game`) and the System page's Games view.
#[derive(Clone, Debug, Default)]
pub struct GameState {
    /// The readout is on screen: Show over games is on and a full-screen
    /// app is in front.
    pub active: bool,
    /// The full-screen app ("cs2.exe").
    pub app: Option<String>,
    /// Its frame rate, while it draws and the frames can be read.
    pub rate: Option<lumo_core::frames::FrameRate>,
    /// Frames per second over the last minute in front.
    pub history: lumo_core::sysmon::Series,
    pub access: FpsAccess,
}

/// Last foreground app, retained while the island itself has focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundApp {
    pub executable: String,
    pub name: String,
    pub maximized_here: bool,
}

pub struct AppState {
    pub closed_view:lumo_core::settings::ClosedView,
    pub search: crate::search::State,
    pub ai: crate::ai::State,
    pub foreground_app: Option<ForegroundApp>,
    pub settings: Settings,
    pub store: Store,
    /// Monotonic seconds at the current frame.
    pub now: f64,
    pub local: LocalDateTime,
    pub unix: i64,
    pub media: Option<MediaSession>,
    /// Temporarily suppress island lyrics while their popup is open.
    pub lyrics_popup_open: bool,
    /// Optimistic seek feedback, shared by both themes and kept through late
    /// queued reports. It expires with the worker's bounded confirmation wait.
    pub media_pending_seek: Option<lumo_core::timeline::PendingSeek>,
    pub lyrics: LyricsState,
    pub pomodoro: Pomodoro,
    pub volume: VolumeState,
    pub power: PowerState,
    pub hud: Option<Hud>,
    pub notice: Option<Notice>,
    pub clip: ClipHistory,
    /// The System page's readings and their last minute (empty while it's closed).
    pub system: lumo_core::sysmon::Monitor,
    /// Show over games.
    pub game: GameState,
    pub shelf_thumbs: HashMap<String, Option<Image>>,
    /// Icons of the apps the System page lists, by executable path (None = loading or none).
    pub app_icons: HashMap<String, Option<Image>>,
    pub ui: PageUiState,
    /// System accent colour (falls back to blue).
    pub accent: Color,
    /// The page currently shown when expanded.
    pub page: PageKind,
    /// When playback went from playing to paused (the pill lingers 30 s).
    pub media_paused_at: Option<f64>,
    pub pinned: bool,
    /// Stepped aside (Settings › Over maximized windows): an app window sits
    /// under where the island rests, so the closed island rests as an
    /// invisible strip at the top edge (`activities::Mode::Aside`). Windows
    /// sets it.
    pub aside: bool,
    /// Files are being dragged over the island right now.
    pub drag_hover: bool,
    /// Phone drop, while it's open.
    pub phone: Option<PhoneDrop>,
    /// Paired phones, newest first (`services::pairdrop`).
    pub paired: Vec<PairedPhone>,
    /// Whether the meeting server answers (None = not asked yet: no pairs).
    pub pair_server: Option<bool>,
    /// Where each add-on stands, by id (every one in `lumo_core::addons::ALL`).
    pub addons: BTreeMap<&'static str, AddonStatus>,
    /// New versions (`crate::updates`): what Settings, the menu and the
    /// island's notice show.
    pub update: UpdateView,
    pub script: lumo_core::script::Reader,
    /// A message under the script until a time (mono seconds).
    pub script_note: Option<(&'static str, f64)>,
    /// Whether Windows leaves the island out of screen capture while the
    /// Script page shows (None = not asked yet, or not on Windows).
    pub script_hidden: Option<bool>,
    /// Lumo Recorder is installed, so the Script page shows Record.
    /// Asked each time the page comes on screen, never at rest.
    pub recorder: bool,
    /// Side effects requested during the frame.
    pub cmds: Vec<Command>,
}

impl AppState {
    /// The Shelf's phone panel keeps the open island from closing, only
    /// while the Shelf is the page on screen.
    pub fn phone_holds_island(&self) -> bool {
        self.page == PageKind::Shelf && self.phone.as_ref().is_some_and(|p| p.holds_island())
    }

    /// A reading under way on the Script page keeps the open island from
    /// closing, while that page is on screen.
    pub fn script_holds_island(&self) -> bool {
        self.page == PageKind::Script && self.script.active
    }

    /// Something on the page on screen keeps the open island from closing.
    pub fn page_holds_island(&self) -> bool {
        self.phone_holds_island() || self.script_holds_island()
    }

    pub fn new(settings: Settings, mut store: Store) -> Self {
        lumo_core::pinned_sites::sanitize(&mut store.pinned_sites);
        store.search_usage.retain(|id,_|!id.starts_with("file:"));
        let pomodoro = Pomodoro::new(settings.pomodoro.clone());
        let page = PageKind::from_key(if settings.remember_page && !store.last_page.is_empty() {
            &store.last_page
        } else {
            &settings.start_page
        });
        let mut st = Self {
            closed_view:Default::default(),
            search: crate::search::State::default(),
            ai: crate::ai::State::default(),
            settings,
            foreground_app: None,
            store,
            now: 0.0,
            local: LocalDateTime::default(),
            unix: 0,
            media: None,
            lyrics_popup_open: false,
            media_pending_seek: None,
            lyrics: LyricsState::default(),
            pomodoro,
            volume: VolumeState::default(),
            power: PowerState::default(),
            hud: None,
            notice: None,
            clip: ClipHistory::default(),
            system: lumo_core::sysmon::Monitor::default(),
            game: GameState::default(),
            shelf_thumbs: HashMap::new(),
            app_icons: HashMap::new(),
            ui: PageUiState::default(),
            accent: palette::BLUE,
            page,
            media_paused_at: None,
            pinned: false,
            aside: false,
            drag_hover: false,
            phone: None,
            paired: Vec::new(),
            pair_server: None,
            addons: BTreeMap::new(),
            update: UpdateView::default(),
            script: lumo_core::script::Reader::default(),
            script_note: None,
            script_hidden: None,
            recorder: false,
            cmds: Vec::new(),
        };
        // On ones start by checking their files (the worker reports back);
        // one without files is ready at once.
        for a in lumo_core::addons::ALL {
            let on = st.settings.addon_on(a.id);
            st.addons.insert(a.id, if !on { AddonStatus::Off } else if a.downloads() { AddonStatus::Checking } else { AddonStatus::Ready });
        }
        // Last open on an optional page, since switched off: open
        // where a new island does.
        if (st.page == PageKind::Script && !st.script_on()) || (st.page == PageKind::Clipboard && !st.settings.clipboard_history) || (st.page == PageKind::Ai && !st.ai_on()) {
            st.page = PageKind::ALL[0];
        }
        st
    }

    /// Where the add-on `id` stands (`Off` for one this version doesn't know).
    pub fn addon(&self, id: &str) -> AddonStatus {
        self.addons.get(id).copied().unwrap_or(AddonStatus::Off)
    }

    /// The Script reader add-on is on: the Script tab shows.
    pub fn script_on(&self) -> bool {
        self.addon(lumo_core::addons::SCRIPT_READER.id) == AddonStatus::Ready
    }

    pub fn ai_on(&self) -> bool {
        self.settings.addon_on(lumo_core::addons::AI_COMPANION.id)
    }

    pub fn cmd(&mut self, c: Command) {
        self.cmds.push(c);
    }

    pub fn media_cmd(&mut self, c: MediaCmd) {
        self.cmds.push(Command::Media(c));
    }

    /// Mark the persisted store as changed.
    pub fn save(&mut self) {
        if !self.cmds.contains(&Command::SaveStore) {
            self.cmds.push(Command::SaveStore);
        }
    }

    pub fn chime(&mut self) {
        if self.settings.sounds {
            self.cmds.push(Command::Play(Sound::Chime));
        }
    }

    /// Show a notice pill for `secs` seconds.
    pub fn notify(&mut self, icon: Icon, color: Color, title: impl Into<String>, subtitle: impl Into<String>, secs: f64, action: Option<NoticeAction>) {
        self.notice = Some(Notice { icon, color, title: title.into(), title_dim: String::new(), subtitle: subtitle.into(), until: self.now + secs, action, inline: None });
    }

    /// The Update sheet: Settings, with the sheet in place of its rows (the
    /// gear goes back to the page it was opened from).
    pub fn open_update(&mut self) {
        self.cmd(Command::OpenUpdateSettings);
    }

    /// Give the current notice a one-click inline action pill.
    pub fn set_notice_inline(&mut self, a: InlineAction) {
        if let Some(n) = self.notice.as_mut() {
            n.inline = Some(a);
        }
    }

    pub fn show_hud(&mut self, kind: HudKind, secs: f64) {
        self.hud = Some(Hud { kind, until: self.now + secs });
    }

    pub fn day_key(&self) -> String {
        self.local.day_key()
    }

    pub fn day_start_unix(&self) -> i64 {
        crate::clock::day_start_unix(&self.local, self.unix)
    }

    /// Accent for media UI: art colour when available, else the system accent.
    pub fn media_accent(&self) -> Color {
        self.media.as_ref().and_then(|m| m.art_color).unwrap_or(self.accent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_panel_holds_only_while_a_code_waits_on_the_shelf() {
        let mut st = AppState::new(Settings::default(), Store::default());
        st.phone = Some(PhoneDrop::default());
        st.page = PageKind::Shelf;
        assert!(st.phone_holds_island(), "a fresh QR code waits for a phone");
        st.page = PageKind::Media;
        assert!(!st.phone_holds_island(), "another page: nothing to scan on screen");
        st.page = PageKind::Shelf;
        // A phone finished pairing: the panel is back on its Wi-Fi code with a result.
        st.phone.as_mut().unwrap().outcome = Some(("iPhone is paired".into(), true, 8.0));
        assert!(!st.phone_holds_island());
        let pd = st.phone.as_mut().unwrap();
        pd.outcome = None;
        pd.view = PhoneView::Paired;
        assert!(!st.phone_holds_island(), "the paired list has no code to scan");
        let pd = st.phone.as_mut().unwrap();
        pd.view = PhoneView::Lan;
        pd.visitor = Some("iPhone".into());
        pd.ask = Some(PhoneAsk { id: 1, from: "iPhone".into(), files: vec![], total: 0 });
        assert!(st.phone_holds_island(), "a phone waits for an answer");
    }

}
