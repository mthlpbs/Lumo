//! Side effects requested by pages; executed by the app after each frame.

use crate::pages::PageKind;

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCmd {
    PlayPause,
    Next,
    Prev,
    /// Seek to absolute position in seconds.
    Seek(f64),
    /// The pointer reached the progress bar or the back button: get ready
    /// (Apple Music's own controls are looked up before the press, not during it).
    Prepare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Gentle chime (timer finished, reminder).
    Chime,
    /// Short tick (button feedback) — currently unused.
    Tick,
}

/// Phone drop (the Shelf's phone panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneCmd {
    /// Start it (or try again after an error).
    Open,
    /// Stop it; files still coming in are dropped.
    Close,
    /// Answer the phone's question.
    Answer { id: u64, accept: bool },
}

/// Paired phones (also in the Shelf's phone panel; `services::pairdrop`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairCmd {
    /// Show a pairing QR code in the panel.
    Begin,
    /// Back to the same-Wi-Fi code (a pairing code stops working).
    Back,
    /// Show the paired phones.
    List,
    /// Forget a paired phone (by its room on the meeting server).
    Remove(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicPick {
    Directory,
    LyricsDirectory,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    OpenSearch,
    RefreshSearchApps,
    ExecuteSearch(lumo_core::search::Target),
    SearchIcons(Vec<(String, lumo_core::search::Target)>),
    CloseSearchWeb,
    SearchWeb(crate::search_web::Action),
    NavigateSearchWeb(String),
    ToggleSearchKeepOpen,
    ToggleSearchResume,
    Ai(crate::ai::Action),
    OpenAiWindow,
    AiHeight(f32),
    Media(MediaCmd),
    ChooseMusicDirectory,
    ChooseLyricsDownloadDirectory,
    /// Open a file/folder with its default handler.
    OpenPath(String),
    /// Show a file in Explorer.
    RevealPath(String),
    /// Begin an OLE drag of these paths out of the island.
    DragOut(Vec<String>),
    /// Request a thumbnail for a shelf item (async; arrives as `AppEvent::ShelfThumb`).
    LoadThumb(String),
    /// Put text on the clipboard (not re-recorded into history).
    CopyText(String),
    /// Give the island keyboard focus for a text field.
    FocusText,
    /// The Script page's Paste: the clipboard's text, line breaks kept,
    /// becomes the script.
    ScriptPaste,
    /// The Script page's Record: Lumo Recorder records the screen
    /// the island is on (`services::recorder`).
    RecordScreen,
    /// Drop keyboard focus.
    BlurText,
    /// Persist `AppState::store` soon.
    SaveStore,
    /// Persist `AppState::settings` now (after a page toggled a setting).
    SaveSettings,
    /// Open or restore the standalone Settings window.
    OpenSettings,
    OpenUpdateSettings,
    CheckUpdates,
    OpenLyricsWindow,
    CloseLyricsWindow,
    SetPage(PageKind),
    /// Keep the island open even when the cursor leaves.
    SetPinned(bool),
    Collapse,
    Play(Sound),
    /// Open a URL in the default browser.
    OpenUrl(String),
    /// Update to the offered version once Setup is downloaded, or open its release page.
    Update,
    /// Start with Windows on or off (the Settings page's switch).
    SetAutostart(bool),
    /// Request an app's icon for the System page (arrives in `AppState::app_icons`).
    LoadAppIcon(String),
    /// Show over games: find out whether Windows lets Lumo read frames
    /// (the answer lands in `AppState::game.access`).
    CheckFps,
    /// Show over games: ask Windows once for the right to read frames.
    AllowFps,
    Phone(PhoneCmd),
    Pair(PairCmd),
}
