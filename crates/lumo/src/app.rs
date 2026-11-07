//! The app: owns the island window, state, renderer and services; routes
//! window messages, service events and page commands.

use crate::activities::{self, Mode};
use crate::clock;
use crate::commands::{Command, MediaCmd, PairCmd, PhoneCmd};
use crate::events::{AppEvent, AppleMusicStatus, EventSink, SendHwnd, WM_APP_APPBAR, WM_APP_EVENT, WM_APP_FRAME, WM_APP_TRAY};
use crate::gfx::{px_rect, px_union, scale_about, union_rect, Canvas, Gfx, HitMask};
use crate::icons::Icon;
use crate::island::{Geom, Island, SHOULDER};
use crate::pages::{self, PageKind};
use crate::paths;
use crate::pill;
use crate::services::{self, Services};
use crate::state::{
    AppState, FpsAccess, HudKind, InlineAction, LyricsState, LyricsStatus, NoticeAction, PairedPhone, PhoneAsk, PhoneDrop,
    PhoneTransfer, PhoneView,
};
use crate::tray::{self, Tray};
use crate::ui::{wid, CursorKind, Hit, Input, KeyInput, MouseButton, Ui};
use crate::window::{self, FrameClock, FrameTick, MonitorInfo};
use crate::{dragdrop, selftest, shell};
use lumo_core::addons::Status as AddonStatus;
use lumo_core::frametime::{ambient_divisor, MotionStats};
use lumo_core::geom::{palette, Color, Point, Rect};
use lumo_core::pairdrop::{MeetEvent, PeerEvent};
use lumo_core::phonedrop::{Event as PhoneEvent, QrMatrix};
use lumo_core::settings::{IdleStyle, Settings, Theme};
use lumo_core::settings::MonitorChoice;
use lumo_core::store::{load_json, parse_json, save_json_atomic, LoadOutcome, Store};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use windows::core::{w, Result};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::*;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Not exported by windows-rs 0.58's WindowsAndMessaging module.
const WM_MOUSELEAVE: u32 = 0x02A3;

const TIMER_HOVER: usize = 1;
const TIMER_COLLAPSE: usize = 2;
const TIMER_WAKE: usize = 3;
const TIMER_SAVE: usize = 4;
const TIMER_MINUTE: usize = 6;
pub const TIMER_SELFTEST: usize = 7;
const TIMER_TRAY: usize = 8;
const TIMER_RAISE: usize = 9;
/// Show over games: the readout's peek ends.
const TIMER_PEEK: usize = 10;
/// The next read of the update file (`crate::updates`).
const TIMER_UPDATE: usize = 11;
const TIMER_AI_SNIP: usize = 72;
/// Show over games: how long the readout shows over a game, the least time
/// between two peeks for a frame-rate drop, and Alt+Shift+F's id.
const PEEK_S: f64 = 5.0;
const DROP_PEEK_EVERY_S: f64 = 30.0;
const HOTKEY_PEEK: i32 = 1;
/// Alt+Shift+S pauses and resumes the Script page's reading from any app.
const HOTKEY_SCRIPT: i32 = 2;
/// Raises that follow a new foreground window, and their spacing.
const RAISE_REPEATS: u8 = 4;
const RAISE_INTERVAL_MS: u32 = 500;

const WM_CAPTURECHANGED_: u32 = 0x0215;
const WM_WTSSESSION_CHANGE_: u32 = 0x02B1;
const WTS_SESSION_LOCK_: usize = 7;
const WTS_SESSION_UNLOCK_: usize = 8;

/// Steps for copying the Apple Music token (README).
const APPLE_MUSIC_HELP_URL: &str = "https://github.com/mthlpbs/lumo#apple-music-lyrics";
/// Phone drop closes itself after this long without a request from a phone.
const PHONE_IDLE_S: f64 = 600.0;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Posted messages that arrived while the app was borrowed (a nested
    /// message loop ran inside a handler). Replayed when the borrow ends.
    static MISSED: RefCell<Vec<(u32, WPARAM, LPARAM)>> = const { RefCell::new(Vec::new()) };
    /// A WM_APP_FRAME was swallowed: re-arm the frame clock.
    static FRAME_MISSED: Cell<bool> = const { Cell::new(false) };
}

/// Modal work that must run without the App borrowed (it pumps messages).
pub enum Deferred {
    DragOut(Vec<String>),
    Menu(POINT),
}

pub fn main_entry() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    // Show over games' one-time OK runs this exe elevated, just for this.
    if let Some(i) = args.iter().position(|a| a == services::frames::ALLOW_ARG) {
        return services::frames::allow_main(args.get(i + 1).map_or("", String::as_str));
    }
    let selftest_dir = args.iter().position(|a| a == "--selftest").map(|i| args.get(i + 1).cloned().unwrap_or_else(|| "selftest-out".into()));
    if let Some(dir) = &selftest_dir {
        let home = std::path::Path::new(dir).join("home");
        let _ = std::fs::remove_dir_all(&home);
        std::env::set_var("LUMO_HOME", &home);
    }
    std::panic::set_hook(Box::new(|info| {
        perror!("panic: {info}");
    }));

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // Single instance (the self-test may run next to a normal instance).
    let mutex_name = if selftest_dir.is_some() { w!("Local\\Lumo.Island.SelfTest") } else { w!("Local\\Lumo.Island") };
    let _mutex = unsafe { CreateMutexW(None, true, mutex_name) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        pinfo!("another instance is running; exiting");
        return 0;
    }
    if let Err(error) = paths::prepare_profile() {
        eprintln!("Lumo could not prepare its Lumo profile: {error}");
        return 2;
    }
    crate::plog::init(paths::log_path());
    pinfo!("Lumo {} starting (args: {:?})", env!("CARGO_PKG_VERSION"), &args[1..]);
    if let Err(e) = unsafe { OleInitialize(None) } {
        perror!("OleInitialize failed: {e}");
        return 2;
    }
    if crate::autostart::packaged() {
        pinfo!("running as the Microsoft Store package");
    }
    // Install on next start: the new version's Setup, downloaded and checked
    // earlier, runs now, before any window, and opens the new version.
    if selftest_dir.is_none() && !crate::autostart::packaged() && crate::updates::pending_install() {
        unsafe { OleUninitialize() };
        return 0;
    }
    let code = match run(selftest_dir) {
        Ok(code) => code,
        Err(e) => {
            perror!("fatal: {e}");
            3
        }
    };
    unsafe { OleUninitialize() };
    pinfo!("Lumo exiting with code {code}");
    code
}

fn run(selftest_dir: Option<String>) -> Result<i32> {
    window::register_classes(island_wndproc, helper_wndproc)?;
    let hwnd = window::create_island_window()?;
    let helper = window::create_helper_window()?;
    pinfo!("windows created");

    let (raw_settings, s_out): (Settings, _) = load_json(&paths::settings_path());
    if let Some(m) = s_out.message() {
        pwarn!("settings: {m}");
    }
    let settings = raw_settings.clone().sanitized();
    // Write new files, and write migrated / clamped values back so a
    // migration (e.g. the v2 hover delay) happens once and sticks.
    if s_out == LoadOutcome::Missing || (settings != raw_settings && s_out.writable()) {
        if let Err(e) = save_json_atomic(&paths::settings_path(), &settings) {
            pwarn!("saving settings failed: {e}");
        } else if s_out != LoadOutcome::Missing {
            pinfo!("settings: wrote back migrated / out-of-range values (v{})", settings.settings_version);
        }
    }
    let (store, d_out): (Store, _) = load_json(&paths::store_path());
    if let Some(m) = d_out.message() {
        pwarn!("data: {m}");
    }

    let (tx, rx) = mpsc::channel();
    let sink = EventSink::new(tx, SendHwnd::new(hwnd));
    let app = App::new(hwnd, helper, sink, rx, settings, store, selftest_dir, s_out.writable(), d_out.writable())?;
    APP.with(|c| *c.borrow_mut() = Some(app));
    with_app(|app| app.start());
    pinfo!("started; entering message loop");

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let code = APP.with(|c| c.borrow_mut().take().map(|mut a| a.shutdown()).unwrap_or(0));
    Ok(code)
}

/// Run `f` with the app if it isn't already borrowed (re-entrancy safe).
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let (r, hwnd) = APP.with(|c| match c.try_borrow_mut() {
        Ok(mut g) => match g.as_mut() {
            Some(app) => {
                let r = f(app);
                if FRAME_MISSED.with(|m| m.replace(false)) {
                    app.clock.frame_handled();
                    app.request_frame();
                }
                (Some(r), Some(app.hwnd))
            }
            None => (None, None),
        },
        Err(_) => (None, None),
    });
    // Re-post messages a nested loop swallowed while we were borrowed.
    if let Some(hwnd) = hwnd {
        let missed = MISSED.with(|m| std::mem::take(&mut *m.borrow_mut()));
        for (msg, wp, lp) in missed {
            unsafe {
                let _ = PostMessageW(hwnd, msg, wp, lp);
            }
        }
    }
    r
}

/// Remember a message that couldn't be handled because the app was borrowed.
fn note_missed(msg: u32, wp: WPARAM, lp: LPARAM) -> bool {
    match msg {
        WM_APP_FRAME => {
            FRAME_MISSED.with(|f| f.set(true));
            true
        }
        WM_APP_EVENT | WM_MOUSELEAVE | WM_TIMER | WM_LBUTTONUP | WM_APP_TRAY | WM_APP_APPBAR | services::ai_shortcut::MODIFIER_CHANGED => {
            MISSED.with(|m| {
                let mut v = m.borrow_mut();
                if v.len() < 64 && !(msg != WM_TIMER && v.iter().any(|x| x.0 == msg && x.1 == wp)) {
                    v.push((msg, wp, lp));
                }
            });
            true
        }
        _ => false,
    }
}

unsafe extern "system" fn island_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mut deferred = Vec::new();
    let res = with_app(|app| app.handle_island(msg, wp, lp, &mut deferred));
    for d in deferred {
        run_deferred(d);
    }
    match res {
        Some(Some(r)) => r,
        Some(None) => DefWindowProcW(hwnd, msg, wp, lp),
        None if note_missed(msg, wp, lp) => LRESULT(0),
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn helper_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mut deferred = Vec::new();
    let res = with_app(|app| app.handle_helper(msg, wp, lp, &mut deferred));
    for d in deferred {
        run_deferred(d);
    }
    match res {
        Some(Some(r)) => r,
        Some(None) => DefWindowProcW(hwnd, msg, wp, lp),
        // Helper messages are re-posted to the island window, which forwards
        // tray/appbar ids back to the helper handler.
        None if matches!(msg, WM_APP_TRAY | WM_APP_APPBAR) => {
            let _ = note_missed(msg, wp, lp);
            LRESULT(0)
        }
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn run_deferred(d: Deferred) {
    match d {
        Deferred::DragOut(paths) => {
            with_app(|app| app.release_mouse());
            if let Err(e) = dragdrop::drag_out(&paths) {
                pwarn!("drag out failed: {e}");
            }
            with_app(|app| app.request_frame());
        }
        Deferred::Menu(pt) => {
            let Some((helper, items)) = with_app(|app| (app.helper, tray::tray_menu_items(crate::autostart::is_enabled()))) else { return };
            if let Some(cmd) = tray::show_menu(helper, pt, &items) {
                with_app(|app| app.on_menu(cmd));
            }
        }
    }
}

pub struct App {
    hwnd: HWND,
    helper: HWND,
    gfx: Gfx,
    ai_ctrl_watch: Option<services::ai_shortcut::Watch>,
    ai_window: Option<crate::ai_window::AiWindow>,
    settings_window: Option<crate::settings_window::SettingsWindow>,
    lyrics_window: Option<crate::lyrics_window::LyricsWindow>,
    pub st: AppState,
    island: Island,
    mode: Mode,
    input: Input,
    hits: Vec<Hit>,
    hovering: bool,
    /// A compact action must not count its shrinking outline as a mouse exit.
    ai_resize_hold: Option<HWND>,
    /// Extra surface room is allocated only while the compact model list is open.
    ai_picker_room: bool,
    /// Only an explicitly opened reply overrides full-screen hiding.
    ai_reply_peek: bool,
    expanded: bool,
    reopen: crate::reopen::Context,
    drag_hover: bool,
    text_focus: bool,
    prev_foreground: HWND,
    monitor: MonitorInfo,
    clock: FrameClock,
    sink: EventSink,
    rx: Receiver<AppEvent>,
    services: Option<Services>,
    /// Animation time of the last frame (the vblank it was shown on).
    last_frame_t: f64,
    /// The window: a fixed-size surface the island is drawn centred in.
    surface: Surface,
    /// Centre of the surface in DIPs (where the island hangs from).
    cx: f32,
    /// Canvas pixels drawn last frame (the next frame clears them).
    prev_canvas_px: Option<RECT>,
    /// GPU renderer: the hit mask last pushed to the window, and when.
    mask: Option<(HitMask, f64)>,
    /// Smoothness of the morph in progress (logged when it settles).
    motion: Option<MotionStats>,
    motion_name: &'static str,
    /// Motions logged so far, and when a hitchy one was last logged.
    motions_logged: u32,
    hitch_logged_at: f64,
    /// Per-frame timing of the first few morphs (for the log): vblanks since
    /// the last frame, then ms from vblank to drawing, to draw, to present
    /// and to update the hit mask.
    trace: Vec<(u32, [f32; 4])>,
    /// DWM's counters when the morph began (refreshes, frames composed,
    /// frames missed), to tell our dropped frames from the compositor's.
    dwm_at_start: Option<(u64, u64, u64)>,
    /// Log lines written off the frame path (file writes can take tens of ms).
    log_queue: Vec<String>,
    /// Self-test: read the next frame back, and where it was.
    capture_next: bool,
    capture_rect: RECT,
    fullscreen_hidden: bool,
    /// The last full-screen state reported, hidden or not.
    fullscreen_on: bool,
    widgets_open: bool,
    /// Finite hide/reveal fade, advanced only by the existing frame clock.
    aside_visibility: f32,
    /// Step aside: the app windows at the top edge of the island's monitor
    /// (screen pixels, cut to the band along it), as last reported.
    top_windows: Vec<[i32; 4]>,
    /// Follow-up raises still due (TIMER_RAISE).
    raises_left: u8,
    /// Window classes already logged for covering the island (once each).
    covered_by: HashSet<String>,
    shown: bool,
    /// The System page's sampler thread: from the start while its history is
    /// kept (`Settings::system_history`), else from the first time the page opens.
    sysmon: Option<services::sysmon::SysMon>,
    /// The System page is open (its readings come each second).
    sysmon_on: bool,
    /// What the System reader was last told about nobody being there and battery saver.
    sysmon_rest: lumo_core::sysmon::Rest,
    /// Show over games: the game's frames, counted while the readout shows.
    frames: Option<services::frames::Frames>,
    /// The process whose frames count (the full-screen app).
    game_pid: Option<u32>,
    /// Show over games, peeks only: the readout shows over the game until this
    /// mono time (a peek), else the island hides so the game goes straight to
    /// the screen.
    game_peek_until: f64,
    /// When a frame-rate drop last made it peek.
    game_drop_at: f64,
    /// Show over games, the readout that stays: Alt+Shift+F hid it until the
    /// next game (or until pressed again).
    game_hidden: bool,
    /// Alt+Shift+F is registered (only while a game is in front).
    peek_hotkey: bool,
    /// Alt+Shift+S is registered (only while the Script page can read).
    script_hotkey: bool,
    /// The Script page has asked for the island to stay out of screen capture.
    script_excluded: bool,
    /// A reading held the island open at the last tick.
    script_held: bool,
    /// The Script page was on screen at the last tick.
    script_shown: bool,
    /// App icons the System page asked for and hasn't got yet (the Shelf's
    /// thumbnail worker answers both pages with the same event).
    icon_wait: HashSet<String>,
    tray: Option<Tray>,
    taskbar_created_msg: u32,
    selftest: Option<selftest::SelfTest>,
    /// Text we put on the clipboard ourselves (don't record it again).
    own_clip: Option<String>,
    exit_code: i32,
    /// Settings as last written to disk (to diff page-made changes).
    saved_settings: Settings,
    /// False when the file existed but couldn't be read: never overwrite it.
    settings_writable: bool,
    store_writable: bool,
    /// The current press started as a double-click (its click is ignored).
    dbl_press: bool,
    /// Pending high surrogate from WM_CHAR.
    high_surrogate: Option<u16>,
    wheel_pager: lumo_core::navigation::WheelPager,
    /// Session locked / display off: pause ambient animation.
    locked: bool,
    display_off: bool,
    tray_retries: u32,
    /// Content of the previous mode, fading out while the outline morphs:
    /// (mode, start time, its geometry, the opacity it fades from).
    outgoing: Option<(Mode, f64, Geom, f32)>,
    /// A page switch in progress: (from page, start time, slide direction).
    page_anim: Option<(PageKind, f64, f32)>,
    /// Where the hover (dwell) timer was last armed; None when it isn't.
    hover_arm: Option<Point>,
    /// The pointer left the open island: close it on the next frame.
    collapse_due: bool,
    /// When the pointer last closed the island by leaving it.
    left_at: f64,
    /// Phone drop's web server while the Shelf's phone panel is open.
    phone: Option<services::phonedrop::PhoneDrop>,
    /// The Shelf paths last handed to it (to notice changes).
    phone_offer: Vec<String>,
    /// The Shelf paths last handed to the paired-phones service.
    pair_offer: Option<Vec<String>>,
    /// Whether the island floated when the window was last laid out (its
    /// room differs: see `sync_room`).
    laid_floating: bool,
    /// New versions (`crate::updates`); None in the self-test and in a
    /// Store copy, which the Store updates.
    updater: Option<crate::updates::Updater>,
}

/// The island window: position and size in screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Surface {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// The grow springs overshoot by about 3 %; the surface leaves 5 %.
const OVERSHOOT: f32 = 1.05;
/// Hit-mask updates while the island moves (the mask is invisible, so it
/// may trail the outline by a frame or two).
const MASK_INTERVAL: f64 = 1.0 / 30.0;

/// How long outgoing content takes to fade (seconds).
const OUTGOING_FADE: f64 = 0.14;
/// Page switch cross-fade duration (seconds).
const PAGE_SWITCH: f64 = 0.18;
/// Pointer travel (DIPs) that restarts the hover dwell: the island opens
/// only once the pointer rests on it, not when it brushes past.
const DWELL_SLOP: f32 = 4.0;
/// The pointer coming back onto the island this soon after leaving it
/// (while it is still shrinking) opens it again at once, from where the
/// outline is: overshooting an edge costs no close-and-reopen.
const REOPEN_S: f64 = 0.3;
/// A hold that kept the island open (typing, a file drag, Phone drop, the
/// pin) ended while the pointer is elsewhere: close after this grace, so
/// the change can be seen.
const HOLD_GRACE_MS: u32 = 380;
/// Collapsed pill growth (w, h, radius) while the pointer is on it.
const HOVER_LIFT: (f32, f32, f32) = (12.0, 4.0, 2.0);
/// Step aside: an app window is at the top when its top edge is within this
/// many DIPs of the edge the island hangs from (a maximized window overhangs
/// it, a snapped one starts on it). One placed lower keeps its title and
/// tabs below the island, which covers a sliver of its frame at most.
const TOP_BAND: f32 = 8.0;
/// How long the inline notice pill shows its check mark (seconds).
const INLINE_DONE_S: f64 = 0.6;
/// Incoming content starts at this scale and this many DIPs up, and
/// settles down from the bezel as it fades in.
const CONTENT_SCALE_FROM: f32 = 0.94;
const CONTENT_DROP: f32 = 6.0;
/// Smallest scale the outgoing content shrinks to as the outline narrows.
const GHOST_MIN_SCALE: f32 = 0.6;

/// How much of the outgoing content still shows `age` seconds into its fade
/// (a smoothstep: it holds, then leaves).
fn outgoing_left(age: f64) -> f32 {
    let t = ((age / OUTGOING_FADE) as f32).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        hwnd: HWND,
        helper: HWND,
        sink: EventSink,
        rx: Receiver<AppEvent>,
        settings: Settings,
        store: Store,
        selftest_dir: Option<String>,
        settings_writable: bool,
        store_writable: bool,
    ) -> Result<Self> {
        let monitor = window::pick_monitor(settings.monitor);
        let gfx = Gfx::new(monitor.dpi as f32, hwnd)?;
        let mut st = AppState::new(settings, store);
        st.now = clock::mono();
        st.local = clock::local_now();
        st.unix = clock::unix_now();
        st.accent = theme_accent(st.settings.theme);
        let mode = Mode::Idle;
        let mut island = Island::new(activities::geom(&mode, &st));
        island.set_floating(pill::on(st.settings.theme).then_some(pill::TOP_GAP));
        let clock = FrameClock::start(SendHwnd::new(hwnd), monitor.handle);
        let taskbar_created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        let selftest = selftest_dir.map(selftest::SelfTest::new);
        let saved_settings = st.settings.clone();
        Ok(Self {
            hwnd,
            helper,
            gfx,
            ai_ctrl_watch: None,
            ai_window: None,
            settings_window: None,
            lyrics_window: None,
            st,
            island,
            mode,
            input: Input::default(),
            hits: Vec::new(),
            hovering: false,
            ai_resize_hold: None,
            ai_picker_room: false,
            ai_reply_peek: false,
            expanded: false,
            reopen: crate::reopen::Context::default(),
            drag_hover: false,
            text_focus: false,
            prev_foreground: HWND::default(),
            monitor,
            clock,
            sink,
            rx,
            services: None,
            last_frame_t: 0.0,
            surface: Surface::default(),
            cx: 0.0,
            prev_canvas_px: None,
            mask: None,
            motion: None,
            motion_name: "morph",
            motions_logged: 0,
            hitch_logged_at: f64::NEG_INFINITY,
            trace: Vec::new(),
            dwm_at_start: None,
            log_queue: Vec::new(),
            capture_next: false,
            capture_rect: RECT::default(),
            fullscreen_hidden: false,
            fullscreen_on: false,
            widgets_open: false,
            aside_visibility: 1.0,
            top_windows: Vec::new(),
            raises_left: 0,
            covered_by: HashSet::new(),
            shown: false,
            sysmon: None,
            sysmon_on: false,
            sysmon_rest: lumo_core::sysmon::Rest::default(),
            frames: None,
            game_pid: None,
            game_peek_until: 0.0,
            game_drop_at: f64::NEG_INFINITY,
            game_hidden: false,
            peek_hotkey: false,
            script_hotkey: false,
            script_excluded: false,
            script_held: false,
            script_shown: false,
            icon_wait: HashSet::new(),
            tray: None,
            taskbar_created_msg,
            selftest,
            own_clip: None,
            exit_code: 0,
            saved_settings,
            settings_writable,
            store_writable,
            dbl_press: false,
            high_surrogate: None,
            wheel_pager: Default::default(),
            locked: false,
            display_off: false,
            tray_retries: 0,
            outgoing: None,
            page_anim: None,
            hover_arm: None,
            collapse_due: false,
            left_at: f64::NEG_INFINITY,
            phone: None,
            phone_offer: Vec::new(),
            pair_offer: None,
            laid_floating: false,
            updater: None,
        })
    }

    fn start(&mut self) {
        self.layout_surface();
        let fake = self.selftest.is_some();
        self.st.power = services::power::query();
        services::power::register(self.helper);
        if !fake {
            self.services = Some(Services::start(&self.sink, self.hwnd, self.helper, &self.st.settings));
            self.sync_top_watch();
            self.st.paired = self.services.as_ref().map(|s| paired_phones(s.pairs())).unwrap_or_default();
            self.sync_pair_offer();
            if self.st.settings.system_history {
                self.start_sysmon();
            }
            // A frame count left running by a Lumo that crashed, and whether
            // Windows lets this sign-in read frames (the System page shows it).
            services::frames::stop_stale();
            if self.st.settings.game_stats {
                self.check_fps();
            }
            self.try_add_tray();
            services::power::register_session(self.helper);
            self.st.update.managed = crate::autostart::packaged();
            if lumo_core::update::ENABLED && !self.st.update.managed {
                self.updater = Some(crate::updates::Updater::start(paths::data_dir().join("update.json"), &mut self.st, crate::updates::unix_now()));
                self.schedule_update();
                self.fetch_setup();
            }
        }
        if let Err(e) = dragdrop::register(self.hwnd, self.sink.clone()) {
            pwarn!("drop target registration failed: {e}");
        }
        // Refresh shelf thumbnails.
        for p in self.st.store.shelf.paths() {
            self.st.cmds.push(Command::LoadThumb(p));
        }
        self.schedule_minute_timer();
        if let Some(t) = self.selftest.as_mut() {
            t.begin(&mut self.st);
            unsafe { SetTimer(self.hwnd, TIMER_SELFTEST, 50, None) };
        }
        self.request_frame();
    }

    fn shutdown(&mut self) -> i32 {
        self.lyrics_window = None;
        self.settings_window = None;
        self.save_now();
        // Stops the frame count's trace session (it would outlive Lumo).
        self.frames = None;
        self.set_peek_hotkey(false);
        self.set_script_hotkey(false);
        if let Some(p) = self.phone.take() {
            p.stop();
        }
        if let Some(t) = self.tray.take() {
            t.remove();
        }
        if let Some(s) = self.services.take() {
            s.stop(self.helper);
        }
        dragdrop::revoke(self.hwnd);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyWindow(self.helper);
        }
        self.exit_code
    }

    /// Leave the message loop. Never call DestroyWindow while the app is
    /// borrowed: WM_DESTROY would re-enter and be swallowed.
    fn quit(&mut self, code: i32) {
        self.exit_code = code;
        self.clock.set_active(false);
        unsafe { PostQuitMessage(code) };
    }

    pub fn request_frame(&mut self) {
        // Input must wake on the next refresh, even if the last frame left
        // the clock at the slower cadence used by ambient animation.
        self.clock.set_divisor(1);
        self.clock.set_active(true);
    }

    /// Switch the renderer to `dpi` and size the window for that scale.
    fn set_dpi(&mut self, dpi: u32) {
        self.gfx.set_dpi(dpi as f32);
        self.layout_surface();
    }

    /// Size and place the window once for the largest island (the widest,
    /// tallest expanded page plus spring overshoot). The island is drawn
    /// centred in it, so a morph never moves or resizes the window: the
    /// centre stays put and both edges glide at sub-pixel precision. Call at
    /// start and whenever the scale, the monitor or the expanded width changes.
    fn layout_surface(&mut self) {
        let middle=self.monitor.work.top+(self.monitor.work.bottom-self.monitor.work.top)/2;
        self.st.ai.max_island_height=(((middle-self.monitor.top()) as f32/self.gfx.scale())-pages::HEADER_H-pages::PAD-if pill::on(self.st.settings.theme){pill::TOP_GAP}else{0.0}).max(240.0);
        self.st.ai.island_height=self.st.ai.island_height.min(self.st.ai.max_island_height);
        let tallest = PageKind::ALL.iter().map(|p| if *p==PageKind::Ai{self.st.ai.canvas_height.max(self.st.ai.island_height)}else{pages::content_height(*p, &self.st)}).fold(0.0f32, f32::max);
        let tallest = if self.ai_picker_room { tallest.max(240.0 + 8.0 + 400.0) } else { tallest };
        self.layout_surface_for(self.st.settings.expanded_width, pages::HEADER_H + tallest + pages::PAD);
        // The band Step aside watches follows the monitor and its scale.
        self.sync_top_watch();
    }

    /// Step aside on: have the full-screen watcher report the app windows
    /// at the top edge of the island's monitor, where the island hangs from
    /// (below a taskbar at the top). Off (always, since 0.2.6; the self-test
    /// still draws it): it looks at nothing, and nothing is under the island.
    fn sync_top_watch(&mut self) {
        let m = &self.monitor;
        let px = (TOP_BAND * self.gfx.scale()).ceil() as i32;
        let band = self.st.settings.step_aside.then(|| RECT { left: m.rect.left, top: m.top(), right: m.rect.right, bottom: m.top() + px });
        if band.is_none() {
            self.top_windows.clear();
        }
        if let Some(s) = &self.services {
            s.watch_top(band);
        }
    }

    /// Lay the window out for an island body up to `body_w`×`body_h` DIPs.
    fn layout_surface_for(&mut self, body_w: f32, body_h: f32) {
        let s = self.gfx.scale();
        let mon_w = self.monitor.rect.right - self.monitor.rect.left;
        let (w, h) = surface_size(&self.island, body_w, body_h, s, mon_w);
        self.laid_floating = self.island.floating();
        let surface = Surface { x: self.monitor.rect.left + (mon_w - w) / 2, y: self.monitor.top(), w, h };
        if surface != self.surface {
            self.surface = surface;
            self.cx = w as f32 / 2.0 / s;
            self.prev_canvas_px = None;
            self.mask = None;
            if let Err(e) = self.gfx.set_surface(w, h) {
                pwarn!("window surface {w}x{h} failed: {e}");
            }
        }
    }

    /// The island started or stopped floating since the window was laid out
    /// (a theme switch into or out of Pill, the self-test picking its theme
    /// after start, or the outline docking at the edge after rising back to
    /// it): lay the window out again for the new room, grown for the gap and
    /// window they start with and Pill gets its own. Called every frame
    /// after the island steps; a layout only on that change.
    fn sync_room(&mut self) {
        let floats = self.island.floating();
        if floats != self.laid_floating {
            self.layout_surface();
            let (w, h) = (self.surface.w, self.surface.h);
            self.log_queue.push(format!("island {}; window {w}x{h}", if floats { "floats below the top edge" } else { "hangs from the top edge" }));
        }
    }

    /// The island no longer fits the window (a target beyond the laid-out
    /// size, with room for its overshoot, or an outline past it): grow it.
    fn ensure_room(&mut self) {
        let g = self.island.current();
        let t = self.island.target();
        let (bw, bh) = ((t.w * OVERSHOOT).max(g.w), (t.h * OVERSHOOT).max(g.h));
        let s = self.gfx.scale();
        let need_w = ((bw + 2.0 * self.island.side_room()) * s).ceil() as i32;
        let need_h = ((bh + self.island.bottom_room() + self.island.top()) * s).ceil() as i32;
        if need_w > self.surface.w || need_h > self.surface.h {
            let tallest = PageKind::ALL.iter().map(|p| pages::content_height(*p, &self.st)).fold(0.0f32, f32::max);
            let body_h = (pages::HEADER_H + tallest + pages::PAD).max(bh / OVERSHOOT * 1.1);
            pinfo!("island outgrew the window; growing it to fit {bw:.0}x{bh:.0} DIP");
            self.layout_surface_for(self.st.settings.expanded_width.max(bw / OVERSHOOT * 1.1), body_h);
        }
    }

    /// Add the tray icon; Explorer can be busy at logon, so retry a few times.
    fn try_add_tray(&mut self) {
        if self.tray.is_some() {
            return;
        }
        match Tray::add(self.helper) {
            Ok(t) => {
                self.tray = Some(t);
                self.tray_retries = 0;
            }
            Err(e) => {
                self.tray_retries += 1;
                if self.tray_retries <= 15 {
                    pwarn!("tray icon failed ({e}); retry {} in 2 s", self.tray_retries);
                    unsafe {
                        SetTimer(self.hwnd, TIMER_TRAY, 2000, None);
                    }
                } else {
                    pwarn!("tray icon failed ({e}); giving up");
                }
            }
        }
    }

    /// Write settings only when they changed and the file is safe to overwrite.
    fn save_settings(&mut self) {
        if self.st.settings == self.saved_settings {
            return;
        }
        if !self.settings_writable {
            pwarn!("settings.json couldn't be read at start; not overwriting it");
            return;
        }
        match save_json_atomic(&paths::settings_path(), &self.st.settings) {
            Ok(()) => self.saved_settings = self.st.settings.clone(),
            Err(e) => pwarn!("saving settings failed: {e}"),
        }
    }

    fn release_mouse(&mut self) {
        self.input.down = false;
        self.input.pressed_id = None;
        unsafe {
            let _ = ReleaseCapture();
        }
    }

    // ------------------------------------------------------------ messages

    fn handle_island(&mut self, msg: u32, wp: WPARAM, lp: LPARAM, deferred: &mut Vec<Deferred>) -> Option<LRESULT> {
        match msg {
            WM_APP_FRAME => {
                // Clear `pending` after the frame so a slow frame can't keep the
                // queue permanently non-empty (which starves WM_TIMER).
                let tick = self.clock.tick();
                self.frame(deferred, Some(tick));
                self.clock.frame_handled();
                Some(LRESULT(0))
            }
            WM_APP_EVENT => {
                while let Ok(ev) = self.rx.try_recv() {
                    self.on_event(ev);
                }
                self.request_frame();
                Some(LRESULT(0))
            }
            0x031d if self.st.ai.snipping => {
                if unsafe { windows::Win32::System::DataExchange::IsClipboardFormatAvailable(8) }.is_ok() {
                    self.stop_ai_snip();
                    services::ai::run(&mut self.st,crate::ai::Action::ReadSnip,SendHwnd::new(self.hwnd),self.sink.clone());
                }
                Some(LRESULT(0))
            }
            services::ai_shortcut::MODIFIER_CHANGED => {self.input.ctrl=services::ai_shortcut::control_down();self.request_frame();Some(LRESULT(0))}
            WM_TIMER => {
                self.on_timer(wp.0);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                self.input.ctrl=services::ai_shortcut::control_down();
                let p = self.mouse_dip(lp);
                if self.ai_resize_hold.is_some() && (!self.island.settled() || !self.hovering_inside(p)) {
                    self.input.mouse=Some(p);self.request_frame();return Some(LRESULT(0));
                }
                self.ai_resize_hold=None;
                if self.over_stale_mask(p) {
                    // The pointer is over a part of the hit mask the outline
                    // already left: it isn't on the island.
                    if self.hovering {
                        self.hovering = false;
                        self.input.mouse = None;
                        self.cancel_hover();
                        self.pointer_left();
                        self.request_frame();
                    }
                    return Some(LRESULT(0));
                }
                self.input.mouse = Some(p);
                if !self.hovering {
                    self.hovering = true;
                    if self.lyrics_window.is_some() { self.raise_topmost(); }
                    self.collapse_due = false;
                    unsafe {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
                        let _ = TrackMouseEvent(&mut tme);
                        let _ = KillTimer(self.hwnd, TIMER_COLLAPSE);
                    }
                    if self.hover_opens() && !self.expanded && clock::mono() - self.left_at < REOPEN_S {
                        // Back onto the island while it is still closing.
                        self.open_island();
                    } else if self.hover_opens() && !self.expanded {
                        self.arm_hover(p);
                    }
                } else if let Some(p0) = self.hover_arm.filter(|_| !self.expanded) {
                    // Dwell: moving on restarts the wait, so brushing past on
                    // the way to a tab or title bar doesn't open the island.
                    if (p.x - p0.x).hypot(p.y - p0.y) > DWELL_SLOP {
                        self.arm_hover(p);
                    }
                }
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.ai_ctrl_watch=None;
                self.input.ctrl=false;
                self.hovering = false;
                // Captured drags still need their last pointer position, even
                // when resizing outruns the old window mask.
                if !self.input.down { self.input.mouse = None; }
                self.cancel_hover();
                self.pointer_left();
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let p = self.mouse_dip(lp);
                if self.over_shadow(p) {
                    // A press on the floating pill's shadow (software renderer): not the island.
                    return Some(LRESULT(0));
                }
                self.page_interaction(p);
                self.dbl_press = msg == WM_LBUTTONDBLCLK;
                self.input.mouse = Some(p);
                self.input.down = true;
                self.input.just_pressed = true;
                self.input.press_pos = Some(p);
                self.input.pressed_id = self.hit_at(p).map(|h| h.id);
                if self.input.pressed_id == Some(wid("ai.resize-grip")) {
                    self.st.ai.resize_from = self.st.ai.island_height;
                    self.st.ai.resize_dragged = false;
                }
                unsafe { SetCapture(self.hwnd) };
                if !self.expanded {
                    let ai_shortcut = self.input.pressed_id == Some(wid("ai.shortcut")) && self.input.ctrl && services::ai_shortcut::control_down();
                    let inline = self.st.notice.as_ref().and_then(|n| n.inline).filter(|_| self.input.pressed_id == Some(wid("notice.act")));
                    if ai_shortcut {
                        self.set_page(PageKind::Ai);
                        self.expand();
                    } else if let Some(a) = inline {
                        // The notice's inline pill acts in place without
                        // expanding. The press is consumed here, so the
                        // release can't click the pill a second time.
                        self.input.pressed_id = None;
                        self.run_inline_action(a);
                    } else {
                        // Clicking a notice jumps to its page.
                        let mut directed = false;
                        if let Some(n) = self.st.notice.take() {
                            directed = n.action.is_some();
                            match n.action {
                                Some(NoticeAction::OpenPage(p)) => self.set_page(p),
                                Some(NoticeAction::OpenAiReply(chat)) => {
                                    self.input.pressed_id=None;
                                    self.open_ai_reply(chat);
                                }
                                Some(NoticeAction::OpenUpdate) => self.st.open_update(),
                                Some(NoticeAction::StartFocus) => {
                                    self.set_page(PageKind::Focus);
                                    if !self.st.pomodoro.is_active() {
                                        self.st.pomodoro.start(self.st.now);
                                    }
                                }
                                None => {}
                            }
                        }
                        self.st.hud = None;
                        if directed { self.expand(); } else { self.open_island(); }
                    }
                } else if self.text_focus && self.input.pressed_id != self.input.focused {
                    self.blur_text();
                }
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let p = self.mouse_dip(lp);
                self.input.mouse = Some(p);
                let resizing = self.input.pressed_id == Some(wid("ai.resize-grip"));
                if resizing {
                    // Commit the final position even if mouse-up arrived
                    // before another paint. Direct manipulation follows the
                    // pointer without a spring lag or an accidental click.
                    if let Some(start) = self.input.press_pos {
                        let dy = p.y - start.y;
                        if dy.abs() > 3.0 || self.st.ai.resize_dragged {
                            self.st.ai.resize_dragged = true;
                            self.resize_ai_chat(self.st.ai.resize_from + dy, true);
                        }
                    }
                    self.hovering = self.hovering_inside(p);
                    self.collapse_due = false;
                    if self.st.ai.resize_dragged {
                        // Present the new shape and hit mask before releasing
                        // capture. Otherwise Windows can queue MouseLeave for
                        // the old outline and dismiss the finished resize.
                        self.input.just_pressed = false;
                        self.frame(deferred, None);
                    }
                    unsafe {
                        let _ = KillTimer(self.hwnd, TIMER_COLLAPSE);
                        if self.hovering {
                            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
                            let _ = TrackMouseEvent(&mut tme);
                        }
                    }
                }
                let hit = self.hit_at(p).map(|h| h.id);
                if let (Some(pid), Some(h)) = (self.input.pressed_id, hit) {
                    // The second press of a double-click doesn't click again
                    // (no double toggles / double opens).
                    if pid == h && !self.dbl_press {
                        self.input.clicked = Some((h, MouseButton::Left));
                    }
                }
                if self.input.down {
                    self.input.released = self.input.pressed_id.map(|id| (id, p));
                }
                self.dbl_press = false;
                self.release_mouse();
                if !self.hovering_inside(p) {
                    self.input.mouse = None;
                }
                if !self.hovering {
                    // Released outside after a press inside (a slider drag).
                    if resizing { self.maybe_schedule_collapse(); } else { self.pointer_left(); }
                }
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED_ => {
                // Capture taken by someone else mid-press: drop the stale press.
                if lp.0 != self.hwnd.0 as isize && (self.input.down || self.input.pressed_id.is_some()) {
                    self.input.down = false;
                    self.input.pressed_id = None;
                    if !self.hovering {
                        self.pointer_left();
                    }
                    self.request_frame();
                }
                Some(LRESULT(0))
            }
            WM_SYSCOMMAND if (wp.0 & 0xFFF0) == SC_CLOSE as usize => {
                // Alt+F4 while typing in a field just closes the island, not the app.
                self.blur_text();
                self.collapse();
                Some(LRESULT(0))
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => Some(LRESULT(0)),
            WM_MOUSEHWHEEL => {
                if self.expanded {
                    let delta = (wp.0 >> 16) as u16 as i16;
                    if let Some(forward) = self.wheel_pager.feed(delta, clock::mono()) {
                        let next = pages::step_tab(self.st.page, &self.st, forward);
                        self.set_page(next);
                        self.request_frame();
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = (wp.0 >> 16) as u16 as i16;
                // Wheel messages carry screen coordinates. Use this packet's
                // position, not an older WM_MOUSEMOVE, to choose the header
                // versus page content. Handle each packet before frames can
                // merge it with another axis or the gesture's inertial tail.
                let mut pt = POINT { x: lp.0 as i16 as i32, y: (lp.0 >> 16) as i16 as i32 };
                unsafe { let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut pt); }
                let scale = self.gfx.scale();
                let p = Point::new(pt.x as f32 / scale, pt.y as f32 / scale);
                self.input.mouse = Some(p);
                let body = self.island.body_rect_at(self.cx);
                let over_header = self.expanded && p.x >= body.x && p.x <= body.right()
                    && p.y >= 0.0 && p.y < body.y + pages::HEADER_H;
                if over_header {
                    if let Some(forward) = self.wheel_pager.feed_vertical(delta, clock::mono()) {
                        let next = pages::step_tab(self.st.page, &self.st, forward);
                        self.set_page(next);
                        self.request_frame();
                    }
                } else {
                    self.page_interaction(p);
                    self.input.wheel += f32::from(delta) / WHEEL_DELTA as f32;
                    self.request_frame();
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                let kind = self.input.mouse.and_then(|p| self.hit_at(p)).map(|h| h.cursor).unwrap_or(CursorKind::Arrow);
                unsafe {
                    let id = match kind {
                        CursorKind::Arrow => IDC_ARROW,
                        CursorKind::Hand => IDC_HAND,
                        CursorKind::Text => IDC_IBEAM,
                    };
                    if let Ok(c) = LoadCursorW(None, id) {
                        SetCursor(c);
                    }
                }
                Some(LRESULT(1))
            }
            WM_MOUSEACTIVATE => Some(LRESULT(if self.text_focus { MA_ACTIVATE as isize } else { MA_NOACTIVATE as isize })),
            WM_CHAR => {
                if self.text_focus {
                    self.reopen.interacted(self.st.page, clock::mono());
                    let u = wp.0 as u32;
                    let c = if (0xD800..0xDC00).contains(&u) {
                        self.high_surrogate = Some(u as u16);
                        None
                    } else if (0xDC00..0xE000).contains(&u) {
                        self.high_surrogate.take().and_then(|h| char::decode_utf16([h, u as u16]).next().and_then(|r| r.ok()))
                    } else {
                        self.high_surrogate = None;
                        char::from_u32(u)
                    };
                    if let Some(c) = c.filter(|c| !c.is_control()) {
                        self.input.keys.push(KeyInput::Char(c));
                        self.request_frame();
                    }
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                if wp.0==VK_ESCAPE.0 as usize && self.st.page==PageKind::Ai && (self.st.ai.model_menu || self.st.ai.reasoning_menu || self.st.ai.attach_menu) {
                    self.st.ai.close_popovers();self.input.focused=Some(wid("ai.prompt"));self.input.caret=self.st.ai.draft.chars().count();self.request_frame();return Some(LRESULT(0));
                }
                if self.text_focus {
                    self.reopen.interacted(self.st.page, clock::mono());
                    let k = match VIRTUAL_KEY(wp.0 as u16) {
                        VK_A if unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0 => Some(KeyInput::SelectAll),
                        VK_BACK => Some(KeyInput::Backspace),
                        VK_DELETE => Some(KeyInput::Delete),
                        VK_RETURN => Some(if self.st.page==PageKind::Ai && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{KeyInput::Char('\n')}else{KeyInput::Enter}),
                        VK_ESCAPE => Some(KeyInput::Escape),
                        VK_LEFT => Some(KeyInput::Left),
                        VK_RIGHT => Some(KeyInput::Right),
                        VK_UP => Some(KeyInput::Up),
                        VK_DOWN => Some(KeyInput::Down),
                        VK_HOME => Some(KeyInput::Home),
                        VK_END => Some(KeyInput::End),
                        VK_V if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 => {
                            let prompt=self.st.page==PageKind::Ai && self.input.focused==Some(wid("ai.prompt"));
                            if !crate::ai::paste_image(&mut self.st,prompt,services::ai::clipboard_has_image) {if let Some(text) = services::clipboard::read_text(self.hwnd) {
                                for c in text.chars().take(16000).filter(|c| !c.is_control() || (self.st.page==PageKind::Ai && *c=='\n')) {
                                    self.input.keys.push(KeyInput::Char(c));
                                }
                            }}
                            None
                        }
                        _ => None,
                    };
                    if let Some(k) = k {
                        self.input.keys.push(k);
                    }
                    self.request_frame();
                    return Some(LRESULT(0));
                }
                None
            }
            WM_PASTE => {
                if self.text_focus {
                    self.reopen.interacted(self.st.page, clock::mono());
                    let prompt=self.st.page==PageKind::Ai && self.input.focused==Some(wid("ai.prompt"));
                    if !crate::ai::paste_image(&mut self.st,prompt,services::ai::clipboard_has_image){crate::ai_window::paste_text(&mut self.input,self.hwnd);}
                    self.request_frame();
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE && self.text_focus {
                    self.text_focus = false;
                    self.input.focused = None;
                    self.set_noactivate(true);
                    self.maybe_schedule_collapse();
                    self.request_frame();
                }
                None
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                self.refresh_monitor();
                if msg == WM_SETTINGCHANGE {
                    if let Some(a) = services::system_accent() {
                        self.st.accent = a;
                    }
                }
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                self.quit(0);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn handle_helper(&mut self, msg: u32, wp: WPARAM, lp: LPARAM, deferred: &mut Vec<Deferred>) -> Option<LRESULT> {
        if msg == self.taskbar_created_msg && msg != 0 {
            if let Some(t) = self.tray.take() {
                t.remove();
            }
            self.tray_retries = 0;
            self.try_add_tray();
            if let Some(s) = self.services.as_ref() {
                s.on_taskbar_created();
            }
            // A restarted Explorer puts its taskbar at the top of the topmost band.
            self.start_raises();
            return Some(LRESULT(0));
        }
        match msg {
            WM_APP_TRAY => {
                let ev = (lp.0 & 0xffff) as u32;
                match ev {
                    WM_LBUTTONUP => {
                        if self.expanded {
                            self.collapse();
                        } else {
                            self.open_island();
                        }
                        self.request_frame();
                    }
                    WM_RBUTTONUP | WM_CONTEXTMENU => {
                        let mut pt = POINT::default();
                        unsafe {
                            let _ = GetCursorPos(&mut pt);
                        }
                        deferred.push(Deferred::Menu(pt));
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_APP_APPBAR => {
                if let Some(s) = self.services.as_ref() {
                    if let Some(fs) = s.on_appbar_message(wp, lp, self.monitor.handle) {
                        self.on_event(AppEvent::Fullscreen(fs));
                    }
                }
                Some(LRESULT(0))
            }
            WM_POWERBROADCAST => {
                if let Some(on) = services::power::display_state_change(wp, lp) {
                    self.display_off = !on;
                    if on { if let Some(window) = self.settings_window.as_ref() { window.invalidate(); } if let Some(window) = self.ai_window.as_ref() { window.invalidate(); } }
                }
                let p = services::power::query();
                self.on_event(AppEvent::Power(p));
                self.tell_sysmon_rest();
                self.update_due();
                self.request_frame();
                Some(LRESULT(1))
            }
            WM_WTSSESSION_CHANGE_ => {
                match wp.0 {
                    WTS_SESSION_LOCK_ => self.locked = true,
                    WTS_SESSION_UNLOCK_ => self.locked = false,
                    _ => {}
                }
                if !self.locked { if let Some(window) = self.settings_window.as_ref() { window.invalidate(); } if let Some(window) = self.ai_window.as_ref() { window.invalidate(); } }
                self.tell_sysmon_rest();
                self.update_due();
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_HOTKEY if wp.0 == HOTKEY_PEEK as usize => {
                self.toggle_readout();
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_HOTKEY if wp.0 == HOTKEY_SCRIPT as usize => {
                self.st.now = clock::mono();
                pages::script::toggle(&mut self.st);
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_QUERYENDSESSION => Some(LRESULT(1)),
            WM_ENDSESSION => {
                self.save_now();
                // Windows ends Lumo without shutdown(): stop the frame count's
                // trace session now, or it would outlive the sign-in.
                if wp.0 != 0 {
                    self.frames = None;
                }
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                self.refresh_monitor();
                None
            }
            _ => None,
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_AI_SNIP => {self.stop_ai_snip();self.st.ai.snipping=false;self.request_frame();}
            TIMER_HOVER => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_HOVER);
                }
                if !self.hover_opens() || !self.hovering || self.expanded {
                    self.hover_arm = None;
                } else if self.st.notice.as_ref().is_some_and(|n| n.inline.is_some()) {
                    // A notice with an inline pill stays put under a resting
                    // pointer so the pill can be clicked (a click on the rest
                    // of the notice still opens its page). The dwell stays
                    // armed: the next movement restarts it.
                } else {
                    self.open_island();
                }
            }
            TIMER_RAISE => {
                self.raises_left = self.raises_left.saturating_sub(1);
                if self.shown && !self.fullscreen_hidden {
                    self.raise_if_covered();
                } else {
                    self.raises_left = 0;
                }
                if self.raises_left == 0 {
                    unsafe {
                        let _ = KillTimer(self.hwnd, TIMER_RAISE);
                    }
                }
            }
            TIMER_PEEK => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_PEEK);
                }
                let now = clock::mono();
                if self.expanded || self.hovering {
                    // Not from under the pointer: it hides once the island closes.
                    self.game_peek_until = now + 1.0;
                    unsafe {
                        SetTimer(self.hwnd, TIMER_PEEK, 1000, None);
                    }
                } else {
                    self.game_peek_until = 0.0;
                    self.apply_fullscreen(now);
                }
            }
            TIMER_COLLAPSE => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_COLLAPSE);
                }
                if self.free_to_collapse() {
                    self.collapse();
                }
            }
            TIMER_WAKE => unsafe {
                let _ = KillTimer(self.hwnd, TIMER_WAKE);
            },
            TIMER_SAVE => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_SAVE);
                }
                self.save_now();
            }
            TIMER_MINUTE => {
                self.schedule_minute_timer();
                self.close_idle_phone();
            }
            TIMER_TRAY => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_TRAY);
                }
                self.try_add_tray();
            }
            TIMER_UPDATE => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_UPDATE);
                }
                self.update_due();
            }
            TIMER_SELFTEST => {
                if let Some(mut t) = self.selftest.take() {
                    let done = t.step(self);
                    if let Some(code) = done {
                        unsafe {
                            let _ = KillTimer(self.hwnd, TIMER_SELFTEST);
                        }
                        self.quit(code);
                    }
                    self.selftest = Some(t);
                }
            }
            _ => {}
        }
        self.request_frame();
    }

    /// Save the Apple Music token copied from the browser (sealed with DPAPI)
    /// and keep it off the clipboard and out of Lumo's clipboard history.
    fn use_apple_music_token(&mut self) {
        let text = services::clipboard::read_text(self.hwnd).unwrap_or_default();
        let Some(token) = lumo_core::applemusic::clean_user_token(&text) else {
            self.st.notify(Icon::Lyrics, palette::ORANGE, "No Apple Music token copied", "Copy media-user-token first", 6.0, None);
            return;
        };
        if let Err(e) = services::secret::save(&paths::apple_music_token_path(), &token) {
            pwarn!("apple music: cannot save the token ({e})");
            self.st.notify(Icon::Lyrics, palette::ORANGE, "Couldn't save the token", "Details in lumo.log", 5.0, None);
            return;
        }
        pinfo!("apple music: token saved");
        self.forget_clipboard_text(&token);
        if let Some(s) = self.services.as_ref() {
            s.apple_music_token_changed();
        }
        self.refetch_lyrics();
        self.st.notify(Icon::Lyrics, palette::BLUE, "Checking the Apple Music token", "One moment", 4.0, None);
    }

    fn forget_apple_music_token(&mut self) {
        if let Err(e) = services::secret::delete(&paths::apple_music_token_path()) {
            pwarn!("apple music: cannot remove the token ({e})");
            self.st.notify(Icon::Lyrics, palette::ORANGE, "Couldn't remove the token", "Details in lumo.log", 5.0, None);
            return;
        }
        pinfo!("apple music: token removed");
        if let Some(s) = self.services.as_ref() {
            s.apple_music_token_changed();
        }
        self.refetch_lyrics();
        self.st.notify(Icon::Lyrics, palette::BLUE, "Apple Music lyrics off", "Lyrics from LRCLIB only", 4.0, None);
    }

    fn on_apple_music(&mut self, status: AppleMusicStatus) {
        let (color, title, subtitle) = match status {
            AppleMusicStatus::Accepted => (palette::GREEN, "Apple Music lyrics on", "LRCLIB fills in the rest"),
            AppleMusicStatus::Rejected => (palette::ORANGE, "Apple Music token not accepted", "Copy a fresh one from music.apple.com"),
            AppleMusicStatus::Unreachable => (palette::ORANGE, "Can't reach Apple Music", "Token saved; trying again later"),
            AppleMusicStatus::NoAccess => (palette::ORANGE, "No lyrics from Apple Music", "They need an Apple Music subscription"),
        };
        self.st.notify(Icon::Lyrics, color, title, subtitle, 6.0, None);
    }

    /// Remove history entries holding `secret` and clear the clipboard.
    fn forget_clipboard_text(&mut self, secret: &str) {
        let hits: Vec<usize> = self.st.clip.iter().enumerate().filter(|(_, c)| c.text.contains(secret)).map(|(i, _)| i).collect();
        for i in hits.into_iter().rev() {
            self.st.clip.remove(i);
        }
        self.st.ui.clip_flash = None;
        self.own_clip = Some(String::new());
        services::clipboard::set_text(self.helper, "");
    }

    /// Look up the current track's lyrics again (the sources changed).
    fn refetch_lyrics(&mut self) {
        if !self.st.settings.lyrics {
            return;
        }
        let Some(m) = self.st.media.clone().filter(|m| m.music_source.is_some() && !m.title.is_empty()) else { return };
        if let Some(svc) = self.services.as_ref() {
            self.st.lyrics = LyricsState { track_key: m.track_key.clone(), status: LyricsStatus::Loading, ..Default::default() };
            self.st.lyrics.request_id = svc.request_lyrics(&m, &self.st.settings);
        }
    }

    fn choose_music_path(&mut self, kind: crate::commands::MusicPick) {
        let flag = match &kind {
            crate::commands::MusicPick::Directory => &mut self.st.ui.choosing_music_dir,
            crate::commands::MusicPick::LyricsDirectory => &mut self.st.ui.choosing_lyrics_dir,
        };
        if *flag { return; }
        *flag = true;
        let initial=if kind==crate::commands::MusicPick::LyricsDirectory {paths::lyrics_download_dir(&self.st.settings).unwrap_or_else(||paths::music_dir(&self.st.settings))} else {paths::music_dir(&self.st.settings)};
        if let Err(e) = services::music_files::choose(kind, SendHwnd::new(self.helper), initial, self.sink.clone()) {
            *flag = false;
            self.st.notify(Icon::Folder, palette::ORANGE, "Couldn't start the picker", e.to_string(), 6.0, None);
        }
    }

    /// Apply side effects of a settings change (from the menu, a reload or a page).
    fn settings_changed(&mut self, before: &Settings) {
        if let Some(window) = self.lyrics_window.as_mut() { window.mark_dirty(); }
        if before.open_on_hover != self.st.settings.open_on_hover || before.hover_delay_ms != self.st.settings.hover_delay_ms {
            self.cancel_hover();
            if self.hover_opens() && self.hovering && !self.expanded {
                if let Some(p) = self.input.mouse { self.arm_hover(p); }
            }
        }
        if let Some(window) = self.settings_window.as_ref() { window.invalidate(); }
        // Show over games and Hide in full-screen apps follow each other.
        self.st.settings.pair_game_stats(before);
        if before.update_check != self.st.settings.update_check || before.automatic_updates != self.st.settings.automatic_updates {
            if let Some(u) = self.updater.as_mut() {
                u.switched(&mut self.st, crate::updates::unix_now());
            }
            self.schedule_update();
            self.fetch_setup();
        }
        let now = &self.st.settings;
        if before.theme != now.theme {
            self.st.accent = theme_accent(now.theme);
        }
        if before.hide_in_fullscreen != now.hide_in_fullscreen {
            // Apply at once: show the island over the full-screen app, or hide it.
            self.on_event(AppEvent::Fullscreen(self.fullscreen_on));
        }
        if before.step_aside != self.st.settings.step_aside && self.selftest.is_none() {
            // Step aside starts or stops watching the top of the screen.
            self.sync_top_watch();
        }
        let now = &self.st.settings;
        if before.clipboard_history && !now.clipboard_history {
            // Privacy: turning history off forgets everything, pinned items included.
            self.st.clip = lumo_core::cliphist::ClipHistory::default();
            self.st.ui.clip_flash = None;
            self.st.ui.clip_scroll = 0.0;
        }
        if !before.lyrics && now.lyrics {
            if let Some(m) = self.st.media.clone() {
                let stale = self.st.lyrics.track_key != m.track_key || self.st.lyrics.status == LyricsStatus::Idle;
                if stale && m.music_source.is_some() && !m.title.is_empty() {
                    if let Some(svc) = self.services.as_ref() {
                        self.st.lyrics = LyricsState { track_key: m.track_key.clone(), status: LyricsStatus::Loading, ..Default::default() };
                        self.st.lyrics.request_id = svc.request_lyrics(&m, &self.st.settings);
                    }
                }
            }
        }
        if before.local_lyrics != self.st.settings.local_lyrics || before.music_directory != self.st.settings.music_directory
            || before.lyrics_auto_save != self.st.settings.lyrics_auto_save || before.lyrics_download_directory != self.st.settings.lyrics_download_directory {
            self.refetch_lyrics();
        }
        if before.lyrics && !self.st.settings.lyrics {
            if let Some(s) = &self.services { s.cancel_lyrics(); }
            self.st.lyrics.request_id = 0;
        }
        self.st.pomodoro.cfg = self.st.settings.pomodoro.clone();
        if let Some(s) = self.services.as_ref() {
            s.set_clipboard_history(self.st.settings.clipboard_history);
        }
        for a in lumo_core::addons::ALL {
            let on = self.st.settings.addon_on(a.id);
            if before.addon_on(a.id) != on {
                pinfo!("add-ons: {} switched {}", a.id, if on { "on" } else { "off" });
                self.st.addons.insert(a.id, if !on { AddonStatus::Off } else if a.downloads() { AddonStatus::Checking } else { AddonStatus::Ready });
                if let Some(s) = self.services.as_ref() {
                    s.set_addon(a.id, on);
                }
            }
        }
        if before.addon_on("script") && !self.st.settings.addon_on("script") {
            self.st.script.pause(self.st.now);
            self.st.script.active = false;
            self.set_script_hotkey(false);
            if self.st.page == PageKind::Script {
                self.set_page(PageKind::Addons);
            }
        }
        if before.game_stats != self.st.settings.game_stats {
            if self.st.settings.game_stats && self.st.game.access == FpsAccess::Unknown {
                self.check_fps();
            }
            self.sync_game();
        }
        if before.game_peek != self.st.settings.game_peek && self.st.game.active {
            // Over a game: peeks start with one (it hides after its few
            // seconds), and the readout that stays shows at once.
            self.game_peek_until = 0.0;
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_PEEK);
            }
            self.peek_game(if self.st.settings.game_peek { "peeks picked" } else { "picked to stay" });
        }
        let history = self.st.settings.system_history;
        if before.system_history != history && self.selftest.is_none() {
            match &self.sysmon {
                Some(s) => s.background(history),
                None if history => self.start_sysmon(),
                None => {}
            }
            // Off while the page is closed: the kept minute goes, so the next opening starts fresh.
            if !history && !self.sysmon_on && !self.st.game.active {
                self.st.system.clear();
            }
        }
    }

    fn on_menu(&mut self, cmd: tray::MenuCmd) {
        use tray::MenuCmd::*;
        let before = self.st.settings.clone();
        let s = &mut self.st.settings;
        match cmd {
            ToggleIdleStyle(style) => s.idle_style = style,
            SetTheme(theme) => s.theme = theme,
            ToggleClipboard => s.clipboard_history = !s.clipboard_history,
            ToggleSystemHistory => s.system_history = !s.system_history,
            ToggleLyrics => s.island_lyrics = !s.island_lyrics,
            TogglePronunciation => s.lyrics_pronunciation = !s.lyrics_pronunciation,
            SetHideFullscreen(hide) => s.hide_in_fullscreen = hide,
            SetHideMaximized(hide) => s.hide_over_maximized = hide,
            ToggleGameStats => s.game_stats = !s.game_stats,
            ToggleGamePeek => s.game_peek = !s.game_peek,
            ToggleSounds => s.sounds = !s.sounds,
            TogglePinned => self.st.pinned = !self.st.pinned,
            RecordScreen => crate::services::recorder::record_screen(self.hwnd),
            ToggleAutostart => {
                let on = crate::autostart::is_enabled();
                if let Err(e) = crate::autostart::set_enabled(!on) {
                    pwarn!("autostart toggle failed: {e}");
                }
                // The Settings page's switch follows.
                crate::autostart::report(self.sink.clone(), None);
            }
            OpenSettings => {
                if !paths::settings_path().exists() {
                    let _ = save_json_atomic(&paths::settings_path(), &self.st.settings);
                }
                shell::open(&paths::settings_path().to_string_lossy());
            }
            OpenSettingsWindow => self.open_settings(),
            ReloadSettings => {
                // Never move the user's file aside or reset on a typo: keep the
                // current settings and say what's wrong.
                let path = paths::settings_path();
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| parse_json::<Settings>(&t)) {
                    Ok(s2) => {
                        let s2 = s2.sanitized();
                        let monitor_changed = s2.monitor != self.st.settings.monitor;
                        self.st.settings = s2.clone();
                        self.saved_settings = s2;
                        self.settings_writable = true;
                        if monitor_changed {
                            self.monitor = window::pick_monitor(self.st.settings.monitor);
                            self.set_dpi(self.monitor.dpi);
                        } else {
                            // The expanded width may have changed.
                            self.layout_surface();
                        }
                        self.refresh_monitor();
                    }
                    Err(e) => {
                        pwarn!("settings.json not reloaded: {e}");
                        self.st.notify(crate::icons::Icon::Sliders, palette::ORANGE, "Settings not reloaded", "settings.json has an error; see lumo.log", 5.0, None);
                    }
                }
            }
            OpenDataFolder => shell::open(&paths::data_dir().to_string_lossy()),
            About => self.st.open_update(),
            LyricsCredit => shell::open("https://lrclib.net"),
            AppleMusicToken => self.use_apple_music_token(),
            AppleMusicHelp => shell::open(APPLE_MUSIC_HELP_URL),
            AppleMusicForget => self.forget_apple_music_token(),
            // "Update to x…": the island opens on the Update sheet.
            Update => {
                self.st.open_update();
                self.expand();
            }
            Quit => {
                self.quit(0);
                return;
            }
        }
        self.settings_changed(&before);
        self.save_settings();
        self.request_frame();
    }

    // ------------------------------------------------------------ events

    pub fn on_event(&mut self, ev: AppEvent) {
        // Only events that change Settings content repaint the auxiliary window.
        let refresh_settings = matches!(&ev, AppEvent::Ai(_) | AppEvent::Autostart(_) | AppEvent::MusicPathChosen { .. } | AppEvent::Update(_) | AppEvent::ForegroundApp(_));
        let now = clock::mono();
        self.st.now = now;
        let media_before = matches!(ev, AppEvent::Media(_) | AppEvent::MediaTimeline { .. }).then(|| self.media_snapshot());
        match ev {
            AppEvent::Ai(event) => {
                let attached=matches!(&event,crate::ai::Event::Attached(_));
                let snipped=self.st.ai.snipping && matches!(&event,crate::ai::Event::Attached(_));
                let reply_chat=match &event{crate::ai::Event::Reply{chat,..}=>Some(*chat),_=>None};
                let showing_ai=(self.expanded && self.st.page==PageKind::Ai) || self.ai_window.as_ref().is_some_and(|w|unsafe{IsWindowVisible(w.hwnd)}.as_bool() && !unsafe{IsIconic(w.hwnd)}.as_bool());
                let notify=!(showing_ai && reply_chat==Some(self.st.ai.current.id));
                crate::ai::apply(&mut self.st,event,notify);
                if reply_chat == Some(self.st.ai.current.id) && self.expanded && self.st.page == PageKind::Ai {
                    self.reopen.interacted(PageKind::Ai, now);
                }
                if snipped {if let Some(w)=self.ai_window.as_ref(){w.show();}else{self.st.page=PageKind::Ai;self.expand();}}
                if attached && !self.locked && !self.display_off {
                    if let Some(w)=self.ai_window.as_mut(){w.focus_prompt(self.st.ai.draft.chars().count());}
                    else if self.expanded && self.st.page==PageKind::Ai {self.input.focused=Some(wid("ai.prompt"));self.input.caret=self.st.ai.draft.chars().count();self.focus_text();}
                }
                if let Some(w)=self.ai_window.as_ref(){w.invalidate();}
                self.apply_fullscreen(now);
                self.start_raises();
            }
            AppEvent::Media(session) => {
                let prev_key = self.st.media.as_ref().map(|m| m.track_key.clone());
                match session {
                    Some(mut s) => {
                        let track_same = self.st.media.as_ref().is_some_and(|prev| prev.track_key == s.track_key && prev.app_id == s.app_id && prev.music_source == s.music_source);
                        if let Some(p) = self.st.media_pending_seek {
                            if track_same {
                                if self.st.media.as_ref().is_some_and(|prev| {
                                    if lumo_core::nowplaying::is_apple_music(&prev.app_id) { p.holds(&s.timeline, now) }
                                    else { p.holds_feedback(&s.timeline, &prev.timeline, now) }
                                }) {
                                    if let Some(prev) = &self.st.media {
                                        s.timeline = prev.timeline;
                                    }
                                } else if p.expired(now) {
                                    self.st.media_pending_seek = None;
                                }
                            } else {
                                self.st.media_pending_seek = None;
                            }
                        }
                        // Keep already-decoded art if the track didn't change.
                        if s.art.is_none() {
                            if let Some(prev) = &self.st.media {
                                if prev.track_key == s.track_key {
                                    s.art = prev.art.clone();
                                    s.art_color = prev.art_color;
                                }
                            }
                        }
                        let changed = prev_key.as_deref() != Some(s.track_key.as_str()) || self.st.media.as_ref().is_some_and(|m| m.app_id != s.app_id || m.music_source != s.music_source);
                        if changed {
                            if let Some(svc) = &self.services { svc.cancel_lyrics(); }
                            self.st.lyrics = LyricsState { track_key: s.track_key.clone(), status: LyricsStatus::Idle, ..Default::default() };
                            if self.st.settings.lyrics && s.music_source.is_some() && !s.title.is_empty() {
                                if let Some(svc) = self.services.as_ref() {
                                    self.st.lyrics.status = LyricsStatus::Loading;
                                    self.st.lyrics.request_id = svc.request_lyrics(&s, &self.st.settings);
                                }
                            }
                        }
                        pages::media::observe_closed_cover(&s, now, self.st.power.saver);
                        self.st.media = Some(s);
                    }
                    None => {
                        self.st.media_pending_seek = None;
                        if let Some(s) = &self.services { s.cancel_lyrics(); }
                        self.st.media = None;
                        pages::media::forget_track_visuals();
                        self.st.lyrics = LyricsState::default();
                    }
                }
            }
            AppEvent::MusicPathChosen { kind, result } => {
                use crate::commands::MusicPick;
                match &kind {
                    MusicPick::Directory => self.st.ui.choosing_music_dir = false,
                    MusicPick::LyricsDirectory => self.st.ui.choosing_lyrics_dir = false,
                }
                match result {
                    Ok(Some(path)) => match kind {
                        MusicPick::Directory => {
                            let directory = path.to_string_lossy().into_owned();
                            if self.st.settings.music_directory == directory {
                                self.refetch_lyrics();
                            }
                            self.st.settings.music_directory = directory;
                            self.st.cmd(Command::SaveSettings);
                        }
                        MusicPick::LyricsDirectory => {
                            self.st.settings.lyrics_download_directory=path.to_string_lossy().into_owned();
                            self.st.cmd(Command::SaveSettings);
                        }
                    },
                    Ok(None) => {},
                    Err(error) => self.st.notify(Icon::Folder, palette::ORANGE, "Couldn't open the picker", error, 6.0, None),
                }
            }
            AppEvent::MediaTimeline { track_key, timeline } => {
                if let Some(m) = self.st.media.as_mut() {
                    if m.track_key == track_key {
                        if let Some(p) = self.st.media_pending_seek {
                            let waiting = if lumo_core::nowplaying::is_apple_music(&m.app_id) { p.holds(&timeline, now) }
                                else { p.holds_feedback(&timeline, &m.timeline, now) };
                            if waiting {
                                return;
                            } else if p.expired(now) {
                                self.st.media_pending_seek = None;
                            }
                        }
                        m.timeline = timeline;
                    }
                }
            }
            AppEvent::MediaArt { track_key, art } => {
                if let Some(m) = self.st.media.as_mut() {
                    if m.track_key == track_key {
                        m.art_color = art.as_ref().map(|a| a.accent_color());
                        m.art = art;
                        pages::media::observe_closed_cover(m, now, self.st.power.saver);
                    }
                }
            }
            AppEvent::Lyrics { track_key, request_id, result } => {
                if self.st.lyrics.track_key == track_key && self.st.lyrics.request_id == request_id {
                    match result {
                        crate::events::LyricsResult::Found { lyrics, source, source_file, ruby } => {
                            self.st.lyrics.status = LyricsStatus::Found;
                            self.st.lyrics.lyrics = Some(lyrics);
                            self.st.lyrics.source = source;
                            self.st.lyrics.source_file = source_file;
                            self.st.lyrics.ruby = ruby;
                        }
                        crate::events::LyricsResult::Instrumental => self.st.lyrics.status = LyricsStatus::Instrumental,
                        crate::events::LyricsResult::NotFound => self.st.lyrics.status = LyricsStatus::NotFound,
                        crate::events::LyricsResult::Error(e) => self.st.lyrics.status = LyricsStatus::Error(e),
                    }
                }
            }
            AppEvent::Volume { level, muted } => {
                let first = !self.st.volume.known;
                self.st.volume = crate::state::VolumeState { level, muted, known: true };
                if !first && self.st.settings.hud_volume {
                    self.st.show_hud(HudKind::Volume { level, muted }, 1.6);
                }
            }
            AppEvent::Power(p) => {
                let old = self.st.power;
                self.st.power = p;
                if old.saver != p.saver { if let Some(window) = self.ai_window.as_ref() { window.invalidate(); } }
                if self.st.settings.hud_power && p.has_battery {
                    if p.on_ac != old.on_ac {
                        self.st.show_hud(HudKind::Charging { percent: p.percent, plugged: p.on_ac }, 2.2);
                    } else if !p.on_ac && ((old.percent > 20 && p.percent <= 20) || (old.percent > 10 && p.percent <= 10)) {
                        self.st.show_hud(HudKind::LowBattery { percent: p.percent }, 4.0);
                    }
                }
            }
            AppEvent::Bluetooth { name, connected, battery, audio } => {
                if self.st.settings.hud_bluetooth {
                    self.st.show_hud(HudKind::Bluetooth { name, connected, battery, audio }, 2.6);
                }
            }
            AppEvent::Fullscreen(on) => {
                self.fullscreen_on = on;
                // The readout and its peeks first, then whether the island shows.
                self.sync_game();
            }
            AppEvent::WidgetsOpen(open) => {
                self.widgets_open = open;
                if open { if let Some(window) = self.lyrics_window.as_mut() { window.hide(); } }
                self.apply_fullscreen(clock::mono());
            }
            AppEvent::Foreground => {
                let foreground=unsafe{GetForegroundWindow()};
                if let Some(previous)=self.ai_resize_hold {
                    if previous.is_invalid() {
                        // Destroying the popup may briefly leave no foreground
                        // HWND. Record its return target before watching for a
                        // subsequent app change; don't dismiss the handoff.
                        if !foreground.is_invalid() && foreground!=self.hwnd {self.ai_resize_hold=Some(foreground);}
                    } else if previous!=foreground && !foreground.is_invalid() && foreground!=self.hwnd {
                        self.ai_resize_hold=None;self.pointer_left();
                    }
                }
                self.start_raises();
            },
            AppEvent::ForegroundApp(app) => {
                self.st.foreground_app = app;
                self.request_frame();
            }
            AppEvent::TopWindows(places) => {
                // The self-test puts its own window under the island.
                if self.selftest.is_none() {
                    self.top_windows = places;
                    self.request_frame();
                }
            }
            AppEvent::ShelfAdd(paths) if self.expanded && self.st.page == PageKind::Script && pages::script::take_drop(&mut self.st, &paths) => {
                // A text file dropped on the Script page becomes the script.
                self.drag_hover = false;
                self.st.drag_hover = false;
                self.request_frame();
            }
            AppEvent::ShelfAdd(paths) => {
                let n = self.st.store.shelf.add_paths(&paths, self.st.unix);
                for p in &paths {
                    self.st.cmds.push(Command::LoadThumb(p.clone()));
                }
                self.st.save();
                self.st.page = PageKind::Shelf;
                self.drag_hover = false;
                self.st.drag_hover = false;
                self.expand();
                pinfo!("shelf: {n} new item(s)");
            }
            AppEvent::DragHover(on) => {
                self.drag_hover = on;
                self.st.drag_hover = on;
                if on {
                    // The Script page takes a text file; everything else goes to the Shelf.
                    if !(self.expanded && self.st.page == PageKind::Script) {
                        self.st.page = PageKind::Shelf;
                    }
                    self.expand();
                } else {
                    self.maybe_schedule_collapse();
                }
            }
            AppEvent::ShelfThumb { path, image } => {
                // The answer to an app icon the System page asked for (only that
                // answer touches its icons), and to the Shelf when it asked too.
                let ours = self.icon_wait.remove(&path);
                let for_shelf = !ours || self.st.shelf_thumbs.contains_key(&path);
                if ours {
                    let px = pages::system::icon_px(self.gfx.scale());
                    self.st.app_icons.insert(path.clone(), image.as_ref().map(|i| pages::system::shrink_icon(i, px)));
                }
                if for_shelf {
                    self.st.shelf_thumbs.insert(path, image);
                }
            }
            AppEvent::System(m) => {
                // A reading still on its way when the page closed is dropped.
                if self.sysmon_on || self.st.game.active {
                    self.st.system = *m;
                }
                // Over a game the frame rate goes with each reading, once a second.
                if self.st.game.active {
                    self.st.game.rate = self.frames.as_ref().and_then(|f| f.rate());
                    let fps = self.st.game.rate.map(|r| r.fps);
                    let before: Vec<f32> = self.st.game.history.values().collect();
                    self.st.game.history.push(fps);
                    // Peeks only: a sharp drop shows the readout (at most every half minute).
                    let now = clock::mono();
                    if let Some(fps) = fps.filter(|_| self.st.settings.game_peek) {
                        if now >= self.game_peek_until && now - self.game_drop_at >= DROP_PEEK_EVERY_S && lumo_core::frames::sharp_drop(&before, fps) {
                            self.game_drop_at = now;
                            self.peek_game("the frame rate dropped");
                        }
                    }
                }
            }
            AppEvent::FpsAccess(a) => {
                if self.st.game.access == FpsAccess::Asking {
                    self.blur_text();
                }
                if a != self.st.game.access {
                    pinfo!("show over games: frames {a:?}");
                }
                self.st.game.access = a;
                self.sync_game();
            }
            AppEvent::ClipboardText(t) => {
                if self.own_clip.as_deref() == Some(t.as_str()) {
                    self.own_clip = None;
                } else {
                    self.st.clip.push(&t, self.st.unix);
                }
            }
            AppEvent::AppleMusic(status) => self.on_apple_music(status),
            AppEvent::Phone(e) => self.on_phone(e),
            AppEvent::Paired(e) => self.on_paired(e),
            AppEvent::Autostart(on) => self.st.ui.autostart = Some(on),
            AppEvent::Addon { id, status } => {
                // A late report from before a switch-off doesn't bring it back.
                if self.st.settings.addon_on(id) || status == AddonStatus::Off {
                    self.st.addons.insert(id, status);
                }
            }
            AppEvent::Update(services::update::UpdateEvent::Checked(r)) => {
                if let Some(u) = self.updater.as_mut() {
                    u.checked(&mut self.st, r, crate::updates::unix_now());
                }
                self.schedule_update();
                self.fetch_setup();
                // A frame, so a new version's notice goes up now, not with the next redraw.
                self.request_frame();
            }
            AppEvent::Update(services::update::UpdateEvent::Downloaded(r)) => {
                if let Some(u) = self.updater.as_mut() {
                    u.fetched(&mut self.st, r);
                }
                self.schedule_update();
                self.request_frame();
            }
            AppEvent::Log(s) => pinfo!("{s}"),
        }
        if let Some(before) = media_before {
            self.after_media_change(before, now);
        }
        if refresh_settings {
            if let Some(window) = self.settings_window.as_ref() { window.invalidate(); }
        }
        self.request_frame();
    }

    /// (player, track, playing) of the current media session.
    fn media_snapshot(&self) -> Option<(String, String, bool)> {
        self.st.media.as_ref().map(|m| (m.app_id.clone(), m.track_key.clone(), m.is_playing()))
    }

    /// Track the pause grace period and the title peek after a media update.
    fn after_media_change(&mut self, before: Option<(String, String, bool)>, now: f64) {
        let Some(m) = self.st.media.as_ref() else {
            self.st.media_paused_at = None;
            return;
        };
        let playing = m.is_playing();
        let same_player = before.as_ref().is_some_and(|b| b.0 == m.app_id);
        let new_track = before.as_ref().is_none_or(|b| b.1 != m.track_key);
        if playing {
            self.st.media_paused_at = None;
        } else if !same_player {
            // A different (or first) session that isn't playing: no grace.
            self.st.media_paused_at = None;
        } else if before.as_ref().is_some_and(|b| b.2) {
            // Playing -> paused: the pill lingers for a while.
            self.st.media_paused_at = Some(now);
        }
        // A new track starting while collapsed: show its title for a moment.
        if playing && new_track && !self.expanded {
            self.st.ui.peek_until = now + 3.0;
        }
    }

    // ------------------------------------------------------------ island state

    fn page_interaction(&mut self, p: Point) {
        // Only actual input inside page content counts. Tab selection,
        // hovering and opening clicks do not extend a continuation, and no
        // interaction bookkeeping runs on animation frames or at rest.
        if self.expanded && pages::content_area(self.island.body_rect_at(self.cx)).contains(p) {
            self.reopen.interacted(self.st.page, clock::mono());
        }
    }

    /// Generic click, dwell or tray opening. Explicit destinations use
    /// `expand` directly so notifications, drops and shortcuts always win.
    fn open_island(&mut self) {
        if !self.expanded {
            self.st.now = clock::mono();
            let page = self.reopen.page(&self.st, self.st.now);
            // Automatic routing must not write a new remembered preference.
            self.apply_page(page, false);
        }
        self.expand();
    }

    fn expand(&mut self) {
        self.ai_ctrl_watch=None;
        if !self.expanded {
            self.expanded = true;
            self.cancel_hover();
            if self.lyrics_window.is_some() { self.raise_topmost(); }
        }
        self.request_frame();
    }

    fn open_ai_reply(&mut self, chat: u64) {
        if self.st.ai.current.id!=chat {crate::ai::local(&mut self.st,&crate::ai::Action::Select(chat));}
        crate::ai::clear_ready(&mut self.st);
        self.st.ai.history=false;
        self.st.ai.scroll_to_reply=self.st.ai.current.messages.iter().rposition(|m|m.role=="assistant");
        self.set_page(PageKind::Ai);
        self.reopen.interacted(PageKind::Ai, clock::mono());
        self.ai_reply_peek=true;
        self.expand();
        self.collapse_due=false;
        unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}
        self.apply_fullscreen(self.st.now);
        self.raise_topmost();
    }

    /// (Re)start the hover dwell from `p`.
    fn arm_hover(&mut self, p: Point) {
        if !self.hover_opens() {
            self.cancel_hover();
            return;
        }
        self.hover_arm = Some(p);
        unsafe {
            SetTimer(self.hwnd, TIMER_HOVER, self.st.settings.hover_delay_ms.max(1), None);
        }
    }

    fn cancel_hover(&mut self) {
        self.hover_arm = None;
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_HOVER);
        }
    }

    fn hover_opens(&self) -> bool {
        self.st.settings.open_on_hover && !self.input.ctrl && !self.st.foreground_app.as_ref().is_some_and(|app| app.maximized_here)
    }

    fn resize_ai_chat(&mut self, height: f32, dragging: bool) {
        let height = height.clamp(240.0, self.st.ai.max_island_height);
        let changed = (height - self.st.ai.island_height).abs() > 0.5;
        if changed && height<self.st.ai.island_height {
            self.ai_resize_hold=Some(unsafe{GetForegroundWindow()});
            self.collapse_due=false;
            unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}
        }
        if changed {
            self.st.ai.island_height = height;
            self.st.ai.canvas_height = self.st.ai.canvas_height.max(height);
            self.layout_surface();
        }
        // A grip must stay underneath the pointer in both directions. The
        // chevron's discrete expand/compact action keeps its spring animation.
        if dragging && self.expanded && self.st.page == PageKind::Ai {
            self.island.snap(self.geom_for(&Mode::Expanded(PageKind::Ai)));
            self.collapse_due = false;
        }
        if changed { self.request_frame(); }
    }

    fn collapse(&mut self) {
        self.st.ai.close_popovers();
        self.ai_resize_hold=None;
        self.ai_reply_peek=false;
        if self.expanded {
            self.expanded = false;
            if !self.st.ai.full_open{pages::ai_markdown::clear();}
            if self.text_focus {
                self.blur_text();
            }
            self.set_sysmon(false);
            self.st.ui.clip_flash = None;
            // Settings closes with the island: it opens again on the page it was opened from.
            if self.st.page == PageKind::Settings {
                self.st.page = self.st.ui.before_settings.take().filter(|p| pages::tabs(&self.st).contains(p)).unwrap_or(PageKind::ALL[0]);
            }
            self.st.update.sheet = false;
            // A notice that arrived while expanded gets its full time now.
            let min_until = self.st.now + 4.0;
            if let Some(n) = self.st.notice.as_mut() {
                n.until = n.until.max(min_until);
            }
        }
        if self.st.page==PageKind::Ai && self.fullscreen_on {self.apply_fullscreen(self.st.now);}
        self.request_frame();
    }

    /// Nothing holds the open island: no pointer on it, no pin, typing,
    /// file drag, press or Phone drop.
    fn free_to_collapse(&self) -> bool {
        !self.hovering && self.ai_resize_hold.is_none() && !self.st.ai.attaching && !self.st.pinned && !self.text_focus && !self.drag_hover && self.input.pressed_id.is_none() && !self.phone_holds()
    }

    /// The pointer left the open island: it starts closing on the next frame,
    /// or after `collapse_delay_ms` when the user set a grace.
    fn pointer_left(&mut self) {
        if !self.expanded || !self.free_to_collapse() {
            return;
        }
        match self.st.settings.collapse_delay_ms {
            0 => {
                self.collapse_due = true;
                self.request_frame();
            }
            ms => unsafe {
                SetTimer(self.hwnd, TIMER_COLLAPSE, ms, None);
            },
        }
    }

    /// A hold ended (see [`HOLD_GRACE_MS`]): close soon unless the pointer is on the island.
    fn maybe_schedule_collapse(&mut self) {
        if self.expanded && self.ai_resize_hold.is_none() && !self.hovering && !self.st.pinned && !self.text_focus && !self.drag_hover && !self.phone_holds() {
            unsafe {
                SetTimer(self.hwnd, TIMER_COLLAPSE, HOLD_GRACE_MS.max(self.st.settings.collapse_delay_ms), None);
            }
        }
    }

    /// The System page opened or closed. Open, its readings arrive each
    /// second; with the history kept the minute behind them arrives at once,
    /// else each opening starts a fresh minute.
    fn set_sysmon(&mut self, on: bool) {
        if on == self.sysmon_on {
            return;
        }
        self.sysmon_on = on;
        // The self-test shows sample readings; the real sources get their own check.
        if self.selftest.is_some() {
            return;
        }
        if on && self.sysmon.is_none() {
            self.start_sysmon();
        }
        if let Some(s) = &self.sysmon {
            s.open(on);
        }
        // The readout over a game still draws from them.
        if !on && !self.st.settings.system_history && !self.st.game.active {
            self.st.system.clear();
        }
    }

    fn start_sysmon(&mut self) {
        self.sysmon_rest = self.rest_now();
        match services::sysmon::SysMon::start(self.sink.clone(), self.st.settings.system_history, self.sysmon_rest) {
            Ok(s) => self.sysmon = Some(s),
            Err(e) => pwarn!("sysmon: couldn't start ({e})"),
        }
    }

    /// Nobody at the PC (locked, or the display off) and battery saver.
    fn rest_now(&self) -> lumo_core::sysmon::Rest {
        lumo_core::sysmon::Rest { away: self.locked || self.display_off, saver: self.st.power.saver }
    }

    /// The System reader rests while nobody is at the PC, and its kept minute
    /// also on battery saver (`lumo_core::sysmon::reads`). What it drops, the
    /// page drops too, so a minute from before never shows as the last one.
    fn tell_sysmon_rest(&mut self) {
        let rest = self.rest_now();
        if rest == self.sysmon_rest || self.selftest.is_some() {
            return;
        }
        self.sysmon_rest = rest;
        if let Some(s) = &self.sysmon {
            s.rest(rest);
        }
        if !lumo_core::sysmon::reads(self.sysmon_on, self.st.game.active, self.st.settings.system_history, rest) {
            self.st.system.clear();
        }
    }

    /// Over a full-screen app the island hides when the setting says Hide, and
    /// under Show over games while the readout is away: between its peeks
    /// (peeks only), or once Alt+Shift+F hid it. Then nothing covers the game,
    /// and Windows sends its frames straight to the screen.
    fn fullscreen_hides(&self, now: f64) -> bool {
        self.widgets_open || (self.fullscreen_on && !(self.ai_reply_peek && self.expanded && self.st.page==PageKind::Ai) && !self.st.notice.as_ref().is_some_and(|n| n.is_ai_reply() || (n.until>now && matches!(n.action,Some(NoticeAction::OpenPage(PageKind::Ai))))) && (self.st.settings.hide_in_fullscreen || (self.st.game.active && self.readout_away(now))))
    }

    /// Show over games: the readout is off the screen. It stays unless the
    /// person picked peeks (`Settings::game_peek`) or hid it.
    fn readout_away(&self, now: f64) -> bool {
        if self.st.settings.game_peek {
            now >= self.game_peek_until
        } else {
            self.game_hidden
        }
    }

    /// Shows or hides the island for what is in front now.
    fn apply_fullscreen(&mut self, now: f64) {
        let hide = self.fullscreen_hides(now);
        if hide != self.fullscreen_hidden {
            self.fullscreen_hidden = hide;
            self.shown = false;
            self.gfx.set_shown(false);
            if hide {
                unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); }
                self.collapse();
                self.cancel_hover();
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_COLLAPSE);
                }
                self.hovering = false;
                self.input.mouse = None;
            } else {
                // Paint the current compact/hidden state before showing the
                // HWND, so closing Widgets never flashes the old open page.
                self.request_frame();
            }
            let why = if self.fullscreen_on && !self.st.settings.hide_in_fullscreen && self.st.game.active { " (show over games)" } else { "" };
            pinfo!("fullscreen app {}: island {}{why}", if self.fullscreen_on { "in front" } else { "gone" }, if hide { "hidden" } else { "shown" });
        }
        if !hide {
            // A borderless game or a browser in F11 may have taken the
            // top of the topmost band, and an island shown again after
            // a full-screen app is not raised by showing it.
            self.start_raises();
        }
    }

    /// Show over games: the readout shows over the game, and stays (peeks
    /// only: for a few seconds).
    fn peek_game(&mut self, why: &str) {
        let now = clock::mono();
        if !self.st.settings.game_peek {
            self.game_hidden = false;
            pinfo!("show over games: readout shows and stays ({why})");
            self.apply_fullscreen(now);
            return;
        }
        self.game_peek_until = now + PEEK_S;
        unsafe {
            SetTimer(self.hwnd, TIMER_PEEK, (PEEK_S * 1000.0) as u32, None);
        }
        pinfo!("show over games: readout shows ({why})");
        self.apply_fullscreen(now);
    }

    /// Alt+Shift+F: hide the readout, or show it again (peeks only: show it
    /// for a few seconds, or hide it again before its time).
    fn toggle_readout(&mut self) {
        if !self.st.game.active {
            return;
        }
        let now = clock::mono();
        if !self.st.settings.game_peek {
            self.game_hidden = !self.game_hidden;
            pinfo!("show over games: readout {} (Alt+Shift+F)", if self.game_hidden { "hidden until the next game" } else { "shows again" });
            self.apply_fullscreen(now);
            return;
        }
        if now < self.game_peek_until {
            self.game_peek_until = 0.0;
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_PEEK);
            }
            self.apply_fullscreen(now);
        } else {
            self.peek_game("Alt+Shift+F");
        }
    }

    /// Alt+Shift+F belongs to Lumo only while a game is in front.
    fn set_peek_hotkey(&mut self, on: bool) {
        if on == self.peek_hotkey {
            return;
        }
        unsafe {
            if on {
                match RegisterHotKey(self.helper, HOTKEY_PEEK, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, u32::from(b'F')) {
                    Ok(()) => self.peek_hotkey = true,
                    Err(e) => pwarn!("show over games: Alt+Shift+F is taken ({e})"),
                }
            } else {
                let _ = UnregisterHotKey(self.helper, HOTKEY_PEEK);
                self.peek_hotkey = false;
            }
        }
    }

    /// Alt+Shift+S belongs to Lumo only while the Script page can read.
    fn set_script_hotkey(&mut self, on: bool) {
        if on == self.script_hotkey {
            return;
        }
        unsafe {
            if on {
                match RegisterHotKey(self.helper, HOTKEY_SCRIPT, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, u32::from(b'S')) {
                    Ok(()) => self.script_hotkey = true,
                    Err(e) => pwarn!("script: Alt+Shift+S is taken ({e})"),
                }
            } else {
                let _ = UnregisterHotKey(self.helper, HOTKEY_SCRIPT);
                self.script_hotkey = false;
            }
        }
    }

    /// The Script page: a reading pauses while its page isn't on screen, the
    /// island may close once nothing holds it, and the shortcut is Lumo's
    /// only while there's a script to read on the page. Whether Record shows
    /// is asked as the page comes on screen.
    fn script_tick(&mut self) {
        let shown = self.st.script_on() && self.expanded && self.st.page == PageKind::Script
            && !self.locked && !self.display_off && !self.st.power.saver && !self.fullscreen_hidden;
        if shown && !self.script_shown && self.selftest.is_none() {
            self.st.recorder = services::recorder::installed();
        }
        self.script_shown = shown;
        if !shown && self.st.script.playing() {
            self.st.script.pause(self.st.now);
        }
        let held = self.st.script_holds_island();
        if self.script_held && !held {
            self.maybe_schedule_collapse();
        }
        self.script_held = held;
        let can_read = self.st.script_on() && !self.locked && !self.display_off && !self.st.power.saver
            && self.st.page == PageKind::Script && !self.st.store.script.is_empty() && (self.expanded || self.st.script.active);
        self.set_script_hotkey(can_read && self.selftest.is_none());
    }

    /// Out of screen shares and recordings while the Script page shows, and
    /// until the island has closed over it.
    fn sync_script_capture(&mut self, on: bool) {
        if on == self.script_excluded {
            return;
        }
        self.script_excluded = on;
        if !on {
            window::exclude_from_capture(self.hwnd, window::CaptureOwner::Script, false);
            return;
        }
        let ok = window::windows_build().is_some_and(|b| b >= window::EXCLUDE_FROM_CAPTURE_BUILD)
            && window::exclude_from_capture(self.hwnd, window::CaptureOwner::Script, true);
        if self.st.script_hidden != Some(ok) {
            pinfo!("script: {}", if ok { "left out of screen capture" } else { "Windows can't leave the island out of screen capture" });
        }
        self.st.script_hidden = Some(ok);
    }

    /// Show over games: the readout shows while the setting is on and a
    /// full-screen app has the island's monitor with the island over it.
    /// It follows the full-screen app; the readings and the frame count run
    /// only meanwhile.
    fn sync_game(&mut self) {
        let want = self.st.settings.game_stats && self.fullscreen_on && !self.st.settings.hide_in_fullscreen && self.selftest.is_none();
        if want != self.st.game.active {
            self.st.game.active = want;
            self.set_peek_hotkey(want);
            if !want {
                self.game_peek_until = 0.0;
                self.game_hidden = false;
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_PEEK);
                }
            }
            if want && self.sysmon.is_none() {
                self.start_sysmon();
            }
            // A failure last time is tried again with each new game.
            if want && matches!(self.st.game.access, FpsAccess::Failed(_)) {
                self.st.game.access = FpsAccess::Unknown;
            }
            if let Some(s) = &self.sysmon {
                s.game(want);
            }
            if !want {
                self.frames = None;
                self.game_pid = None;
                self.st.game.rate = None;
                self.st.game.app = None;
                if !self.sysmon_on && !self.st.settings.system_history {
                    self.st.system.clear();
                }
            }
            pinfo!("show over games: readout {}", if want { "on" } else { "off" });
        }
        if !want {
            self.apply_fullscreen(clock::mono());
            return;
        }
        let pid = services::fullscreen::fullscreen_pid();
        if pid != self.game_pid {
            self.game_pid = pid;
            self.st.game.app = pid.and_then(services::sysmon::app_name);
            self.st.game.rate = None;
            self.st.game.history = Default::default();
            if let (Some(f), Some(p)) = (&self.frames, pid) {
                f.set_target(p);
            }
            pinfo!("show over games: {} in front", self.st.game.app.as_deref().unwrap_or("an app"));
            // A game came to the front: the readout shows for a moment.
            self.peek_game("a game came to the front");
        } else {
            self.apply_fullscreen(clock::mono());
        }
        let Some(pid) = pid else { return };
        if self.frames.is_none() && matches!(self.st.game.access, FpsAccess::Unknown | FpsAccess::Ready) {
            match services::frames::Frames::start(pid) {
                Ok(f) => {
                    self.frames = Some(f);
                    self.st.game.access = FpsAccess::Ready;
                }
                // Tells the one-time OK apart from a sign-out still to come
                // (once: a check that said yes isn't asked again).
                Err(services::frames::StartError::NoAccess) if self.st.game.access == FpsAccess::Ready => {
                    self.st.game.access = FpsAccess::NeedsOk;
                }
                Err(services::frames::StartError::NoAccess) => self.check_fps(),
                Err(services::frames::StartError::Failed(why)) => {
                    pwarn!("frames: couldn't start ({why})");
                    self.st.game.access = FpsAccess::Failed(why);
                }
            }
        }
    }

    /// Asks (off the UI thread) whether Windows lets Lumo read frames.
    fn check_fps(&mut self) {
        if self.selftest.is_some() || matches!(self.st.game.access, FpsAccess::Checking | FpsAccess::Asking) {
            return;
        }
        self.st.game.access = FpsAccess::Checking;
        services::frames::check_async(self.sink.clone());
    }

    /// Puts the island back on top after another window came forward. Apps
    /// often make themselves topmost just after they take the focus, with no
    /// event of their own, so a few more checks follow over the next two
    /// seconds. (Exclusive full-screen still covers every window.)
    fn start_raises(&mut self) {
        if !self.shown || self.fullscreen_hidden || self.widgets_open || self.settings_in_front() {
            return;
        }
        self.raise_if_covered();
        self.raises_left = RAISE_REPEATS;
        unsafe {
            SetTimer(self.hwnd, TIMER_RAISE, RAISE_INTERVAL_MS, None);
        }
    }

    /// Raises the island when a window sits above it or it lost its topmost
    /// style, and logs each culprit once.
    fn raise_if_covered(&mut self) {
        if self.fullscreen_hidden || self.widgets_open || self.settings_in_front() { return; }
        let topmost = unsafe { GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST.0 as isize != 0;
        let above = services::fullscreen::window_above(self.hwnd);
        if topmost && above.is_none() {
            return;
        }
        let key = if topmost { above.clone().unwrap_or_default() } else { String::new() };
        if self.covered_by.insert(key) {
            match (&above, topmost) {
                (_, false) => pinfo!("island lost its topmost style; raised again"),
                (Some(class), true) => pinfo!("island covered by a {class} window; raised again"),
                (None, true) => {}
            }
        }
        self.raise_topmost();
    }

    /// Top of the topmost band, without taking the focus.
    fn raise_topmost(&self) {
        if self.fullscreen_hidden || self.widgets_open || self.settings_in_front() { return; }
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    fn settings_in_front(&self) -> bool {
        self.settings_window.as_ref().is_some_and(|window| window.has_open_priority())
    }

    fn set_noactivate(&self, on: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_NOACTIVATE.0 as isize;
            let new = if on { ex | flag } else { ex & !flag };
            if new != ex {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, new);
            }
        }
    }

    fn focus_text(&mut self) {
        if self.text_focus {
            return;
        }
        self.text_focus = true;
        unsafe {
            self.prev_foreground = GetForegroundWindow();
            self.set_noactivate(false);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(self.hwnd);
        }
    }

    fn blur_text(&mut self) {
        self.input.focused = None;
        if !self.text_focus {
            return;
        }
        self.text_focus = false;
        self.set_noactivate(true);
        unsafe {
            if !self.prev_foreground.is_invalid() && IsWindow(self.prev_foreground).as_bool() && GetForegroundWindow() == self.hwnd {
                let _ = SetForegroundWindow(self.prev_foreground);
            }
        }
        self.maybe_schedule_collapse();
    }

    fn refresh_monitor(&mut self) {
        let m = match self.st.settings.monitor {
            MonitorChoice::Cursor => {
                window::monitor_info(windows::Win32::Graphics::Gdi::HMONITOR(self.monitor.handle as *mut core::ffi::c_void))
                    .unwrap_or_else(|| window::pick_monitor(MonitorChoice::Primary))
            }
            c => window::pick_monitor(c),
        };
        if m != self.monitor {
            pinfo!("monitor: {:?} dpi {}", m.rect, m.dpi);
            self.monitor = m;
            self.set_dpi(m.dpi);
            self.layout_surface();
        }
        // The refresh rate may have changed too (60 <-> 144 Hz).
        self.clock.set_monitor(m.handle);
        self.request_frame();
    }

    fn mouse_dip(&self, lp: LPARAM) -> Point {
        let x = (lp.0 & 0xffff) as u16 as i16 as f32;
        let y = ((lp.0 >> 16) & 0xffff) as u16 as i16 as f32;
        let s = self.gfx.scale();
        Point::new(x / s, y / s)
    }

    fn hit_at(&self, p: Point) -> Option<Hit> {
        self.hits.iter().rev().find(|h| h.rect.contains(p)).copied()
    }

    /// Whether `p` is over a part of the window that takes the mouse: the
    /// whole canvas while expanded (hit margin), only the island collapsed
    /// (floating: its outline and the gap above it, see [`hit_mask_for`]).
    fn hovering_inside(&self, p: Point) -> bool {
        if self.expanded {
            return if self.ai_picker_rect().is_some() { self.hit_mask().contains(p) }
                else { self.island.canvas_rect_at(self.cx).contains(p) };
        }
        if self.island.floating() {
            return self.hit_mask().contains(p);
        }
        self.island.body_rect_at(self.cx).contains(p)
    }

    /// Where the window takes the mouse now (see [`hit_mask_for`]).
    fn hit_mask(&self) -> HitMask {
        if let Some(picker) = self.ai_picker_rect() {
            let top = self.island.body_rect_at(self.cx).bottom();
            let bridge = Rect::new(picker.x, top, picker.w, picker.bottom() - top);
            return HitMask { rect: Some(self.island.canvas_rect_at(self.cx)), notch: None, pill: Some((bridge, 0.0)) };
        }
        hit_mask_for(&self.island, self.cx, self.expanded, self.rests_hidden())
    }

    fn ai_picker_rect(&self) -> Option<Rect> {
        if !self.expanded || self.st.page != PageKind::Ai || !pages::ai::picker_below(&self.st) { return None; }
        let body = self.island.body_rect_at(self.cx);
        let y = body.bottom() + 8.0;
        let available = (self.monitor.work.bottom - self.surface.y) as f32 / self.gfx.scale() - y - 12.0;
        Some(pages::ai_models::popup_rect(&self.st, Rect::new(body.right() - pages::PAD - (body.w - 2.0 * pages::PAD).min(460.0), y,
            (body.w - 2.0 * pages::PAD).min(460.0), available.clamp(120.0, 400.0))))
    }

    /// Include the small bridge to the picker, so moving between the island
    /// and its list never looks like leaving the island.
    fn open_canvas_rect(&self) -> Rect {
        let card = self.island.canvas_rect_at(self.cx);
        self.ai_picker_rect().map_or(card, |picker| union_rect(card, picker))
    }

    /// The closed island rests invisible, a strip at the top edge that only
    /// catches the pointer: the Hidden idle look, or stepped aside.
    fn rests_hidden(&self) -> bool {
        self.mode == Mode::Aside || (self.mode == Mode::Idle && self.st.settings.idle_style == IdleStyle::Hidden)
    }

    /// Step aside: where the island rests (what it shows at rest, the clock
    /// or the song, never the strip it steps aside to), from the edge it
    /// hangs from to its bottom, in screen pixels. Classic's shoulders and
    /// the floating Pill's gap count: they cover what is under them too.
    fn rest_rect(&self) -> RECT {
        let g = activities::geom(&activities::resting_mode(&self.st), &self.st);
        let floating = self.island.floating();
        let w = g.w + if floating { 0.0 } else { 2.0 * SHOULDER };
        let h = g.h + if floating { pill::TOP_GAP } else { 0.0 };
        let s = self.gfx.scale();
        let cx = self.surface.x as f32 + self.cx * s;
        let left = (cx - w * s / 2.0).floor() as i32;
        let right = (cx + w * s / 2.0).ceil() as i32;
        RECT { left, top: self.surface.y, right, bottom: self.surface.y + (h * s).ceil() as i32 }
    }

    /// Clear a maximized app's top bar according to its saved rule. The legacy
    /// Step aside path also remains available to the visual self-test.
    /// Over a full-screen app the
    /// full-screen choice decides instead (Hide, or Stay on top for the
    /// readout over games). Each frame, before the mode is picked: it only
    /// compares a few rects.
    fn sync_aside(&mut self) {
        let r = self.rest_rect();
        let under = self.top_windows.iter().any(|t| t[0] < r.right && t[2] > r.left && t[1] < r.bottom && t[3] > r.top);
        let per_app = self.st.foreground_app.as_ref().is_some_and(|app| {
            app.maximized_here && self.st.settings.hide_for_maximized_app(&app.executable)
        });
        let keep_visible = self.st.foreground_app.as_ref().is_some_and(|app| self.st.settings.keeps_app_visible(&app.executable));
        let aside = !self.fullscreen_on && !keep_visible && ((per_app && !self.hovering) || (self.st.settings.step_aside && under));
        if aside != self.st.aside {
            self.st.aside = aside;
            if !per_app {
                let why = if aside { "a window is under the island; it rests out of the way" } else { "nothing under the island; it rests as before" };
                self.log_queue.push(format!("step aside: {why}"));
            }
        }
    }

    /// Whether `p`, where the window took the mouse, is off the island.
    /// GPU renderer: the invisible hit mask trails a moving outline by up to
    /// [`MASK_INTERVAL`], and a pointer over the part the island already left
    /// (well outside its shape and shoulders) gets no hover. Software
    /// renderer: see [`App::over_shadow`].
    fn over_stale_mask(&self, p: Point) -> bool {
        if self.input.down {
            return false;
        }
        if !self.gfx.needs_hit_mask() {
            return self.over_shadow(p);
        }
        if self.island.settled() {
            return false;
        }
        let r = if self.expanded { self.open_canvas_rect() } else { self.island.hover_rect_at(self.cx).inset(-SHOULDER - 2.0, 0.0) };
        !Rect::new(r.x, r.y, r.w, r.h + 2.0).contains(p)
    }

    /// Software renderer, floating island (the Pill theme) collapsed: a
    /// layered window takes the mouse wherever its pixels aren't fully
    /// transparent. That renderer draws the pill without its shadow
    /// (`pill::shape::draw_body`), so what's left beside the outline is its
    /// anti-aliased edge: a pointer there is off the island (no hover, no
    /// click, no menu). (The GPU renderer's hit mask decides on its own.)
    /// `p` is a pixel's corner: its centre is tested, so an edge pixel the
    /// outline mostly covers still counts.
    fn over_shadow(&self, p: Point) -> bool {
        if self.expanded || !self.island.floating() || self.gfx.needs_hit_mask() {
            return false;
        }
        let half = 0.5 / self.gfx.scale();
        !self.hit_mask().contains(Point::new(p.x + half, p.y + half))
    }

    fn target_mode(&self) -> Mode {
        if self.expanded {
            return Mode::Expanded(self.st.page);
        }
        activities::collapsed_mode(&self.st)
    }

    /// Island geometry for `m`. The resting pill lifts a little under the
    /// pointer: instant feedback while the hover dwell runs.
    fn geom_for(&self, m: &Mode) -> Geom {
        if *m == Mode::Aside {
            let rest = activities::geom(&activities::resting_mode(&self.st), &self.st);
            // Retract at the compact width, without stretching sideways into
            // a fixed-width strip. Pill docks at the edge as it disappears.
            return Geom::new(rest.w, 5.0, 2.5).below(-pill::TOP_GAP);
        }
        let g = activities::geom(m, &self.st);
        let liftable = match m {
            Mode::Idle => self.st.settings.idle_style != IdleStyle::Hidden,
            Mode::Activity(_) => true,
            _ => false,
        };
        if liftable && self.hovering && !self.expanded {
            if pill::on(self.st.settings.theme) {
                // Floating, it swells evenly about its middle and stays a capsule.
                pill::motion::hover_lift(g)
            } else {
                Geom::new(g.w + HOVER_LIFT.0, g.h + HOVER_LIFT.1, g.r + HOVER_LIFT.2)
            }
        } else {
            g
        }
    }

    // ------------------------------------------------------------ frame

    fn refresh_time(&mut self) {
        self.st.now = clock::mono();
        self.st.local = clock::local_now();
        self.st.unix = clock::unix_now();
    }

    fn tick(&mut self) {
        let now = self.st.now;
        if let Some(h) = &self.st.hud {
            if now >= h.until {
                self.st.hud = None;
            }
        }
        if let Some(n) = &self.st.notice {
            if !self.expanded && !n.is_ai_reply() && now >= n.until {
                self.st.notice = None;
                if self.st.ai.ready_reply.is_none(){self.apply_fullscreen(now);}
            }
        }
        if let Some(ready)=self.st.ai.ready_reply.as_ref() {
            let current=ready.chat==self.st.ai.current.id;
            let exists=current || self.st.ai.chats.iter().any(|c|c.id==ready.chat);
            let viewed=current && ((self.expanded && self.st.page==PageKind::Ai) || self.ai_window.as_ref().is_some_and(|w|unsafe{IsWindowVisible(w.hwnd)}.as_bool() && !unsafe{IsIconic(w.hwnd)}.as_bool()));
            if !exists || viewed {crate::ai::clear_ready(&mut self.st);self.apply_fullscreen(now);}
            else if self.st.notice.is_none() {crate::ai::announce_ready(&mut self.st);self.apply_fullscreen(now);}
        }
        // A new version's notice (or "Updated to") waits for the island to be
        // free. The first frame counts (the window shows at its end).
        let free = !self.fullscreen_hidden && !self.locked && !self.display_off && !self.st.power.saver && !self.expanded && self.lyrics_window.is_none() && self.st.notice.is_none();
        if self.updater.as_mut().is_some_and(|u| u.announce(&mut self.st, free)) {
            self.request_frame();
        }
        if let Some(ended) = self.st.pomodoro.tick(now) {
            pages::focus::on_phase_ended(&mut self.st, ended);
        }
        self.script_tick();
    }

    /// Draw and show one frame. `tick` is the vblank that triggered it (None
    /// for self-test captures, which run off the clock).
    fn frame(&mut self, deferred: &mut Vec<Deferred>, tick: Option<FrameTick>) {
        let watch=self.hovering && !self.expanded && !self.locked && !self.display_off && !self.st.power.saver && !self.fullscreen_hidden && self.st.notice.is_none() && self.st.hud.is_none();
        if watch && self.ai_ctrl_watch.is_none(){self.input.ctrl=services::ai_shortcut::control_down();self.ai_ctrl_watch=services::ai_shortcut::Watch::start(self.hwnd).ok();}else if !watch{self.ai_ctrl_watch=None;}
        if !self.expanded && !matches!(self.mode,Mode::Expanded(_)) && self.island.settled() && self.st.ai.canvas_height>240.0{self.st.ai.canvas_height=240.0;self.st.ai.island_height=240.0;self.layout_surface();}

        if self.lyrics_window.is_some() && (self.locked || self.display_off) {
            self.refresh_time(); self.tick();
            if let Some(window) = self.lyrics_window.as_mut() { window.hide(); }
            self.run_commands(deferred);
            self.clock.set_active(false); self.schedule_wake(); return;
        }
        if self.fullscreen_hidden {
            // Nothing is drawn, but timers, chimes and commands keep working.
            self.refresh_time();
            self.tick();
            self.run_commands(deferred);
            let (moving, live) = self.draw_lyrics_window(deferred);
            self.clock.set_divisor(if moving { 1 } else { ambient_divisor(self.clock.nominal_hz(), 30.0) });
            self.clock.set_active(moving || live);
            self.motion = None;
            self.schedule_wake();
            return;
        }
        let rendered_at = clock::mono();
        self.refresh_time();
        // Stamp the frame with the vblank it will be shown after, so every
        // animation step is a whole number of refresh periods: even motion at
        // 60, 144 or 240 Hz, whatever the thread's wake-up jitter.
        let period = tick.map_or_else(|| self.clock.period(), |t| t.period);
        let anim_t = tick.map_or(rendered_at, |t| t.vblank + t.period).max(self.last_frame_t);
        let resumed = tick.is_some_and(|t| t.resumed) || self.last_frame_t == 0.0;
        // Coming out of rest, a motion advances one refresh, not the whole
        // time the clock was parked.
        let mut dt = if resumed { period } else { anim_t - self.last_frame_t };
        self.last_frame_t = anim_t;
        self.st.now = self.st.now.max(anim_t);
        self.tick();
        let now = self.st.now;

        // The pointer left the open island since the last frame: the close
        // starts on this frame, no timer in between.
        if std::mem::take(&mut self.collapse_due) && self.expanded && self.free_to_collapse() {
            self.collapse();
            self.left_at = clock::mono();
        }

        // A window under where the island rests: it rests out of the way.
        self.sync_aside();
        let target = self.target_mode();
        let mut retarget = false;
        if target != self.mode {
            let new_content = activities::content_key(&target) != activities::content_key(&self.mode);
            // A morph turned around mid-fade (the pointer came back while the
            // island closed): the content coming back is the one still fading
            // out, and it fades back in from where it is, not from nothing.
            let back = self
                .outgoing
                .filter(|o| new_content && activities::content_key(&o.0) == activities::content_key(&target))
                .map(|(_, t0, _, from)| from * outgoing_left(now - t0));
            if new_content {
                // The leaving content fades from how visible it is now.
                self.outgoing = Some((self.mode, now, self.island.target(), self.island.content_alpha()));
            }
            if let (Mode::Expanded(a), Mode::Expanded(b)) = (self.mode, target) {
                if a != b {
                    let dir = if b.tab_index() > a.tab_index() { 1.0 } else { -1.0 };
                    // Like the outline, show one refresh of motion on the
                    // first frame instead of drawing the old page again.
                    self.page_anim = Some((a, now - period.min(PAGE_SWITCH), dir));
                }
            }
            self.motion_name = motion_name(&self.mode, &target);
            self.island.set_target(self.geom_for(&target), now, new_content);
            if let Some(a) = back {
                self.island.resume_content(a);
            }
            self.mode = target;
            retarget = true;
        } else {
            // Size can change within a mode (a page's height, the hover lift); follow it.
            let g = self.geom_for(&self.mode);
            if g != self.island.target() {
                if self.island.settled() {
                    self.motion_name = "resize";
                }
                if self.st.power.saver && matches!(self.mode, Mode::Activity(activities::Activity::Media)) {
                    // Lyrics resize every line: on saver, swap size without
                    // spending a refresh-rate animation on each change.
                    self.island.snap(g);
                } else {
                    self.island.set_target(g, now, false);
                }
                retarget = true;
            }
        }
        // A motion that starts on this frame moves one refresh's worth, even
        // when the previous frame was a slower ambient one.
        if retarget {
            dt = dt.min(period);
        }
        let dt = dt.clamp(0.0, 0.05) as f32;
        if self.st.power.saver && self.lyrics_window.is_some() {
            self.island.snap(self.geom_for(&self.mode));
            self.outgoing = None; self.page_anim = None;
        }
        let aside_target = if self.mode == Mode::Aside { 0.0 } else { 1.0 };
        let delta = dt / 0.18;
        self.aside_visibility = if self.locked || self.display_off || self.st.power.saver {
            aside_target
        } else if self.aside_visibility < aside_target {
            (self.aside_visibility + delta).min(aside_target)
        } else {
            (self.aside_visibility - delta).max(aside_target)
        };
        let visibility = self.aside_visibility * self.aside_visibility * (3.0 - 2.0 * self.aside_visibility);
        self.set_sysmon(self.mode == Mode::Expanded(PageKind::System));
        // The Pill theme floats the island below the top edge.
        self.island.set_floating(pill::on(self.st.settings.theme).then_some(pill::TOP_GAP));
        self.island.step(dt, now);
        // Only a real WM_MOUSEMOVE into the settled card may hand a resize
        // back to hover. A cached pointer coordinate can be from before the
        // card shrank and must not silently release its hold here.
        let picker_room = self.expanded && self.st.page == PageKind::Ai && pages::ai::picker_below(&self.st);
        if picker_room != self.ai_picker_room {
            self.ai_picker_room = picker_room;
            self.layout_surface();
        }
        self.sync_room();
        self.ensure_room();
        self.note_motion(retarget, rendered_at, period, dt as f64);

        let scale = self.gfx.scale();
        let (sw, sh) = (self.surface.w, self.surface.h);
        let body = self.island.body_rect_at(self.cx);
        let canvas_r = self.open_canvas_rect();
        let ai_picker = self.ai_picker_rect();
        let geom = self.island.current();
        let content_alpha = self.island.content_alpha();
        // Clear what the island covered last frame as well as this one.
        let canvas_px = px_rect(canvas_r, scale, 1, sw, sh);
        let full = RECT { left: 0, top: 0, right: sw, bottom: sh };
        let dirty = px_union(self.prev_canvas_px.unwrap_or(full), canvas_px);
        self.prev_canvas_px = Some(canvas_px);
        if std::mem::take(&mut self.capture_next) {
            self.capture_rect = px_rect(canvas_r, scale, 0, sw, sh);
            self.gfx.request_capture(self.capture_rect);
        }
        let mut hits = std::mem::take(&mut self.hits);
        hits.clear();

        // Stepping aside, the island flattens into the top edge and goes
        // invisible once there; coming back, it drops from the edge.
        let hidden_idle = self.rests_hidden() && (self.mode != Mode::Aside || self.island.settled());
        let floating = self.island.floating();
        // Software renderer: what of the floating island's hit area isn't
        // drawn (the gap above it) is painted at 1/255 (see hit_mask_for).
        let float_hit = if floating && !self.expanded { self.hit_mask().rect } else { None };
        // Content is masked to the island silhouette only while the outline
        // or the content moves. At rest every layout fits inside the body,
        // and skipping the mask layer keeps ambient frames cheap.
        let morphing = !self.island.settled() || self.outgoing.is_some();
        let mode = self.mode;
        let transition = self.page_anim.and_then(|(from, t0, dir)| {
            let t = ((now - t0) / PAGE_SWITCH) as f32;
            (t < 1.0).then_some(pages::PageTransition { from, t, dir })
        });
        if transition.is_none() {
            self.page_anim = None;
        }
        // Pages lay out at their final size, centred on the morphing outline,
        // so the header and the page body don't slide with the edges; the
        // outline reveals them.
        let lay = match mode {
            Mode::Expanded(_) => {
                let tgt = self.island.target();
                Rect::new(body.center().x - tgt.w / 2.0, body.y, tgt.w, tgt.h)
            }
            _ => body,
        };
        let draw_from = clock::mono();
        let mut drawn_at = draw_from;
        self.sync_script_capture(self.st.page == PageKind::Script && (self.expanded || !self.island.settled()));
        self.gfx.set_look(lumo_core::look::of(self.st.settings.theme));
        let (repaint, anim, focus, ui_live) = match self.gfx.begin(dirty) {
            Ok(mut canvas) => {
                if self.expanded && !canvas.on_gpu() {
                    // Near-invisible hit margin: a forgiving edge for the open
                    // panel. Collapsed, only the visible shape takes the mouse.
                    // (The GPU renderer keeps this in its hit mask instead.)
                    canvas.fill_rect(self.island.canvas_rect_at(self.cx), Color::black(1.0 / 255.0));
                    if let Some(picker) = ai_picker {
                        canvas.fill_rect(Rect::new(picker.x, body.bottom(), picker.w, picker.bottom() - body.bottom()), Color::black(1.0 / 255.0));
                    }
                } else if let Some(r) = float_hit.filter(|_| !canvas.on_gpu()) {
                    // The gap above the floating pill (or the Hidden style's
                    // band at the top edge) takes the mouse too.
                    canvas.fill_rect(r, Color::black(1.0 / 255.0));
                }
                let island_color = if hidden_idle { Color::black(1.0 / 255.0) } else { palette::ISLAND.mul_alpha(visibility.max(1.0 / 255.0)) };
                let border = if hidden_idle { None } else { Some(canvas.look().border.mul_alpha(visibility)) };
                if floating {
                    // Hidden floats nothing: its band takes the mouse.
                    if !hidden_idle {
                        pill::shape::draw_body(&mut canvas, body, geom.r, island_color, border);
                    }
                } else {
                    canvas.notch(body, geom.r, SHOULDER, island_color, border);
                }


                // Content scales about the top centre, where the island hangs
                // (floating, about the middle of the pill end of the morph).
                let pivot = if floating { pill::motion::pivot(body, self.island.target()) } else { Point::new(body.center().x, body.y) };
                if morphing {
                    canvas.clip(body);
                    if floating {
                        canvas.push_outline_mask(&pill::shape::segments(body, geom.r));
                    } else {
                        canvas.push_shape_mask(body, geom.r, SHOULDER);
                    }
                }

                // Outgoing content: laid out at its own (old) size, centred,
                // shrinking with the outline instead of being sliced, and
                // fading out on a smoothstep (it holds, then leaves). Drawn as
                // one layer, so it shrinks as a picture. Ghost input: no clicks.
                if let Some((m, t0, g, from)) = self.outgoing {
                    if now - t0 >= OUTGOING_FADE {
                        self.outgoing = None;
                    } else {
                        let old_body = Rect::new(body.center().x - g.w / 2.0, body.y, g.w, g.h);
                        let alpha = from * outgoing_left(now - t0) * visibility;
                        let shrink = if floating { pill::motion::ghost_scale(geom, g, GHOST_MIN_SCALE) } else { (geom.w / g.w.max(1.0)).clamp(GHOST_MIN_SCALE, 1.0) };
                        let xf = scale_about(pivot, shrink);
                        let st = &mut self.st;
                        canvas.layered(old_body, alpha, xf, |c| {
                            let mut ghost_input = Input::default();
                            let mut ghost_hits = Vec::new();
                            let mut ghost = Ui::new(c, &mut ghost_input, &mut ghost_hits, now);
                            match m {
                                Mode::Expanded(page) => pages::draw_expanded(&mut ghost, st, old_body, page, None),
                                _ => activities::draw(&mut ghost, st, &m, old_body),
                            }
                        });
                    }
                }

                // Incoming content settles down from the bezel as it fades in,
                // as one layer, so its text doesn't re-hint every frame.
                let mut out = if content_alpha < 1.0 || visibility < 1.0 {
                    let grow = scale_about(pivot, CONTENT_SCALE_FROM + (1.0 - CONTENT_SCALE_FROM) * content_alpha);
                    let drop_by = if floating { pill::motion::CONTENT_DROP } else { CONTENT_DROP };
                    let xf = grow * Matrix3x2::translation(0.0, -drop_by * (1.0 - content_alpha));
                    let (input, st) = (&mut self.input, &mut self.st);
                    canvas.layered(union_rect(lay, body), content_alpha * visibility, xf, |c| draw_content(c, input, &mut hits, st, mode, lay, body, transition, now))
                } else {
                    draw_content(&mut canvas, &mut self.input, &mut hits, &mut self.st, mode, lay, body, transition, now)
                };
                if morphing {
                    canvas.pop_shape_mask();
                    canvas.unclip();
                }
                if let Some(picker) = ai_picker {
                    // This surface sits outside the chat's clip, on the same
                    // renderer and input stream. No second window or worker.
                    let mut ui = Ui::new(&mut canvas, &mut self.input, &mut hits, now);
                    pages::ai::model_picker(&mut ui, &mut self.st, picker, false);
                    out.0 |= ui.repaint_requested();
                    out.1 |= ui.anim_requested();
                    out.2 = ui.focus_request().or(out.2);
                    out.3 |= ui.live_requested();
                }
                drop(canvas);
                if let Err(e) = self.gfx.end() {
                    pwarn!("EndDraw failed: {e}");
                }
                drawn_at = clock::mono();
                out
            }
            Err(e) => {
                pwarn!("begin frame failed: {e}");
                (false, false, None, false)
            }
        };
        self.hits = hits;

        self.input.end_frame();

        // Present. A renderer switch (GPU lost, or back) leaves the other
        // renderer's content in the window's bitmap: push the hit mask in
        // full, and redraw everything on the next frame too.
        let mut renderer_changed = self.gfx.take_renderer_changed();
        let (x, y) = (self.surface.x, self.surface.y);
        if let Err(e) = self.gfx.present(x, y) {
            pwarn!("present failed: {e}");
        }
        renderer_changed |= self.gfx.take_renderer_changed();
        if renderer_changed {
            self.mask = None;
            self.prev_canvas_px = None;
        }
        let presented_at = clock::mono();
        if self.gfx.needs_hit_mask() {
            self.update_hit_mask(now);
        }
        if let (Some(t), true) = (tick, self.motion.is_some() && self.motions_logged < 6 && self.trace.len() < 200) {
            let ms = |a: f64, b: f64| ((b - a) * 1000.0) as f32;
            self.trace.push((t.periods, [ms(t.vblank, draw_from), ms(draw_from, drawn_at), ms(drawn_at, presented_at), ms(presented_at, clock::mono())]));
        }
        if !self.shown {
            self.shown = true;
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.raise_topmost();
            self.gfx.set_shown(true);
        }

        match focus {
            Some(true) => self.focus_text(),
            Some(false) => self.blur_text(),
            None => {}
        }
        self.run_commands(deferred);

        let (popup_moving, popup_live) = self.draw_lyrics_window(deferred);

        // Decide whether to keep the frame clock running. Real motion runs
        // at the display's full refresh rate; ambient motion runs on every
        // n-th refresh (the equalizer near 60 fps, a drifting progress bar
        // near 30) and pauses while the session is locked or the display off.
        let ambient_ok = !self.locked && !self.display_off && !self.st.power.saver;
        let eq_live = ambient_ok && activities::is_live(&self.mode, &self.st);
        let drift_live = ambient_ok && ui_live;
        let moving = !self.island.settled()
            || self.ai_picker_room != (self.expanded && self.st.page == PageKind::Ai && pages::ai::picker_below(&self.st))
            || self.aside_visibility != aside_target
            // Commands run after drawing; keep the clock awake for a page
            // chosen by a tab click even when the old page was at rest.
            || self.target_mode() != self.mode
            || anim
            || repaint
            || renderer_changed
            || !self.st.cmds.is_empty()
            || self.outgoing.is_some()
            || self.page_anim.is_some()
            || popup_moving;
        // The display mode's rate, not the measured period: a divisor that
        // flips between 1 and 2 would be the uneven cadence it exists to avoid.
        let hz = self.clock.nominal_hz();
        self.clock.set_divisor(if moving {
            1
        } else if eq_live {
            ambient_divisor(hz, 60.0)
        } else {
            ambient_divisor(hz, 30.0)
        });
        self.clock.set_active(moving || eq_live || drift_live || popup_live);
        self.schedule_wake();
        if !self.log_queue.is_empty() {
            let lines = std::mem::take(&mut self.log_queue);
            std::thread::spawn(move || lines.iter().for_each(|l| crate::plog::write("INFO", l)));
        }
    }

    /// GPU renderer: keep the window's invisible hit mask on the island.
    /// While the outline moves it follows at [`MASK_INTERVAL`]; at rest it
    /// matches exactly.
    fn update_hit_mask(&mut self, now: f64) {
        let mask = self.hit_mask();
        let due = match &self.mask {
            None => true,
            Some((m, at)) => *m != mask && (self.island.settled() || now - at >= MASK_INTERVAL),
        };
        if !due {
            return;
        }
        let prev = self.mask.map(|(m, _)| m);
        match self.gfx.push_hit_mask(self.surface.x, self.surface.y, &mask, prev.as_ref()) {
            Ok(()) => self.mask = Some((mask, now)),
            Err(e) => {
                pwarn!("hit mask update failed: {e}");
                self.mask = None;
            }
        }
    }

    /// Track how smoothly the outline moves. The first morphs after start
    /// are logged (the CI smoke test reads them); after that only morphs
    /// that dropped frames, at most one a minute.
    fn note_motion(&mut self, retarget: bool, t: f64, period: f64, dt: f64) {
        let moving = !self.island.settled();
        if retarget || !moving {
            if let Some(mut m) = self.motion.take() {
                if !moving {
                    m.frame(t);
                }
                let early = self.motions_logged < 20;
                if m.frames >= 4 && (early || (m.hitches > 0 && t - self.hitch_logged_at > 60.0)) {
                    let dwm = match (self.dwm_at_start, dwm_counters()) {
                        (Some(a), Some(b)) => format!(", dwm {} refreshes / {} composed / {} missed", b.0.wrapping_sub(a.0), b.1.wrapping_sub(a.1), b.2.wrapping_sub(a.2)),
                        _ => String::new(),
                    };
                    self.log_queue.push(format!("{} ({}{dwm})", m.summary(), self.renderer_name()));
                    if self.motions_logged < 6 && !self.trace.is_empty() {
                        let col = |i: usize| self.trace.iter().map(|f| format!("{:.1}", f.1[i])).collect::<Vec<_>>().join(" ");
                        let periods: Vec<String> = self.trace.iter().map(|f| f.0.to_string()).collect();
                        self.log_queue.push(format!("  vblanks {}; late ms {}; draw ms {}; present ms {}; mask ms {}", periods.join(" "), col(0), col(1), col(2), col(3)));
                    }
                    self.motions_logged += 1;
                    if !early {
                        self.hitch_logged_at = t;
                    }
                }
            }
        }
        if moving {
            match self.motion.as_mut() {
                Some(m) => m.frame(t),
                None => {
                    self.motion = Some(MotionStats::begin(self.motion_name, t, period, dt));
                    self.trace.clear();
                    self.dwm_at_start = dwm_counters();
                }
            }
        }
    }

    fn renderer_name(&self) -> &'static str {
        match self.gfx.renderer() {
            crate::gfx::Renderer::Gpu { warp: false } => "gpu",
            crate::gfx::Renderer::Gpu { warp: true } => "gpu-warp",
            crate::gfx::Renderer::Software => "software",
        }
    }

    /// Wake up for the next visible change (HUD expiry, countdown second, …).
    fn schedule_wake(&mut self) {
        let now = self.st.now;
        let mut next: Option<f64> = None;
        let mut consider = |t: f64| {
            if t > now {
                next = Some(next.map_or(t, |n: f64| n.min(t)));
            }
        };
        if let Some(h) = &self.st.hud {
            consider(h.until);
        }
        if let Some(n) = &self.st.notice {
            if !n.is_ai_reply(){consider(n.until);}
        }
        // While hidden for a full-screen app only the phase end matters.
        let visible = !self.fullscreen_hidden;
        if let Some(b) = self.st.pomodoro.next_second_boundary(now).filter(|_| visible && !self.st.notice.as_ref().is_some_and(|n|n.is_ai_reply())) {
            consider(now + b + 0.005);
        }
        if let lumo_core::pomodoro::RunState::Running { ends_at } = self.st.pomodoro.run {
            consider(ends_at + 0.005);
        }
        // The music pill wakes for every sung line (in every theme), and Pill's
        // pills for their own changes; nobody sees those while the session is
        // locked or the display is off, so they rest then (the lock and
        // display events ask for a frame, which picks them up again).
        let singing = self.mode == Mode::Activity(activities::Activity::Media);
        let watched = visible && !((pill::on(self.st.settings.theme) || singing) && (self.locked || self.display_off));
        if let Some(t) = activities::next_wake(&self.mode, &self.st).filter(|_| watched) {
            consider(t);
        }
        if let Mode::Expanded(p) = self.mode {
            let away = self.locked || self.display_off;
            if let Some(t) = pages::next_wake(p, &self.st).filter(|_| visible && !away) {
                consider(t + 0.005);
            }
        }
        if !self.locked && !self.display_off && !self.widgets_open {
            if let Some(t) = self.lyrics_window.as_ref().and_then(|window| window.next_wake(&self.st)) { consider(t); }
        }
        if let Some(t) = next {
            let ms = (((t - now) * 1000.0).ceil() as u32).clamp(10, 3_600_000);
            unsafe {
                SetTimer(self.hwnd, TIMER_WAKE, ms, None);
            }
        }
    }

    fn schedule_minute_timer(&mut self) {
        let t = clock::local_now();
        let ms = (60 - t.second as u32).max(1) * 1000 + 20;
        unsafe {
            SetTimer(self.hwnd, TIMER_MINUTE, ms, None);
        }
    }

    // ------------------------------------------------------------ new versions

    /// Arm the update file's timer for the next read, or stop it while the
    /// checks are off or a read runs.
    fn schedule_update(&mut self) {
        let secs = self.updater.as_ref().filter(|_| !self.locked && !self.display_off && !self.st.power.saver).and_then(|u| u.due_in(&self.st, crate::updates::unix_now()));
        unsafe {
            match secs {
                // SetTimer waits at most about 24 days; reads are 12 hours apart.
                Some(s) => {
                    SetTimer(self.hwnd, TIMER_UPDATE, s.saturating_mul(1000).clamp(1000, 0x7FFF_FFFF) as u32, None);
                }
                None => {
                    let _ = KillTimer(self.hwnd, TIMER_UPDATE);
                }
            }
        }
    }

    /// The timer fired, or someone came back to the PC: read the update file
    /// if a read is due and someone is here, not on battery saver. Otherwise
    /// it waits; the lock, display and power events call this again.
    fn update_due(&mut self) {
        let now = crate::updates::unix_now();
        let away = self.locked || self.display_off || self.st.power.saver;
        match self.updater.as_ref().and_then(|u| u.due_in(&self.st, now)) {
            Some(0) if away => self.schedule_update(),
            Some(0) => self.check_updates(),
            _ => self.schedule_update(),
        }
        self.fetch_setup();
    }

    fn check_updates(&mut self) {
        if self.st.update.managed { return; }
        if !self.updater.as_mut().is_some_and(|u| u.begin(&mut self.st)) { return; }
        let job = services::update::check_job(self.sink.clone());
        if !self.services.as_ref().is_some_and(|s| s.run_job(job)) {
            if let Some(u) = self.updater.as_mut() {
                u.checked(&mut self.st, Err("Couldn't start the update check. Try again.".into()), crate::updates::unix_now());
            }
        }
        self.schedule_update();
        if let Some(window) = self.settings_window.as_ref() { window.invalidate(); }
    }

    /// The offered version's Setup, once per release, on the add-on thread
    /// (after a look at the file, and at start for one already offered).
    fn fetch_setup(&mut self) {
        self.download_update(false);
    }

    fn download_update(&mut self, manual: bool) {
        if self.st.update.managed || (!manual && (self.locked || self.display_off || self.st.power.saver)) { return; }
        if matches!(self.st.update.status, crate::state::UpdateStatus::Checking) { return; }
        let Some((d, path)) = self.updater.as_mut().and_then(|u| u.want_setup(&mut self.st)) else { return };
        let job = services::update::download_job(self.sink.clone(), d, path);
        if !self.services.as_ref().is_some_and(|s| s.run_job(job)) {
            if let Some(u) = self.updater.as_mut() {
                u.fetched(&mut self.st, Err("the add-on thread isn't running".into()));
            }
        }
        self.schedule_update();
        if let Some(window) = self.settings_window.as_ref() { window.invalidate(); }
    }

    /// About's update action downloads first, then installs and restarts on
    /// the next explicit click. A failed install stays here with a retry.
    fn update_now(&mut self) {
        if self.st.update.managed || matches!(self.st.update.status, crate::state::UpdateStatus::Checking | crate::state::UpdateStatus::Downloading | crate::state::UpdateStatus::Installing) { return; }
        if let Some((setup, d)) = self.updater.as_ref().and_then(|u| u.ready_setup(&self.st)) {
            self.st.update.status = crate::state::UpdateStatus::Installing;
            match services::update::install(&setup, &d) {
                Ok(()) => {
                    pinfo!("updates: installing {}", d.file_name());
                    self.quit(0);
                    return;
                }
                Err(e) => {
                    if let Some(u) = self.updater.as_mut() { u.installation_failed(&mut self.st, format!("Couldn't install the update: {e}")); }
                    if let Some(window) = self.settings_window.as_ref() { window.invalidate(); }
                    return;
                }
            }
        }
        if let Some(u) = self.updater.as_mut() { u.request_download(&mut self.st); }
        self.download_update(true);
    }

    /// Run a notice's inline action in place (the island stays collapsed),
    /// then show a check mark in the pill briefly before the notice leaves.
    fn run_inline_action(&mut self, a: InlineAction) {
        self.refresh_time();
        if a == InlineAction::Hide {
            if self.st.notice.as_ref().is_some_and(|n|n.is_ai_reply()) {
                crate::ai::clear_ready(&mut self.st);self.apply_fullscreen(self.st.now);return;
            }
            // Gone at once, the way a notice leaves when its time is up.
            if let Some(n) = self.st.notice.as_mut() {
                n.until = self.st.now;
            }
            return;
        }
        if a == InlineAction::WhatsNew && self.st.notice.as_ref().is_some_and(|n| n.icon != Icon::Check) {
            self.st.open_update();
        }
        let st = &mut self.st;
        // Already done: the check mark is showing.
        if st.notice.as_ref().is_none_or(|n| n.icon == Icon::Check) {
            return;
        }
        match a {
            InlineAction::StartFocus => st.pomodoro.start(st.now),
            InlineAction::Hide | InlineAction::WhatsNew => {}
        }
        let until = st.now + INLINE_DONE_S;
        if let Some(n) = st.notice.as_mut() {
            n.icon = Icon::Check;
            n.until = until;
        }
        pinfo!("notice: inline action {a:?} done");
    }

    fn open_lyrics_window(&mut self) {
        if let Some(window) = self.lyrics_window.as_mut() { window.focus(); self.request_frame(); return; }
        if !crate::lyrics_window::available(&self.st) { return; }
        match crate::lyrics_window::LyricsWindow::new(self.hwnd, &self.st) {
            Ok(window) => {
                self.collapse();
                self.lyrics_window = Some(window);
                self.st.lyrics_popup_open = true;
                self.cancel_hover();
                self.raises_left = 0;
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_RAISE);
                    let _ = KillTimer(self.hwnd, TIMER_COLLAPSE);
                }
                self.request_frame();
            }
            Err(error) => pwarn!("lyrics window: {error}"),
        }
    }

    fn finish_lyrics_window(&mut self) {
        self.lyrics_window = None;
        self.st.lyrics_popup_open = false;
        self.clock.set_monitor(self.monitor.handle);
        self.st.page = PageKind::Media;
        self.page_anim = None;
        self.expand();
        // Returning from the top-right popup gives time to reach the island.
        if !self.st.pinned { unsafe { SetTimer(self.hwnd, TIMER_COLLAPSE, 1800, None); } }
    }

    fn draw_lyrics_window(&mut self, deferred: &mut Vec<Deferred>) -> (bool, bool) {
        let away = self.locked || self.display_off || self.widgets_open;
        let animate = !away && !self.st.power.saver;
        let mut closed = false;
        let (mut moving, mut live) = (false, false);
        if let Some(window) = self.lyrics_window.as_mut() {
            if away { window.hide(); }
            else {
                match window.frame(&mut self.st, animate) {
                    Ok(frame) => { moving = frame.moving; live = frame.live; closed = frame.closed; }
                    Err(error) => { pwarn!("lyrics draw: {error}"); closed = true; }
                }
            }
        }
        // Popup input uses the same media/settings command queue as the island.
        if !self.st.cmds.is_empty() { self.run_commands(deferred); }
        if self.lyrics_window.as_ref().is_some_and(|window| window.is_closing()) {
            if animate { moving = true; } else { closed = true; }
        }
        if closed {
            self.finish_lyrics_window();
            moving = !self.fullscreen_hidden;
        }
        (moving && !away, live && animate)
    }

    pub fn handle_lyrics_window(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if !self.lyrics_window.as_ref().is_some_and(|window| window.hwnd == hwnd) { return None; }
        if msg == WM_CLOSE {
            self.lyrics_window.as_mut()?.close(clock::mono());
            self.request_frame();
            return Some(LRESULT(0));
        }
        if msg == WM_PAINT {
            unsafe { let _ = windows::Win32::Graphics::Gdi::ValidateRect(hwnd, None); }
            self.request_frame();
            return Some(LRESULT(0));
        }
        let result = self.lyrics_window.as_mut()?.message(&mut self.st, msg, wp, lp);
        if result.is_some() && matches!(msg, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSELEAVE | WM_CAPTURECHANGED | WM_MOUSEWHEEL | WM_KEYDOWN | WM_SIZE | WM_DPICHANGED | WM_EXITSIZEMOVE) {
            self.request_frame();
        }
        result
    }

    fn stop_ai_snip(&mut self) {
        unsafe {let _=windows::Win32::System::DataExchange::RemoveClipboardFormatListener(self.hwnd);let _=KillTimer(self.hwnd,TIMER_AI_SNIP);}
    }
    fn open_ai_window(&mut self) {
        if self.ai_window.is_none(){match crate::ai_window::AiWindow::new(){Ok(w)=>self.ai_window=Some(w),Err(e)=>{self.st.ai.error=format!("Couldn't open full chat: {e}");return;}}}
        self.st.ai.full_open=true;if let Some(w)=self.ai_window.as_ref(){w.show();}self.collapse();
    }
    pub fn handle_ai_window(&mut self,hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->Option<LRESULT> {
        if !self.ai_window.as_ref().is_some_and(|w|w.hwnd==hwnd){return None;}
        if (msg==WM_PASTE || (msg==WM_KEYDOWN && wp.0==VK_V.0 as usize && unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0)) && crate::ai::paste_image(&mut self.st,self.ai_window.as_ref().is_some_and(|w|w.prompt_focused()),services::ai::clipboard_has_image) {
            self.request_frame();return Some(LRESULT(0));
        }
        if msg==WM_KEYDOWN && wp.0==VK_ESCAPE.0 as usize && (self.st.ai.model_menu || self.st.ai.reasoning_menu || self.st.ai.attach_menu) {
            self.st.ai.close_popovers();if let Some(w)=self.ai_window.as_mut(){w.focus_prompt(self.st.ai.draft.chars().count());}return Some(LRESULT(0));
        }
        if msg==WM_CLOSE {
            self.ai_window=None;self.st.ai.full_open=false;self.set_page(PageKind::Ai);self.ai_reply_peek=true;self.expand();
            // The pointer is still by the bottom popup, far from the island.
            // Wait for it to return instead of immediately auto-closing.
            self.ai_resize_hold=Some(HWND::default());self.collapse_due=false;
            unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}
            self.apply_fullscreen(self.st.now);self.request_frame();return Some(LRESULT(0));
        }
        if msg==WM_PAINT {unsafe{let _=windows::Win32::Graphics::Gdi::ValidateRect(hwnd,None);}self.st.now=clock::mono();if !self.locked && !self.display_off && !unsafe{IsIconic(hwnd)}.as_bool() {if let Some(w)=self.ai_window.as_mut(){if let Err(e)=w.draw(&mut self.st){pwarn!("AI window: {e}");}}let mut deferred=vec![];self.run_commands(&mut deferred);self.request_frame();}return Some(LRESULT(0));}
        self.ai_window.as_mut()?.message(msg,wp,lp)
    }

    fn open_settings(&mut self) {
        if self.settings_window.is_none() {
            match crate::settings_window::SettingsWindow::new(self.hwnd) {
                Ok(window) => self.settings_window = Some(window),
                Err(e) => {
                    pwarn!("settings window: {e}");
                    self.st.notify(Icon::Gear, palette::ORANGE, "Settings couldn't open", "Please try again", 4.0, None);
                    return;
                }
            }
            if self.selftest.is_none() { crate::autostart::report(self.sink.clone(), None); }
        }
        if let Some(window) = self.settings_window.as_mut() { window.show(); }
        if self.settings_in_front() {
            self.raises_left = 0;
            unsafe { let _ = KillTimer(self.hwnd, TIMER_RAISE); }
        }
    }

    pub fn handle_settings(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if !self.settings_window.as_ref().is_some_and(|w| w.hwnd == hwnd) { return None; }
        if msg==WM_KEYDOWN && wp.0==VK_ESCAPE.0 as usize && self.st.ai.model_menu && self.st.ai.model_settings {
            self.st.ai.close_models();if let Some(w)=self.settings_window.as_mut(){w.dismiss_model_picker();}return Some(LRESULT(0));
        }
        if msg == WM_CLOSE {
            self.settings_window = None;
            self.st.cmd(Command::SaveSettings);
            let mut deferred=vec![];self.run_commands(&mut deferred);
            return Some(LRESULT(0));
        }
        if msg == WM_PAINT {
            unsafe { let _ = windows::Win32::Graphics::Gdi::ValidateRect(hwnd, None); }
            if !self.locked && !self.display_off && !unsafe { IsIconic(hwnd) }.as_bool() {
                if let Some(window) = self.settings_window.as_mut() {
                    self.st.now=clock::mono();
                    if let Err(e) = window.draw(&mut self.st) { pwarn!("settings draw: {e}"); }
                }
                let mut deferred = Vec::new();
                self.run_commands(&mut deferred);
                self.request_frame();
            }
            return Some(LRESULT(0));
        }
        self.settings_window.as_mut()?.message(msg, wp, lp)
    }

    fn set_page(&mut self, p: PageKind) {
        if p != self.st.page { self.reopen.selected(); }
        self.apply_page(p, true);
    }

    fn apply_page(&mut self, p: PageKind, remember: bool) {
        if p==PageKind::Ai {crate::ai::ensure(&mut self.st);}
        if p != self.st.page {
            self.st.ai.close_popovers();
            self.ai_resize_hold=None;
            if p!=PageKind::Ai {self.ai_reply_peek=false;}
            self.st.page = p;
            // Settings is never where the island opens next time.
            if p != PageKind::Settings {
                self.st.update.sheet = false;
                if remember {
                    self.st.store.last_page = p.key().to_string();
                    unsafe {
                        SetTimer(self.hwnd, TIMER_SAVE, 800, None);
                    }
                }
            } else {
                crate::autostart::report(self.sink.clone(), None);
            }
            self.set_sysmon(p == PageKind::System && self.expanded);
            if self.text_focus {
                self.blur_text();
            }
        }
    }

    fn run_commands(&mut self, deferred: &mut Vec<Deferred>) {
        let cmds = std::mem::take(&mut self.st.cmds);
        for c in cmds {
            match c {
                Command::AiHeight(height) => {let dragging=self.input.pressed_id==Some(wid("ai.resize-grip"));self.resize_ai_chat(height,dragging);},
                Command::OpenAiWindow => self.open_ai_window(),
                Command::Ai(crate::ai::Action::Snip) => {
                    self.stop_ai_snip();
                    if !self.locked && !self.display_off {
                        self.st.ai.snipping=true;unsafe{let _=windows::Win32::System::DataExchange::AddClipboardFormatListener(self.hwnd);SetTimer(self.hwnd,TIMER_AI_SNIP,60000,None);}
                        self.collapse();if let Some(w)=self.ai_window.as_ref(){unsafe{let _=ShowWindow(w.hwnd,SW_MINIMIZE);}}shell::open("ms-screenclip:");
                    }
                }
                Command::Ai(a) => {
                    let owner=self.ai_window.as_ref().map(|w|w.hwnd).or_else(||self.settings_window.as_ref().map(|w|w.hwnd)).unwrap_or(self.hwnd);
                    services::ai::run(&mut self.st,a,SendHwnd::new(owner),self.sink.clone());
                }
                Command::OpenSettings => {self.open_settings();if self.st.page==PageKind::Ai {if let Some(w)=self.settings_window.as_mut(){w.show_ai();}}},
                Command::OpenUpdateSettings => { self.open_settings(); if let Some(window) = self.settings_window.as_mut() { window.show_about(); } },
                Command::CheckUpdates => self.check_updates(),
                Command::OpenLyricsWindow => self.open_lyrics_window(),
                Command::CloseLyricsWindow => {
                    if let Some(window) = self.lyrics_window.as_mut() { window.close(self.st.now); self.request_frame(); }
                }
                Command::Media(m) => {
                    if matches!(m, MediaCmd::Next | MediaCmd::Prev) {
                        self.st.media_pending_seek = None;
                    }
                    if let Some(s) = self.services.as_ref() {
                        s.media_cmd(m.clone());
                    }
                    // Optimistic UI for play/pause.
                    if m == MediaCmd::PlayPause {
                        if let Some(ms) = self.st.media.as_mut() {
                            let now = self.st.now;
                            let pos = ms.timeline.position_at(now);
                            ms.timeline.position_s = pos;
                            ms.timeline.sampled_at = now;
                            ms.timeline.playing = !ms.timeline.playing;
                            // The player's confirmation will match this, so
                            // start (or end) the pause grace period here.
                            self.st.media_paused_at = (!ms.timeline.playing).then_some(now);
                        }
                    }
                }
                Command::ChooseMusicDirectory => self.choose_music_path(crate::commands::MusicPick::Directory),
                Command::ChooseLyricsDownloadDirectory => self.choose_music_path(crate::commands::MusicPick::LyricsDirectory),
                Command::OpenPath(p) => shell::open(&p),
                Command::RevealPath(p) => shell::reveal(&p),
                Command::DragOut(paths) => deferred.push(Deferred::DragOut(paths)),
                Command::LoadThumb(p) => {
                    if !self.st.shelf_thumbs.contains_key(&p) {
                        self.st.shelf_thumbs.insert(p.clone(), None);
                        services::imaging::load_thumb_async(p, self.sink.clone());
                    }
                }
                Command::CopyText(t) => {
                    self.own_clip = Some(t.clone());
                    services::clipboard::set_text(self.helper, &t);
                }
                Command::FocusText => {
                    let foreground=unsafe{GetForegroundWindow()};
                    if !self.ai_window.as_ref().is_some_and(|w|w.hwnd==foreground) && !self.settings_window.as_ref().is_some_and(|w|w.hwnd==foreground){self.focus_text();}
                },
                Command::BlurText => self.blur_text(),
                Command::RecordScreen => services::recorder::record_screen(self.hwnd),
                Command::ScriptPaste => {
                    // Read here, not through the text field path: that one drops line breaks.
                    let text = services::clipboard::read_text(self.hwnd).map(|t| lumo_core::script::clean(&t)).filter(|t| !t.is_empty());
                    match text {
                        Some(text) => pages::script::set_text(&mut self.st, text),
                        None => pages::script::note(&mut self.st, "Copy your script first"),
                    }
                }
                Command::SaveSettings => {
                    let before = self.saved_settings.clone();
                    self.settings_changed(&before);
                    self.save_settings();
                }
                Command::SaveStore => unsafe {
                    SetTimer(self.hwnd, TIMER_SAVE, 800, None);
                },
                Command::SetPage(p) => self.set_page(p),
                Command::CheckFps => self.check_fps(),
                Command::AllowFps => {
                    // The island takes the focus for Windows' prompt (a
                    // background app's prompt waits in the taskbar), as for typing.
                    self.focus_text();
                    self.st.game.access = FpsAccess::Asking;
                    services::frames::request_access(self.sink.clone(), SendHwnd::new(self.hwnd));
                }
                Command::LoadAppIcon(path) => {
                    if !self.st.app_icons.contains_key(&path) {
                        self.st.app_icons.insert(path.clone(), None);
                        self.icon_wait.insert(path.clone());
                        services::imaging::load_thumb_async(path, self.sink.clone());
                    }
                }
                Command::SetPinned(b) => {
                    self.st.pinned = b;
                    if !b {
                        self.maybe_schedule_collapse();
                    }
                }
                Command::Collapse => self.collapse(),
                Command::Play(s) => services::sound::play(s),
                Command::OpenUrl(u) => shell::open(&u),
                Command::Update => self.update_now(),
                Command::SetAutostart(on) => {
                    self.st.ui.autostart = Some(on);
                    crate::autostart::report(self.sink.clone(), Some(on));
                }
                Command::Phone(c) => self.phone_cmd(c),
                Command::Pair(c) => self.pair_cmd(c),
            }
        }
        self.sync_phone_offer();
        self.sync_pair_offer();
    }

    // ------------------------------------------------------------ phone drop

    /// The QR code waits for a phone, or a phone waits for an answer (Shelf
    /// page only), or a reading is under way (Script page only).
    fn phone_holds(&self) -> bool {
        self.st.page_holds_island()
    }

    fn phone_cmd(&mut self, c: PhoneCmd) {
        match c {
            PhoneCmd::Open => self.open_phone(),
            PhoneCmd::Close => self.close_phone(),
            PhoneCmd::Answer { id, accept } => {
                if let Some(svc) = &self.phone {
                    svc.answer(id, accept);
                }
                let until = self.st.now + pages::shelf::PHONE_OUTCOME_S;
                if let Some(pd) = self.st.phone.as_mut() {
                    if let Some(a) = pd.ask.take_if(|a| a.id == id) {
                        if accept {
                            pd.transfer = Some(PhoneTransfer { id, from: a.from, file: 0, files: a.files.len(), done: 0, total: a.total });
                        } else {
                            pd.outcome = Some((format!("Declined {} from {}", files_label(a.files.len()), a.from), false, until));
                        }
                    }
                }
                self.maybe_schedule_collapse();
            }
        }
    }

    fn open_phone(&mut self) {
        if let Some(old) = self.phone.take() {
            old.stop();
        }
        let mut pd = PhoneDrop::default();
        if self.selftest.is_some() {
            pd.error = Some("Phone drop is off in the self-test.".into());
        } else {
            let offer = self.st.store.shelf.paths();
            match services::phonedrop::PhoneDrop::start(self.sink.clone(), offer.iter().map(PathBuf::from).collect()) {
                Ok((svc, url)) => {
                    pd.qr = QrMatrix::encode(&url);
                    pd.url = url;
                    self.phone_offer = offer;
                    self.phone = Some(svc);
                }
                Err(e) => {
                    pwarn!("phone drop: {e}");
                    pd.error = Some(e);
                }
            }
        }
        self.st.phone = Some(pd);
        self.st.page = PageKind::Shelf;
        self.expand();
    }

    fn close_phone(&mut self) {
        if let Some(svc) = self.phone.take() {
            svc.stop();
        }
        if matches!(self.st.phone.as_ref().map(|p| &p.view), Some(PhoneView::Pairing { .. })) {
            if let Some(s) = &self.services {
                s.pair_cancel();
            }
        }
        self.st.phone = None;
        self.phone_offer.clear();
        self.maybe_schedule_collapse();
        self.request_frame();
    }

    /// Minute tick: nobody has used the link for a while, so close it.
    fn close_idle_phone(&mut self) {
        let idle = self.phone.as_ref().is_some_and(|svc| !svc.busy() && svc.idle_secs() >= PHONE_IDLE_S);
        let quiet = self.st.phone.as_ref().is_some_and(|p| p.ask.is_none() && p.transfer.is_none());
        if idle && quiet {
            pinfo!("phone drop: closed after {:.0} minutes without a phone", PHONE_IDLE_S / 60.0);
            self.close_phone();
        }
    }

    /// Keep the files a phone can save in step with the Shelf.
    fn sync_phone_offer(&mut self) {
        let Some(svc) = &self.phone else { return };
        let paths = self.st.store.shelf.paths();
        if paths != self.phone_offer {
            svc.set_offer(paths.iter().map(PathBuf::from).collect());
            self.phone_offer = paths;
        }
    }

    fn on_phone(&mut self, e: PhoneEvent) {
        let now = self.st.now;
        let until = now + pages::shelf::PHONE_OUTCOME_S;
        let Some(pd) = self.st.phone.as_mut() else { return };
        match e {
            PhoneEvent::Visit { from } => {
                pd.visitor = Some(from);
                self.maybe_schedule_collapse();
            }
            PhoneEvent::Ask(a) => {
                pd.outcome = None;
                pd.visitor.get_or_insert_with(|| a.from.clone());
                let files: Vec<(String, u64)> = a.files.iter().map(|f| (f.name.clone(), f.size)).collect();
                pinfo!("phone drop: {} asks to send {}", a.from, files_label(files.len()));
                pd.ask = Some(PhoneAsk { id: a.id, from: a.from.clone(), total: a.total(), files });
                self.st.page = PageKind::Shelf;
                self.expand();
                self.st.chime();
            }
            PhoneEvent::AskEnded { id } => {
                if let Some(a) = pd.ask.take_if(|a| a.id == id) {
                    pd.outcome = Some((format!("{} stopped waiting", a.from), false, until));
                }
                self.maybe_schedule_collapse();
            }
            PhoneEvent::Progress { id, file, files, done, total } => match pd.transfer.as_mut() {
                Some(t) if t.id == id => {
                    t.file = file;
                    t.files = files;
                    t.done = done;
                    t.total = total;
                }
                _ => {
                    let from = pd.visitor.clone().unwrap_or_else(|| "Phone".into());
                    pd.transfer = Some(PhoneTransfer { id, from, file, files, done, total });
                }
            },
            PhoneEvent::Received { from, paths, .. } => {
                pd.transfer = None;
                let n = paths.len();
                let title = format!("{} from {from} {} on the Shelf", files_label(n), if n == 1 { "is" } else { "are" });
                pd.outcome = Some((title.clone(), true, until));
                let added: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                self.st.store.shelf.add_paths(&added, self.st.unix);
                for p in added {
                    self.st.cmds.push(Command::LoadThumb(p));
                }
                self.st.save();
                pinfo!("phone drop: {} from {from}", files_label(n));
                if !self.expanded {
                    self.st.notify(Icon::Phone, palette::GREEN, title, "Open the Shelf to see them", 6.0, Some(NoticeAction::OpenPage(PageKind::Shelf)));
                }
                self.st.chime();
                self.maybe_schedule_collapse();
            }
            PhoneEvent::Failed { from, reason, detail, .. } => {
                pd.transfer = None;
                pd.outcome = Some((format!("Nothing kept from {from}. {reason}."), false, until));
                pwarn!("phone drop: transfer from {from} failed: {reason} {detail}");
                self.maybe_schedule_collapse();
            }
        }
    }

    // ------------------------------------------------------------ paired phones

    fn pair_cmd(&mut self, c: PairCmd) {
        let now = self.st.now;
        match c {
            PairCmd::Begin => {
                let r = match &self.services {
                    Some(s) => s.pair_begin(),
                    None => Err("Pairing is off in the self-test.".into()),
                };
                let Some(pd) = self.st.phone.as_mut() else { return };
                pd.outcome = None;
                match r {
                    Ok(url) => {
                        pinfo!("pair drop: showing a pairing code");
                        pd.view = PhoneView::Pairing { qr: QrMatrix::encode(&url), url, until: now + lumo_core::pairdrop::meet::PAIRING_FOR.as_secs_f64() };
                    }
                    Err(e) => {
                        pwarn!("pair drop: {e}");
                        pd.outcome = Some((e, false, now + pages::shelf::PHONE_OUTCOME_S));
                    }
                }
            }
            PairCmd::Back => {
                if matches!(self.st.phone.as_ref().map(|p| &p.view), Some(PhoneView::Pairing { .. })) {
                    if let Some(s) = &self.services {
                        s.pair_cancel();
                    }
                }
                if let Some(pd) = self.st.phone.as_mut() {
                    pd.view = PhoneView::Lan;
                }
            }
            PairCmd::List => {
                if matches!(self.st.phone.as_ref().map(|p| &p.view), Some(PhoneView::Pairing { .. })) {
                    if let Some(s) = &self.services {
                        s.pair_cancel();
                    }
                }
                if let Some(pd) = self.st.phone.as_mut() {
                    pd.view = PhoneView::Paired;
                }
            }
            PairCmd::Remove(room) => {
                if let Some(s) = &self.services {
                    s.pair_remove(&room);
                }
                if let Some(i) = self.st.paired.iter().position(|p| p.room == room) {
                    let p = self.st.paired.remove(i);
                    pinfo!("pair drop: removed a paired phone");
                    if let Some(pd) = self.st.phone.as_mut() {
                        pd.outcome = Some((format!("{} can't send to this PC any more", p.name), false, now + pages::shelf::PHONE_OUTCOME_S));
                        pd.outcome_line = Some("Pair it again any time from Pair a phone.".into());
                    }
                }
            }
        }
        self.request_frame();
    }

    /// Keep the files paired phones may save in step with the Shelf.
    fn sync_pair_offer(&mut self) {
        let Some(svc) = &self.services else { return };
        let paths = self.st.store.shelf.paths();
        if self.pair_offer.as_ref() != Some(&paths) {
            svc.pair_shelf(paths.iter().map(PathBuf::from).collect());
            self.pair_offer = Some(paths);
        }
    }

    fn on_paired(&mut self, e: MeetEvent) {
        let now = self.st.now;
        let until = now + pages::shelf::PHONE_OUTCOME_S;
        match e {
            MeetEvent::Pairs(list) => self.st.paired = paired_phones(list),
            MeetEvent::Server { reachable } => {
                if self.st.pair_server.is_some() || !reachable {
                    pinfo!("pair drop: meeting server {}", if reachable { "reachable again" } else { "unreachable" });
                }
                self.st.pair_server = Some(reachable);
            }
            MeetEvent::Paired { name, .. } => {
                pinfo!("pair drop: a phone finished pairing");
                if let Some(pd) = self.st.phone.as_mut() {
                    pd.view = PhoneView::Lan;
                    pd.outcome = Some((format!("{name} is paired"), true, until));
                    pd.outcome_line = Some("It can now send from any network, even with this panel closed.".into());
                }
                if !self.hovering {
                    // The island closes now that nothing waits: say it on the pill.
                    self.st.notify(Icon::Phone, palette::GREEN, format!("{name} is paired"), "It can send from any network now", 6.0, Some(NoticeAction::OpenPage(PageKind::Shelf)));
                }
                self.st.chime();
                self.maybe_schedule_collapse();
            }
            MeetEvent::Phone { phone, ev, .. } => match ev {
                PeerEvent::Hello { .. } => pinfo!("pair drop: a paired phone connected"),
                PeerEvent::Progress { files, done: 0, .. } => {
                    if !self.expanded {
                        self.st.notify(Icon::Phone, palette::GREEN, format!("{phone} is sending {}", files_label(files)), "They go straight onto the Shelf", 4.0, None);
                    }
                }
                PeerEvent::Progress { .. } => {}
                PeerEvent::Received { paths } => {
                    let n = paths.len();
                    let title = format!("{} from {phone} {} on the Shelf", files_label(n), if n == 1 { "is" } else { "are" });
                    let added: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                    self.st.store.shelf.add_paths(&added, self.st.unix);
                    for p in added {
                        self.st.cmds.push(Command::LoadThumb(p));
                    }
                    self.st.save();
                    pinfo!("pair drop: {} from a paired phone", files_label(n));
                    if let Some(pd) = self.st.phone.as_mut() {
                        pd.outcome = Some((title.clone(), true, until));
                        pd.outcome_line = None;
                    }
                    if !self.expanded {
                        self.st.notify(Icon::Phone, palette::GREEN, title, "Open the Shelf to see them", 6.0, Some(NoticeAction::OpenPage(PageKind::Shelf)));
                    }
                    self.st.chime();
                    self.maybe_schedule_collapse();
                }
                PeerEvent::Failed { reason, detail } => {
                    pwarn!("pair drop: a send from a paired phone failed: {reason} {detail}");
                    if !self.expanded {
                        self.st.notify(Icon::Phone, palette::TEXT, format!("Nothing kept from {phone}"), format!("{reason}."), 6.0, None);
                    } else if let Some(pd) = self.st.phone.as_mut() {
                        pd.outcome = Some((format!("Nothing kept from {phone}. {reason}."), false, until));
                        pd.outcome_line = None;
                    }
                }
                PeerEvent::Ended { .. } => {}
            },
        }
        self.request_frame();
    }

    fn save_now(&mut self) {
        if !self.store_writable {
            pwarn!("data.json couldn't be read at start; not overwriting it");
            return;
        }
        if let Err(e) = save_json_atomic(&paths::store_path(), &self.st.store) {
            pwarn!("saving data failed: {e}");
        }
    }

    // ------------------------------------------------------------ self-test hooks

    /// Self-test only: render as if the monitor had `dpi` (120 = 125 %, 144 = 150 %, 192 = 200 %).
    pub fn selftest_set_dpi(&mut self, dpi: u32) {
        let dpi = dpi.clamp(96, 480);
        self.monitor.dpi = dpi;
        self.set_dpi(dpi);
        self.request_frame();
    }

    pub fn selftest_set_expanded(&mut self, on: bool) {
        if on {
            self.expand();
        } else {
            self.collapse();
        }
    }

    /// Self-test: a maximized window across the top of the island's monitor
    /// (what the full-screen watcher reports for one), or no window there.
    pub fn selftest_window_under(&mut self, under: bool) {
        let m = &self.monitor;
        let px = (TOP_BAND * self.gfx.scale()).ceil() as i32;
        self.top_windows = if under { vec![[m.rect.left, m.top(), m.rect.right, m.top() + px]] } else { Vec::new() };
    }

    /// Self-test: what the closed island shows now, and whether it stepped aside.
    pub fn selftest_mode(&self) -> (Mode, bool) {
        (self.mode, self.st.aside)
    }

    /// Self-test only: start the real Phone drop service, fetch its page from
    /// the network address it picked, and stop it. Returns (ok, detail).
    pub fn selftest_phone_drop(&mut self) -> (bool, String) {
        match services::phonedrop::PhoneDrop::start(self.sink.clone(), Vec::new()) {
            Err(e) => (false, e),
            Ok((svc, url)) => {
                let r = selftest::fetch_phone_page(&url);
                svc.stop();
                r
            }
        }
    }

    /// At rest: outline settled, mode current, no page switch or outgoing fade.
    pub fn selftest_settled(&self) -> bool {
        self.island.settled() && self.target_mode() == self.mode && self.page_anim.is_none() && self.outgoing.is_none()
    }

    /// Self-test: frames are drawn by Microsoft's software rasterizer, not a GPU.
    pub fn selftest_software_gpu(&self) -> bool {
        self.gfx.software_gpu()
    }

    /// Render a frame right now (bypassing the clock) and return the pixels
    /// of the island's canvas (island, shoulders and hit margin) and where
    /// that canvas sits on screen.
    pub fn selftest_capture(&mut self) -> (i32, i32, Vec<u8>, (i32, i32)) {
        let mut d = Vec::new();
        self.capture_next = true;
        self.frame(&mut d, None);
        let r = self.capture_rect;
        (r.right - r.left, r.bottom - r.top, self.gfx.take_capture(), (self.surface.x + r.left, self.surface.y + r.top))
    }

    /// The window's screen rectangle as laid out (left, top, width, height).
    pub fn selftest_surface(&self) -> (i32, i32, i32, i32) {
        (self.surface.x, self.surface.y, self.surface.w, self.surface.h)
    }

    pub fn renderer(&self) -> &'static str {
        self.renderer_name()
    }

    pub fn selftest_hover(&mut self, p: Option<Point>) {
        self.input.mouse = p;
        self.hovering = p.is_some();
    }

    /// The middle of a widget drawn in the last frame, found by its id.
    pub fn selftest_settings_open(&self) -> bool { self.settings_window.is_some() }

    pub fn selftest_close_settings(&mut self) { self.settings_window = None; }

    pub fn selftest_capture_settings(&mut self, section: crate::settings_window::Section) -> windows::core::Result<(i32, i32, Vec<u8>, (i32, i32), f32)> {
        if self.settings_window.is_none() { self.open_settings(); }
        let window = self.settings_window.as_mut().ok_or_else(windows::core::Error::from_win32)?;
        window.selftest_capture(&mut self.st, section)
    }

    pub fn selftest_hit_center(&self, id: crate::ui::WidgetId) -> Option<Point> {
        self.hits.iter().find(|h| h.id == id).map(|h| h.rect.center())
    }

    pub fn selftest_click(&mut self, p: Point) {
        self.input.mouse = Some(p);
        if let Some(h) = self.hit_at(p) {
            self.input.clicked = Some((h.id, MouseButton::Left));
        }
    }

    /// The island body in window DIPs (where clicks land).
    pub fn island_body(&self) -> Rect {
        self.island.body_rect_at(self.cx)
    }

    /// The island body relative to the last captured frame (DIPs).
    pub fn island_body_in_capture(&self) -> Rect {
        let s = self.gfx.scale();
        let r = self.capture_rect;
        self.island.body_rect_at(self.cx).offset(-(r.left as f32) / s, -(r.top as f32) / s)
    }

    pub fn scale(&self) -> f32 {
        self.gfx.scale()
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

fn theme_accent(_theme: Theme) -> Color {
    services::system_accent().unwrap_or(palette::BLUE)
}

/// The island's current content: a page (expanded) or the collapsed activity.
/// Returns (repaint, anim, focus, live) as requested by the drawing code.
#[allow(clippy::too_many_arguments)]
fn draw_content(
    c: &mut Canvas,
    input: &mut Input,
    hits: &mut Vec<Hit>,
    st: &mut AppState,
    mode: Mode,
    lay: Rect,
    body: Rect,
    transition: Option<pages::PageTransition>,
    now: f64,
) -> (bool, bool, Option<bool>, bool) {
    let mut ui = Ui::new(c, input, hits, now);
    match mode {
        Mode::Expanded(page) => pages::draw_expanded(&mut ui, st, lay, page, transition),
        _ => activities::draw(&mut ui, st, &mode, body),
    }
    (ui.repaint_requested(), ui.anim_requested(), ui.focus_request(), ui.live_requested())
}

/// DWM's (refreshes, frames composed, frames missed) counters, if available.
fn dwm_counters() -> Option<(u64, u64, u64)> {
    use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
    let mut info = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.ok()?;
    Some((info.cRefresh, info.cFrame, info.cFramesMissed))
}

/// The window's size in pixels for an island body up to `body_w`×`body_h`
/// DIPs at `scale`, on a monitor `mon_w` pixels wide: room for the spring's
/// overshoot, and the island's own room around the body (Classic: the
/// shoulders and hit margin; floating: the gap above, the shadow's room
/// beside and below).
fn surface_size(island: &Island, body_w: f32, body_h: f32, scale: f32, mon_w: i32) -> (i32, i32) {
    let mut w = ((body_w * OVERSHOOT + 2.0 * island.side_room()) * scale).ceil() as i32;
    // An even margin on both sides puts the centre on the monitor's centre.
    if (mon_w - w) % 2 != 0 {
        w += 1;
    }
    // The gap the island rests at, not where an outline sliding into or out
    // of it is now: the window never resizes during that motion.
    let top = if island.floating() { pill::TOP_GAP } else { 0.0 };
    let h = ((body_h * OVERSHOOT + island.bottom_room() + top) * scale).ceil() as i32;
    (w, h)
}

/// Where the island window takes the mouse (DIPs, window space). Open: the
/// whole canvas, a forgiving margin around the panel. Collapsed: the
/// island's own outline. While it floats (the Pill theme) the gap above it
/// takes the mouse too, down to the pill's middle, so a pointer thrown at
/// the top edge lands on it as it lands on Classic's island; beside and
/// below the pill (its shadow) clicks pass through. The Hidden idle style's
/// invisible island takes it only in a band at the top edge as tall as its
/// body, as Classic's hidden strip does, so what sits under the gap keeps
/// its clicks. The GPU renderer pushes this as the window's hit mask; the
/// software renderer paints it at alpha 1/255 under the island.
fn hit_mask_for(island: &Island, cx: f32, expanded: bool, hidden: bool) -> HitMask {
    if expanded {
        return HitMask { rect: Some(island.canvas_rect_at(cx)), notch: None, pill: None };
    }
    let body = island.body_rect_at(cx);
    if !island.floating() {
        return HitMask { rect: None, notch: Some((body, island.current().r, SHOULDER)), pill: None };
    }
    if hidden {
        return HitMask { rect: Some(Rect::new(body.x, 0.0, body.w, body.h)), notch: None, pill: None };
    }
    let gap = island.hover_rect_at(cx);
    HitMask { rect: Some(Rect::new(gap.x, gap.y, gap.w, body.y + body.h / 2.0 - gap.y)), notch: None, pill: Some((body, island.current().r)) }
}

/// Name of a morph for the motion log.
fn motion_name(from: &Mode, to: &Mode) -> &'static str {
    match (from, to) {
        (Mode::Expanded(_), Mode::Expanded(_)) => "page",
        (_, Mode::Expanded(_)) => "expand",
        (Mode::Expanded(_), _) => "collapse",
        _ => "pill",
    }
}

/// "1 file", "3 files".
/// The panel's list: most recently used first.
fn paired_phones(pairs: Vec<lumo_core::pairdrop::Pair>) -> Vec<PairedPhone> {
    let mut v: Vec<PairedPhone> = pairs.into_iter().map(|p| PairedPhone { room: p.room(), name: p.name, seen: p.seen.max(p.added) }).collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.seen));
    v
}

fn files_label(n: usize) -> String {
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::HIT_MARGIN;
    use crate::state::MediaSession;

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn reopening_follows_activity_without_overriding_explicit_navigation() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        let point = |p: Point, scale: f32| LPARAM(((((p.y * scale).round() as u16 as usize) << 16)
            | (p.x * scale).round() as u16 as usize) as isize);
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        let mut deferred = vec![];
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let now = 3000.0 + index as f64 * 300.0;
            clock::hold(Some(now));
            let sample = crate::ai_window::sample_state_for_selftest();
            app.st.ai = sample.ai;
            app.st.ai.draft.clear();
            app.st.ai.pending.clear();
            app.st.settings.ai = sample.settings.ai;
            app.st.settings.theme = theme;
            app.st.settings.open_on_hover = false;
            app.st.foreground_app = None;
            app.st.pinned = false;
            app.st.notice = None;
            app.st.hud = None;
            app.st.now = now;
            app.st.media = Some(MediaSession {
                app_id: "player".into(), app_name: "Player".into(), music_source: None,
                title: "Track".into(), artist: String::new(), album: String::new(), art: None, art_color: None,
                timeline: lumo_core::timeline::Timeline::new(0.0, 300.0, now, true),
                can_prev: true, can_next: true, can_play_pause: true, can_seek: true, track_key: "track".into(),
            });
            app.reopen = crate::reopen::Context::default();
            app.expanded = false;
            app.st.page = PageKind::Ai;
            app.st.store.last_page = "ai".into();
            app.input = Input::default();
            app.layout_surface();
            app.mode = activities::collapsed_mode(&app.st);
            app.island.snap(app.geom_for(&app.mode));
            app.frame(&mut deferred, None);
            let lp = point(app.island.body_rect_at(app.cx).center(), app.gfx.scale());
            app.handle_island(WM_LBUTTONDOWN, WPARAM(0), lp, &mut deferred);
            assert!(app.expanded && app.st.page == PageKind::Media, "{theme:?}: normal click follows music");
            assert_eq!(app.st.store.last_page, "ai", "automatic routing must not save a preference");
            app.handle_island(WM_LBUTTONUP, WPARAM(0), lp, &mut deferred);
            app.input.end_frame();

            app.set_page(PageKind::Ai);
            app.st.ai.draft = "Continue this".into();
            app.collapse();
            app.open_island();
            assert_eq!(app.st.page, PageKind::Ai, "unfinished prompt resumes");
            app.st.ai.draft.clear();
            app.island.snap(app.geom_for(&Mode::Expanded(PageKind::Ai)));
            app.mode = Mode::Expanded(PageKind::Ai);
            app.hovering = true;
            let content = pages::content_area(app.island.body_rect_at(app.cx)).center();
            let mut screen = POINT { x: (content.x * app.gfx.scale()) as i32, y: (content.y * app.gfx.scale()) as i32 };
            unsafe { let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut screen); }
            let wheel_lp = LPARAM(((screen.y as u16 as usize) << 16 | screen.x as u16 as usize) as isize);
            app.handle_island(WM_MOUSEWHEEL, WPARAM(((-120i16) as u16 as usize) << 16), wheel_lp, &mut deferred);
            app.frame(&mut deferred, None);
            app.collapse();
            app.open_island();
            assert_eq!(app.st.page, PageKind::Ai, "scrolling keeps a conversation ready");

            app.st.ai.busy = true;
            app.collapse();
            app.open_island();
            assert_eq!(app.st.page, PageKind::Media, "waiting for AI yields to music");
            app.st.ai.busy = false;
            app.set_page(PageKind::Ai);
            app.collapse();
            app.st.settings.open_on_hover = true;
            app.on_timer(TIMER_HOVER);
            assert!(app.expanded && app.st.page == PageKind::Media, "hover follows the same policy");

            app.collapse();
            app.st.page = PageKind::Ai;
            app.handle_helper(WM_APP_TRAY, WPARAM(0), LPARAM(WM_LBUTTONUP as isize), &mut deferred);
            assert!(app.expanded && app.st.page == PageKind::Media, "tray follows the same policy");

            app.collapse();
            app.st.notify(Icon::Timer, palette::BLUE, "Timer", "Open Focus", 4.0, Some(NoticeAction::OpenPage(PageKind::Focus)));
            app.handle_island(WM_LBUTTONDOWN, WPARAM(0), lp, &mut deferred);
            assert!(app.expanded && app.st.page == PageKind::Focus, "explicit notice overrides music");
            app.handle_island(WM_LBUTTONUP, WPARAM(0), lp, &mut deferred);
            app.st.ai.draft = "Keep this".into();
            app.set_page(PageKind::Ai);
            app.set_page(PageKind::Today);
            app.open_island();
            assert_eq!(app.st.page, PageKind::Today, "an open page never switches automatically");
            app.collapse();
            app.open_island();
            assert_eq!(app.st.page, PageKind::Media, "a hidden draft cannot override an explicit switch");
            app.collapse();
        }
        clock::hold(None);
        unsafe { let _ = DestroyWindow(hwnd); let _ = DestroyWindow(helper); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn maximized_hover_only_reveals_compact_and_split_windows_stay_visible() {
        unsafe extern "system" fn test_proc(hwnd: HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT { DefWindowProcW(hwnd,msg,wp,lp) }
        fn point(app:&App,p:Point)->LPARAM {
            let x=(p.x*app.gfx.scale()).round() as u16 as u32; let y=(p.y*app.gfx.scale()).round() as u16 as u32;
            LPARAM((x|(y<<16)) as isize)
        }
        window::register_classes(test_proc,test_proc).unwrap();
        let hwnd=window::create_island_window().unwrap(); let helper=window::create_helper_window().unwrap();
        let (tx,rx)=mpsc::channel(); let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();
        app.settings_writable=false; app.layout_surface();
        let mut deferred=Vec::new();
        for (index,theme) in [Theme::Classic,Theme::Pill].into_iter().enumerate() {
            let base=300.0+index as f64*10.0; clock::hold(Some(base));
            app.st.settings.theme=theme; app.st.settings.open_on_hover=true;
            app.st.settings.hide_over_maximized=true; app.st.settings.step_aside=false;
            app.st.settings.idle_style=IdleStyle::Clock; app.st.notice=None; app.st.hud=None;
            app.expanded=false; app.hovering=false;
            app.st.foreground_app=Some(crate::state::ForegroundApp { executable:"code.exe".into(),name:"Visual Studio Code".into(),maximized_here:true });
            for frame in 1..=60 {clock::hold(Some(base+frame as f64/60.0));app.frame(&mut deferred,None);}
            assert!(app.st.aside && app.aside_visibility==0.0);
            let edge=app.hit_mask().rect.unwrap_or_else(||app.island.body_rect_at(app.cx)).center();
            app.handle_island(WM_MOUSEMOVE,WPARAM(0),point(&app,edge),&mut deferred);
            clock::hold(Some(base+1.1)); app.frame(&mut deferred,None);
            assert!(!app.st.aside && !app.expanded && app.hover_arm.is_none());
            app.on_timer(TIMER_HOVER); assert!(!app.expanded,"maximized hover must never expand even with general hover opening enabled");
            app.island.snap(app.geom_for(&app.mode));
            let center=point(&app,app.island.body_rect_at(app.cx).center());
            app.handle_island(WM_LBUTTONDOWN,WPARAM(0),center,&mut deferred);
            assert!(app.expanded,"a click must open the revealed compact island");
            app.handle_island(WM_LBUTTONUP,WPARAM(0),center,&mut deferred);
            app.collapse(); app.hovering=false;
            app.st.foreground_app.as_mut().unwrap().maximized_here=false;
            // A split window can touch the island's entire top band; that
            // alone must not turn normal/snapped windows into maximized ones.
            app.top_windows=vec![[0,0,1920,8]];
            for frame in 1..=60 {clock::hold(Some(base+1.2+frame as f64/60.0));app.frame(&mut deferred,None);}
            assert!(!app.st.aside && app.aside_visibility==1.0 && !app.expanded);
            let (_,_,pixels,_)=app.selftest_capture();
            assert!(pixels.chunks_exact(4).any(|pixel|pixel[3]>250),"split screen must keep real compact content visible");
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);
        unsafe {DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn lyrics_popup_handoff_visibility_idle_clock_and_return_work_in_both_themes() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { DefWindowProcW(hwnd, msg, wp, lp) }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.layout_surface();
        let mut deferred = Vec::new();
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let base = 200.0+index as f64*10.0;
            clock::hold(Some(base));
            let fixture = crate::lyrics_window::sample_state();
            app.st.media = fixture.media; app.st.lyrics = fixture.lyrics;
            app.st.media_pending_seek = None;
            app.st.settings.theme = theme;
            app.st.settings.hide_over_maximized = false;
            app.st.foreground_app = Some(crate::state::ForegroundApp { executable: "code.exe".into(), name: "Visual Studio Code".into(), maximized_here: true });
            app.expanded = false; app.st.notice = None; app.st.hud = None;
            app.frame(&mut deferred, None);
            assert!(!app.expanded && !app.st.aside, "maximized apps must initially show the compact island");
            app.st.page = PageKind::Media;
            app.expand();
            app.mode = Mode::Expanded(PageKind::Media);
            for width in [480.0, 620.0] {
                app.st.settings.expanded_width = width; app.layout_surface(); app.island.snap(app.geom_for(&app.mode));
                app.frame(&mut deferred, None);
                let pop = app.hits.iter().find(|hit| hit.id == wid("media.lyrics-popup")).unwrap().rect;
                let cues=(0..7).filter(|&i|app.hits.iter().any(|hit|hit.id==crate::ui::wid_n("media_lyric",i))).count();
                assert!(cues>=3,"{theme:?} at {width}: keep the current cue and its neighbours visible, got {cues}");
                assert!(!app.hits.iter().any(|hit|hit.id==wid("media_lyrics_credit")),"lyric source labels must not appear in the player");
                let controls:[Rect;4]=["media_prev","media_play","media_next","media_seek"].map(|id|app.hits.iter().find(|hit|hit.id==wid(id)).unwrap().rect);
                let height=pages::media::height(&app.st);
                for status in [crate::state::LyricsStatus::Loading,crate::state::LyricsStatus::NotFound,crate::state::LyricsStatus::Found] {
                    app.st.lyrics.status=status;app.frame(&mut deferred,None);
                    assert_eq!(pages::media::height(&app.st),height,"lookup states must not resize the island");
                    for (id,expected) in ["media_prev","media_play","media_next","media_seek"].into_iter().zip(controls) {
                        assert_eq!(app.hits.iter().find(|hit|hit.id==wid(id)).unwrap().rect,expected,"{theme:?}: lookup moved {id}");
                    }
                }
                for id in ["media_prev", "media_play", "media_next", "media_seek"] {
                    let other = app.hits.iter().find(|hit| hit.id == wid(id)).unwrap().rect;
                    assert!(pop.right() <= other.x || pop.x >= other.right() || pop.bottom() <= other.y || pop.y >= other.bottom(), "{theme:?} at {width}: pop-out overlaps {id}: {pop:?}/{other:?}");
                }
            }
            let (on_w,on_h,on_pixels,_) = app.selftest_capture();
            if let Some(dir)=std::env::var_os("LUMO_LYRICS_QA").map(std::path::PathBuf::from) {
                app.input.mouse=app.hits.iter().find(|hit|hit.id==wid("media.lrc-toggle")).map(|hit|hit.rect.center());
                let (w,h,pixels,_)=app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("lyrics-tooltip-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();
                app.input.mouse=None;
            }
            app.input.clicked=Some((wid("media.lrc-toggle"),MouseButton::Left));app.frame(&mut deferred,None);
            assert!(!app.st.settings.island_lyrics);
            let (off_w,off_h,off_pixels,_) = app.selftest_capture();
            assert_eq!((on_w,on_h),(off_w,off_h));
            assert_ne!(on_pixels,off_pixels,"the toggle must visibly change between On and Off");
            if let Some(dir)=std::env::var_os("LUMO_LYRICS_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                crate::png::save_rgba(dir.join(format!("lyrics-on-{theme:?}.png")),on_w as u32,on_h as u32,&crate::png::pbgra_to_rgba(&on_pixels)).unwrap();
                crate::png::save_rgba(dir.join(format!("lyrics-off-{theme:?}.png")),off_w as u32,off_h as u32,&crate::png::pbgra_to_rgba(&off_pixels)).unwrap();
            }
            app.input.clicked=Some((wid("media.lrc-toggle"),MouseButton::Left));app.frame(&mut deferred,None);
            assert!(app.st.settings.island_lyrics);
            app.st.settings.lyrics_popup_button = false; app.frame(&mut deferred, None);
            assert!(!app.hits.iter().any(|hit| hit.id == wid("media.lyrics-popup")));
            app.st.settings.lyrics_popup_button = true;
            app.st.lyrics.status = crate::state::LyricsStatus::NotFound; app.frame(&mut deferred, None);
            assert!(!app.hits.iter().any(|hit| hit.id == wid("media.lyrics-popup")));
            app.st.lyrics.status = crate::state::LyricsStatus::Found; app.frame(&mut deferred, None);
            if let Some(dir) = std::env::var_os("LUMO_LYRICS_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                let (w,h,pixels,_) = app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("music-lyrics-buttons-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
            app.input.clicked = Some((wid("media.lyrics-popup"), MouseButton::Left));
            app.frame(&mut deferred, None);
            let popup = app.lyrics_window.as_ref().expect("the real music button must open the window").hwnd;
            assert!(unsafe { IsWindowVisible(hwnd).as_bool() }, "the popup must not hide the island");
            assert_ne!(unsafe { GetWindowLongPtrW(popup, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0, 0);
            for frame in 1..=60 { clock::hold(Some(base+frame as f64/60.0)); app.frame(&mut deferred, None); }
            assert!(unsafe { IsWindowVisible(popup).as_bool() });
            assert!(!app.clock.pacing().0, "the paused lyrics player must park the shared frame clock");
            assert!(app.st.lyrics_popup_open && app.st.settings.island_lyrics,"keep the saved lyrics preference while the popup is open");
            assert!(!pill::lyric::sings(&app.st),"the closed island must not show lyrics while their popup is open");
            assert_eq!(pill::lyric::next_wake(&app.st),None,"no closed lyric wakeups while the popup is open");
            app.st.ui.lyric_track.clear();
            app.expand();app.mode=Mode::Expanded(PageKind::Media);app.island.snap(app.geom_for(&app.mode));
            clock::hold(Some(base+1.02));app.frame(&mut deferred,None);
            assert!(app.st.ui.lyric_track.is_empty(),"the expanded Music lyrics column must not render while the popup is open");
            assert!(!app.hits.iter().any(|hit|hit.id==wid("media.lrc-toggle")),"the lyrics toggle must be disabled while the popup is open");
            app.input.clicked=Some((wid("media.lrc-toggle"),MouseButton::Left));app.frame(&mut deferred,None);
            assert!(app.st.settings.island_lyrics,"a queued click must not change the disabled lyrics toggle");
            app.st.lyrics.status=crate::state::LyricsStatus::Loading;app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|hit|hit.id==wid("media.lyrics-popup")),"the return button must stay available during track changes");
            app.st.lyrics.status=crate::state::LyricsStatus::Found;app.frame(&mut deferred,None);
            if let Some(dir) = std::env::var_os("LUMO_LYRICS_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                let (w,h,pixels,_)=app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("music-with-popup-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();
                let (w,h,pixels) = app.lyrics_window.as_mut().unwrap().capture(&mut app.st);
                crate::png::save_rgba(dir.join(format!("lyrics-popup-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
            app.on_event(AppEvent::WidgetsOpen(true)); app.frame(&mut deferred, None);
            assert!(!unsafe { IsWindowVisible(popup).as_bool() }); assert!(!app.clock.pacing().0);
            app.on_event(AppEvent::WidgetsOpen(false)); app.frame(&mut deferred, None);
            assert!(unsafe { IsWindowVisible(popup).as_bool() }); assert!(unsafe { IsWindowVisible(hwnd).as_bool() });
            app.locked = true; app.frame(&mut deferred, None); assert!(!app.clock.pacing().0);
            app.locked = false; app.st.power.saver = true;
            app.st.media.as_mut().unwrap().timeline = lumo_core::timeline::Timeline::new(19.0,210.0,base+0.7,true);
            app.frame(&mut deferred, None); app.frame(&mut deferred, None);
            assert!(!app.clock.pacing().0, "saver must stop repeating frames after the compact text has fitted");
            app.st.power.saver = false;
            app.expand();app.mode=Mode::Expanded(PageKind::Media);app.island.snap(app.geom_for(&app.mode));
            let open_width=app.island.current().w;
            clock::hold(Some(base+1.1));app.input.clicked=Some((wid("media.lyrics-popup"),MouseButton::Left));app.frame(&mut deferred,None);
            assert!(app.lyrics_window.as_ref().unwrap().is_closing(),"the inward button must close the popup");
            clock::hold(Some(base+1.4)); app.frame(&mut deferred, None);
            assert!(app.lyrics_window.is_none() && app.expanded && app.st.page == PageKind::Media);
            assert_eq!(app.island.current().w,open_width,"closing the popup must not collapse an already-open Music panel");
            assert!(!app.st.lyrics_popup_open);
            assert!(pill::lyric::sings(&app.st),"closing must restore closed-island lyrics when enabled");
            assert!(!unsafe { IsWindow(popup).as_bool() });
            clock::hold(Some(base+1.5)); app.frame(&mut deferred, None);
            assert!(unsafe { IsWindowVisible(hwnd).as_bool() });
            assert_eq!(app.st.ui.lyric_track,"lyrics-window-fixture","closing must restore the expanded lyrics column");
            assert!(app.hits.iter().any(|hit|hit.id==wid("media.lrc-toggle")),"closing must re-enable the lyrics toggle");
            app.collapse();
        }
        clock::hold(None);
        app.clock.set_active(false); app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn maximized_hide_reveal_widgets_priority_and_island_right_click() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.st.settings.hide_over_maximized = true;
        app.st.settings.idle_style = IdleStyle::Clock;
        app.st.notice = None;
        app.st.hud = None;
        app.layout_surface();
        let mut deferred = Vec::new();
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let base = 200.0 + index as f64 * 10.0;
            clock::hold(Some(base));
            app.st.settings.theme = theme;
            app.st.foreground_app = None;
            app.st.aside = false;
            app.expanded = false;
            app.mode = app.target_mode();
            app.aside_visibility = 1.0;
            app.island.snap(app.geom_for(&app.mode));
            app.frame(&mut deferred, None);
            let rest_w = app.island.current().w;
            app.on_event(AppEvent::ForegroundApp(Some(crate::state::ForegroundApp {
                executable: "c:\\apps\\code.exe".into(), name: "Visual Studio Code".into(), maximized_here: true,
            })));
            for frame in 1..=60 {
                clock::hold(Some(base + frame as f64 / 60.0));
                let before = app.aside_visibility;
                app.frame(&mut deferred, None);
                assert!(app.aside_visibility <= before);
                assert_eq!(app.mode, Mode::Aside);
                assert!(!app.expanded);
                if frame == 5 {
                    assert!(app.aside_visibility > 0.0 && app.aside_visibility < 1.0, "{theme:?}: hiding must fade over several frames");
                    assert!((app.island.target().w - rest_w).abs() < 0.1, "a clock must not stretch sideways while hiding");
                    let (w, h, pixels, _) = app.selftest_capture();
                    let alpha = pixels.chunks_exact(4).map(|pixel| pixel[3]).max().unwrap_or(0);
                    assert!((10..245).contains(&alpha), "{theme:?}: the entire rendered island must fade, alpha={alpha}");
                    if let Some(dir) = std::env::var_os("LUMO_VISIBILITY_QA").map(std::path::PathBuf::from) {
                        std::fs::create_dir_all(&dir).unwrap();
                        crate::png::save_rgba(dir.join(format!("hide-transition-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
                    }
                }
            }
            assert_eq!(app.aside_visibility, 0.0);
            assert!(app.island.settled());
            let (_, _, pixels, _) = app.selftest_capture();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] <= 1), "{theme:?}: no visible strip may remain after hiding");
            if theme == Theme::Pill { assert!(app.island.body_rect_at(app.cx).y.abs() < 0.1); }
            app.st.foreground_app.as_mut().unwrap().maximized_here = false;
            for frame in 1..=60 {
                clock::hold(Some(base + 1.0 + frame as f64 / 60.0));
                let before = app.aside_visibility;
                app.frame(&mut deferred, None);
                assert!(app.aside_visibility >= before);
                if frame == 5 { assert!(app.aside_visibility > 0.0 && app.aside_visibility < 1.0); }
            }
            assert_eq!(app.aside_visibility, 1.0);
            assert!(!app.st.aside && !app.expanded);

            // Widgets wins even over explicit Show rules and full-screen Show.
            app.st.settings.hide_in_fullscreen = false;
            app.st.foreground_app.as_mut().unwrap().maximized_here = true;
            app.st.settings.keep_app_visible("c:\\apps\\code.exe".into(), true);
            app.on_event(AppEvent::WidgetsOpen(true));
            assert!(app.fullscreen_hidden && app.fullscreen_hides(app.st.now));
            assert!(!unsafe { IsWindowVisible(hwnd).as_bool() });
            app.on_event(AppEvent::Foreground);
            assert!(!unsafe { IsWindowVisible(hwnd).as_bool() });
            app.on_event(AppEvent::WidgetsOpen(false));
            assert!(!app.fullscreen_hidden);
            assert!(!unsafe { IsWindowVisible(hwnd).as_bool() }, "the old bitmap must stay hidden until a fresh frame is ready");
            app.st.settings.keep_app_visible("c:\\apps\\code.exe".into(), false);
            app.st.foreground_app.as_mut().unwrap().maximized_here = false;

            // Neither an empty region nor a control may react to right-click.
            app.expanded = true;
            app.mode = Mode::Expanded(PageKind::Today);
            app.st.page = PageKind::Today;
            app.island.snap(app.geom_for(&app.mode));
            app.frame(&mut deferred, None);
            assert!(!app.hits.iter().any(|hit| hit.id == wid("header.keep-visible")), "restored apps have no visibility override button");
            for message in [WM_RBUTTONUP, WM_CONTEXTMENU] {
                app.handle_island(message, WPARAM(0), LPARAM(0), &mut deferred);
                assert!(deferred.is_empty() && app.input.clicked.is_none());
            }
            if let Some(dir) = std::env::var_os("LUMO_VISIBILITY_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                let (w, h, pixels, _) = app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("today-calendar-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn click_mode_never_expands_on_hover_or_maximize_and_hover_remains_opt_in() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        fn inside(app: &App) -> LPARAM {
            let body = app.island.body_rect_at(app.cx);
            let rect = if app.rests_hidden() { app.hit_mask().rect.unwrap_or(body) } else { body };
            let p = rect.center();
            let x = (p.x * app.gfx.scale()).round() as i32;
            let y = (p.y * app.gfx.scale()).round() as i32;
            LPARAM(((x as u16 as u32) | ((y as u16 as u32) << 16)) as isize)
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.layout_surface();
        let mut deferred = Vec::new();
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let now = 100.0 + index as f64 * 10.0;
            clock::hold(Some(now));
            app.st.settings.theme = theme;
            app.st.settings.open_on_hover = false;
            app.st.settings.hide_over_maximized = false;
            app.expanded = false;
            app.hovering = false;
            app.st.aside = false;
            app.mode = app.target_mode();
            app.island.snap(app.geom_for(&app.mode));
            app.left_at = now - 0.05;
            let at = inside(&app);
            app.handle_island(WM_MOUSEMOVE, WPARAM(0), at, &mut deferred);
            assert!(!app.expanded && app.hover_arm.is_none(), "{theme:?}: no dwell or fast-return opening in click mode");
            app.on_timer(TIMER_HOVER);
            assert!(!app.expanded, "a queued hover timer must not open click mode");
            app.on_event(AppEvent::ForegroundApp(Some(crate::state::ForegroundApp { executable: "c:\\apps\\editor.exe".into(), name: "Editor".into(), maximized_here: true })));
            app.frame(&mut deferred, None);
            assert!(!app.expanded && !app.st.aside);
            assert!(!matches!(app.target_mode(), Mode::Expanded(_) | Mode::Aside), "maximizing must show only the compact island");
            let at = inside(&app);
            app.handle_island(WM_LBUTTONDOWN, WPARAM(0), at, &mut deferred);
            assert!(app.expanded, "{theme:?}: clicking must open the island");
            app.handle_island(WM_LBUTTONUP, WPARAM(0), at, &mut deferred);
            app.collapse();
            app.handle_island(WM_MOUSELEAVE, WPARAM(0), LPARAM(0), &mut deferred);
            app.left_at = now;
            app.mode = app.target_mode();
            app.island.snap(app.geom_for(&app.mode));
            let at = inside(&app);
            app.handle_island(WM_MOUSEMOVE, WPARAM(0), at, &mut deferred);
            assert!(!app.expanded, "returning during close must still require a click");
            let before = app.st.settings.clone();
            app.st.foreground_app.as_mut().unwrap().maximized_here = false;
            app.st.settings.open_on_hover = true;
            app.settings_changed(&before);
            app.left_at = now - 2.0;
            app.arm_hover(app.input.mouse.unwrap());
            assert!(app.hover_arm.is_some());
            app.on_timer(TIMER_HOVER);
            assert!(app.expanded, "hover must work when explicitly enabled");
            app.collapse();
            app.arm_hover(Point::new(app.cx, 10.0));
            let before = app.st.settings.clone();
            app.st.settings.open_on_hover = false;
            app.settings_changed(&before);
            assert!(app.hover_arm.is_none());
            app.on_timer(TIMER_HOVER);
            assert!(!app.expanded, "switching to click must cancel an already armed dwell");
            app.st.foreground_app.as_mut().unwrap().maximized_here = true;
            app.hovering = false;
            app.st.settings.hide_over_maximized = true;
            app.sync_aside();
            app.mode = app.target_mode();
            app.island.snap(app.geom_for(&app.mode));
            let at = inside(&app);
            app.handle_island(WM_LBUTTONDOWN, WPARAM(0), at, &mut deferred);
            assert!(app.expanded, "an explicitly hidden island must still open when its edge is clicked");
            app.handle_island(WM_LBUTTONUP, WPARAM(0), at, &mut deferred);
            app.collapse();
            app.hovering = false;
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn seek_bar_keeps_the_clicked_position_through_queued_reports() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        fn release_at(app: &mut App, fraction: f32, deferred: &mut Vec<Deferred>) {
            let hit = app.hits.iter().find(|hit| hit.id == wid("media_seek")).unwrap().rect;
            let point = Point::new(hit.x + hit.w * fraction, hit.center().y);
            app.input.mouse = Some(point);
            app.input.released = Some((wid("media_seek"), point));
            app.frame(deferred, None);
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.expanded = true;
        app.mode = Mode::Expanded(PageKind::Media);
        app.st.page = PageKind::Media;
        app.st.settings.lyrics = false;
        app.layout_surface();
        let mut deferred = Vec::new();
        for (index, (theme, playing)) in [(Theme::Classic, false), (Theme::Pill, false), (Theme::Classic, true), (Theme::Pill, true)].into_iter().enumerate() {
            let base = 100.0 + index as f64 * 10.0;
            clock::hold(Some(base));
            app.st.settings.theme = theme;
            app.st.media_pending_seek = None;
            app.st.media = Some(MediaSession { app_id: "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic".into(), app_name: "Media Player".into(),
                music_source: Some(lumo_core::nowplaying::MusicSource::Local), title: "Seek fixture".into(), artist: "Artist".into(), album: "Album".into(),
                art: None, art_color: None, timeline: lumo_core::timeline::Timeline::new(30.0, 200.0, base, playing),
                can_prev: true, can_next: true, can_play_pause: true, can_seek: true, track_key: "seek-fixture".into() });
            app.island.snap(app.geom_for(&app.mode));
            app.frame(&mut deferred, None);
            release_at(&mut app, 0.75, &mut deferred);
            assert!((app.st.media.as_ref().unwrap().timeline.position_at(base) - 150.0).abs() < 0.001);
            assert!(app.st.media_pending_seek.is_some());
            let old = lumo_core::timeline::Timeline::new(30.0, 200.0, base - 0.1, playing);
            app.on_event(AppEvent::MediaTimeline { track_key: "seek-fixture".into(), timeline: old });
            let mut capabilities = app.st.media.clone().unwrap();
            capabilities.timeline = old;
            capabilities.can_next = false;
            app.on_event(AppEvent::Media(Some(capabilities)));
            app.frame(&mut deferred, None);
            assert!((app.st.media.as_ref().unwrap().timeline.position_at(base) - 150.0).abs() < 0.001);
            clock::hold(Some(base + 0.2));
            let pos = if playing { 150.2 } else { 150.0 };
            app.on_event(AppEvent::MediaTimeline { track_key: "seek-fixture".into(), timeline: lumo_core::timeline::Timeline::new(pos, 200.0, base + 0.2, playing) });
            assert!(app.st.media_pending_seek.is_some(), "one confirmation must not let a late stale report through");
            clock::hold(Some(base + 0.3));
            app.on_event(AppEvent::MediaTimeline { track_key: "seek-fixture".into(), timeline: lumo_core::timeline::Timeline::new(30.0, 200.0, base + 0.29, playing) });
            app.frame(&mut deferred, None);
            let want = if playing { 150.3 } else { 150.0 };
            assert!((app.st.media.as_ref().unwrap().timeline.position_at(base + 0.3) - want).abs() < 0.001, "{theme:?}: feedback must keep moving from the selected position");
            clock::hold(Some(base + 0.4));
            release_at(&mut app, 0.1, &mut deferred);
            clock::hold(Some(base + 0.5));
            app.on_event(AppEvent::MediaTimeline { track_key: "seek-fixture".into(), timeline: lumo_core::timeline::Timeline::new(150.0, 200.0, base + 0.5, playing) });
            let want = if playing { 20.1 } else { 20.0 };
            assert!((app.st.media.as_ref().unwrap().timeline.position_at(base + 0.5) - want).abs() < 0.001, "the latest seek must win");
            clock::hold(Some(base + 3.5));
            app.on_event(AppEvent::MediaTimeline { track_key: "seek-fixture".into(), timeline: lumo_core::timeline::Timeline::new(30.0, 200.0, base + 3.5, playing) });
            assert!(app.st.media_pending_seek.is_none());
            assert_eq!(app.st.media.as_ref().unwrap().timeline.position_at(base + 3.5), 30.0);
            pages::media::push_media(&mut app.st, MediaCmd::Seek(100.0));
            let mut other_player = app.st.media.clone().unwrap();
            other_player.app_id = "Spotify.exe".into(); other_player.app_name = "Spotify".into();
            other_player.music_source = Some(lumo_core::nowplaying::MusicSource::Streaming);
            other_player.timeline = lumo_core::timeline::Timeline::new(10.0, 200.0, base + 3.5, false);
            app.on_event(AppEvent::Media(Some(other_player)));
            assert!(app.st.media_pending_seek.is_none(), "a different player must not inherit the pending seek");
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn track_art_crossfades_in_both_themes() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.expanded = true;
        app.mode = Mode::Expanded(PageKind::Media);
        app.st.page = PageKind::Media;
        app.st.settings.lyrics = false;
        app.layout_surface();
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let base = 20.0 + index as f64 * 2.0;
            app.st.settings.theme = theme;
            pages::media::put_anim(Box::default());
            app.st.media = Some(MediaSession { app_id: "MusicBee.exe".into(), app_name: "MusicBee".into(),
                music_source: Some(lumo_core::nowplaying::MusicSource::Local),
                title: "First track".into(), artist: "First artist".into(), album: "Album".into(),
                art: Some(crate::gfx::ImageData::new(1, 1, vec![0, 0, 255, 255])), art_color: None,
                timeline: lumo_core::timeline::Timeline::new(0.0, 201.0, base, false),
                can_prev: true, can_next: true, can_play_pause: true, can_seek: true, track_key: "first".into() });
            app.island.snap(app.geom_for(&app.mode));
            clock::hold(Some(base));
            app.selftest_capture();
            let m = app.st.media.as_mut().unwrap();
            m.title = "Second track".into(); m.artist = "Second artist".into(); m.track_key = "second".into();
            m.art = Some(crate::gfx::ImageData::new(1, 1, vec![255, 0, 0, 255]));
            clock::hold(Some(base + 0.1));
            app.selftest_capture();
            clock::hold(Some(base + 0.21));
            let (w, h, pixels, _) = app.selftest_capture();
            let body = app.island_body_in_capture();
            let x = ((body.x + pages::PAD + 52.0) * app.gfx.scale()).round() as usize;
            let y = ((body.y + pages::HEADER_H + 52.0) * app.gfx.scale()).round() as usize;
            let pixel = &pixels[(y * w as usize + x) * 4..][..4];
            if let Some(dir) = std::env::var_os("LUMO_VISIBILITY_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                crate::png::save_rgba(dir.join(format!("track-transition-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
            assert!(pixel[0] > 40 && pixel[2] > 40, "{theme:?}: halfway cover must blend both images: {pixel:?}, body {body:?}, sample {x},{y}, size {w},{h}, time {}", app.st.now);
            clock::hold(Some(base + 0.4));
            let (_, _, pixels, _) = app.selftest_capture();
            let pixel = &pixels[(y * w as usize + x) * 4..][..4];
            assert!(pixel[0] > 200 && pixel[2] < 20, "{theme:?}: final cover must be the new image: {pixel:?}");
            let anim = pages::media::take_anim();
            assert!(!anim.track.moving());
            pages::media::put_anim(anim);
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn keep_visible_icon_toggles_the_current_apps_saved_rule() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        // Exercise the real header/input without touching the person's profile.
        app.settings_writable = false;
        app.layout_surface();
        app.expanded = true;
        app.mode = Mode::Expanded(PageKind::Media);
        app.st.page = PageKind::Media;
        app.island.snap(app.geom_for(&app.mode));
        app.st.foreground_app = Some(crate::state::ForegroundApp { executable: "c:\\apps\\editor.exe".into(), name: "Visual Studio Code".into(), maximized_here: true });
        app.st.settings.step_aside = true;
        app.st.settings.hide_over_maximized = true;
        clock::hold(Some(10.0));
        let mut deferred = Vec::new();
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let base = 10.0 + index as f64 * 10.0;
            clock::hold(Some(base));
            app.st.settings.theme = theme;
            app.frame(&mut deferred, None);
            assert!(app.hits.iter().any(|hit| hit.id == wid("header.keep-visible")));
            app.input.mouse = Some(app.hits.iter().find(|hit| hit.id == wid("header.keep-visible")).unwrap().rect.center());
            app.frame(&mut deferred, None);
            clock::hold(Some(base + 0.6));
            if let Some(dir) = std::env::var_os("LUMO_VISIBILITY_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                let (w, h, pixels, _) = app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("keep-visible-tooltip-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
            app.input.clicked = Some((wid("header.keep-visible"), MouseButton::Left));
            app.frame(&mut deferred, None);
            assert!(app.st.settings.keeps_app_visible("c:\\apps\\editor.exe"));
            app.sync_aside();
            assert!(!app.st.aside, "the saved Show rule must override maximized hiding and step-aside");
            if let Some(dir) = std::env::var_os("LUMO_VISIBILITY_QA").map(std::path::PathBuf::from) {
                std::fs::create_dir_all(&dir).unwrap();
                let (w, h, pixels, _) = app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("keep-visible-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
            app.input.clicked = Some((wid("header.keep-visible"), MouseButton::Left));
            app.frame(&mut deferred, None);
            assert!(!app.st.settings.keeps_app_visible("c:\\apps\\editor.exe"));
            app.sync_aside();
            assert!(app.st.aside);
            app.st.settings.expanded_width = 480.0;
            app.layout_surface();
            app.island.snap(app.geom_for(&app.mode));
            app.frame(&mut deferred, None);
            for id in ["header.keep-visible", "header.gear", "pin"] {
                assert!(app.hits.iter().any(|hit| hit.id == wid(id)), "{theme:?}: {id} must fit at the narrowest width");
            }
            app.st.settings.expanded_width = 620.0;
            app.layout_surface();
            app.island.snap(app.geom_for(&app.mode));
            app.input.mouse = None;
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn lyrics_controls_follow_music_source_in_both_themes() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.layout_surface();
        app.expanded = true;
        app.mode = Mode::Expanded(PageKind::Media);
        app.st.page = PageKind::Media;
        app.island.snap(app.geom_for(&app.mode));
        clock::hold(Some(10.0));
        let mut deferred = Vec::new();
        for theme in [Theme::Classic, Theme::Pill] {
            app.st.settings.theme = theme;
            for (id, name, audio, expected) in [
                ("Spotify.exe", "Spotify", None, true),
                ("Deezer.exe", "Deezer", None, true),
                ("MusicBee.exe", "MusicBee", Some(true), true),
                ("VLC.exe", "VLC", Some(false), false),
                ("Chrome", "YouTube Music", Some(true), true),
                ("Chrome", "YouTube", Some(true), false),
            ] {
                let music_source = lumo_core::nowplaying::music_source(id, name, audio);
                // Same metadata deliberately exercises switching source without
                // changing the song, including two sites in the same browser.
                let session = MediaSession { app_id: id.into(), app_name: name.into(), music_source,
                    title: "Paper Boats".into(), artist: "The Lanterns".into(), album: "Harbor Lights".into(),
                    art: None, art_color: None, timeline: lumo_core::timeline::Timeline::new(10.0, 201.0, 10.0, true),
                    can_prev: true, can_next: true, can_seek: true, can_play_pause: true,
                    track_key: MediaSession::make_track_key("Paper Boats", "The Lanterns", "Harbor Lights") };
                app.st.lyrics.source = "Local files".into();
                app.st.lyrics.source_file = Some("C:\\Music\\old.lrc".into());
                app.on_event(AppEvent::Media(Some(session)));
                assert!(app.st.lyrics.source_file.is_none());
                app.frame(&mut deferred, None);
                assert_eq!(app.hits.iter().any(|h| h.id == wid("media.lrc-toggle")), expected, "{theme:?}: {name}");
            }
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    /// Feed real Windows message handlers and render between packets. This
    /// catches skips hidden by tests of the pager or static screenshots alone.
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn touchpad_swipes_advance_continuously_without_queued_skips() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let settings = Settings { system_history: false, ..Settings::default() };
        let mut app = App::new(hwnd, helper, sink, rx, settings, Store::default(), Some("unused".into()), false, false).unwrap();
        app.layout_surface();
        app.expanded = true;
        app.mode = Mode::Expanded(PageKind::Media);
        app.st.page = PageKind::Media;
        app.island.snap(app.geom_for(&app.mode));
        let mut deferred = Vec::new();
        let wheel = |delta: i16| WPARAM(usize::from(delta as u16) << 16);
        for (index, theme) in [Theme::Classic, Theme::Pill].into_iter().enumerate() {
            let start = 10.0 + index as f64 * 10.0;
            app.st.settings.theme = theme;
            app.mode = Mode::Expanded(PageKind::Media);
            app.st.page = PageKind::Media;
            app.wheel_pager = Default::default();
            clock::hold(Some(start));
            app.clock.set_active(false);
            app.clock.set_divisor(4);
            app.handle_island(WM_MOUSEHWHEEL, wheel(60), LPARAM(0), &mut deferred);
            assert_eq!(app.st.page, PageKind::Focus, "{theme:?}: first packet must select the adjacent tab");
            assert_eq!(app.clock.pacing(), (true, 1), "input must escape ambient pacing immediately");
            app.frame(&mut deferred, None);
            assert_eq!(app.mode, Mode::Expanded(PageKind::Focus));
            assert!(app.page_anim.is_some());
            for (delta, elapsed) in [(120, 0.02), (120, 0.04), (80, 0.06), (40, 0.09), (24, 0.12), (12, 0.16), (8, 0.2), (4, 0.24)] {
                clock::hold(Some(start + elapsed));
                app.handle_island(WM_MOUSEHWHEEL, wheel(delta), LPARAM(0), &mut deferred);
                app.frame(&mut deferred, None);
                assert_eq!(app.st.page, PageKind::Focus, "{theme:?}: a short flick must not bank extra tabs");
            }
            let mut advances = 0;
            let mut last_advance = start;
            for i in 1..=36 {
                let now = start + 0.24 + f64::from(i) * 0.02;
                clock::hold(Some(now));
                let before = app.st.page;
                app.handle_island(WM_MOUSEHWHEEL, wheel(40), LPARAM(0), &mut deferred);
                // Diagonal packets must not form a second independent gesture.
                let mut pt = POINT { x: (app.cx * app.gfx.scale()) as i32, y: 10 };
                unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt).unwrap(); }
                let at = LPARAM(((pt.x as u16 as u32) | ((pt.y as u16 as u32) << 16)) as isize);
                app.handle_island(WM_MOUSEWHEEL, wheel(-120), at, &mut deferred);
                app.frame(&mut deferred, None);
                assert_eq!(app.mode, Mode::Expanded(app.st.page));
                if app.st.page != before {
                    assert_eq!(app.st.page, pages::step_tab(before, &app.st, true), "{theme:?}: every intermediate tab must be visited");
                    assert!(now - last_advance >= PAGE_SWITCH - 1e-8, "{theme:?}: repeating must allow each transition to finish");
                    last_advance = now;
                    advances += 1;
                }
            }
            assert!(advances >= 3, "{theme:?}: continuing the same swipe must move through multiple tabs");
            let stopped_on = app.st.page;
            clock::hold(Some(start + 1.5));
            app.frame(&mut deferred, None);
            assert_eq!(app.st.page, stopped_on, "no queued page changes after input stops");
            app.st.page = PageKind::Media;
            app.mode = Mode::Expanded(PageKind::Media);
            app.wheel_pager = Default::default();
            clock::hold(Some(start + 2.0));
            app.handle_island(WM_MOUSEHWHEEL, wheel(-60), LPARAM(0), &mut deferred);
            app.frame(&mut deferred, None);
            assert_eq!(app.mode, Mode::Expanded(PageKind::Media));
            for i in 1..=20 {
                clock::hold(Some(start + 2.0 + f64::from(i) * 0.02));
                app.handle_island(WM_MOUSEHWHEEL, wheel(-120), LPARAM(0), &mut deferred);
            }
            assert_eq!(app.st.page, PageKind::Media, "overscroll at the first tab must not wrap to the last");
            clock::hold(Some(start + 3.0));
            let screen_at = |y: i32| {
                let mut pt = POINT { x: (app.cx * app.gfx.scale()) as i32, y };
                unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt).unwrap(); }
                LPARAM(((pt.x as u16 as u32) | ((pt.y as u16 as u32) << 16)) as isize)
            };
            let header_at = screen_at(10);
            let content_at = screen_at((100.0 * app.gfx.scale()) as i32);
            app.handle_island(WM_MOUSEWHEEL, wheel(-60), header_at, &mut deferred);
            app.frame(&mut deferred, None);
            assert_eq!(app.mode, Mode::Expanded(PageKind::Focus), "a fresh vertical header swipe must respond immediately");
            clock::hold(Some(start + 4.0));
            app.handle_island(WM_MOUSEWHEEL, wheel(-120), content_at, &mut deferred);
            assert_eq!(app.st.page, PageKind::Focus, "scrolling page content must not switch tabs");
            assert_eq!(app.input.wheel, -1.0);
            app.input.end_frame();
        }
        clock::hold(None);
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    /// The window's centre in DIPs (any value: the geometry follows it).
    const CX: f32 = 336.6;
    const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

    fn island(g: Geom, floating: bool) -> Island {
        let mut i = Island::new(g);
        i.set_floating(floating.then_some(pill::TOP_GAP));
        i
    }

    fn pt(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    /// The pixel centre `over_shadow` tests for the pixel at (x, y) px.
    fn px_centre(x: i32, y: i32, s: f32) -> Point {
        pt((x as f32 + 0.5) / s, (y as f32 + 0.5) / s)
    }

    #[test]
    fn classic_hangs_from_the_edge() {
        let i = island(Geom::new(84.0, 30.0, 14.0), false);
        let body = i.body_rect_at(CX);
        assert_eq!(body, Rect::new(CX - 42.0, 0.0, 84.0, 30.0));
        assert_eq!(i.hover_rect_at(CX), body, "Classic hovers on its body");
        let side = SHOULDER + HIT_MARGIN;
        assert_eq!(i.canvas_rect_at(CX), Rect::new(CX - 42.0 - side, 0.0, 84.0 + 2.0 * side, 30.0 + HIT_MARGIN));
        assert_eq!((i.top(), i.side_room(), i.bottom_room()), (0.0, side, HIT_MARGIN));
    }

    #[test]
    fn pill_floats_below_the_edge() {
        let i = island(Geom::new(152.0, 32.0, 16.0), true);
        let (gap, side, below) = (pill::TOP_GAP, i.side_room(), i.bottom_room());
        assert!(i.floating() && i.top() == gap && side > 0.0 && below > 0.0, "room for the gap and the shadow");
        let body = i.body_rect_at(CX);
        assert_eq!(body, Rect::new(CX - 76.0, gap, 152.0, 32.0));
        assert_eq!(i.hover_rect_at(CX), Rect::new(CX - 76.0, 0.0, 152.0, gap + 32.0), "the gap above it and the body");
        assert_eq!(i.canvas_rect_at(CX), Rect::new(CX - 76.0 - side, 0.0, 152.0 + 2.0 * side, gap + 32.0 + below));
    }

    #[test]
    fn classic_masks_are_unchanged() {
        let i = island(Geom::new(84.0, 30.0, 14.0), false);
        let body = i.body_rect_at(CX);
        assert_eq!(hit_mask_for(&i, CX, false, false), HitMask { rect: None, notch: Some((body, 14.0, SHOULDER)), pill: None });
        // Hidden is Classic's own strip there, with its shoulders.
        assert_eq!(hit_mask_for(&i, CX, false, true), HitMask { rect: None, notch: Some((body, 14.0, SHOULDER)), pill: None });
        assert_eq!(hit_mask_for(&i, CX, true, false), HitMask { rect: Some(i.canvas_rect_at(CX)), notch: None, pill: None });
    }

    #[test]
    fn pill_takes_the_gap_not_its_shadow() {
        let i = island(Geom::new(152.0, 32.0, 16.0), true);
        let body = i.body_rect_at(CX);
        let m = hit_mask_for(&i, CX, false, false);
        assert_eq!(m.pill, Some((body, 16.0)));
        // A pointer thrown at the top edge: anywhere above the pill.
        for x in [body.x + 0.1, CX, body.right() - 0.1] {
            assert!(m.contains(pt(x, 0.0)), "top edge at x {x}");
            assert!(m.contains(pt(x, pill::TOP_GAP - 0.1)), "just above the pill at x {x}");
        }
        assert!(m.contains(pt(body.x + 0.5, body.y + 0.5)), "the top corners: the gap reaches the pill's middle");
        assert!(m.contains(pt(CX, body.bottom() - 0.5)), "the pill's bottom edge");
        // Beside it at the top edge, beside and below it (its shadow): through.
        assert!(!m.contains(pt(body.x - 0.5, 0.0)) && !m.contains(pt(body.right() + 0.5, 0.0)));
        for d in [0.5, 3.0, 8.0] {
            assert!(!m.contains(pt(body.x - d, body.center().y)), "beside, {d} out");
            assert!(!m.contains(pt(body.right() + d, body.center().y)), "beside, {d} out");
            assert!(!m.contains(pt(CX, body.bottom() + d)), "below, {d} down");
        }
        assert!(!m.contains(pt(body.x + 1.0, body.bottom() - 1.0)), "the round bottom corners");
    }

    #[test]
    fn pill_open_takes_the_card_its_margin_and_the_gap() {
        let i = island(Geom::new(620.0, 216.0, 40.0), true);
        let canvas = i.canvas_rect_at(CX);
        let m = hit_mask_for(&i, CX, true, false);
        assert_eq!(m, HitMask { rect: Some(canvas), notch: None, pill: None });
        let body = i.body_rect_at(CX);
        assert!(m.contains(pt(CX, 0.0)), "the gap above the card");
        assert!(m.contains(pt(body.x - i.side_room() + 0.5, body.center().y)), "the margin beside it");
        assert!(m.contains(pt(CX, body.bottom() + i.bottom_room() - 0.5)), "the margin below it");
        assert!(!m.contains(pt(canvas.right() + 0.5, body.center().y)) && !m.contains(pt(CX, canvas.bottom() + 0.5)));
    }

    #[test]
    fn hidden_pill_takes_a_band_at_the_edge() {
        // Classic's hidden strip (150 x 5, drawn at least 8 tall), floating:
        // the mouse finds it at the top edge in a band as tall as Classic's,
        // not down the gap and the invisible body under it.
        let i = island(Geom::new(150.0, 5.0, 2.5), true);
        let classic = island(Geom::new(150.0, 5.0, 2.5), false).body_rect_at(CX);
        let body = i.body_rect_at(CX);
        let m = hit_mask_for(&i, CX, false, true);
        assert_eq!(m, HitMask { rect: Some(Rect::new(classic.x, 0.0, classic.w, classic.h)), notch: None, pill: None });
        assert!(m.contains(pt(CX, 0.0)) && m.contains(pt(body.x + 0.1, classic.h - 0.1)));
        assert!(!m.contains(pt(CX, classic.h)) && !m.contains(pt(CX, body.center().y)), "below the band, the invisible body included");
    }

    #[test]
    fn pixels_at_every_scale() {
        // What over_shadow sees, pixel by pixel, at 100 / 125 / 150 / 200 %:
        // every pixel row of the gap above the pill takes the mouse, the
        // pixels beside and under the pill don't.
        let i = island(Geom::new(84.0, 32.0, 16.0), true);
        let body = i.body_rect_at(CX);
        let m = hit_mask_for(&i, CX, false, false);
        for s in SCALES {
            let x = (CX * s) as i32;
            let top_px = (pill::TOP_GAP * s).round() as i32;
            assert_eq!(top_px as f32, pill::TOP_GAP * s, "the gap is whole pixels at {s}");
            for y in 0..top_px {
                assert!(m.contains(px_centre(x, y, s)), "gap row {y} at {s}");
            }
            let mid = ((body.y + body.h / 2.0) * s) as i32;
            let left = (body.x * s).floor() as i32 - 1;
            let right = (body.right() * s).ceil() as i32;
            assert!(!m.contains(px_centre(left, mid, s)) && !m.contains(px_centre(right, mid, s)), "beside at {s}");
            assert!(m.contains(px_centre(left + 2, mid, s)) && m.contains(px_centre(right - 2, mid, s)), "inside the ends at {s}");
            let below = (body.bottom() * s).ceil() as i32;
            assert!(!m.contains(px_centre(x, below, s)) && m.contains(px_centre(x, below - 1, s)), "the bottom edge at {s}");
            assert!(m.contains(px_centre(x, 0, s)), "the top edge at {s}");
        }
    }

    #[test]
    fn window_room_follows_the_theme_and_comes_back() {
        // A theme switch lays the window out again: Pill grows it for the gap
        // (and needs less room beside: no shoulders), Classic gets its exact
        // size back.
        let mut i = island(Geom::new(84.0, 30.0, 14.0), false);
        for s in SCALES {
            for mon_w in [1920, 1366, 2560, 3841] {
                let classic = surface_size(&i, 620.0, 400.0, s, mon_w);
                // The formula Classic has always laid out with.
                let mut w = ((620.0 * OVERSHOOT + 2.0 * (SHOULDER + HIT_MARGIN)) * s).ceil() as i32;
                if (mon_w - w) % 2 != 0 {
                    w += 1;
                }
                assert_eq!(classic, (w, ((400.0 * OVERSHOOT + HIT_MARGIN) * s).ceil() as i32), "Classic at {s} on {mon_w}");
                assert_eq!((mon_w - classic.0) % 2, 0, "centred on whole pixels");

                i.set_floating(Some(pill::TOP_GAP));
                let floating = surface_size(&i, 620.0, 400.0, s, mon_w);
                let want_h = ((400.0 * OVERSHOOT + i.bottom_room() + pill::TOP_GAP) * s).ceil() as i32;
                assert_eq!(floating.1, want_h, "Pill has room for the gap at {s}");
                assert_eq!((mon_w - floating.0) % 2, 0);
                i.set_floating(None);
                assert_eq!(surface_size(&i, 620.0, 400.0, s, mon_w), classic, "back to Classic at {s} on {mon_w}");
            }
        }
    }
}

#[cfg(test)]mod ai_native_tests {
    use super::*;
    #[test]
    #[ignore="requires a Windows desktop"]
    fn ready_reply_overrides_music_focus_and_compact_action_stays_open() {
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();
        let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;
        let mut deferred=vec![];
        let point=|p:Point,s:f32|LPARAM(((((p.y*s).round() as i32 as u16) as usize)<<16|((p.x*s).round() as i32 as u16) as usize) as isize);
        for (ti,theme) in [Theme::Classic,Theme::Pill].into_iter().enumerate() {
            for activity in 0..3 {
                let base=1500.0+ti as f64*300.0+activity as f64*80.0;clock::hold(Some(base));
                let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;
                app.st.settings.theme=theme;app.st.settings.collapse_delay_ms=0;app.st.settings.open_on_hover=false;
                app.st.notice=None;app.st.hud=None;app.st.pinned=false;app.st.aside=false;app.fullscreen_on=false;app.widgets_open=false;
                app.st.media=if activity!=1 {Some(crate::state::MediaSession{app_id:"Spotify.exe".into(),app_name:"Spotify".into(),music_source:Some(lumo_core::nowplaying::MusicSource::Streaming),title:"Fixture".into(),artist:"Artist".into(),album:"Album".into(),art:None,art_color:None,timeline:lumo_core::timeline::Timeline::new(20.0,400.0,base,true),can_prev:true,can_next:true,can_play_pause:true,can_seek:true,track_key:"fixture".into()})}else{None};
                app.st.pomodoro.run=if activity!=0 {lumo_core::pomodoro::RunState::Running{ends_at:base+1500.0}}else{lumo_core::pomodoro::RunState::Idle};
                if activity==1 {app.st.ai.snapshot();app.st.ai.current=lumo_core::ai::Chat{id:77,..Default::default()};}
                app.st.page=if activity==1 {PageKind::Focus}else{PageKind::Media};app.expanded=true;app.hovering=true;app.mode=Mode::Expanded(app.st.page);
                app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
                app.st.ai.busy=true;app.st.ai.request=1;app.st.ai.running_chat=1;
                app.on_event(AppEvent::Ai(crate::ai::Event::Reply{request:1,chat:1,result:Ok(lumo_core::ai::Message{role:"assistant".into(),text:"# Answer ready\nThe result is here.".into(),..Default::default()}),images:vec![]}));
                assert!(app.st.notice.as_ref().unwrap().is_ai_reply(),"another open page must not suppress a ready reply");
                assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::Play(_))));app.st.cmds.clear();
                app.collapse();app.hovering=false;app.input.mouse=None;
                app.fullscreen_on=true;app.st.settings.hide_in_fullscreen=true;app.apply_fullscreen(base);
                clock::hold(Some(base+30.0));app.st.now=base+30.0;app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
                assert_eq!(app.mode,Mode::Notice);assert!(app.st.ai.ready_reply.is_some());assert!(!app.fullscreen_hides(app.st.now));
                if activity==0 {
                    let preview=app.st.ai.ready_reply.as_ref().unwrap().preview.clone();
                    app.run_inline_action(InlineAction::Hide);
                    assert!(app.st.ai.ready_reply.is_none() && app.st.notice.is_none() && !app.expanded);
                    assert!(app.fullscreen_hides(app.st.now),"dismiss returns to the normal visibility policy");
                    app.st.ai.ready_reply=Some(crate::ai::ReadyReply{chat:1,preview});crate::ai::announce_ready(&mut app.st);app.apply_fullscreen(app.st.now);
                }
                if let Some(root)=std::env::var_os("LUMO_AI_QA") {let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();let(w,h,p,_) = app.selftest_capture();crate::png::save_rgba(root.join(format!("reply-ready-{theme:?}-{activity}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
                // A timer/status notice can show briefly without losing the reply.
                app.st.notify(Icon::Timer,palette::ORANGE,"Focus status","Fixture",1.0,Some(NoticeAction::OpenPage(PageKind::Focus)));
                clock::hold(Some(base+32.0));app.st.now=base+32.0;app.tick();assert!(app.st.notice.as_ref().unwrap().is_ai_reply());
                app.widgets_open=true;app.apply_fullscreen(app.st.now);assert!(app.fullscreen_hides(app.st.now));
                clock::hold(Some(base+50.0));app.st.now=base+50.0;app.tick();assert!(app.st.ai.ready_reply.is_some());
                app.widgets_open=false;app.apply_fullscreen(app.st.now);app.frame(&mut deferred,None);app.island.snap(app.geom_for(&Mode::Notice));app.frame(&mut deferred,None);
                let body=app.island.body_rect_at(app.cx);let at=Point::new(body.x+70.0,body.center().y);let scale=app.gfx.scale();
                app.handle_island(WM_MOUSEMOVE,WPARAM(0),point(at,scale),&mut deferred);
                app.handle_island(WM_LBUTTONDOWN,WPARAM(0),point(at,scale),&mut deferred);
                assert!(app.expanded);assert_eq!(app.st.page,PageKind::Ai);assert_eq!(app.st.ai.current.id,1,"the notice opens the completed chat");assert!(app.st.ai.ready_reply.is_none());
                app.handle_island(WM_LBUTTONUP,WPARAM(0),point(at,scale),&mut deferred);app.st.cmds.clear();app.frame(&mut deferred,None);
                assert_eq!(app.st.page,PageKind::Ai);assert!(!app.fullscreen_hides(app.st.now),"the explicitly opened reply remains visible");
                // A Compact action under the old bottom edge must not dismiss
                // the island when Windows reports leaving the smaller panel.
                app.resize_ai_chat(app.st.ai.max_island_height,false);app.island.snap(app.geom_for(&Mode::Expanded(PageKind::Ai)));app.frame(&mut deferred,None);
                let old=app.island.body_rect_at(app.cx);app.input.mouse=Some(Point::new(old.center().x,old.bottom()-30.0));
                app.st.cmd(Command::AiHeight(240.0));app.run_commands(&mut deferred);
                app.handle_island(WM_MOUSELEAVE,WPARAM(0),LPARAM(0),&mut deferred);app.on_timer(TIMER_COLLAPSE);
                app.island.snap(app.geom_for(&Mode::Expanded(PageKind::Ai)));app.frame(&mut deferred,None);
                assert!(app.expanded && app.ai_resize_hold.is_some(),"compacting keeps the smaller chat open");
                let body=app.island.body_rect_at(app.cx);let at=Point::new(body.center().x,body.y+20.0);
                app.input.mouse=Some(at);app.frame(&mut deferred,None);
                assert!(app.ai_resize_hold.is_some() && app.expanded,"cached coordinates cannot release a compacting hold");
                app.handle_island(WM_MOUSEMOVE,WPARAM(0),point(at,scale),&mut deferred);assert!(app.ai_resize_hold.is_none());
                app.handle_island(WM_MOUSELEAVE,WPARAM(0),LPARAM(0),&mut deferred);app.frame(&mut deferred,None);assert!(!app.expanded);
                app.fullscreen_on=false;app.apply_fullscreen(app.st.now);app.st.cmds.clear();
                if activity==0 {
                    let popup=crate::ai_window::AiWindow::new().unwrap();let popup_hwnd=popup.hwnd;
                    app.ai_window=Some(popup);app.st.ai.full_open=true;app.hovering=false;app.input.mouse=None;
                    app.handle_ai_window(popup_hwnd,WM_CLOSE,WPARAM(0),LPARAM(0));app.st.cmds.clear();app.on_event(AppEvent::Foreground);app.on_timer(TIMER_COLLAPSE);
                    app.frame(&mut deferred,None);assert!(app.expanded && app.st.page==PageKind::Ai && app.ai_resize_hold.is_some(),"returning from the popup keeps the island open");
                    app.collapse();
                }
            }
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }
    #[test]#[ignore="requires a Windows desktop"]
    fn markdown_hover_actions_ctrl_shortcut_and_mid_screen_resize(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        for (index,theme)in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            clock::hold(Some(800.0+index as f64*20.0));let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.ai.current.messages[1].text=crate::ai_window::markdown_fixture().into();app.st.settings.ai=sample.settings.ai;app.st.settings.theme=theme;app.st.page=PageKind::Ai;app.st.pinned=true;app.st.notice=None;app.expanded=true;app.mode=Mode::Expanded(PageKind::Ai);app.hovering=true;app.input.mouse=None;app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            assert!(!app.hits.iter().any(|h|h.id==wid("ai.copy.1")),"message actions stay hidden without hover");
            app.st.cmd(Command::AiHeight(app.st.ai.max_island_height));app.run_commands(&mut deferred);app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            let body=app.island.body_rect_at(app.cx);let middle=app.monitor.work.top+(app.monitor.work.bottom-app.monitor.work.top)/2;let bottom=app.surface.y+(body.bottom()*app.gfx.scale()).round() as i32;
            if app.st.ai.max_island_height>245.0{assert!(bottom<=middle+2,"expanded chat must stop at the desktop middle: {bottom} > {middle}");}
            let area=pages::content_area(body);app.input.mouse=Some(Point::new(area.x+110.0,area.y+145.0));app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==wid("ai.resize-grip")));
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();let(w,h,p,_) = app.selftest_capture();crate::png::save_rgba(root.join(format!("markdown-expanded-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.st.ai.scroll=f32::MAX;app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==wid("ai.copy.1") && h.rect.w<=34.0));assert!(app.hits.iter().any(|h|h.id==wid("ai.markdown.1")));
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let root=std::path::PathBuf::from(root);let(w,h,p,_) = app.selftest_capture();crate::png::save_rgba(root.join(format!("markdown-actions-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            // Dragging is clamped, and the next closed state releases the larger surface.
            app.st.cmd(Command::AiHeight(100000.0));app.run_commands(&mut deferred);assert_eq!(app.st.ai.island_height,app.st.ai.max_island_height);
            if app.st.ai.max_island_height > 245.0 {
                // Reproduce a grip outrunning the window mask, including a
                // mouse-up delivered before the final mouse-move can paint.
                for (start_height,dy) in [(240.0,(app.st.ai.max_island_height-240.0).min(70.0)),(app.st.ai.max_island_height,240.0-app.st.ai.max_island_height)] {
                    app.st.pinned=false;app.hovering=true;app.resize_ai_chat(start_height,true);app.frame(&mut deferred,None);
                    let start=app.hits.iter().find(|h|h.id==wid("ai.resize-grip")).unwrap().rect.center();
                    let point=|p:Point,s:f32|LPARAM(((((p.y*s).round() as i32 as u16) as usize)<<16|((p.x*s).round() as i32 as u16) as usize) as isize);
                    let scale=app.gfx.scale();
                    app.handle_island(WM_LBUTTONDOWN,WPARAM(0),point(start,scale),&mut deferred);if dy>0.0{app.frame(&mut deferred,None);}
                    app.handle_island(WM_MOUSELEAVE,WPARAM(0),LPARAM(0),&mut deferred);
                    assert!(app.input.mouse.is_some(),"capture retains the drag pointer");
                    let end=Point::new(start.x,start.y+dy);
                    app.handle_island(WM_MOUSEMOVE,WPARAM(0),point(end,scale),&mut deferred);
                    app.handle_island(WM_MOUSELEAVE,WPARAM(0),LPARAM(0),&mut deferred);
                    app.handle_island(WM_LBUTTONUP,WPARAM(0),point(end,scale),&mut deferred);
                    if app.gfx.needs_hit_mask(){assert_eq!(app.mask.map(|(mask,_)|mask),Some(app.hit_mask()),"release presents the final input mask before capture ends");}
                    app.frame(&mut deferred,None);
                    assert!(app.expanded && app.hovering,"resizing {dy} must not dismiss the chat");
                    assert!((app.st.ai.island_height-(start_height+dy)).abs()<1.0,"final pointer position is committed without a paint");
                    assert!(app.st.ai.resize_dragged);
                }
            }
            app.st.pinned=false;app.collapse();app.mode=Mode::Idle;app.st.page=PageKind::Media;app.st.media=None;app.st.notice=None;app.st.hud=None;app.st.settings.idle_style=IdleStyle::Clock;app.st.settings.open_on_hover=false;app.island.snap(app.geom_for(&Mode::Idle));app.hovering=true;app.input.ctrl=false;app.input.mouse=Some(app.island.body_rect_at(app.cx).center());app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.mode));app.input.mouse=Some(app.island.body_rect_at(app.cx).center());app.frame(&mut deferred,None);
            assert!(!app.hits.iter().any(|h|h.id==wid("ai.shortcut")));assert_eq!(app.st.ai.canvas_height,240.0);assert!(app.ai_ctrl_watch.is_some());
            app.locked=true;app.frame(&mut deferred,None);assert!(app.ai_ctrl_watch.is_none());app.locked=false;app.frame(&mut deferred,None);assert!(app.ai_ctrl_watch.is_some());
            app.input.ctrl=true;app.frame(&mut deferred,None);assert!(app.hits.iter().any(|h|h.id==wid("ai.shortcut")),"Ctrl+hover reveals Ask AI");
            // Releasing Ctrl before a queued click cannot activate a stale shortcut hit.
            app.input.ctrl=false;let p=app.island.body_rect_at(app.cx).center();let lp=LPARAM((((p.y*app.gfx.scale()).round() as u32 as usize)<<16 | (p.x*app.gfx.scale()).round() as u16 as usize) as isize);app.handle_island(WM_LBUTTONDOWN,WPARAM(0),lp,&mut deferred);assert_eq!(app.st.page,PageKind::Media);assert!(app.expanded);assert!(app.ai_ctrl_watch.is_none());app.handle_island(WM_LBUTTONUP,WPARAM(0),lp,&mut deferred);app.collapse();app.hovering=false;app.ai_ctrl_watch=None;
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a Windows desktop"]
    fn companion_renders_in_both_themes_and_quietly_notifies_over_fullscreen(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;app.layout_surface();let mut deferred=vec![];
        for (i,theme)in [Theme::Classic,Theme::Pill].into_iter().enumerate(){let base=500.0+i as f64*20.0;clock::hold(Some(base));let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.theme=theme;app.st.page=PageKind::Ai;app.st.aside=false;app.st.notice=None;app.expanded=true;app.st.pinned=true;app.mode=Mode::Expanded(PageKind::Ai);app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==wid("ai.prompt")));assert!(app.hits.iter().any(|h|h.id==wid("ai.send")));
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();let(w,h,pixels,_)=app.selftest_capture();crate::png::save_rgba(&root.join(format!("compact-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
            app.st.ai.model_menu=true;app.frame(&mut deferred,None);let model=app.hits.iter().find(|h|h.id==crate::ui::wid_n("ai.model",0)).unwrap().rect;app.input.mouse=Some(model.center());app.frame(&mut deferred,None);assert_eq!(pages::ai::next_wake(&app.st),Some(app.st.now+1.0));clock::hold(Some(base+1.1));app.frame(&mut deferred,None);assert!(pages::ai::next_wake(&app.st).is_none());app.st.ai.model_menu=false;
            app.st.ai.model_menu=true;app.st.ai.attach_menu=true;app.collapse();assert!(!app.st.ai.model_menu && !app.st.ai.attach_menu,"closing the island dismisses transient popovers");app.expand();app.island.snap(app.geom_for(&Mode::Expanded(PageKind::Ai)));app.frame(&mut deferred,None);assert!(!app.st.ai.model_menu,"reopening returns to chat");
            app.st.ai.attaching=true;app.st.pinned=false;app.hovering=false;assert!(!app.free_to_collapse(),"a live file picker holds its owning island open");app.st.ai.attaching=false;app.st.pinned=true;
            app.input.clicked=Some((wid("ai.plus"),MouseButton::Left));app.frame(&mut deferred,None);assert!(app.st.ai.attach_menu);
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let(w,h,pixels,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(root).join(format!("compact-attachments-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
            app.st.ai.close_popovers();
            app.st.ai.models[0].reasoning=lumo_core::ai_reasoning::Reasoning::parse(&serde_json::json!({"reasoning":{"supported_efforts":["low","medium","high"],"default_effort":"medium"}})).unwrap();
            app.input.clicked=Some((wid("ai.models"),MouseButton::Left));app.frame(&mut deferred,None);
            app.frame(&mut deferred,None);
            assert!(app.st.ai.model_menu && !app.st.ai.reasoning_menu);
            let picker=app.hits.iter().find(|h|h.id==wid("ai.model-surface")).unwrap().rect;
            assert!(picker.y>=app.island.body_rect_at(app.cx).bottom(),"the compact model picker opens below the island");
            assert!(picker.h>=240.0 && picker.w>=400.0,"the picker must have room to browse");
            assert!(app.hit_mask().contains(picker.center()),"the entire detached picker receives input");
            assert!(!app.hit_mask().contains(Point::new(app.island.body_rect_at(app.cx).x + 4.0, picker.center().y)),"desktop beside the picker stays click-through");
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let(w,h,pixels,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(root).join(format!("compact-models-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
            app.input.clicked=Some((crate::ui::wid_n("ai.model",0),MouseButton::Left));app.frame(&mut deferred,None);app.frame(&mut deferred,None);
            assert!(app.st.ai.model_menu && app.st.ai.reasoning_menu,"a starred model opens its effort controls below compact chat");
            assert!(app.hits.iter().any(|h|h.id==wid("ai.effort-slider")),"reselecting the active model restores the original effort slider");
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let(w,h,pixels,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(root).join(format!("compact-effort-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
            app.input.clicked=Some((wid("ai.effort-model"),MouseButton::Left));app.frame(&mut deferred,None);app.frame(&mut deferred,None);
            assert!(app.st.ai.model_menu && !app.st.ai.reasoning_menu);
            let point=|p:Point,s:f32|LPARAM(((((p.y*s).round() as u16) as usize)<<16 | (p.x*s).round() as u16 as usize) as isize);
            app.st.pinned=false;app.blur_text();app.ai_resize_hold=None;
            app.handle_island(WM_MOUSELEAVE,WPARAM(0),LPARAM(0),&mut deferred);
            let bridge=Point::new(picker.center().x,picker.y-4.0);
            app.handle_island(WM_MOUSEMOVE,WPARAM(0),point(bridge,app.gfx.scale()),&mut deferred);
            app.frame(&mut deferred,None);
            assert!(app.expanded && app.hovering,"moving across the picker gap must not collapse chat");
            app.input.clicked=Some((wid("ai.model-filters"),MouseButton::Left));app.frame(&mut deferred,None);
            assert!(app.st.ai.model_filters_open);
            assert!([1,2,4,8].into_iter().all(|mask|app.hits.iter().any(|h|h.id==crate::ui::wid_n("ai.model-filter",mask))));
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let(w,h,pixels,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(root).join(format!("compact-filters-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
            let draft="Keep this unfinished message";app.st.ai.draft=draft.into();
            let send=app.hits.iter().find(|h|h.id==wid("ai.send")).unwrap().rect.center();
            let lp=point(send,app.gfx.scale());
            app.handle_island(WM_LBUTTONDOWN,WPARAM(0),lp,&mut deferred);app.frame(&mut deferred,None);
            assert!(!app.st.ai.model_menu && app.expanded && app.input.pressed_id.is_none(),"clicking chat dismisses the picker and consumes the press");
            app.handle_island(WM_LBUTTONUP,WPARAM(0),lp,&mut deferred);app.frame(&mut deferred,None);
            assert_eq!(app.st.ai.draft,draft,"dismissing above Send must never submit the message");
            app.st.pinned=true;app.st.ai.model_menu=true;app.frame(&mut deferred,None);
            app.handle_island(WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0),&mut deferred);assert!(app.expanded && !app.st.ai.model_menu,"Escape dismisses the picker without closing chat");
            let popup=crate::ai_window::AiWindow::new().unwrap();let popup_hwnd=popup.hwnd;app.ai_window=Some(popup);app.st.ai.full_open=true;app.st.ai.model_menu=true;
            app.handle_ai_window(popup_hwnd,WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0));assert!(app.ai_window.is_some() && !app.st.ai.model_menu,"Escape keeps the full chat open");app.ai_window=None;app.st.ai.full_open=false;
            app.st.ai.busy=true;app.st.ai.request=8;app.st.ai.running_chat=1;app.expanded=false;app.st.pinned=false;app.fullscreen_on=true;app.st.settings.hide_in_fullscreen=true;app.st.notice=None;assert!(app.fullscreen_hides(app.st.now));app.on_event(AppEvent::Ai(crate::ai::Event::Reply{request:8,chat:1,result:Ok(lumo_core::ai::Message{role:"assistant".into(),text:"Ready".into(),..Default::default()}),images:vec![]}));assert!(!app.fullscreen_hides(app.st.now));assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::Play(_))));app.widgets_open=true;assert!(app.fullscreen_hides(app.st.now));app.widgets_open=false;clock::hold(Some(base+10.0));app.st.now=base+10.0;app.tick();assert!(!app.fullscreen_hides(app.st.now));crate::ai::clear_ready(&mut app.st);assert!(app.fullscreen_hides(app.st.now));app.fullscreen_on=false;app.apply_fullscreen(app.st.now);
            // Do not execute fixture persistence commands against the user's profile.
            app.st.cmds.clear();
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }
}
