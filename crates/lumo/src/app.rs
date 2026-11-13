//! The app: owns the island window, state, renderer and services; routes
//! window messages, service events and page commands.

use crate::activities::{self, Mode};
use crate::clock;
use crate::commands::{Command, MediaCmd, PhoneCmd};
use crate::events::{AppEvent, AppleMusicStatus, EventSink, SendHwnd, WM_APP_APPBAR, WM_APP_EVENT, WM_APP_FRAME, WM_APP_TRAY};
use crate::gfx::{px_rect, px_union, scale_about, union_rect, Canvas, Gfx, HitMask};
use crate::icons::Icon;
use crate::island::{Geom, Island, SHOULDER};
use crate::pages::{self, PageKind};
use crate::paths;
use crate::pill;
use crate::services::{self, Services};
use crate::state::{
    AppState, FpsAccess, HudKind, InlineAction, LyricsState, LyricsStatus, NoticeAction, PhoneAsk, PhoneDrop,
    PhoneTransfer,
};
use crate::tray::{self, Tray};
use crate::ui::{wid, CursorKind, Hit, Input, KeyInput, MouseButton, Ui};
use crate::window::{self, FrameClock, FrameTick, MonitorInfo};
use crate::{dragdrop, selftest, shell};
use lumo_core::addons::Status as AddonStatus;
use lumo_core::frametime::{ambient_divisor, MotionStats};
#[cfg(test)]#[path="closed_checks.rs"]mod closed_checks;
use lumo_core::geom::{palette, Color, Point, Rect};
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
/// Reconcile a deactivation delivered inside a nested native message loop.
const WM_SEARCH_ACTIVATION: u32 = WM_APP + 97;

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
    BrowserMenu(POINT),
    SearchMenu(lumo_core::search::Entry, POINT),
    DragOut(Vec<String>),
    Menu(POINT),
}

pub fn main_entry() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|s|s=="--usage-statusline"){return services::usage_cleanup::retired_statusline(args.get(3).map_or("",String::as_str));}
    if args.get(1).is_some_and(|s|s=="--disconnect-usage"){return services::usage_cleanup::restore_all();}
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

    // One-time cleanup before old account preferences are discarded on save.
    let _=services::usage_cleanup::restore_all();
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
pub(crate) fn note_missed(msg: u32, wp: WPARAM, lp: LPARAM) -> bool {
    match msg {
        WM_APP_FRAME => {
            FRAME_MISSED.with(|f| f.set(true));
            true
        }
        services::workspace_dismiss::OUTSIDE_PRESS | WM_APP_EVENT | WM_SEARCH_ACTIVATION | crate::ai_window::CHECK_ACTIVATION | WM_MOUSELEAVE | WM_TIMER | WM_LBUTTONUP | WM_APP_TRAY | WM_APP_APPBAR | services::ai_shortcut::MODIFIER_CHANGED => {
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
    if let Some(result)=crate::search_menu::message(msg,wp,lp){return result;}
    let mut deferred = Vec::new();
    let res = with_app(|app| app.handle_island(msg, wp, lp, &mut deferred));
    for d in deferred {
        run_deferred(d);
    }
    match res {
        Some(Some(r)) => r,
        Some(None) => DefWindowProcW(hwnd, msg, wp, lp),
        None if msg==WM_ACTIVATE && (wp.0&0xffff) as u32==WA_INACTIVE=>{
            // SetFocus, native menus and WebView calls can dispatch messages
            // while App is already borrowed. Check the final foreground after
            // that call returns instead of dropping the outside activation.
            let _=PostMessageW(hwnd,WM_SEARCH_ACTIVATION,WPARAM(0),LPARAM(0));
            DefWindowProcW(hwnd,msg,wp,lp)
        },
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
        Deferred::BrowserMenu(point)=>{
            let Some((owner,keep,resume,forward,detached,home))=with_app(|app|(app.browser_window.as_ref().map_or(app.hwnd,|w|w.hwnd),app.st.settings.search.keep_open,app.st.settings.search.resume_page,app.st.search.navigation.forward,app.st.search.detached,crate::browser_home::is_home(&app.st)))else{return;};
            let action=crate::search_menu::show_browser(owner,point,keep,resume,forward,detached,home);
            with_app(|app|{match action{
                Some(crate::search_menu::BrowserAction::Forward)=>app.search_web_action(crate::search_web::Action::Forward),
                Some(crate::search_menu::BrowserAction::External)=>{if !crate::browser_home::is_home(&app.st){app.st.cmd(Command::OpenUrl(app.st.search.navigation.url.clone()));}},
                Some(crate::search_menu::BrowserAction::Keep)=>app.st.cmd(Command::ToggleSearchKeepOpen),
                Some(crate::search_menu::BrowserAction::Resume)=>app.st.cmd(Command::ToggleSearchResume),
                Some(crate::search_menu::BrowserAction::Filters)=>app.st.cmd(Command::OpenAdBlocking),
                Some(crate::search_menu::BrowserAction::Close)=>app.st.cmd(if app.st.search.detached{Command::CloseMiniBrowser}else{Command::Collapse}),None=>{}
            }app.request_frame();});
        },
        Deferred::SearchMenu(entry,point)=>{
            let Some((hwnd,frequent,pinned))=with_app(|app|(app.hwnd,app.st.store.search_usage.contains_key(&entry.id),app.st.store.search_pins.contains(&entry.id)))else{return;};
            let action=crate::search_menu::show(hwnd,&entry,point,frequent,pinned);
            with_app(|app|{
                app.st.search.context_menu=false;
                match action{
                    crate::search_menu::Action::Execute=>app.execute_search(entry.target.clone()),
                    crate::search_menu::Action::Invoked(opened)=>{if opened{app.record_search(&entry);}app.close_search();},
                    crate::search_menu::Action::Copy(text)=>services::clipboard::set_text(hwnd,&text),
                    crate::search_menu::Action::CopyIcon(file)=>{if !app.st.search.action_pending{if let lumo_core::search::Target::App(application)=&entry.target{app.st.search.action_pending=true;services::imaging::copy_application_icon(application.clone(),entry.title.clone(),file,app.sink.clone());}}},
                    crate::search_menu::Action::Error(message)=>app.st.search.notice=message,
                    crate::search_menu::Action::Pin(pin)=>{if lumo_core::search::set_pinned(&mut app.st.store.search_pins,&entry,pin){app.st.save();crate::search::refresh(&mut app.st);}else if pin&&app.st.store.search_pins.len()>=lumo_core::search::MAX_PINS{app.st.search.notice="Up to 32 apps can be pinned.".into();}},
                    crate::search_menu::Action::Forget=>{app.st.store.search_usage.remove(&entry.id);app.st.save();crate::search::refresh(&mut app.st);},
                    crate::search_menu::Action::Service(start)=>{if let lumo_core::search::Target::Service(service)=&entry.target{app.search_service(&service.name,start);}},
                    crate::search_menu::Action::None=>{},
                }
                if app.st.search.active{unsafe{let foreground=GetForegroundWindow();if foreground==hwnd || IsChild(hwnd,foreground).as_bool(){let _=SetFocus(hwnd);}else{app.search_lost_activation(foreground);}}}
                app.request_frame();
            });
        },
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
    wheel_input: Option<services::wheel_input::Watch>,
    workspace_dismiss: Option<services::workspace_dismiss::Watch>,
    ai_web:Option<crate::search_web::Browser>,
    ai_web_resting:bool,
    ai_window: Option<crate::ai_window::AiWindow>,
    settings_window: Option<crate::settings_window::SettingsWindow>,
    search_gesture: lumo_core::search::Gesture,
    search_web: Option<crate::search_web::Browser>,
    browser_window: Option<crate::browser_window::BrowserWindow>,
    browser_resting: bool,
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
    drag_owner: Option<HWND>,
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
    sysmon_compact: bool,
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
    /// Input-only pacing; never schedules a delayed track change.
    music_wheel: lumo_core::navigation::WheelPager,
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
    focus_light: Option<crate::focus_light::Spotlight>,
    focus_light_preview: bool,
    /// The Shelf paths last handed to it (to notice changes).
    phone_offer: Vec<String>,
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
            wheel_input: None,
            workspace_dismiss: None,
            ai_window: None,
            ai_web:None,
            ai_web_resting:false,
            settings_window: None,
            search_gesture: Default::default(),
            search_web: None,
            browser_window: None,
            browser_resting: false,
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
            drag_owner: None,
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
            sysmon_compact: false,
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
            music_wheel: Default::default(),
            locked: false,
            display_off: false,
            tray_retries: 0,
            outgoing: None,
            page_anim: None,
            hover_arm: None,
            collapse_due: false,
            left_at: f64::NEG_INFINITY,
            phone: None,
            focus_light: None,
            focus_light_preview: false,
            phone_offer: Vec::new(),
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
        self.focus_light = None;
        self.close_search();
        self.close_search_web();
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
        let middle=self.monitor.work.bottom;
        self.st.ai.max_island_height=(((middle-self.monitor.top()) as f32/self.gfx.scale())-32.0-if pill::on(self.st.settings.theme){pill::TOP_GAP}else{0.0}).max(240.0);
        self.st.ai.island_height=self.st.ai.island_height.min(self.st.ai.max_island_height);
        let tallest = PageKind::ALL.iter().map(|p| if *p==PageKind::Ai{if self.st.page==PageKind::Ai{self.st.ai.canvas_height.max(self.st.ai.island_height)}else{0.0}}else{pages::content_height(*p, &self.st)}).fold(0.0f32, f32::max);
        let tallest = if self.ai_picker_room { tallest.max(240.0 + 8.0 + 400.0) } else { tallest };
        let search = self.st.search.view.is_some();
        self.layout_surface_for(if search || self.st.page==PageKind::Ai { self.st.settings.expanded_width.max(crate::search::WIDTH) } else { self.st.settings.expanded_width },
            (pages::HEADER_H + tallest + pages::PAD).max(if search {crate::search::HEIGHT} else {0.0}));
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
            let tallest = PageKind::ALL.iter().filter(|p|**p!=PageKind::Ai||self.st.page==PageKind::Ai).map(|p| pages::content_height(*p, &self.st)).fold(0.0f32, f32::max);
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
            services::workspace_dismiss::OUTSIDE_PRESS=>{self.search_lost_activation(HWND(lp.0 as _));Some(LRESULT(0))},
            WM_SEARCH_ACTIVATION=>{let next=unsafe{GetForegroundWindow()};if !next.is_invalid(){self.search_lost_activation(next);}Some(LRESULT(0))},
            crate::ai_window::CHECK_ACTIVATION=>{self.handle_ai_window(HWND(lp.0 as *mut _),crate::ai_window::CHECK_ACTIVATION,WPARAM(0),LPARAM(0));Some(LRESULT(0))},
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
            services::ai_shortcut::MODIFIER_CHANGED => {self.input.ctrl=services::ai_shortcut::control_down();self.request_frame();Some(LRESULT(0))}
            WM_TIMER => {
                self.on_timer(wp.0);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                if self.wheel_input.is_none() && !self.locked && !self.display_off && self.st.settings.mouse_wheel_shortcuts {
                    self.wheel_input=Some(services::wheel_input::Watch::start());
                }
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
                self.wheel_input=None;
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
                // Search and AI own keyboard focus for the whole workspace.
                // Moving from their editor to a toolbar/button must not give
                // focus back to the previous app and trigger outside dismissal.
                let workspace=self.st.search.active||(self.expanded&&self.st.page==PageKind::Ai);
                if workspace{self.focus_workspace();unsafe{let _=SetFocus(self.hwnd);}}
                self.page_interaction(p);
                self.dbl_press = msg == WM_LBUTTONDBLCLK;
                self.input.double_click=self.dbl_press;self.input.shift=unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0;
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
                        self.open_ai(false);
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
                                Some(NoticeAction::OpenAiSite(site))=>{
                                    let url=self.st.ai.sites.ready.as_ref().filter(|r|r.site==site).map(|r|r.url.clone());
                                    if self.st.ai.sites.selected!=Some(site){self.select_ai_site(Some(site));}
                                    if let Some(url)=url{self.st.ai.sites.url=url;}
                                    crate::ai_sites::clear_ready(&mut self.st);self.open_ai(true);
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
                } else if self.text_focus && !workspace && self.input.pressed_id != self.input.focused {
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
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                if self.st.search.embedded()&&crate::browser_home::is_home(&self.st){return Some(LRESULT(0));}
                self.search_context_menu(if msg==WM_CONTEXTMENU{None}else{Some(self.mouse_dip(lp))},deferred);Some(LRESULT(0))},
            crate::search_web::CHANGED=>{self.sync_search_web();self.sync_ai_web();if let Some(w)=self.ai_window.as_ref(){w.invalidate();}self.request_frame();Some(LRESULT(0))},
            crate::search_web::CLOSE=>{if wp.0==1{self.close_ai_host();}else if self.st.search.detached{self.close_search_web();}else{self.close_search();}self.request_frame();Some(LRESULT(0))},
            crate::search_web::ACTION=>{if let Some(action)=crate::search_web::Action::from_message(wp.0){if lp.0==1{if let Some(web)=self.ai_web.as_ref(){web.action(action);}}else{self.search_web_action(action);}}Some(LRESULT(0))},
            crate::search_web::SWIPE=>{
                if self.ai_window.as_ref().is_some_and(|w|w.hwnd.0 as isize==lp.0) {
                    let delta=(wp.0>>16)as u16 as i16;
                    if !self.locked&&!self.display_off&&self.search_gesture.feed(delta,clock::mono()) {
                        self.st.closed_view=Default::default();self.close_ai_host();self.request_frame();
                    }
                    Some(LRESULT(0))
                } else {self.handle_island(WM_MOUSEHWHEEL,wp,LPARAM(0),deferred)}
            },
            crate::search_web::PAGE_FOCUS=>{if wp.0==1{self.input.focused=None;if self.st.ai.sites.menu{self.st.ai.sites.menu=false;if let Some(w)=self.ai_window.as_ref(){w.invalidate();}self.request_frame();}}else if self.st.search.embedded()||self.st.search.detached{self.input.focused=None;if let Some(w)=self.browser_window.as_mut(){w.blur_address();}self.st.search.address_editing=false;self.st.search.address_suggestions.clear();self.st.search.suggestions.cancel();self.st.search.address_dirty=false;self.st.search.address=self.st.search.navigation.url.clone();self.request_frame();}Some(LRESULT(0))},
            WM_KILLFOCUS if self.st.search.embedded()=>{crate::search::cancel_address_edit(&mut self.st,&mut self.input);self.request_frame();None},
            WM_MOUSEHWHEEL => {
                let delta = (wp.0 >> 16) as u16 as i16;
                if self.locked || self.display_off { return Some(LRESULT(0)); }
                if self.st.search.active || !self.expanded || self.st.page==PageKind::Ai {
                    if self.search_gesture.feed(delta,clock::mono()) {
                        if self.st.search.active { self.close_search(); }
                        else if self.expanded && self.st.page==PageKind::Ai {self.st.closed_view=Default::default();self.collapse();}
                        else if delta>0 && self.st.settings.search.horizontal_scroll { self.open_search(); }
                        else if delta<0 && self.st.settings.ai.horizontal_scroll && self.st.ai_on(){self.open_ai(false);}
                    }
                } else {
                    if let Some(forward)=self.wheel_pager.feed(delta,clock::mono()) {
                        self.set_page(pages::step_tab(self.st.page,&self.st,forward));self.request_frame();
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta=(wp.0>>16)as u16 as i16;
                let source=self.wheel_input.as_ref().map_or(services::wheel_input::Source::Unknown,|w|w.source());
                let shift=wp.0&4!=0;
                self.island_vertical_wheel(delta,lp,source,shift,deferred)
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
            WM_SYSKEYDOWN if self.st.search.active && self.st.search.embedded()=>{
                let alt=(lp.0 & (1<<29))!=0 || unsafe{GetKeyState(VK_MENU.0 as i32)}<0;
                if let Some(action)=crate::search_web::Action::from_key(wp.0 as u32,unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0,alt){self.search_web_action(action);Some(LRESULT(0))}else{None}
            }
            WM_KEYDOWN => {
                if self.expanded&&self.st.page==PageKind::Ai&&self.st.ai.sites.selected.is_some()&&self.st.ai.sites.menu&&wp.0==VK_ESCAPE.0 as usize{
                    self.st.ai.sites.menu=false;self.input.focused=None;if let Some(web)=self.ai_web.as_ref(){web.focus();}self.request_frame();return Some(LRESULT(0));
                }
                if self.expanded&&self.st.page==PageKind::Ai&&!self.st.search.active{if let Some(field)=crate::ai::workspace_key(&mut self.st,wp.0 as u32,unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0,unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0){self.input.focused=Some(field);self.input.caret=0;self.focus_text();self.request_frame();return Some(LRESULT(0));}}
                if self.st.search.active {
                    let key=VIRTUAL_KEY(wp.0 as u16);
                    if key==VK_ESCAPE {if self.st.search.embedded()&&self.input.focused==Some(wid("search.query")){self.input.keys.push(KeyInput::Escape);self.request_frame();}else{self.close_search();}return Some(LRESULT(0));}
                    if self.st.search.embedded(){if let Some(action)=crate::search_web::Action::from_key(key.0 as u32,unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0,unsafe{GetKeyState(VK_MENU.0 as i32)}<0){self.search_web_action(action);return Some(LRESULT(0));}}
                    if key==VK_APPS || (key==VK_F10 && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0){self.search_context_menu(None,deferred);return Some(LRESULT(0));}
                    if key==VK_TAB && self.st.search.embedded(){if self.complete_address_suggestion(){return Some(LRESULT(0));}if let Some(browser)=self.search_web.as_ref(){browser.focus();}return Some(LRESULT(0));}
                    if self.st.search.embedded()&&self.input.focused==Some(wid("search.query"))&&matches!(key,VK_UP|VK_DOWN){self.input.keys.push(if key==VK_UP{KeyInput::Up}else{KeyInput::Down});self.request_frame();return Some(LRESULT(0));}
                    if let Some(view)=self.st.search.view.as_mut() {
                        match key {
                            VK_UP=>view.navigate(-1), VK_DOWN=>view.navigate(1),
                            VK_PRIOR=>view.navigate(-5), VK_NEXT=>view.navigate(5),
                            VK_TAB=>view.navigate(if unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{-1}else{1}),
                            _=>{},
                        }
                    }
                    if matches!(key,VK_UP|VK_DOWN|VK_PRIOR|VK_NEXT|VK_TAB) {self.request_frame();return Some(LRESULT(0));}
                }
                if wp.0==VK_ESCAPE.0 as usize && self.st.page==PageKind::Ai && (self.st.ai.model_menu || self.st.ai.reasoning_menu || self.st.ai.attach_menu || self.st.ai.sites.menu) {
                    self.st.ai.dismiss_popup();self.input.focused=Some(wid(if self.st.ai.model_menu{"ai.model-search"}else{"ai.prompt"}));self.input.caret=0;self.request_frame();return Some(LRESULT(0));
                }
                if wp.0==VK_ESCAPE.0 as usize&&self.expanded&&self.st.page==PageKind::Ai&&!self.st.search.active{self.collapse();return Some(LRESULT(0));}
                if self.text_focus {
                    self.reopen.interacted(self.st.page, clock::mono());
                    let k = match VIRTUAL_KEY(wp.0 as u16) {
                        VK_A if unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0 => Some(KeyInput::SelectAll),
                        VK_C if (self.st.search.active||self.st.page==PageKind::Ai) && unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0=>Some(KeyInput::Copy),
                        VK_X if (self.st.search.active||self.st.page==PageKind::Ai) && unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0=>Some(KeyInput::Cut),
                        VK_BACK => Some(KeyInput::Backspace),
                        VK_DELETE => Some(KeyInput::Delete),
                        VK_RETURN => Some(if !self.st.search.active && self.st.page==PageKind::Ai && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{KeyInput::Char('\n')}else{KeyInput::Enter}),
                        VK_ESCAPE => Some(KeyInput::Escape),
                        VK_LEFT => Some(if (self.st.search.active||self.st.page==PageKind::Ai) && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{KeyInput::SelectLeft}else{KeyInput::Left}),
                        VK_RIGHT => Some(if (self.st.search.active||self.st.page==PageKind::Ai) && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{KeyInput::SelectRight}else{KeyInput::Right}),
                        VK_UP => Some(KeyInput::Up),
                        VK_DOWN => Some(KeyInput::Down),
                        VK_HOME => Some(if (self.st.search.active||self.st.page==PageKind::Ai) && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{KeyInput::SelectHome}else{KeyInput::Home}),
                        VK_END => Some(if (self.st.search.active||self.st.page==PageKind::Ai) && unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0{KeyInput::SelectEnd}else{KeyInput::End}),
                        VK_V if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 => {
                            let prompt=!self.st.search.active && self.st.page==PageKind::Ai && self.input.focused==Some(wid("ai.prompt"));
                            if !crate::ai::paste_attachment(&mut self.st,prompt,services::ai::clipboard_has_attachment) {if let Some(text) = services::clipboard::read_text(self.hwnd) {
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
                    let prompt=!self.st.search.active && self.st.page==PageKind::Ai && self.input.focused==Some(wid("ai.prompt"));
                    if !crate::ai::paste_attachment(&mut self.st,prompt,services::ai::clipboard_has_attachment){crate::ai_window::paste_text(&mut self.input,self.hwnd);}
                    self.request_frame();
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    if self.st.search.active || (self.expanded&&self.st.page==PageKind::Ai) {self.search_lost_activation(HWND(lp.0 as *mut _));}
                    else if self.text_focus && !self.st.search.context_menu {
                        self.text_focus = false;
                        self.input.focused = None;
                        self.set_noactivate(true);
                        self.maybe_schedule_collapse();
                        self.request_frame();
                    }
                }
                None
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                if msg != WM_SETTINGCHANGE { self.focus_light = None; }
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
                    if !on { self.focus_light=None;self.wheel_input=None;self.workspace_dismiss=None;self.close_search(); }
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
                if self.locked { self.focus_light=None;self.wheel_input=None;self.workspace_dismiss=None;self.close_search(); }
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

    fn choose_folder(&mut self, kind: crate::commands::FolderPick) {
        let flag = match &kind {
            crate::commands::FolderPick::Directory => &mut self.st.ui.choosing_music_dir,
            crate::commands::FolderPick::LyricsDirectory => &mut self.st.ui.choosing_lyrics_dir,
            crate::commands::FolderPick::PhoneInbox => &mut self.st.ui.choosing_phone_dir,
        };
        if *flag { return; }
        *flag = true;
        let initial = match kind {
            crate::commands::FolderPick::Directory => paths::music_dir(&self.st.settings),
            crate::commands::FolderPick::LyricsDirectory => paths::lyrics_download_dir(&self.st.settings).unwrap_or_else(|| paths::music_dir(&self.st.settings)),
            crate::commands::FolderPick::PhoneInbox => paths::phone_inbox_dir(&self.st.settings),
        };
        if let Err(e) = services::music_files::choose(kind, SendHwnd::new(self.helper), initial, self.sink.clone()) {
            *flag = false;
            self.st.notify(Icon::Folder, palette::ORANGE, "Couldn't start the picker", e.to_string(), 6.0, None);
        }
    }

    fn island_vertical_wheel(&mut self,delta:i16,lp:LPARAM,source:services::wheel_input::Source,shift:bool,_deferred:&mut Vec<Deferred>)->Option<LRESULT> {
        if self.locked||self.display_off||delta==0{return Some(LRESULT(0));}
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
                // Search/address chrome has no vertical wheel shortcut. In
                // particular, don't fall through to the older header-close
                // gesture after removing the Browser/Search bar switch.
                if self.st.search.active && (if self.st.search.embedded(){crate::search::browser_toolbar(body,false).bar.contains(p)}else{self.hit_at(p).is_some_and(|h|h.id==wid("search.query"))}) {return Some(LRESULT(0));}
                let mouse_shortcut=source==services::wheel_input::Source::Mouse && self.st.settings.mouse_wheel_shortcuts && !shift && !self.input.down;
                if mouse_shortcut && !self.expanded && body.contains(p) {
                    if self.search_gesture.feed(delta,clock::mono()) {
                        if delta>0 && self.st.ai_on(){self.open_ai(false);}
                        else if delta<0{self.open_search();}
                    }
                    return Some(LRESULT(0));
                }
                let workspace_header=self.expanded && (self.st.search.active||self.st.page==PageKind::Ai)
                    && p.x>=body.x&&p.x<=body.right()&&p.y>=0.0&&p.y<body.y+pages::HEADER_H;
                if mouse_shortcut&&workspace_header {
                    if self.search_gesture.feed(delta,clock::mono()) {
                        self.st.closed_view=Default::default();self.collapse();
                    }
                    return Some(LRESULT(0));
                }
                let over_header = self.expanded && !self.st.search.active && self.st.page!=PageKind::Ai && p.x >= body.x && p.x <= body.right()
                    && p.y >= 0.0 && p.y < body.y + pages::HEADER_H;
                if !self.expanded && self.st.settings.closed_scroll && body.contains(p) {
                    if let Some(next)=self.wheel_pager.feed_vertical(delta,clock::mono()){
                        self.st.closed_view=self.st.closed_view.step(next);self.request_frame();
                    }
                } else if over_header {
                    self.music_wheel=Default::default();
                    if let Some(forward) = self.wheel_pager.feed_vertical(delta, clock::mono()) {
                        let next = pages::step_tab(self.st.page, &self.st, forward);
                        self.set_page(next);
                        self.request_frame();
                    }
                } else if self.expanded && !self.st.search.active && self.st.page==PageKind::Media
                    && self.st.settings.music_scroll_tracks && self.st.media.is_some() && !self.input.down
                    && crate::pill::music::player_rect(&self.st,pages::content_area(body)).contains(p) {
                    self.page_interaction(p);
                    if let Some(next)=self.music_wheel.feed_vertical(delta,clock::mono()) {
                        let allowed=self.st.media.as_ref().is_some_and(|m|if next{m.can_next}else{m.can_prev});
                        if allowed{pages::media::push_media(&mut self.st,if next{crate::commands::MediaCmd::Next}else{crate::commands::MediaCmd::Prev});}
                    }
                    self.request_frame();
                } else {
                    self.music_wheel=Default::default();
                    self.page_interaction(p);
                    self.input.wheel += f32::from(delta) / WHEEL_DELTA as f32;
                    self.request_frame();
                }
        Some(LRESULT(0))
    }

    /// Apply side effects of a settings change (from the menu, a reload or a page).
    fn settings_changed(&mut self, before: &Settings) {
        if before.web != self.st.settings.web {
            if self.st.ai.sites.selected.is_some_and(|s|s.configured(&self.st.settings).is_none()) { self.select_ai_site(None); }
            if before.web.adblocking!=self.st.settings.web.adblocking {
                if let Some(c)=&self.st.ui.web.cancel{c.cancel();}
                if self.st.settings.web.adblocking&&!self.locked&&!self.display_off&&!self.st.power.saver{crate::web_blocking::ensure(&mut self.st,self.sink.clone());}
                self.refresh_web_blocking();
            }
        }
        if before.mouse_wheel_shortcuts!=self.st.settings.mouse_wheel_shortcuts {self.wheel_input=None;self.search_gesture=Default::default();}
        if before.closed_scroll&&!self.st.settings.closed_scroll{self.st.closed_view=Default::default();self.wheel_pager=Default::default();}
        if (before.ai.mini_width,before.ai.mini_height)!=(self.st.settings.ai.mini_width,self.st.settings.ai.mini_height){if let Some(w)=self.ai_window.as_ref(){w.apply_size(&self.st.settings.ai);}}
        if before.search != self.st.settings.search {
            if before.search.apps!=self.st.settings.search.apps && !self.st.settings.search.apps {
                for pending in [&mut self.st.search.pending,&mut self.st.search.icon_pending]{if let Some(p)=pending.take(){p.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}}
                self.st.search.apps.clear();self.st.search.icons.clear();self.st.search.loaded_at=None;
            }
            if before.search.web && !self.st.settings.search.web{self.close_search_web();}
            if !self.st.settings.search.google_suggestions || !self.st.settings.search.web{self.st.search.suggestions.cancel();if let Some(v)=self.st.search.view.as_mut(){v.suggestions.clear();}}
            if let Some(window)=self.browser_window.as_mut(){
                if before.search.mini_width!=self.st.settings.search.mini_width || before.search.mini_height!=self.st.settings.search.mini_height {window.apply_size(&self.st.settings.search);}
                window.mark_dirty();
            }
            if before.search.apps!=self.st.settings.search.apps || before.search.settings!=self.st.settings.search.settings || before.search.web!=self.st.settings.search.web {
                crate::search::refresh(&mut self.st);
                self.start_search_catalog(false);
            }
            if before.search.services!=self.st.settings.search.services || before.search.powertoys!=self.st.settings.search.powertoys || before.search.vscode_commands!=self.st.settings.search.vscode_commands{
                if let Some(p)=self.st.search.integrations_pending.take(){p.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}
                self.st.search.integrations.retain(|entry|entry.enabled(&self.st.settings.search));
                crate::search::refresh(&mut self.st);self.start_search_integrations();
            }
        }
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
        if before.music_scroll_tracks!=now.music_scroll_tracks {self.music_wheel=Default::default();}
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
        if before.focus_spotlight && !self.st.settings.focus_spotlight { self.focus_light = None; }
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
        if before.addon_on("ai") && !self.st.ai_on() {
            if self.st.ai.busy || self.st.ai.cancel.is_some() {
                crate::ai::local(&mut self.st, &crate::ai::Action::Stop);
            }
            crate::ai::clear_ready(&mut self.st);
            self.st.ai.close_popovers();
            self.st.ai.full_open = false;
            self.ai_ctrl_watch = None;
            self.ai_web=None;
            self.st.ai.sites.attempted=false;self.st.ai.sites.working=false;self.st.ai.sites.web_ready=false;self.ai_web_resting=false;
            self.ai_window = None;
            self.ai_resize_hold = None;
            self.ai_reply_peek = false;
            if self.st.page == PageKind::Ai { self.set_page(PageKind::Addons); }
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

    }

    fn on_menu(&mut self, cmd: tray::MenuCmd) {
        use tray::MenuCmd::*;
        let before = self.st.settings.clone();
        let s = &mut self.st.settings;
        match cmd {
            ToggleIdleStyle(style) => s.idle_style = style,
            SetTheme(theme) => s.theme = theme,
            ToggleClipboard => s.clipboard_history = !s.clipboard_history,
            ToggleSystemHistory => s.system_history = false,
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
        let refresh_settings = matches!(&ev, AppEvent::AdBlockingUpdated(_) | AppEvent::Ai(_) | AppEvent::Autostart(_) | AppEvent::FolderChosen { .. } | AppEvent::Update(_) | AppEvent::ForegroundApp(_));
        let now = clock::mono();
        self.st.now = now;
        let media_before = matches!(ev, AppEvent::Media(_) | AppEvent::MediaTimeline { .. }).then(|| self.media_snapshot());
        match ev {
            AppEvent::AdBlockingUpdated(result)=>{
                self.st.ui.web.downloading=false;self.st.ui.web.cancel=None;
                let ready=result.is_ok();
                self.st.ui.web.protection_status=match result{Ok(())=>"Protection ready. Reload open websites to apply all changes.".into(),Err(e)=>e};
                crate::web_blocking::invalidate();self.refresh_web_blocking();
                if ready {
                    if self.ai_web.is_none(){self.st.ai.sites.attempted=false;}
                    if self.search_web.is_none(){self.st.search.web_error.clear();}
                    self.sync_ai_web();self.sync_search_web();
                }
            },
            AppEvent::SearchIcons(event)=>{
                if self.st.search.icon_pending.as_ref().is_some_and(|p|p.request==event.request) {
                    let cancelled=self.st.search.icon_pending.take().unwrap().cancel.load(std::sync::atomic::Ordering::Relaxed);
                    if !cancelled && self.st.search.active && (self.st.settings.search.apps||self.st.settings.search.powertoys) {for (id,image) in event.images {self.st.search.icons.insert(id,image);}}
                }
            },
            AppEvent::Search(event)=>{
                self.st.search.apply(event,clock::mono(),self.st.settings.search.apps);
                crate::search::refresh(&mut self.st);
                self.start_search_catalog(false);
            },
            AppEvent::SearchSuggestions(event)=>{
                if !self.locked && !self.display_off && !self.widgets_open && !self.st.power.saver {crate::search::apply_suggestions(&mut self.st,event);}
                if let Some(window)=self.browser_window.as_mut(){window.mark_dirty();}
            },
            AppEvent::SearchIntegrations(event)=>{
                if self.st.search.integrations_pending.as_ref().is_some_and(|p|p.request==event.request){
                    let p=self.st.search.integrations_pending.take().unwrap();
                    if !p.cancel.load(std::sync::atomic::Ordering::Relaxed) && self.st.search.active && !self.locked && !self.display_off && !self.st.power.saver{
                        self.st.search.integrations=event.entries.into_iter().filter(|e|e.enabled(&self.st.settings.search)).collect();
                        self.st.search.integrations_warning=event.warning;crate::search::refresh(&mut self.st);
                    }
                }
            },
            AppEvent::SearchAction{result,refresh}=>{
                self.st.search.action_pending=false;
                if let Err(message)=&result{if !self.st.search.active && !self.locked && !self.display_off && self.st.notice.is_none(){
                    self.st.notice=Some(crate::state::Notice{icon:Icon::Search,color:self.st.accent,title:"Search action could not open".into(),title_dim:String::new(),subtitle:message.clone(),until:self.st.now+10.0,action:None,inline:None});
                }}
                self.st.search.notice=match result{Ok(message)|Err(message)=>message};
                if refresh{if let Some(p)=self.st.search.integrations_pending.take(){p.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}self.start_search_integrations();}
            },
            AppEvent::Ai(event) => {
                let attached=matches!(&event,crate::ai::Event::Attached{..});
                let reply_chat=match &event{crate::ai::Event::Reply{chat,..}=>Some(*chat),_=>None};
                let showing_ai=(!self.st.search.active && self.expanded && self.st.page==PageKind::Ai) || self.ai_window.as_ref().is_some_and(|w|unsafe{IsWindowVisible(w.hwnd)}.as_bool() && !unsafe{IsIconic(w.hwnd)}.as_bool());
                let notify=self.st.ai_on() && !(showing_ai && reply_chat==Some(self.st.ai.current.id));
                crate::ai::apply(&mut self.st,event,notify);
                if reply_chat == Some(self.st.ai.current.id) && !self.st.search.active && self.expanded && self.st.page == PageKind::Ai {
                    self.reopen.interacted(PageKind::Ai, now);
                }
                if attached && self.st.ai_on() && !self.locked && !self.display_off {
                    if let Some(w)=self.ai_window.as_mut(){w.focus_prompt(self.st.ai.draft.chars().count());}
                    else if !self.st.search.active && self.expanded && self.st.page==PageKind::Ai {self.input.focused=Some(wid("ai.prompt"));self.input.caret=self.st.ai.draft.chars().count();self.focus_text();}
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
            AppEvent::FolderChosen { kind, result } => {
                use crate::commands::FolderPick;
                match &kind {
                    FolderPick::Directory => self.st.ui.choosing_music_dir = false,
                    FolderPick::LyricsDirectory => self.st.ui.choosing_lyrics_dir = false,
                    FolderPick::PhoneInbox => self.st.ui.choosing_phone_dir = false,
                }
                match result {
                    Ok(Some(path)) => match kind {
                        FolderPick::Directory => {
                            let directory = path.to_string_lossy().into_owned();
                            if self.st.settings.music_directory == directory {
                                self.refetch_lyrics();
                            }
                            self.st.settings.music_directory = directory;
                            self.st.cmd(Command::SaveSettings);
                        }
                        FolderPick::LyricsDirectory => {
                            self.st.settings.lyrics_download_directory=path.to_string_lossy().into_owned();
                            self.st.cmd(Command::SaveSettings);
                        }
                        FolderPick::PhoneInbox => {
                            self.st.settings.phone_download_directory = path.to_string_lossy().into_owned();
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
                if p.saver {if let Some(cancel)=&self.st.ui.web.cancel {cancel.cancel();}}
                if p.saver { self.focus_light = None; }
                if p.saver {self.st.search.cancel();if self.search_web.is_some(){self.search_web=None;self.st.search.web_ready=false;self.st.search.web_visible=false;self.st.search.web_error="Web search is paused in battery saver.".into();self.st.search.navigation.loading=false;self.st.search.navigation.progress=0.0;}}
                self.request_frame();
                if old.saver && !p.saver {self.start_search_catalog(false);}
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
                if !foreground.is_invalid(){self.search_lost_activation(foreground);}
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
            AppEvent::FilesDropped { owner, paths } if self.ai_drop_owner(owner.hwnd()) => {
                self.drag_hover=false;self.st.drag_hover=false;self.drag_owner=None;
                self.st.ai.close_popovers();
                self.st.cmd(Command::Ai(crate::ai::Action::AttachFiles(paths)));
                self.request_frame();
            }
            AppEvent::FilesDropped { .. } if self.expanded&&self.st.page==PageKind::Ai&&self.st.ai.sites.selected.is_some()=>{
                // WebView2 owns file drops on the site itself. Dropping on our
                // overlay must not attach to native chat or switch to Shelf.
                self.drag_hover=false;self.st.drag_hover=false;self.drag_owner=None;
            }
            AppEvent::FilesDropped { owner, .. } if owner.hwnd()!=self.hwnd => {},
            AppEvent::FilesDropped { paths, .. } if self.expanded && self.st.page == PageKind::Script && pages::script::take_drop(&mut self.st, &paths) => {
                // A text file dropped on the Script page becomes the script.
                self.drag_hover = false;
                self.st.drag_hover = false;
                self.drag_owner=None;
                self.request_frame();
            }
            AppEvent::FilesDropped { paths, .. } => {
                let n = self.st.store.shelf.add_paths(&paths, self.st.unix);
                for p in &paths {
                    self.st.cmds.push(Command::LoadThumb(p.clone()));
                }
                self.st.save();
                self.st.page = PageKind::Shelf;
                self.drag_hover = false;
                self.st.drag_hover = false;
                self.drag_owner=None;
                self.expand();
                pinfo!("shelf: {n} new item(s)");
            }
            AppEvent::DragHover { owner, on } => {
                if on&&self.expanded&&self.st.page==PageKind::Ai&&self.st.ai.sites.selected.is_some(){return;}
                let ai=self.ai_drop_owner(owner.hwnd());
                if on {
                    if owner.hwnd()!=self.hwnd && !ai {return;}
                    self.drag_owner=Some(owner.hwnd());
                }else{
                    if self.drag_owner!=Some(owner.hwnd()){return;}
                    self.drag_owner=None;
                }
                self.drag_hover = on;
                self.st.drag_hover = on;
                if on && ai {
                    self.st.ai.close_popovers();
                    self.collapse_due=false;unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}
                    if let Some(w)=self.ai_window.as_ref(){w.invalidate();}
                } else if on {
                    // Script handles text files; ordinary island pages use Shelf.
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
                if self.sysmon_on || self.sysmon_compact || self.st.game.active {
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
            let page = if self.st.closed_view!=lumo_core::settings::ClosedView::Time{PageKind::System}else{self.reopen.page(&self.st, self.st.now)};
            // Automatic routing must not write a new remembered preference.
            self.apply_page(page, false);
        }
        self.expand();
    }

    fn expand(&mut self) {
        if !self.st.search.active && self.st.search.view.is_some() {self.park_search_view();self.layout_surface();}
        self.ai_ctrl_watch=None;
        if !self.expanded {
            self.expanded = true;
            self.cancel_hover();
            if self.lyrics_window.is_some() { self.raise_topmost(); }
        }
        self.request_frame();
    }

    fn open_ai_reply(&mut self, chat: u64) {
        if !self.st.ai_on() { return; }
        if self.st.ai.sites.selected.is_some(){self.select_ai_site(None);}
        if self.st.ai.current.id!=chat {crate::ai::local(&mut self.st,&crate::ai::Action::Select(chat));}
        crate::ai::clear_ready(&mut self.st);
        self.st.ai.history=false;
        self.st.ai.scroll_to_reply=self.st.ai.current.messages.iter().rposition(|m|m.role=="assistant");
        self.open_ai(true);
        if self.st.ai.full_open{return;}
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
        if dragging && !self.st.search.active && self.expanded && self.st.page == PageKind::Ai {
            self.island.snap(self.geom_for(&Mode::Expanded(PageKind::Ai)));
            self.collapse_due = false;
        }
        if changed { self.request_frame(); }
    }

    fn collapse(&mut self) {
        self.workspace_dismiss=None;
        let closing_ai=self.expanded&&self.st.page==PageKind::Ai;
        if self.st.search.active {self.st.search.active=false;self.st.search.cancel();self.cancel_hover();self.park_search_web();}
        if self.expanded&&self.st.page==PageKind::Ai {
            if !self.st.ai.full_open&&self.st.ai.sites.selected.is_none(){self.st.ai.close_workspace(self.st.settings.ai.resume);}
            if let Some(page)=self.st.ai.return_page.take(){self.st.page=page;}
        }
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
        if closing_ai && self.fullscreen_on {self.apply_fullscreen(self.st.now);}
        self.request_frame();
    }

    /// Nothing holds the open island: no pointer on it, no pin, typing,
    /// file drag, press or Phone drop.
    fn free_to_collapse(&self) -> bool {
        !self.hovering && !(self.st.search.active&&self.st.settings.search.keep_open) && !(self.expanded&&self.st.page==PageKind::Ai&&self.st.settings.ai.keep_open) && self.ai_resize_hold.is_none() && !self.st.ai.attaching && !self.st.pinned && !self.text_focus && !self.drag_hover && self.input.pressed_id.is_none() && !self.phone_holds()
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
        if self.expanded && !(self.st.search.active&&self.st.settings.search.keep_open) && !(self.expanded&&self.st.page==PageKind::Ai&&self.st.settings.ai.keep_open) && self.ai_resize_hold.is_none() && !self.hovering && !self.st.pinned && !self.text_focus && !self.drag_hover && !self.phone_holds() {
            unsafe {
                SetTimer(self.hwnd, TIMER_COLLAPSE, HOLD_GRACE_MS.max(self.st.settings.collapse_delay_ms), None);
            }
        }
    }

    /// Full System-page sampling only. Compact visibility is independent so
    /// an ordinary page-close message cannot cancel its live kernel readings.
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
        if !on && !self.sysmon_compact && !self.st.game.active {
            self.st.system.clear();
        }
    }

    fn sync_system_views(&mut self){
        self.sync_workspace_dismiss();
        if self.locked||self.display_off||!self.hovering {self.wheel_input=None;}
        let rest=self.locked||self.display_off||self.st.power.saver||self.fullscreen_hidden||self.widgets_open;
        let compact=!rest&&!self.expanded&&matches!(self.target_mode(),Mode::Readout(_));
        if compact!=self.sysmon_compact{
            self.sysmon_compact=compact;
            if compact&&self.sysmon.is_none()&&self.selftest.is_none(){self.start_sysmon();}
            if let Some(s)=&self.sysmon{s.compact(compact);}
            if !compact&&!self.sysmon_on&&!self.st.game.active{self.st.system.clear();}
        }
        self.set_sysmon(!rest&&!self.st.search.active&&self.expanded&&self.st.page==PageKind::System);
    }

    fn start_sysmon(&mut self) {
        self.sysmon_rest = self.rest_now();
        match services::sysmon::SysMon::start(self.sink.clone(), false, self.sysmon_rest) {
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
        if self.locked || self.display_off || self.st.power.saver {
            if let Some(cancel)=&self.st.ui.web.cancel {cancel.cancel();}
        }
        let rest = self.rest_now();
        if rest == self.sysmon_rest || self.selftest.is_some() {
            return;
        }
        self.sysmon_rest = rest;
        if let Some(s) = &self.sysmon {
            s.rest(rest);
        }
        if !lumo_core::sysmon::reads(self.sysmon_on||self.sysmon_compact, self.st.game.active, false, rest) {
            self.st.system.clear();
        }
    }

    /// Over a full-screen app the island hides when the setting says Hide, and
    /// under Show over games while the readout is away: between its peeks
    /// (peeks only), or once Alt+Shift+F hid it. Then nothing covers the game,
    /// and Windows sends its frames straight to the screen.
    fn fullscreen_hides(&self, now: f64) -> bool {
        self.widgets_open || (self.fullscreen_on && !(self.ai_reply_peek && !self.st.search.active && self.expanded && self.st.page==PageKind::Ai) && !self.st.notice.as_ref().is_some_and(|n| n.is_ai_reply() || (n.until>now && matches!(n.action,Some(NoticeAction::OpenPage(PageKind::Ai))))) && (self.st.settings.hide_in_fullscreen || (self.st.game.active && self.readout_away(now))))
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
        let shown = !self.st.search.active && self.st.script_on() && self.expanded && self.st.page == PageKind::Script
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
            && !self.st.search.active && self.st.page == PageKind::Script && !self.st.store.script.is_empty() && (self.expanded || self.st.script.active);
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

    /// Retry activation only on an explicit workspace opening/press, never on
    /// each editor repaint when Windows refuses a wheel-opened focus request.
    fn focus_workspace(&mut self) {
        if unsafe{GetForegroundWindow()}!=self.hwnd{self.text_focus=false;}
        self.focus_text();
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
        if self.st.search.active || !self.expanded || self.st.page != PageKind::Ai || !pages::ai::picker_below(&self.st) { return None; }
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
        if matches!(m,Mode::Expanded(_)) && self.st.search.view.is_some() {
            let available=(self.monitor.rect.right-self.monitor.rect.left) as f32/self.gfx.scale()-32.0;
            let web=self.st.search.embedded();let height=((self.monitor.work.bottom-self.monitor.top()) as f32/self.gfx.scale()-32.0).max(300.0);
            return Geom::new((if web && self.st.settings.search.browser_wide{900.0}else{crate::search::WIDTH}).min(available).max(320.0),(if web{f32::from(self.st.settings.search.browser_height)}else{self.st.search.view.as_ref().map(|view|view.height(&self.st)).unwrap_or(crate::search::HEIGHT)}).min(height),if web{16.0}else if pill::on(self.st.settings.theme){40.0}else{30.0});
        }
        if *m==Mode::Expanded(PageKind::Ai) {
            let available=(self.monitor.rect.right-self.monitor.rect.left) as f32/self.gfx.scale()-32.0;
            return Geom::new(crate::search::WIDTH.min(available).max(320.0),self.st.ai.island_height.min(self.st.ai.max_island_height),if pill::on(self.st.settings.theme){40.0}else{30.0});
        }
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
            let viewed=current && self.st.ai.sites.selected.is_none() && ((!self.st.search.active && self.expanded && self.st.page==PageKind::Ai) || self.ai_window.as_ref().is_some_and(|w|unsafe{IsWindowVisible(w.hwnd)}.as_bool() && !unsafe{IsIconic(w.hwnd)}.as_bool()));
            if !exists || viewed {crate::ai::clear_ready(&mut self.st);self.apply_fullscreen(now);}
            else if self.st.notice.is_none() {crate::ai::announce_ready(&mut self.st);self.apply_fullscreen(now);}
        }
        if self.st.ai.sites.ready.is_some(){
            if self.ai_site_visible(){crate::ai_sites::clear_ready(&mut self.st);}
            else if self.st.notice.is_none(){crate::ai_sites::announce_ready(&mut self.st);self.apply_fullscreen(now);}
        }
        // A new version's notice (or "Updated to") waits for the island to be
        // free. The first frame counts (the window shows at its end).
        let free = !self.fullscreen_hidden && !self.locked && !self.display_off && !self.st.power.saver && !self.expanded && self.lyrics_window.is_none() && self.st.notice.is_none();
        if self.updater.as_mut().is_some_and(|u| u.announce(&mut self.st, free)) {
            self.request_frame();
        }
        let focus_due = match self.st.pomodoro.run {
            lumo_core::pomodoro::RunState::Running { ends_at } => now - ends_at < 5.0,
            _ => false,
        };
        if let Some(ended) = self.st.pomodoro.tick(now) {
            pages::focus::on_phase_ended(&mut self.st, ended);
            if ended.ended == lumo_core::pomodoro::Phase::Focus && focus_due { self.start_focus_light(false); }
        }
        self.script_tick();
    }

    /// Draw and show one frame. `tick` is the vblank that triggered it (None
    /// for self-test captures, which run off the clock).
    fn frame(&mut self, deferred: &mut Vec<Deferred>, tick: Option<FrameTick>) {
        if self.locked || self.display_off || self.st.power.saver || self.fullscreen_hidden || self.widgets_open || (!self.st.settings.focus_spotlight && !self.focus_light_preview) { self.focus_light = None; }
        self.sync_system_views();
        self.input.begin_frame();
        if self.locked || self.display_off || self.widgets_open || self.st.power.saver {self.st.search.suggestions.cancel();self.st.search.browser_transition=None;if let Some(p)=self.st.search.integrations_pending.take(){p.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}}
        if self.st.search.detached && (self.locked || self.display_off || self.widgets_open || self.st.power.saver) { self.sync_search_web(); }
        if !self.expanded && !matches!(self.mode,Mode::Expanded(_)) && self.island.settled() && !self.st.search.active && self.st.search.view.is_some() {self.park_search_view();self.layout_surface();}
        if let Some(w)=self.ai_window.as_mut(){w.rest(self.locked||self.display_off||self.st.power.saver||self.widgets_open);}
        self.sync_ai_web();
        let watch=self.st.ai_on() && self.hovering && !self.expanded && !self.locked && !self.display_off && !self.st.power.saver && !self.fullscreen_hidden && self.st.notice.is_none() && self.st.hud.is_none();
        if watch && self.ai_ctrl_watch.is_none(){self.input.ctrl=services::ai_shortcut::control_down();self.ai_ctrl_watch=services::ai_shortcut::Watch::start(self.hwnd).ok();}else if !watch{self.ai_ctrl_watch=None;}
        if !self.expanded && !matches!(self.mode,Mode::Expanded(_)) && self.island.settled() && self.st.ai.canvas_height>240.0{self.st.ai.canvas_height=240.0;self.layout_surface();}

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
            self.sync_search_web(); self.draw_browser_window(deferred);
            self.sync_search_suggestions();
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
        self.sync_system_views();
        // The Pill theme floats the island below the top edge.
        self.island.set_floating(pill::on(self.st.settings.theme).then_some(pill::TOP_GAP));
        self.island.step(dt, now);
        // Only a real WM_MOUSEMOVE into the settled card may hand a resize
        // back to hover. A cached pointer coordinate can be from before the
        // card shrank and must not silently release its hold here.
        let picker_room = !self.st.search.active && self.expanded && self.st.page == PageKind::Ai && pages::ai::picker_below(&self.st);
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
        self.sync_script_capture(!self.st.search.active && self.st.page == PageKind::Script && (self.expanded || !self.island.settled()));
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
        if let Some(text)=self.input.clipboard_write.take(){self.st.cmd(Command::CopyText(text));}
        self.run_commands(deferred);
        self.sync_search_suggestions();
        let ai_web_changed=self.sync_ai_web();
        let web_changed=self.sync_search_web();
        self.draw_browser_window(deferred);

        let (popup_moving, popup_live) = self.draw_lyrics_window(deferred);
        let focus_light_live = self.draw_focus_light();

        // Decide whether to keep the frame clock running. Real motion runs
        // at the display's full refresh rate; ambient motion runs on every
        // n-th refresh (the equalizer near 60 fps, a drifting progress bar
        // near 30) and pauses while the session is locked or the display off.
        let ambient_ok = !self.locked && !self.display_off && !self.st.power.saver;
        let eq_live = ambient_ok && activities::is_live(&self.mode, &self.st);
        let drift_live = ambient_ok && ui_live;
        let moving = !self.island.settled()
            || web_changed
            || ai_web_changed
            // Ranking can change the compact Search height during drawing.
            // Apply that geometry on the next frame before parking the clock.
            || (self.st.search.active && self.island.target()!=self.geom_for(&self.target_mode()))
            // Run one final frame after settling so the transient Search
            // view and enlarged surface are released before the clock parks.
            || (!self.expanded && self.st.search.view.is_some())
            || self.ai_picker_room != (!self.st.search.active && self.expanded && self.st.page == PageKind::Ai && pages::ai::picker_below(&self.st))
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
        self.clock.set_active(moving || eq_live || drift_live || popup_live || focus_light_live);
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
        if self.expanded&&self.text_focus&&!self.locked&&!self.display_off&&!self.widgets_open&&!self.st.power.saver&&unsafe{GetForegroundWindow()}==self.hwnd{
            if let Some(t)=self.input.caret_wake{consider(t+0.002);}
        }
        if !self.locked && !self.display_off && !self.widgets_open && !self.st.power.saver {
            if let Some(t)=self.st.search.suggestions.due{consider(t);}
        }
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
        }else{unsafe{let _=KillTimer(self.hwnd,TIMER_WAKE);}}
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
                if matches!(self.st.notice.as_ref().and_then(|n|n.action.as_ref()),Some(NoticeAction::OpenAiSite(_))){crate::ai_sites::clear_ready(&mut self.st);}else{crate::ai::clear_ready(&mut self.st);}self.apply_fullscreen(self.st.now);return;
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

    fn ai_drop_owner(&self,owner:HWND)->bool{
        self.st.ai_on()&&self.st.ai.sites.selected.is_none()&&!self.locked&&!self.display_off&&
            ((owner==self.hwnd&&self.expanded&&!self.st.search.active&&self.st.page==PageKind::Ai)||
             self.ai_window.as_ref().is_some_and(|w|w.hwnd==owner&&unsafe{IsWindowVisible(owner)}.as_bool()&&!unsafe{IsIconic(owner)}.as_bool()))
    }
    fn ai_site_visible(&self)->bool{
        !self.locked&&!self.display_off&&!self.widgets_open&&!self.st.power.saver&&
        ((self.expanded&&self.st.page==PageKind::Ai&&!self.st.search.active&&!self.fullscreen_hidden)||
         self.ai_window.as_ref().is_some_and(|w|unsafe{IsWindowVisible(w.hwnd)}.as_bool()&&!unsafe{IsIconic(w.hwnd)}.as_bool()))
    }
    fn park_ai_parent(&mut self){
        if let Some(web)=self.ai_web.as_mut(){if web.parent()!=self.hwnd{let _=web.reparent(self.hwnd);}}
    }
    fn close_ai_host(&mut self){
        self.park_ai_parent();self.ai_window=None;self.st.ai.full_open=false;
        if self.expanded&&self.st.page==PageKind::Ai{self.collapse();}
        if self.st.ai.sites.selected.is_none(){self.st.ai.close_workspace(self.st.settings.ai.resume);}self.sync_ai_web();self.request_frame();
    }
    fn return_ai_host(&mut self){
        self.park_ai_parent();self.ai_window=None;self.st.ai.full_open=false;self.open_ai(true);
    }
    fn refresh_web_blocking(&mut self) {
        for web in [&self.ai_web,&self.search_web].into_iter().flatten() {web.set_blocking(self.st.settings.web.adblocking);}
    }
    fn select_ai_site(&mut self,site:Option<crate::ai_sites::Site>){
        let site=site.filter(|s|s.configured(&self.st.settings).is_some());
        if site.is_none()&&self.st.ai.fresh_on_open{let temporary=self.st.ai.temporary;crate::ai::local(&mut self.st,&crate::ai::Action::New(temporary));self.st.ai.fresh_on_open=false;}
        self.ai_web=None;self.ai_web_resting=false;crate::ai_sites::clear_ready(&mut self.st);
        self.st.ai.close_popovers();self.st.ai.sites=crate::ai_sites::State{selected:site,url:site.and_then(|s|s.configured(&self.st.settings)).map_or(String::new(),|s|s.url.clone()),..Default::default()};
        self.input.focused=None;
        if let Some(w)=self.ai_window.as_mut(){
            if site.is_none(){let _=w.enable_file_drop(self.sink.clone());w.focus_prompt(self.st.ai.draft.chars().count());}
            else{w.disable_file_drop();}w.invalidate();
        }
        self.sync_ai_web();self.request_frame();
    }
    fn sync_ai_web(&mut self)->bool{
        let Some(site)=self.st.ai.sites.selected.filter(|_|self.st.ai_on())else{self.ai_web=None;self.st.ai.sites.web_visible=false;return false;};
        let visible=self.ai_site_visible();
        let rest=self.locked||self.display_off||self.widgets_open||self.st.power.saver||self.ai_window.as_ref().is_some_and(|w|unsafe{IsIconic(w.hwnd)}.as_bool());
        if self.st.power.saver{self.st.ai.sites.error="AI websites are paused in battery saver.".into();}
        if self.ai_web.as_ref().is_some_and(|web|web.discarded()){
            self.ai_web=None;self.st.ai.sites.attempted=false;self.st.ai.sites.working=false;self.ai_web_resting=false;
        }
        if self.ai_web.is_none()&&visible&&!self.st.ai.sites.attempted{
            self.st.ai.sites.attempted=true;
            let url=if self.st.ai.sites.url.is_empty(){site.configured(&self.st.settings).map_or(site.url(),|s|s.url.as_str())}else{&self.st.ai.sites.url};
            match crate::search_web::Browser::new_ai_filtered(self.hwnd,url,site,&self.st.settings.web){Ok(web)=>{self.ai_web=Some(web);self.ai_web_resting=false;self.st.ai.sites.error.clear();},Err(error)=>self.st.ai.sites.error=error}
        }
        let (parent,bounds,scale,area)=if let Some(window)=self.ai_window.as_ref(){
            let(bounds,scale)=window.page_bounds();(window.hwnd,bounds,scale,Rect::new(0.0,0.0,bounds.right as f32/scale,bounds.bottom as f32/scale))
        }else{
            let scale=self.gfx.scale();let area=self.island.body_rect_at(self.cx);
            (self.hwnd,RECT{left:(area.x*scale).round()as i32,top:(area.y*scale).round()as i32,right:(area.right()*scale).round()as i32,bottom:(area.bottom()*scale).round()as i32},scale,area)
        };
        let radius=if self.ai_window.is_some(){24.0}else{self.island.current().r};
        let Some(web)=self.ai_web.as_mut()else{self.st.ai.sites.web_ready=false;self.st.ai.sites.web_visible=false;return false;};
        if visible{web.clip_radius(radius);}
        let(ready,error)=web.status();self.st.ai.sites.web_ready=ready;self.st.ai.sites.error=error;
        if self.st.power.saver{self.st.ai.sites.error="AI websites are paused in battery saver.".into();}
        let nav=web.navigation();self.st.ai.sites.url=nav.url.clone();
        for event in web.take_site_events(){match event{
            crate::ai_sites::Event::Working=>{self.st.ai.sites.working=true;crate::ai_sites::clear_ready(&mut self.st);},
            crate::ai_sites::Event::Idle=>self.st.ai.sites.working=false,
            crate::ai_sites::Event::DismissMenu=>{if self.st.ai.sites.menu{self.st.ai.sites.menu=false;if let Some(w)=self.ai_window.as_ref(){w.invalidate();}}},
            crate::ai_sites::Event::Ready(preview)=>{
                self.st.ai.sites.working=false;
                if !visible{self.st.ai.sites.ready=Some(crate::ai_sites::Reply{site,url:nav.url.clone(),preview});crate::ai_sites::announce_ready(&mut self.st);}
            },
        }}
        let reparented=web.parent()!=parent;
        if reparented{if let Err(error)=web.reparent(parent){self.st.ai.sites.error=error;return false;}}
        if visible{let covers=crate::ai_sites::covers(area,true,self.st.ai.sites.menu,self.st.ai.sites.hint.is_some()).into_iter().map(|r|RECT{
            left:((r.x-area.x)*scale).floor()as i32,top:((r.y-area.y)*scale).floor()as i32,
            right:((r.right()-area.x)*scale).ceil()as i32,bottom:((r.bottom()-area.y)*scale).ceil()as i32,
        }).collect();web.overlays(covers);}
        let show=visible&&ready&&(self.ai_window.is_some()||self.island.settled())&&self.st.ai.sites.error.is_empty();
        let changed=self.st.ai.sites.web_visible!=show;self.st.ai.sites.web_visible=show;
        if visible{web.place_scaled(bounds,show,scale);}else{web.hide();}
        if rest||(!visible&&!self.st.ai.sites.working){
            if !self.ai_web_resting{self.ai_web_resting=true;if !web.pause(){self.ai_web=None;self.st.ai.sites.attempted=false;self.st.ai.sites.working=false;}}
        }else if self.ai_web_resting{self.ai_web_resting=false;let _=web.resume();}
        if show&&(changed||reparented)&&!self.st.ai.sites.menu&&unsafe{GetForegroundWindow()}==parent{if let Some(web)=self.ai_web.as_ref(){web.focus();}}
        if changed{if let Some(window)=self.ai_window.as_ref(){window.invalidate();}}
        changed
    }
    fn open_ai(&mut self,preserve:bool) {
        if !self.st.ai_on()||self.locked||self.display_off{return;}
        if self.ai_window.is_some(){if let Some(w)=self.ai_window.as_ref(){w.show();}return;}
        if self.st.search.active{self.close_search();}self.park_search_view();
        if self.st.ai.sites.selected.is_none()&&!preserve&&self.st.ai.fresh_on_open {let temporary=self.st.ai.temporary;crate::ai::local(&mut self.st,&crate::ai::Action::New(temporary));}
        self.st.closed_view=Default::default();if self.st.ai.sites.selected.is_none(){self.st.ai.fresh_on_open=false;}self.st.ai.close_popovers();self.st.ai.canvas_height=self.st.ai.island_height;
        if self.st.page!=PageKind::Ai {self.st.ai.return_page=Some(self.st.page);}
        self.apply_page(PageKind::Ai,false);self.page_anim=None;self.ai_reply_peek=true;
        if self.st.ai.previews.is_empty()&&self.st.ai.current.messages.iter().any(|m|m.attachments.iter().any(|a|a.mime.starts_with("image/"))){self.st.cmd(Command::Ai(crate::ai::Action::Previews));}
        self.layout_surface();self.expand();self.collapse_due=false;unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}
        self.input.focused=self.st.ai.sites.selected.is_none().then(||wid("ai.prompt"));self.input.caret=self.st.ai.draft.chars().count();self.focus_workspace();self.sync_workspace_dismiss();self.apply_fullscreen(self.st.now);self.raise_topmost();self.sync_ai_web();self.request_frame();
    }
    fn open_ai_window(&mut self) {
        if !self.st.ai_on() { return; }
        if self.ai_window.is_none(){match crate::ai_window::AiWindow::with_config(&self.st.settings.ai){Ok(mut w)=>{if let Err(e)=w.enable_file_drop(self.sink.clone()){pwarn!("AI file drop: {e}");}self.ai_window=Some(w);},Err(e)=>{self.st.ai.error=format!("Couldn't open full chat: {e}");return;}}}
        self.st.ai.full_open=true;if let Some(w)=self.ai_window.as_mut(){if self.st.ai.sites.selected.is_none(){w.focus_prompt(self.st.ai.draft.chars().count());}else{w.disable_file_drop();}w.show();}self.collapse();self.sync_ai_web();
    }
    pub fn handle_ai_window(&mut self,hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->Option<LRESULT> {
        if !self.ai_window.as_ref().is_some_and(|w|w.hwnd==hwnd){return None;}
        if msg==WM_KEYDOWN&&wp.0==VK_ESCAPE.0 as usize&&self.st.ai.sites.selected.is_some()&&self.st.ai.sites.menu{
            self.st.ai.sites.menu=false;if let Some(web)=self.ai_web.as_ref(){web.focus();}if let Some(window)=self.ai_window.as_ref(){window.invalidate();}self.request_frame();return Some(LRESULT(0));
        }
        if msg==crate::ai_window::CHECK_ACTIVATION {if !self.locked&&!self.display_off&&!self.st.power.saver&&!self.widgets_open{let next=unsafe{GetForegroundWindow()};if !next.is_invalid(){self.handle_ai_window(hwnd,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(next.0 as isize));}}return Some(LRESULT(0));}
        if msg==WM_KEYDOWN&&self.st.ai.sites.selected.is_none(){if let Some(field)=crate::ai::workspace_key(&mut self.st,wp.0 as u32,unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0,unsafe{GetKeyState(VK_SHIFT.0 as i32)}<0){if let Some(w)=self.ai_window.as_mut(){w.focus_field(field,0);}self.request_frame();return Some(LRESULT(0));}}
        if self.st.ai.sites.selected.is_none() && (msg==WM_PASTE || (msg==WM_KEYDOWN && wp.0==VK_V.0 as usize && unsafe{GetKeyState(VK_CONTROL.0 as i32)}<0)) && crate::ai::paste_attachment(&mut self.st,self.ai_window.as_ref().is_some_and(|w|w.prompt_focused()),services::ai::clipboard_has_attachment) {
            self.request_frame();return Some(LRESULT(0));
        }
        if msg==WM_KEYDOWN && wp.0==VK_ESCAPE.0 as usize && (self.st.ai.model_menu || self.st.ai.reasoning_menu || self.st.ai.attach_menu || self.st.ai.sites.menu) {
            self.st.ai.dismiss_popup();if let Some(w)=self.ai_window.as_mut(){if self.st.ai.model_menu{w.focus_field(wid("ai.model-search"),0);}else{w.focus_prompt(self.st.ai.draft.chars().count());}}self.request_frame();return Some(LRESULT(0));
        }
        if msg==WM_KEYDOWN&&wp.0==VK_ESCAPE.0 as usize{self.handle_ai_window(hwnd,WM_CLOSE,WPARAM(0),LPARAM(0));return Some(LRESULT(0));}
        let next=HWND(lp.0 as *mut _);
        let outside=msg==WM_ACTIVATE&&(wp.0&0xffff)as u32==WA_INACTIVE&&!self.st.settings.ai.keep_open&&!self.st.ai.attaching&&next!=hwnd&&!unsafe{IsChild(hwnd,next)}.as_bool()&&unsafe{GetAncestor(next,GA_ROOTOWNER)}!=hwnd;
        if msg==WM_CLOSE || outside {
            self.close_ai_host();return Some(LRESULT(0));
        }
        if msg==WM_EXITSIZEMOVE {if let Some(w)=self.ai_window.as_ref(){let(width,height)=w.size();self.st.settings.ai.mini_width=width;self.st.settings.ai.mini_height=height;self.st.cmd(Command::SaveSettings);}}
        if msg==WM_PAINT {unsafe{let _=windows::Win32::Graphics::Gdi::ValidateRect(hwnd,None);}self.st.now=clock::mono();if !self.locked && !self.display_off && !self.st.power.saver && !self.widgets_open && !unsafe{IsIconic(hwnd)}.as_bool() {if let Some(w)=self.ai_window.as_mut(){if let Err(e)=w.draw(&mut self.st){pwarn!("AI window: {e}");}}let mut deferred=vec![];self.run_commands(&mut deferred);self.request_frame();}return Some(LRESULT(0));}
        self.ai_window.as_mut()?.message(msg,wp,lp)
    }

    fn switch_search_browser(&mut self,browser:bool){
        if self.locked || self.display_off{return;}
        if self.st.search.detached {if browser{return;}self.toggle_mini_browser();}
        if !self.st.search.active{return;}
        if browser {
            if !self.st.settings.search.web{return;}
            self.st.search.launcher_mode=false;self.st.search.suggestions.cancel();self.st.search.address_suggestions.clear();self.st.search.address_editing=false;self.input.focused=None;
            if crate::browser_home::is_home(&self.st){self.focus_home();}
            else if let Some(web)=self.search_web.as_ref(){let _=web.resume();self.browser_resting=false;}
            else{let target=self.st.search.web_url.clone().map(|url|lumo_core::search::Target::Web{query:String::new(),url}).unwrap_or_else(||crate::search::browser_target("",self.st.settings.search.engine));self.execute_search(target);}
        }else{
            self.st.search.launcher_mode=true;self.st.search.suggestions.cancel();self.st.search.address_suggestions.clear();self.st.search.address_editing=false;
            if let Some(web)=self.search_web.as_mut(){self.st.search.navigation=web.navigation();self.st.search.web_url=Some(self.st.search.navigation.url.clone());web.hide();if !web.pause(){self.search_web=None;}}
            self.browser_resting=true;self.st.search.web_visible=false;self.input.focused=Some(wid("search.query"));self.input.caret=self.st.search.view.as_ref().map_or(0,|v|v.query.chars().count());
            self.input.selection_all=false;self.input.selection_anchor=None;self.focus_workspace();
        }
        self.layout_surface();self.request_frame();
    }
    fn focus_browser_field(&mut self,id:crate::ui::WidgetId,length:usize){
        self.st.search.address_editing=id==wid("search.query");
        if let Some(window)=self.browser_window.as_mut(){window.focus_field(id,length);}
        else{self.input.focused=Some(id);self.input.caret=length;self.input.selection_all=false;self.input.selection_anchor=None;self.input.selection_owner=Some(id);self.focus_workspace();unsafe{let _=SetFocus(self.hwnd);}}
    }
    fn focus_home(&mut self){
        self.st.search.address_editing=false;
        if let Some(w)=self.browser_window.as_mut(){w.blur_address();w.focus();}
        else{self.input.focused=None;self.focus_workspace();unsafe{let _=SetFocus(self.hwnd);}}
    }
    fn open_browser(&mut self){
        if !self.st.settings.search.web||self.locked||self.display_off{return;}
        if self.st.settings.search.resume_page&&self.st.search.web_url.is_some(){
            if self.st.search.detached{self.close_search();if let Some(w)=self.browser_window.as_ref(){w.focus();}}
            else{self.switch_search_browser(true);}
        }else{self.search_web=None;self.st.search.home_return=None;self.open_browser_home(false);if self.st.search.detached{self.close_search();}}
    }
    fn open_browser_home(&mut self,remember:bool){
        if !self.st.settings.search.web||self.locked||self.display_off{return;}
        if remember&&!crate::browser_home::is_home(&self.st){self.st.search.home_return=self.st.search.web_url.clone();}
        if !remember{self.st.search.home_return=None;}
        if let Some(web)=self.search_web.as_mut(){web.hide();if !web.pause(){self.search_web=None;}}
        self.browser_resting=false;self.st.search.launcher_mode=false;self.st.search.web_url=Some(crate::browser_home::URL.into());self.st.search.web_ready=true;self.st.search.web_visible=false;self.st.search.web_error.clear();
        self.st.search.navigation=crate::search_web::Navigation{url:crate::browser_home::URL.into(),title:String::new(),back:self.st.search.home_return.is_some(),..Default::default()};
        self.st.search.address.clear();self.st.search.address_dirty=false;self.st.search.address_suggestions.clear();self.st.search.suggestions.cancel();
        self.focus_home();
        if let Some(w)=self.browser_window.as_mut(){w.mark_dirty();}self.layout_surface();self.request_frame();
    }
    fn navigate_browser(&mut self,url:String){
        if url==crate::browser_home::URL{self.open_browser_home(true);return;}
        self.st.search.address_editing=false;self.st.search.address_suggestions.clear();self.st.search.suggestions.cancel();
        if let Some(w)=self.browser_window.as_mut(){w.blur_address();}
        if let Some(browser)=self.search_web.as_ref(){
            let returning=crate::browser_home::is_home(&self.st)&&self.st.search.home_return.as_deref()==Some(url.as_str());
            let _=browser.resume();self.browser_resting=false;if !returning{browser.navigate(&url);}
            self.st.search.web_url=Some(url.clone());self.st.search.launcher_mode=false;self.st.search.address=url;self.st.search.address_dirty=false;self.input.focused=None;
            self.st.search.navigation=browser.navigation();if let Some(w)=self.browser_window.as_mut(){w.mark_dirty();}
        }else if self.st.search.detached{self.restart_mini_browser(url);}else{self.execute_search(lumo_core::search::Target::Web{query:String::new(),url});}
        self.request_frame();
    }
    fn complete_address_suggestion(&mut self)->bool{
        if !self.st.search.address_editing{return false;}
        let index=self.st.search.address_selected.unwrap_or(0);
        let Some(text)=self.st.search.address_suggestions.get(index).cloned()else{return false;};
        self.st.search.address=text;self.st.search.address_dirty=true;self.st.search.address_selected=None;self.st.search.address_suggestions.clear();self.st.search.suggestions.cancel();
        let length=self.st.search.address.chars().count();
        if let Some(window)=self.browser_window.as_mut(){window.complete_address(length);}
        else{self.input.focused=Some(wid("search.query"));self.input.caret=length;self.input.selection_all=false;self.input.selection_anchor=None;self.input.selection_owner=Some(wid("search.query"));}
        self.request_frame();true
    }
    fn open_search(&mut self) {
        if self.locked || self.display_off {return;}
        self.st.now=clock::mono();
        if self.expanded&&self.st.page==PageKind::Ai {self.collapse();}
        self.st.ai.close_popovers();
        self.st.search.saved_view=None;self.st.search.view=Some(crate::search::View::new(&self.st));
        self.st.search.active=true;self.st.search.launcher_mode=true;
        if !self.st.search.error.is_empty(){self.st.search.loaded_at=None;}
        self.page_anim=None;self.collapse_due=false;unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}
        self.layout_surface();self.expand();
        self.input.focused=Some(wid("search.query"));self.input.caret=self.st.search.view.as_ref().map_or(0,|v|v.query.chars().count());self.input.selection_all=false;self.input.selection_anchor=None;self.input.selection_owner=None;self.input.text_scroll=0.0;self.input.keys.clear();
        self.st.closed_view=Default::default();self.st.search.notice.clear();self.focus_workspace();self.sync_workspace_dismiss();self.start_search_catalog(false);self.start_search_integrations();

    }
    fn park_search_view(&mut self){
        if let Some(view)=self.st.search.view.take(){if self.st.settings.search.resume_page{self.st.search.saved_view=Some(view);}}
    }
    fn park_search_web(&mut self){
        if self.st.search.detached {return;}
        if crate::browser_home::is_home(&self.st){if !self.st.settings.search.resume_page{self.close_search_web();}return;}
        if !self.st.settings.search.resume_page{self.close_search_web();return;}
        if let Some(browser)=self.search_web.as_mut(){let navigation=browser.navigation();self.st.search.web_url=Some(navigation.url.clone());self.st.search.address=navigation.url.clone();self.st.search.navigation=navigation;if !browser.pause(){self.search_web=None;}}
        self.st.search.web_visible=false;self.st.search.address_dirty=false;
    }
    fn close_search(&mut self) {
        self.st.search.cancel();
        if self.st.search.active {self.collapse();}
    }
    /// Search is a transient surface even after its editor loses focus. A
    /// native menu defers dismissal until it returns; its outside click must
    /// then be reconciled with the real foreground window, without stealing
    /// focus back. WebView child windows belong to the same surface.
    fn search_lost_activation(&mut self,next:HWND) {
        let ai=self.expanded&&self.st.page==PageKind::Ai&&!self.st.search.active;
        if !(self.st.search.active||ai) || self.st.search.context_menu || (ai&&(self.st.ai.attaching)) || next==self.hwnd || unsafe{IsChild(self.hwnd,next)}.as_bool() || unsafe{GetAncestor(next,GA_ROOTOWNER)}==self.hwnd{return;}
        self.text_focus=false;self.input.focused=None;self.set_noactivate(true);
        if ai {if !self.st.settings.ai.keep_open{self.collapse();}}else if !self.st.settings.search.keep_open{self.close_search();}
        self.request_frame();
    }
    fn sync_workspace_dismiss(&mut self) {
        let open=self.expanded&&(if self.st.search.active{!self.st.settings.search.keep_open&&!self.st.search.context_menu}else{self.st.page==PageKind::Ai&&!self.st.settings.ai.keep_open&&!self.st.ai.attaching});
        if open&&!self.locked&&!self.display_off&&!self.widgets_open {
            if self.workspace_dismiss.is_none(){self.workspace_dismiss=services::workspace_dismiss::Watch::start(self.hwnd).ok();}
        }else{self.workspace_dismiss=None;}
    }
    fn start_search_icons(&mut self, items:Vec<(String,lumo_core::search::Target)>) {
        if !self.st.search.active || !(self.st.settings.search.apps||self.st.settings.search.powertoys) || self.locked || self.display_off || self.st.power.saver || self.st.search.icon_pending.is_some() || self.selftest.is_some(){return;}
        let items:Vec<_>=items.into_iter().filter(|(id,_)|!self.st.search.icons.contains_key(id)).take(16).collect();
        if items.is_empty(){return;}
        // Bounded 48px icon cache: under 576 KiB of pixels, no persistent worker.
        if self.st.search.icons.len()+items.len()>64 {self.st.search.icons.clear();}
        self.st.search.request=self.st.search.request.wrapping_add(1);
        let request=self.st.search.request;
        let cancel=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.st.search.icon_pending=Some(crate::search::Scan{request,cancel:cancel.clone()});
        services::search::start_icons(request,items,cancel,self.sink.clone());
    }
    fn sync_search_suggestions(&mut self){
        let allowed=!self.locked && !self.display_off && !self.widgets_open && !self.st.power.saver && (!self.fullscreen_hidden||self.st.search.detached);
        let query=if allowed{crate::search::suggestion_query(&mut self.st)}else{None};
        let Some(query)=query else{
            if !self.st.search.suggestions.query.is_empty(){self.st.search.suggestions.cancel();}
            if let Some(v)=self.st.search.view.as_mut(){v.suggestions.clear();}self.st.search.address_suggestions.clear();self.st.search.address_selected=None;return;
        };
        let address=self.st.search.address_editing && (self.st.search.embedded()||self.st.search.detached);
        let state=&mut self.st.search.suggestions;
        if state.query!=query || state.address!=address {
            state.cancel();state.query=query.clone();state.address=address;
            self.st.search.address_suggestions.clear();self.st.search.address_selected=None;
            // One trailing-edge wake after typing pauses; never an idle poll.
            state.due=Some(self.st.now+0.35);
            if let Some(v)=self.st.search.view.as_mut(){v.suggestions.clear();}
        }
        if state.due.is_some_and(|t|self.st.now>=t) && state.pending.is_none(){
            state.due=None;
            let cancel=std::sync::Arc::new(services::http::Cancel::default());let id=state.serial;
            state.pending=Some((id,cancel.clone()));services::search::start_suggestions(id,query,cancel,self.sink.clone());
        }
    }
    fn start_search_catalog(&mut self,force:bool) {
        if !self.st.settings.search.apps || self.locked || self.display_off || self.st.power.saver || self.st.search.pending.is_some() || self.selftest.is_some() {return;}
        if !force && !self.st.search.active{return;}
        let now=clock::mono();
        // App installations are re-read on explicit refresh, or on an open
        // after ten minutes. This is a cache age check, never a polling timer.
        if !force && self.st.search.loaded_at.is_some_and(|last|now-last<600.0){return;}
        self.st.search.request=self.st.search.request.wrapping_add(1);
        let request=self.st.search.request;
        let cancel=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.st.search.pending=Some(crate::search::Scan{request,cancel:cancel.clone()});
        self.st.search.error.clear();
        services::search::start(request,cancel,self.sink.clone());
        self.request_frame();
    }
    fn start_search_integrations(&mut self){
        if self.locked || self.display_off || self.st.power.saver || self.widgets_open || !self.st.search.active || self.selftest.is_some(){return;}
        if let Some(p)=self.st.search.integrations_pending.as_ref(){if !p.cancel.load(std::sync::atomic::Ordering::Relaxed){return;}}
        self.st.search.integrations_pending=None;
        if !services::search_integrations::enabled(&self.st.settings.search){self.st.search.integrations.clear();return;}
        self.st.search.integrations_request=self.st.search.integrations_request.wrapping_add(1);let request=self.st.search.integrations_request;
        let cancel=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.st.search.integrations_pending=Some(crate::search::Scan{request,cancel:cancel.clone()});self.st.search.integrations_warning.clear();
        services::search_integrations::start(request,self.st.settings.search.clone(),cancel,self.sink.clone());
    }
    fn search_service(&mut self,name:&str,start:bool){
        if !self.st.settings.search.services || self.st.search.action_pending || self.locked || self.display_off || self.st.power.saver{return;}
        let target=self.st.search.integrations.iter().find_map(|e|if let lumo_core::search::Target::Service(s)=&e.target{(s.name==name && if start{s.can_start()}else{s.can_stop()}).then(||e.target.clone())}else{None});
        if let Some(target)=target{self.st.search.action_pending=true;self.st.search.notice=if start{"Requesting service start…"}else{"Requesting service stop…"}.into();services::search_integrations::run_action(target,Some(start),self.sink.clone());self.request_frame();}
    }
    fn execute_search(&mut self,target:lumo_core::search::Target) {
        use lumo_core::search::Target;
        if !matches!(target,Target::App(_)|Target::Windows(_)|Target::Web{..}){
            if self.st.search.action_pending{return;}
            let Some(entry)=crate::search::entries(&self.st).into_iter().find(|e|e.target==target && e.enabled(&self.st.settings.search))else{return;};
            self.record_search(&entry);self.st.search.action_pending=true;self.close_search();
            services::search_integrations::run_action(target,None,self.sink.clone());self.request_frame();return;
        }
        if let Target::Web{url,..}=&target{
            if !self.st.settings.search.web{return;}
            if url==crate::browser_home::URL{self.record_search(&lumo_core::search::browser_entry());self.open_browser();return;}
            if self.st.search.detached {
                if let Some(browser)=self.search_web.as_ref(){browser.navigate(url);self.st.search.address=url.clone();self.st.search.address_dirty=false;}
                else {self.restart_mini_browser(url.clone());}
                self.close_search();if let Some(window)=self.browser_window.as_ref(){window.focus();}self.request_frame();return;
            }
            let transition=self.st.search.view.as_ref().map(|v|v.query.clone()).unwrap_or_default();
            self.st.search.suggestions.cancel();
            self.close_search_web();self.st.search.browser_transition=Some(self.st.now);self.st.search.transition_query=transition;
            self.st.search.launcher_mode=false;
            self.st.search.web_url=Some(url.clone());self.st.search.address=url.clone();self.st.search.navigation=crate::search_web::Navigation{url:url.clone(),loading:true,progress:0.08,..Default::default()};self.input.focused=None;self.st.search.web_ready=false;
            if self.st.power.saver{self.st.search.web_error="Web search is paused in battery saver.".into();self.st.search.navigation.loading=false;self.st.search.navigation.progress=0.0;}
            else{match crate::search_web::Browser::new_filtered(self.hwnd,url,&self.st.settings.web){Ok(browser)=>self.search_web=Some(browser),Err(error)=>{self.st.search.web_error=error;self.st.search.navigation.loading=false;self.st.search.navigation.progress=0.0;}}}
            self.layout_surface_for(if self.st.settings.search.browser_wide{960.0}else{crate::search::WIDTH+60.0},f32::from(self.st.settings.search.browser_height)+60.0);self.request_frame();return;
        }
        if let Some(entry)=crate::search::entries(&self.st).into_iter().find(|e|e.target==target){self.record_search(&entry);}
        self.close_search();
        match target {Target::App(app)=>shell::open_application(&app),Target::Windows(setting)=>shell::open_windows_setting(setting),_=>{}}
        self.request_frame();
    }
    fn record_search(&mut self,entry:&lumo_core::search::Entry){
        lumo_core::search::record_usage(&mut self.st.store.search_usage,entry.id.clone(),clock::unix_now().max(0)as u64);self.st.save();
    }
    fn search_context_menu(&mut self,point:Option<Point>,deferred:&mut Vec<Deferred>){
        if !self.st.search.active || self.st.search.context_menu || self.st.search.embedded(){return;}
        let id=point.and_then(|p|self.hit_at(p).map(|h|h.id));
        let Some(view)=self.st.search.view.as_mut()else{return;};
        let entry=if id==Some(wid("search.web")) {
            crate::search::web_entry(&view.query,self.st.settings.search.engine)
        }else if let Some(id)=id{view.result_at(id)}else if point.is_none(){view.selected()}else{None};
        let Some(entry)=entry else{return;};
        let body=self.island.body_rect_at(self.cx);let p=point.unwrap_or(Point::new(body.x+80.0,body.y+96.0));let scale=self.gfx.scale();let mut screen=POINT{x:(p.x*scale).round()as i32,y:(p.y*scale).round()as i32};unsafe{let _=windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd,&mut screen);}
        self.st.search.context_menu=true;self.sync_workspace_dismiss();deferred.push(Deferred::SearchMenu(entry,screen));self.request_frame();
    }
    fn search_web_action(&mut self,action:crate::search_web::Action){
        use crate::search_web::Action;
        if (!self.st.search.active && !self.st.search.detached) || self.st.search.web_url.is_none(){return;}
        if crate::browser_home::is_home(&self.st) {
            match action {
                Action::Address=>{self.st.search.address.clear();self.focus_browser_field(wid("search.query"),0);self.st.search.address_editing=true;},
                Action::Back=>{if let Some(url)=self.st.search.home_return.clone(){self.navigate_browser(url);self.st.search.home_return=None;}else{let detached=self.st.search.detached;self.close_search_web();if detached{self.open_search();}self.focus_browser_field(wid("search.query"),0);}},
                Action::Reload=>self.focus_home(),
                _=>{},
            }
            self.request_frame();return;
        }
        if matches!(action,Action::Back|Action::Forward|Action::Reload){self.st.search.address_dirty=false;self.st.search.address=self.st.search.navigation.url.clone();self.input.focused=None;}
        match action {
            Action::Address=>{self.st.search.address_editing=true;if let Some(window)=self.browser_window.as_mut(){window.focus_address(self.st.search.address.chars().count());}else{self.focus_text();unsafe{let _=SetFocus(self.hwnd);}self.input.focused=Some(wid("search.query"));self.input.caret=self.st.search.address.chars().count();self.input.selection_all=true;self.input.selection_anchor=Some(0);}},
            Action::Back if !self.st.search.navigation.back=>self.open_browser_home(false),
            Action::Reload if self.search_web.is_none() || (!self.st.search.web_ready && !self.st.search.web_error.is_empty())=>{if let Some(url)=self.st.search.web_url.clone(){if self.st.search.detached{self.restart_mini_browser(url);}else{self.execute_search(lumo_core::search::Target::Web{query:String::new(),url});}}},
            _=>{if let Some(browser)=self.search_web.as_ref(){browser.action(action);}},
        }
        self.request_frame();
    }
    fn close_search_web(&mut self){self.search_web=None;self.browser_window=None;self.browser_resting=false;self.st.search.detached=false;self.st.search.browser_transition=None;self.st.search.web_url=None;self.st.search.web_ready=false;self.st.search.web_visible=false;self.st.search.web_error.clear();self.st.search.navigation=Default::default();self.st.search.address.clear();self.st.search.address_dirty=false;self.st.search.address_editing=false;self.st.search.address_suggestions.clear();self.st.search.address_selected=None;self.st.search.suggestions.cancel();}
    fn sync_search_web(&mut self)->bool{
        let settled=self.island.settled() && self.island.target()==self.geom_for(&self.target_mode());
        if self.search_web.as_ref().is_some_and(|b|b.discarded()){self.search_web=None;self.st.search.web_ready=false;}
        if !self.st.search.active && !self.st.search.detached{return false;}
        if self.st.search.launcher_mode&&!self.st.search.detached {
            if !self.browser_resting{if let Some(browser)=self.search_web.as_mut(){browser.hide();if !browser.pause(){self.search_web=None;}}self.browser_resting=true;}
            let changed=self.st.search.web_visible;self.st.search.web_visible=false;return changed;
        }
        if crate::browser_home::is_home(&self.st){
            let away=self.locked||self.display_off||self.widgets_open||self.st.power.saver||self.browser_window.as_ref().is_some_and(|w|unsafe{IsIconic(w.hwnd)}.as_bool());
            if let Some(window)=self.browser_window.as_mut(){if away{window.hide();}else if self.browser_resting{window.mark_dirty();}}
            self.browser_resting=away;let changed=self.st.search.web_visible;self.st.search.web_visible=false;self.st.search.web_ready=true;return changed;
        }
        if !self.st.search.detached&&self.browser_resting {self.browser_resting=false;if let Some(browser)=self.search_web.as_ref(){let _=browser.resume();}}
        if self.st.search.detached {
            let away=self.locked || self.display_off || self.widgets_open || self.st.power.saver || self.browser_window.as_ref().is_some_and(|w|unsafe{IsIconic(w.hwnd)}.as_bool());
            if away {
                if let Some(window)=self.browser_window.as_mut(){window.hide();}
                if !self.browser_resting {self.browser_resting=true;if self.search_web.as_mut().is_some_and(|b|!b.pause()){self.search_web=None;}}
                let changed=self.st.search.web_visible;self.st.search.web_visible=false;return changed;
            }
            if self.browser_resting {
                self.browser_resting=false;
                if let Some(browser)=self.search_web.as_ref(){let _=browser.resume();}
                else if let Some(url)=self.st.search.web_url.as_deref(){match crate::search_web::Browser::new_filtered(self.hwnd,url,&self.st.settings.web){Ok(mut b)=>{if let Some(w)=self.browser_window.as_ref(){let _=b.reparent(w.hwnd);}self.search_web=Some(b);self.st.search.web_error.clear();},Err(e)=>self.st.search.web_error=e}}
                if let Some(w)=self.browser_window.as_mut(){w.mark_dirty();}
            }
        }
        let Some(browser)=self.search_web.as_mut()else{return false;};let (ready,error)=browser.status();self.st.search.web_ready=ready;self.st.search.web_error=error;
        let navigation=browser.navigation();
        let updated=self.st.search.navigation!=navigation;
        if navigation.url!=self.st.search.navigation.url && !self.st.search.address_dirty {self.st.search.address=navigation.url.clone();}
        self.st.search.web_url=Some(navigation.url.clone());self.st.search.navigation=navigation;
        let bounds=if let Some(window)=self.browser_window.as_ref(){window.page_bounds().0}else{let page=crate::search::snapped_web_rect(self.island.body_rect_at(self.cx),self.gfx.scale());let scale=self.gfx.scale();RECT{left:(page.x*scale).round()as i32,top:(page.y*scale).round()as i32,right:(page.right()*scale).round()as i32,bottom:(page.bottom()*scale).round()as i32}};
        let transition_ready=self.st.search.browser_transition.is_none_or(|t|self.st.now-t>=0.22);
        let visible=(self.st.search.active || self.st.search.detached) && ready && transition_ready && (settled || self.st.search.detached) && self.st.search.web_error.is_empty() && !self.locked && !self.display_off && !self.st.power.saver;
        let scale=self.browser_window.as_ref().map_or(self.gfx.scale(),|w|w.page_bounds().1);
        let body=if let Some(window)=self.browser_window.as_ref(){let mut r=RECT::default();unsafe{let _=GetClientRect(window.hwnd,&mut r);}Rect::new(0.0,0.0,r.right as f32/scale,r.bottom as f32/scale)}else{self.island.body_rect_at(self.cx)};
        browser.clip_radius(crate::search::web_radius(&self.st));
        browser.cover_top(crate::search::address_cover(&self.st,body,scale));
        let changed=self.st.search.web_visible!=visible;self.st.search.web_visible=visible;browser.place_scaled(bounds,visible,scale);
        if let Some(window)=self.browser_window.as_mut(){if changed || updated{window.mark_dirty();}}
        changed
    }
    fn toggle_mini_browser(&mut self) {
        self.st.search.browser_transition=None;
        if self.st.search.detached {
            if let Some(browser)=self.search_web.as_mut(){if let Err(e)=browser.reparent(self.hwnd){pwarn!("browser return: {e}");return;}}
            self.browser_window=None;self.st.search.detached=false;self.st.search.web_visible=false;
            self.open_search();self.switch_search_browser(true);
        } else {
            if self.st.search.web_url.is_none() || self.locked || self.display_off || self.st.power.saver{return;}
            let window=match crate::browser_window::BrowserWindow::new(self.hwnd,&self.st.settings.search){Ok(w)=>w,Err(e)=>{pwarn!("mini browser: {e}");return;}};
            if let Some(browser)=self.search_web.as_mut(){if let Err(e)=browser.reparent(window.hwnd){pwarn!("mini browser: {e}");return;}}
            self.browser_window=Some(window);self.st.search.detached=true;self.st.search.web_visible=false;
            self.collapse();
        }
        self.request_frame();
    }
    fn restart_mini_browser(&mut self,url:String){
        self.search_web=None;self.st.search.web_url=Some(url.clone());self.st.search.address=url.clone();self.st.search.address_dirty=false;
        self.st.search.web_ready=false;self.st.search.web_visible=false;self.st.search.web_error.clear();
        self.st.search.navigation=crate::search_web::Navigation{url:url.clone(),loading:true,progress:0.08,..Default::default()};
        match crate::search_web::Browser::new_filtered(self.hwnd,&url,&self.st.settings.web){
            Ok(mut browser)=>{if let Some(w)=self.browser_window.as_ref(){if let Err(e)=browser.reparent(w.hwnd){self.st.search.web_error=e;return;}}self.search_web=Some(browser);},
            Err(e)=>{self.st.search.web_error=e;self.st.search.navigation.loading=false;self.st.search.navigation.progress=0.0;}
        }
        if let Some(w)=self.browser_window.as_mut(){w.mark_dirty();}self.request_frame();
    }
    fn draw_browser_window(&mut self,deferred:&mut Vec<Deferred>){
        if !self.st.search.detached || self.browser_resting{return;}
        if let Some(window)=self.browser_window.as_mut(){match window.draw(&mut self.st){Ok(true)=>self.request_frame(),Err(e)=>pwarn!("mini browser draw: {e}"),_=>{}}}
        if !self.st.cmds.is_empty(){self.run_commands(deferred);if let Some(w)=self.browser_window.as_mut(){w.mark_dirty();}self.request_frame();}
    }
    pub fn handle_browser_window(&mut self,hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->Option<LRESULT>{
        if !self.browser_window.as_ref().is_some_and(|w|w.hwnd==hwnd){return None;}
        if msg==WM_KEYDOWN && wp.0==VK_TAB.0 as usize{if self.complete_address_suggestion(){return Some(LRESULT(0));}if let Some(browser)=self.search_web.as_ref(){browser.focus();}return Some(LRESULT(0));}
        let result=self.browser_window.as_mut()?.message(&mut self.st,msg,wp,lp);
        if matches!(msg,WM_MOVE|WM_EXITSIZEMOVE|WM_DPICHANGED){if let Some(browser)=self.search_web.as_ref(){browser.parent_moved();}}
        if result.is_some(){self.request_frame();}
        result
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
            self.st.ai.dismiss_popup();if let Some(w)=self.settings_window.as_mut(){w.dismiss_model_picker();}return Some(LRESULT(0));
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
        if self.st.search.view.is_some() {self.st.search.active=false;self.st.search.cancel();self.park_search_web();self.park_search_view();self.blur_text();self.layout_surface();}
        let p = if p == PageKind::Ai && !self.st.ai_on() { PageKind::Media } else { p };
        if p==PageKind::Ai && self.st.ai.sites.selected.is_none(){crate::ai::ensure(&mut self.st);}
        if p != self.st.page {
            self.st.ai.close_popovers();
            self.ai_resize_hold=None;
            if p!=PageKind::Ai {self.ai_reply_peek=false;}
            self.st.page = p;
            // Settings is never where the island opens next time.
            if p != PageKind::Settings && p!=PageKind::Ai {
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
            if !self.st.ai_on() && matches!(c, Command::Ai(_) | Command::SelectAiSite(_) | Command::OpenAiWindow | Command::ReturnAiToIsland | Command::CloseAi | Command::ToggleAiKeepOpen | Command::ToggleAiResume | Command::AiHeight(_)) { continue; }
            match c {
                Command::OpenSearch => self.open_search(),
                Command::ExecuteSearch(target) => self.execute_search(target),
                Command::ReorderSearchPin{source,target}=>{if lumo_core::search::move_pin(&mut self.st.store.search_pins,&source,&target){self.st.save();crate::search::refresh(&mut self.st);}},
                Command::SearchService{name,start}=>self.search_service(&name,start),
                Command::SearchBrowserMenu(point)=>{let owner=self.browser_window.as_ref().map_or(self.hwnd,|w|w.hwnd);let scale=self.browser_window.as_ref().map_or(self.gfx.scale(),|w|w.page_bounds().1);let mut at=POINT{x:(point.x*scale).round() as i32,y:(point.y*scale).round() as i32};unsafe{let _=windows::Win32::Graphics::Gdi::ClientToScreen(owner,&mut at);}deferred.push(Deferred::BrowserMenu(at));},
                Command::SearchIcons(items) => self.start_search_icons(items),
                Command::CloseSearchWeb=>{let detached=self.st.search.detached;self.close_search_web();if detached{self.open_search();}self.input.focused=Some(wid("search.query"));unsafe{let _=SetFocus(self.hwnd);}},
                Command::ToggleMiniBrowser=>self.toggle_mini_browser(),
                Command::CloseMiniBrowser=>self.close_search_web(),
                Command::ToggleSearchKeepOpen=>{self.st.settings.search.keep_open=!self.st.settings.search.keep_open;self.collapse_due=false;unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}if !self.st.settings.search.keep_open{self.maybe_schedule_collapse();}self.st.cmd(Command::SaveSettings);},
                Command::ToggleSearchResume=>{self.st.settings.search.resume_page=!self.st.settings.search.resume_page;if !self.st.settings.search.resume_page{self.st.search.saved_view=None;if !self.st.search.active && !self.st.search.detached{self.close_search_web();}}self.st.cmd(Command::SaveSettings);},
                Command::SearchWeb(action)=>self.search_web_action(action),
                Command::NavigateSearchWeb(url)=>self.navigate_browser(url),
                Command::RefreshSearchApps => {if let Some(p)=self.st.search.icon_pending.take(){p.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}self.st.search.icons.clear();self.start_search_catalog(true);if let Some(p)=self.st.search.integrations_pending.take(){p.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}self.start_search_integrations();},
                Command::AiHeight(height) => {let dragging=self.input.pressed_id==Some(wid("ai.resize-grip"));self.resize_ai_chat(height,dragging);},
                Command::OpenAiWindow => self.open_ai_window(),
                Command::ReturnAiToIsland=>self.return_ai_host(),
                Command::CloseAi=>self.close_ai_host(),
                Command::OpenAdBlocking => {
                    let key="browser";
                    self.open_settings();if let Some(w)=self.settings_window.as_mut(){w.show_section(key);}
                },
                Command::PrepareAdBlocking=>{if !self.locked&&!self.display_off&&!self.st.power.saver{crate::web_blocking::ensure(&mut self.st,self.sink.clone());}},
                Command::SelectAiSite(site)=>self.select_ai_site(site),
                Command::ToggleAiKeepOpen=>{self.st.settings.ai.keep_open=!self.st.settings.ai.keep_open;self.collapse_due=false;unsafe{let _=KillTimer(self.hwnd,TIMER_COLLAPSE);}if !self.st.settings.ai.keep_open{let next=unsafe{GetForegroundWindow()};if let Some(hwnd)=self.ai_window.as_ref().map(|w|w.hwnd){self.handle_ai_window(hwnd,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(next.0 as isize));}else{self.search_lost_activation(next);}}self.st.cmd(Command::SaveSettings);},
                Command::ToggleAiResume=>{self.st.settings.ai.resume=!self.st.settings.ai.resume;if self.st.settings.ai.resume{self.st.ai.fresh_on_open=false;}self.st.cmd(Command::SaveSettings);},
                Command::Ai(a) => {
                    let owner=self.ai_window.as_ref().map(|w|w.hwnd).or_else(||self.settings_window.as_ref().map(|w|w.hwnd)).unwrap_or(self.hwnd);
                    services::ai::run(&mut self.st,a,SendHwnd::new(owner),self.sink.clone());
                }
                Command::OpenSettings => {let ai=self.st.page==PageKind::Ai||self.st.ai.full_open;self.open_settings();if ai {if let Some(w)=self.settings_window.as_mut(){w.show_ai();}}},
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
                Command::ChooseMusicDirectory => self.choose_folder(crate::commands::FolderPick::Directory),
                Command::ChoosePhoneDownloadDirectory => self.choose_folder(crate::commands::FolderPick::PhoneInbox),
                Command::PreviewFocusSpotlight => self.start_focus_light(true),
                Command::ChooseLyricsDownloadDirectory => self.choose_folder(crate::commands::FolderPick::LyricsDirectory),
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
            }
        }
        self.sync_phone_offer();
    }

    // ------------------------------------------------------------ phone drop

    fn start_focus_light(&mut self, preview: bool) {
        if (!self.st.settings.focus_spotlight && !preview) || self.locked || self.display_off || self.st.power.saver || self.fullscreen_hidden || self.widgets_open { return; }
        let body = self.island.body_rect_at(self.cx);
        let scale = self.gfx.scale();
        let island = RECT {
            left: self.surface.x + (body.x * scale).round() as i32,
            top: self.surface.y + (body.y * scale).round() as i32,
            right: self.surface.x + (body.right() * scale).round() as i32,
            bottom: self.surface.y + (body.bottom() * scale).round() as i32,
        };
        let color = crate::focus_light::color(self.st.settings.focus_spotlight_color, self.st.accent);
        self.focus_light = match crate::focus_light::Spotlight::new(self.hwnd, self.monitor.rect, island, scale, color, self.st.now) {
            Ok(light) => Some(light),
            Err(error) => { pwarn!("focus light couldn't start: {error}"); None }
        };
        self.focus_light_preview = preview;
        self.request_frame();
    }

    fn draw_focus_light(&mut self) -> bool {
        let Some(light) = self.focus_light.as_mut() else { return false; };
        match light.frame(self.st.now) {
            Ok(true) => true,
            Ok(false) => { self.focus_light = None; self.focus_light_preview = false; false }
            Err(error) => { pwarn!("focus light stopped: {error}"); self.focus_light = None; self.focus_light_preview = false; false }
        }
    }

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
            match services::phonedrop::PhoneDrop::start(self.sink.clone(), offer.iter().map(PathBuf::from).collect(), paths::phone_inbox_dir(&self.st.settings)) {
                Ok((svc, url)) => {
                    pd.qr = QrMatrix::encode(&url);
                    pd.url = url;
                    pd.pc_name = services::phonedrop::pc_name();
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
        match services::phonedrop::PhoneDrop::start(self.sink.clone(), Vec::new(), paths::phone_inbox_dir(&self.st.settings)) {
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
fn files_label(n: usize) -> String {
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::HIT_MARGIN;
    use crate::state::MediaSession;

    #[test]
    #[ignore = "native effect uses an off-screen monitor fixture, with no personal files"]
    fn focus_completion_preview_and_suspend_respect_the_setting() {
        use lumo_core::pomodoro::{Phase, RunState};
        unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { DefWindowProcW(hwnd, msg, wp, lp) }
        window::register_classes(proc, proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel(); let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.monitor.rect = RECT { left: -4200, top: -1800, right: -3400, bottom: -1200 };
        app.surface.x = -4200; app.surface.y = -1800;
        app.st.now = 100.0; clock::hold(Some(100.0));
        let mut deferred = vec![];
        app.st.cmd(Command::PreviewFocusSpotlight);
        app.run_commands(&mut deferred);
        assert!(app.focus_light.is_some() && app.focus_light_preview);
        assert!(!app.st.settings.focus_spotlight);
        assert!(app.draw_focus_light());
        app.st.now = 104.0;
        assert!(!app.draw_focus_light()); assert!(app.focus_light.is_none());
        app.st.settings.focus_spotlight = true;
        app.st.now = 105.0;
        app.st.pomodoro.phase = Phase::Focus; app.st.pomodoro.run = RunState::Running { ends_at: 105.0 };
        app.tick(); assert!(app.focus_light.is_some());
        assert!(!app.focus_light_preview);
        app.focus_light = None;
        app.st.pomodoro.phase = Phase::ShortBreak; app.st.pomodoro.run = RunState::Running { ends_at: 105.0 };
        app.tick(); assert!(app.focus_light.is_none(), "break completion does not trigger the effect");
        app.st.pomodoro.phase = Phase::Focus; app.st.pomodoro.run = RunState::Running { ends_at: 90.0 };
        app.tick(); assert!(app.focus_light.is_none(), "stale completions after sleep do not replay lighting");
        for state in 0..3 {
            app.locked = false; app.display_off = false; app.st.power.saver = false;
            app.start_focus_light(true); assert!(app.focus_light.is_some());
            match state { 0 => app.locked = true, 1 => app.display_off = true, _ => app.st.power.saver = true }
            app.frame(&mut deferred, None);
            assert!(app.focus_light.is_none());
            app.start_focus_light(true); assert!(app.focus_light.is_none());
        }
        app.clock.set_active(false); app.gfx.set_shown(false); clock::hold(None);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }

    #[test]
    #[ignore = "renders isolated native fixtures, without starting network services"]
    fn search_scroll_and_local_phone_drop_layouts() {
        unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { DefWindowProcW(hwnd, msg, wp, lp) }
        window::register_classes(proc, proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        app.st.settings.search.settings = false;
        app.st.search.loaded_at = Some(0.0);
        app.st.search.apps = (0..18).map(|i| lumo_core::search::Entry::new(format!("example-{i}"), format!("Example app {i:02}"), String::new(), "example", lumo_core::search::Group::App, lumo_core::search::Target::Windows(lumo_core::search::WindowsSetting::About))).collect();
        app.st.pinned = true;
        app.st.notice = None;
        let mut deferred = vec![];
        clock::hold(Some(100.0));
        app.open_search();
        app.input.keys.extend("example".chars().map(KeyInput::Char));
        app.frame(&mut deferred, None);
        app.island.snap(app.geom_for(&app.target_mode()));
        app.frame(&mut deferred, None);
        let first = app.hits.iter().find(|h| h.id == crate::ui::wid_n("search.result", 0)).unwrap().rect;
        app.input.wheel = -0.125;
        clock::hold(Some(100.016));
        app.frame(&mut deferred, None);
        let moving = app.hits.iter().find(|h| h.id == crate::ui::wid_n("search.result", 0)).unwrap().rect;
        assert!(moving.h < first.h && moving.h > first.h - 5.0, "fractional movement clips only the scrolled-away part of a row");
        assert_eq!(app.st.search.view.as_ref().unwrap().selected, 0, "scrolling does not move keyboard selection");
        for step in 1..=90 { clock::hold(Some(100.016 + step as f64 / 60.0)); app.frame(&mut deferred, None); }
        for _ in 0..12 { app.handle_island(WM_KEYDOWN, WPARAM(VK_DOWN.0 as usize), LPARAM(0), &mut deferred); }
        for step in 1..=90 { clock::hold(Some(102.0 + step as f64 / 60.0)); app.frame(&mut deferred, None); }
        assert!(app.hits.iter().any(|h| h.id == crate::ui::wid_n("search.result", 12)), "keyboard navigation reveals the selected row");
        app.collapse();
        app.st.search.view = None;

        let url = "HTTP://192.168.1.20:53530/EXAMPLESESSION/";
        for theme in [Theme::Classic, Theme::Pill] {
            app.st.settings.theme = theme;
            app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));
            for scene in ["ip", "ask", "receiving", "complete", "error"] {
                clock::hold(Some(200.0)); app.st.now = 200.0;
                let mut pd = PhoneDrop { url: url.into(), qr: QrMatrix::encode(url), pc_name: "Studio-PC".into(), ..Default::default() };
                match scene {
                    "ask" => pd.ask = Some(PhoneAsk { id: 1, from: "iPhone".into(), files: vec![("Harbor walk.mov".into(), 18_402_331)], total: 18_402_331 }),
                    "receiving" => pd.transfer = Some(PhoneTransfer { id: 1, from: "iPhone".into(), file: 0, files: 2, done: 9_100_000, total: 24_000_000 }),
                    "complete" => pd.outcome = Some(("2 files are on the Shelf".into(), true, 208.0)),
                    "error" => pd.error = Some("This PC isn't on a Wi-Fi or wired network.".into()),
                    _ => {}
                }
                app.st.search.active = false;
                app.st.phone = Some(pd);
                app.st.page = PageKind::Shelf;
                app.expanded = true;
                app.mode = Mode::Expanded(PageKind::Shelf);
                app.island.snap(app.geom_for(&app.mode));
                app.frame(&mut deferred, None);
                assert!(!app.hits.iter().any(|h| h.id == wid("shelf.pair.open") || h.id == wid("shelf.pair.another")));
                assert!(app.hits.iter().any(|h| h.id == wid("shelf.phone.done")));
                assert!(!app.hits.iter().any(|h| h.id == wid("shelf.phone.name")));
                if let Some(root) = std::env::var_os("LUMO_PHONE_QA") {
                    let root = PathBuf::from(root); std::fs::create_dir_all(&root).unwrap();
                    let (w, h, pixels, _) = app.selftest_capture();
                    crate::png::save_rgba(root.join(format!("phone-{scene}-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
                }
            }
        }
        app.clock.set_active(false); app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
        clock::hold(None);
    }

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
            app.st.settings.set_addon("ai", true);
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
            app.set_page(PageKind::Tasks);
            app.open_island();
            assert_eq!(app.st.page, PageKind::Tasks, "an open page never switches automatically");
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

    #[test]#[ignore="requires a Windows desktop; offline song-transition and action-hint fixture"]
    fn music_toolbar_stays_fixed_across_song_gaps_and_lyric_updates(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let fixture=crate::lyrics_window::sample_state();let song=fixture.media.unwrap();let lyrics=fixture.lyrics;
        for(index,theme)in[Theme::Classic,Theme::Pill].into_iter().enumerate(){for width in[480.0,620.0]{
            clock::hold(Some(100.0+index as f64*10.0));app.st.settings.theme=theme;app.st.settings.expanded_width=width;app.st.settings.hide_over_maximized=false;app.st.media=Some(song.clone());app.st.lyrics=lyrics.clone();app.st.power.saver=true;app.st.foreground_app=Some(crate::state::ForegroundApp{executable:"code.exe".into(),name:"Code".into(),maximized_here:true});app.st.settings.keep_app_visible("code.exe".into(),true);app.st.page=PageKind::Media;app.expanded=true;app.mode=Mode::Expanded(PageKind::Media);app.st.notice=None;app.st.hud=None;app.input.mouse=None;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            let find=|app:&App,id:&str|app.hits.iter().find(|h|h.id==wid(id)).unwrap().rect;
            let pin=find(&app,"pin");let mini=find(&app,"media.lyrics-popup");let visible=find(&app,"header.keep-visible");
            for status in[crate::state::LyricsStatus::Idle,crate::state::LyricsStatus::Loading,crate::state::LyricsStatus::NotFound,crate::state::LyricsStatus::Instrumental,crate::state::LyricsStatus::Found]{app.st.lyrics.status=status;app.frame(&mut deferred,None);assert_eq!(find(&app,"pin"),pin);assert_eq!(find(&app,"header.keep-visible"),visible);if app.st.lyrics.status==crate::state::LyricsStatus::Found{assert_eq!(find(&app,"media.lyrics-popup"),mini);}}
            app.on_event(AppEvent::Media(None));app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);assert_eq!(find(&app,"pin"),pin);assert_eq!(find(&app,"header.keep-visible"),visible);
            let mut next=song.clone();next.track_key="next-song-fixture".into();next.title="Next song".into();app.on_event(AppEvent::Media(Some(next)));app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);assert_eq!(find(&app,"header.keep-visible"),visible);assert!(!app.hits.iter().any(|h|h.id==wid("media.lyrics-popup")));
            app.input.mouse=Some(mini.center());if let Some(dir)=std::env::var_os("LUMO_LYRICS_QA"){let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(dir).join(format!("waiting-mini-hint-{theme:?}-{width}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.input.mouse=None;app.st.lyrics=lyrics.clone();app.st.lyrics.track_key="next-song-fixture".into();app.frame(&mut deferred,None);assert_eq!(find(&app,"media.lyrics-popup"),mini);assert_eq!(find(&app,"header.keep-visible"),visible);
            let(w,_,quiet,_)=app.selftest_capture();for(id,button)in[("pin",pin),("mini",mini),("visible",visible)]{app.input.mouse=Some(button.center());let(_,_,hovered,_)=app.selftest_capture();let scale=app.gfx.scale();let changed=quiet.chunks_exact(4).zip(hovered.chunks_exact(4)).enumerate().filter(|(n,(a,b))|a!=b&&!button.inset(-2.0,-2.0).contains(Point::new(((n%w as usize)as f32+app.capture_rect.left as f32)/scale,((n/w as usize)as f32+app.capture_rect.top as f32)/scale))).count();assert_eq!(changed,0,"{id} must not draw hover descriptions in Music");}
        }}
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }
    #[test]#[ignore="requires a Windows desktop; offline Music layout fixture"]
    fn music_player_controls_fit_with_and_without_lyrics_at_supported_sizes(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let sample=crate::lyrics_window::sample_state();app.st.media=sample.media;app.st.lyrics=sample.lyrics;app.st.page=PageKind::Media;app.expanded=true;app.mode=Mode::Expanded(PageKind::Media);app.st.notice=None;app.st.hud=None;app.st.power.saver=true;
        for theme in [Theme::Classic,Theme::Pill]{for scale in [1.0,1.25,1.5,2.0]{for width in [480.0,620.0]{for lyrics in [false,true]{
            app.st.settings.theme=theme;app.gfx.set_dpi(scale*96.0);app.st.settings.expanded_width=width;app.st.settings.lyrics=true;app.st.settings.music_lyrics=lyrics;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            let body=app.island.body_rect_at(app.cx);let area=pages::content_area(body);
            for id in ["media_prev","media_play","media_next","media_seek","media.lrc-toggle"]{let r=app.hits.iter().find(|h|h.id==wid(id)).unwrap().rect;assert!(r.x>=area.x-1.0&&r.y>=area.y-1.0&&r.right()<=area.right()+1.0&&r.bottom()<=area.bottom()+1.0,"{id} exceeds {theme:?} at {width}/{scale}");}
            let bar=app.hits.iter().find(|h|h.id==wid("media_seek")).unwrap().rect;
            let player=crate::pill::music::player_rect(&app.st,area);
            assert!(bar.w>player.w*0.5&&bar.right()<=player.right(),"progress spans the player and stays clear of lyrics");
            let rows:Vec<_>=app.hits.iter().filter(|h|(0..7).any(|i|h.id==crate::ui::wid_n("media_lyric",i))).collect();
            assert_eq!(!rows.is_empty(),lyrics,"lyrics preference controls the inline column");
            if let Some(col)=crate::pill::music::lyrics_rect(&app.st,area){for row in rows{assert!(row.rect.x>=col.x&&row.rect.right()<=col.right()+1.0);}}
            for id in ["media_prev","media_play","media_next"]{let r=app.hits.iter().find(|h|h.id==wid(id)).unwrap().rect;assert!(r.x>=player.x&&r.right()<=player.right(),"transport must stay inside its column");}
            if scale==1.25&&width==620.0&&lyrics{if let Some(dir)=std::env::var_os("LUMO_LYRICS_QA"){let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(dir).join(format!("music-inline-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}}
            if theme==Theme::Pill&&scale==1.25&&width==620.0&&!lyrics{if let Some(dir)=std::env::var_os("LUMO_LYRICS_QA"){let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(dir).join("music-player-wide.png"),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}}
        }}}}
        app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }
    #[test]#[ignore="requires a Windows desktop; offline native Music wheel routing"]
    fn music_scroll_changes_tracks_only_over_the_enabled_player_and_respects_capabilities(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let sample=crate::lyrics_window::sample_state();app.st.media=sample.media;app.st.lyrics=sample.lyrics;app.st.page=PageKind::Media;app.expanded=true;app.mode=Mode::Expanded(PageKind::Media);app.st.notice=None;app.st.hud=None;app.st.power.saver=true;clock::hold(Some(100.0));
        for theme in [Theme::Classic,Theme::Pill] {
            app.st.settings.theme=theme;app.st.page=PageKind::Media;app.st.settings.music_scroll_tracks=true;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);app.music_wheel=Default::default();app.wheel_pager=Default::default();app.input.wheel=0.0;
            let body=app.island.body_rect_at(app.cx);let area=pages::content_area(body);let player=crate::pill::music::player_rect(&app.st,area);let lyrics=crate::pill::music::lyrics_rect(&app.st,area).unwrap();
            let send=|app:&mut App,delta:i16,p:Point,now:f64,deferred:&mut Vec<Deferred>|{clock::hold(Some(now));app.st.cmds.clear();let scale=app.gfx.scale();let x=app.surface.x+(p.x*scale).round()as i32;let y=app.surface.y+(p.y*scale).round()as i32;let packet=LPARAM((((y as u16 as usize)<<16)|(x as u16 as usize))as isize);app.handle_island(WM_MOUSEWHEEL,WPARAM((delta as u16 as usize)<<16),packet,deferred);};
            for(delta,now,expected)in[(120,100.0,Some(crate::commands::MediaCmd::Prev)),(120,100.03,None),(-120,100.04,Some(crate::commands::MediaCmd::Next)),(-20,100.5,None),(-40,100.51,Some(crate::commands::MediaCmd::Next))]{
                send(&mut app,delta,player.center(),now,&mut deferred);let commands:Vec<_>=app.st.cmds.iter().filter_map(|c|if let Command::Media(m)=c{Some(m.clone())}else{None}).collect();assert_eq!(commands,expected.into_iter().collect::<Vec<_>>(),"{theme:?}: wheel pacing/direction");
            }
            app.st.settings.music_scroll_tracks=false;send(&mut app,120,player.center(),101.0,&mut deferred);assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::Media(_))));
            app.input.wheel=0.0;app.st.settings.music_scroll_tracks=true;app.st.media.as_mut().unwrap().can_next=false;send(&mut app,-120,player.center(),102.0,&mut deferred);assert!(app.st.cmds.is_empty(),"disabled Next must not dispatch");app.st.media.as_mut().unwrap().can_next=true;
            send(&mut app,-120,lyrics.center(),103.0,&mut deferred);assert!(app.st.cmds.is_empty(),"lyrics scrolling must not skip tracks");assert!(app.input.wheel<0.0);app.input.wheel=0.0;
            app.input.down=true;send(&mut app,-120,player.center(),104.0,&mut deferred);assert!(app.st.cmds.is_empty(),"a seek drag must not skip tracks");app.input.down=false;app.input.wheel=0.0;
            let row=app.hits.iter().find(|h|h.id==crate::ui::wid_n("media_lyric",1)).unwrap().id;app.input.clicked=Some((row,MouseButton::Left));app.frame(&mut deferred,None);assert_eq!(app.st.media.as_ref().unwrap().timeline.position_s,8.0,"inline lyrics retain click-to-seek");
            app.expanded=false;send(&mut app,-120,player.center(),105.0,&mut deferred);assert!(app.st.cmds.is_empty(),"closed island must not dispatch track scrolling");app.expanded=true;app.input.wheel=0.0;
            send(&mut app,-120,pages::header_rect(body).center(),106.0,&mut deferred);assert_eq!(app.st.page,PageKind::Focus,"header keeps section navigation");assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::Media(_))));
            app.st.page=PageKind::Media;app.page_anim=None;app.st.settings.lyrics=false;assert!(crate::pill::music::lyrics_rect(&app.st,area).is_none());app.st.settings.lyrics=true;
            app.st.media.as_mut().unwrap().music_source=None;assert!(crate::pill::music::lyrics_rect(&app.st,area).is_none());app.st.media.as_mut().unwrap().music_source=Some(lumo_core::nowplaying::MusicSource::Streaming);
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
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
                let pin=app.hits.iter().find(|hit|hit.id==wid("pin")).unwrap().rect;
                let visible=app.hits.iter().find(|hit|hit.id==wid("header.keep-visible")).unwrap().rect;
                assert!((pop.center().y-pin.center().y).abs()<1.0 && (pin.x-pop.right()).abs()<=3.0,"mini player belongs beside Pin in the header");
                let lrc=app.hits.iter().find(|hit|hit.id==wid("media.lrc-toggle")).unwrap().rect;
                assert!(lrc.w>=44.0,"the LRC text button remains readable");
                let cues=(0..7).filter(|&i|app.hits.iter().any(|hit|hit.id==crate::ui::wid_n("media_lyric",i))).count();
                assert!(cues>0,"{theme:?} at {width}: expanded Music restores synced lyrics");
                assert!(!app.hits.iter().any(|hit|hit.id==wid("media_lyrics_credit")),"lyric source labels must not appear in the player");
                let controls:[Rect;4]=["media_prev","media_play","media_next","media_seek"].map(|id|app.hits.iter().find(|hit|hit.id==wid(id)).unwrap().rect);
                let height=pages::media::height(&app.st);
                for status in [crate::state::LyricsStatus::Loading,crate::state::LyricsStatus::NotFound,crate::state::LyricsStatus::Found] {
                    app.st.lyrics.status=status;app.frame(&mut deferred,None);
                    assert_eq!(pages::media::height(&app.st),height,"lookup states must not resize the island");
                    assert_eq!(app.hits.iter().find(|hit|hit.id==wid("pin")).unwrap().rect,pin,"lyric updates must not move Pin");
                    assert_eq!(app.hits.iter().find(|hit|hit.id==wid("header.keep-visible")).unwrap().rect,visible,"lyric updates must not move the toolbar into the mini-player slot");
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
            assert!(crate::pill::music::lyrics_rect(&app.st,pages::content_area(app.island.body_rect_at(app.cx))).is_none(),"the popup owns lyrics during the handoff");
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
            assert_eq!(app.st.ui.lyric_track,app.st.media.as_ref().unwrap().track_key,"returning from mini player restores the inline lyrics");
            assert!(app.hits.iter().any(|hit|hit.id==wid("media.lrc-toggle")),"closing must re-enable the lyrics toggle");
            if let Some(dir) = std::env::var_os("LUMO_LYRICS_QA").map(std::path::PathBuf::from) {
                let (w,h,pixels,_)=app.selftest_capture();
                crate::png::save_rgba(dir.join(format!("music-mini-player-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
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
            app.mode = Mode::Expanded(PageKind::Tasks);
            app.st.page = PageKind::Tasks;
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
                crate::png::save_rgba(dir.join(format!("tasks-{theme:?}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
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
        app.st.settings.lyrics = true;
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
            app.st.lyrics=LyricsState{track_key:"seek-fixture".into(),status:LyricsStatus::Found,lyrics:Some(lumo_core::lrc::parse_lrc("[00:00]Start\n[00:20]Earlier\n[00:40]Before\n[02:00]Approaching\n[02:30]Selected\n[03:00]Later")),..Default::default()};
            app.island.snap(app.geom_for(&app.mode));
            app.frame(&mut deferred, None);
            release_at(&mut app, 0.75, &mut deferred);
            assert!((app.st.ui.lyric_pos-4.0).abs()<0.01,"seek-release frame must show the selected lyric immediately");
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
            assert!((app.st.ui.lyric_pos-4.0).abs()<0.01,"stale reports must never animate lyrics back toward the beginning");
            let want = if playing { 150.3 } else { 150.0 };
            assert!((app.st.media.as_ref().unwrap().timeline.position_at(base + 0.3) - want).abs() < 0.001, "{theme:?}: feedback must keep moving from the selected position");
            clock::hold(Some(base + 0.4));
            release_at(&mut app, 0.1, &mut deferred);
            assert!((app.st.ui.lyric_pos-1.0).abs()<0.01,"backward seeks must also snap to their cue");
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
            for id in ["header.keep-visible", "pin"] {
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

    #[test]#[ignore="requires a Windows desktop; section rail and native wheel fixture"]
    fn section_tabs_restore_icons_and_active_name_with_wheel_navigation(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        for(i,theme)in[Theme::Classic,Theme::Pill].into_iter().enumerate(){
            let now=100.0+i as f64*10.0;clock::hold(Some(now));app.st.settings.theme=theme;app.st.page=PageKind::Media;app.expanded=true;app.mode=Mode::Expanded(PageKind::Media);app.page_anim=None;app.st.notice=None;app.st.hud=None;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            let body=app.island.body_rect_at(app.cx);let rail=pages::header_rect(body);let area=pages::content_area(body);
            assert!(rail.bottom()<area.y);assert!(!app.hits.iter().any(|h|h.id==wid("header.gear")));
            assert!(app.hits.iter().find(|h|h.id==crate::ui::wid_n("tab",0)).unwrap().rect.w>60.0,"Music includes its name");
            let count=pages::tabs(&app.st).len();for n in 0..count{let hit=app.hits.iter().find(|h|h.id==crate::ui::wid_n("tab",n as u64)).unwrap();assert!(rail.contains(hit.rect.center()));}
            if let Some(dir)=std::env::var_os("LUMO_AI_QA"){let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(dir).join(format!("section-tabs-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            let scale=app.gfx.scale();let p=POINT{x:app.surface.x+(rail.center().x*scale)as i32,y:app.surface.y+(rail.center().y*scale)as i32};let packet=LPARAM((((p.y as u16 as usize)<<16)|(p.x as u16 as usize))as isize);app.wheel_pager=Default::default();
            app.handle_island(WM_MOUSEWHEEL,WPARAM(((-120i16 as u16 as usize)<<16)as usize),packet,&mut deferred);assert_eq!(app.st.page,PageKind::Focus);
            clock::hold(Some(now+1.0));app.handle_island(WM_MOUSEWHEEL,WPARAM((120usize)<<16),packet,&mut deferred);assert_eq!(app.st.page,PageKind::Media);
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
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

    #[test]
    #[ignore="requires a Windows desktop; offline mouse versus touchpad routing"]
    fn mouse_wheel_opens_workspaces_and_touchpad_retains_existing_navigation() {
        use services::wheel_input::Source;
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();
        let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();
        app.settings_writable=false;app.st.settings.set_addon("ai",true);app.st.ai.loaded=true;app.st.power.saver=true;let mut deferred=vec![];
        let send=|app:&mut App,source,delta:i16,shift,p:Point,now:f64,deferred:&mut Vec<Deferred>|{
            clock::hold(Some(now));let mut point=POINT{x:(p.x*app.gfx.scale()).round()as i32,y:(p.y*app.gfx.scale()).round()as i32};
            unsafe{windows::Win32::Graphics::Gdi::ClientToScreen(hwnd,&mut point).unwrap();}
            let lp=LPARAM(((point.x as u16 as u32)|((point.y as u16 as u32)<<16))as isize);
            app.island_vertical_wheel(delta,lp,source,shift,deferred);
        };
        let closed=|app:&mut App|{app.expanded=false;app.st.search.active=false;app.st.page=PageKind::Media;app.st.closed_view=Default::default();app.st.notice=None;app.mode=Mode::Idle;app.search_gesture=Default::default();app.wheel_pager=Default::default();app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.input.end_frame();};
        for (i,theme) in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            app.st.settings.theme=theme;closed(&mut app);let t=900.0+i as f64*30.0;let p=app.island.body_rect_at(app.cx).center();
            send(&mut app,Source::Mouse,-120,false,p,t,&mut deferred);assert!(app.expanded&&app.st.search.active,"mouse down opens Search");
            app.island.snap(app.geom_for(&app.target_mode()));let body=app.island.body_rect_at(app.cx);let header=Point::new(body.center().x,body.y+12.0);
            send(&mut app,Source::Mouse,120,false,header,t+0.7,&mut deferred);assert!(!app.expanded&&!app.st.search.active,"the next wheel gesture returns to Time first");
            app.island.snap(app.geom_for(&app.target_mode()));let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,120,false,p,t+1.1,&mut deferred);assert!(app.expanded&&app.st.page==PageKind::Ai);
            app.island.snap(app.geom_for(&app.target_mode()));let body=app.island.body_rect_at(app.cx);let header=Point::new(body.center().x,body.y+12.0);
            send(&mut app,Source::Mouse,-120,false,header,t+1.4,&mut deferred);assert!(!app.expanded&&!app.st.search.active,"AI also closes to Time before Search");
            app.island.snap(app.geom_for(&app.target_mode()));let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,-120,false,p,t+1.8,&mut deferred);assert!(app.st.search.active);
            app.st.search.view.as_mut().unwrap().query="settings".into();app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));let body=app.island.body_rect_at(app.cx);let p=Point::new(body.center().x,body.y+90.0);send(&mut app,Source::Mouse,120,false,p,t+2.1,&mut deferred);assert!(app.st.search.active,"result scrolling does not change the workspace");
            closed(&mut app);let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,120,false,p,t+3.0,&mut deferred);assert!(app.expanded&&app.st.page==PageKind::Ai,"mouse up opens AI");
            // The same event delta from a touchpad (or an unknown sender)
            // retains compact views instead of opening either workspace.
            for source in [Source::Touchpad,Source::Unknown] {
                closed(&mut app);let p=app.island.body_rect_at(app.cx).center();send(&mut app,source,-120,false,p,t+4.0,&mut deferred);
                assert!(!app.expanded&&app.st.closed_view==lumo_core::settings::ClosedView::Performance);
            }
            closed(&mut app);let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,-120,true,p,t+5.0,&mut deferred);assert!(!app.expanded&&app.st.closed_view==lumo_core::settings::ClosedView::Performance,"Shift+wheel keeps compact views accessible");
            closed(&mut app);app.st.settings.mouse_wheel_shortcuts=false;let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,-120,false,p,t+6.0,&mut deferred);assert!(!app.expanded);app.st.settings.mouse_wheel_shortcuts=true;
            closed(&mut app);app.st.settings.set_addon("ai",false);let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,120,false,p,t+7.0,&mut deferred);assert!(!app.expanded,"disabled AI is never opened");app.st.settings.set_addon("ai",true);
            closed(&mut app);app.expanded=true;app.mode=Mode::Expanded(PageKind::Media);app.island.snap(app.geom_for(&app.mode));let body=app.island.body_rect_at(app.cx);let p=Point::new(body.center().x,body.y+12.0);
            send(&mut app,Source::Mouse,-120,false,p,t+8.0,&mut deferred);assert_eq!(app.st.page,PageKind::Focus,"mouse header wheel advances sections");
            send(&mut app,Source::Mouse,120,false,p,t+8.7,&mut deferred);assert_eq!(app.st.page,PageKind::Media,"mouse up returns a section");
            app.locked=true;closed(&mut app);let p=app.island.body_rect_at(app.cx).center();send(&mut app,Source::Mouse,-120,false,p,t+9.4,&mut deferred);assert!(!app.expanded);app.locked=false;
            app.hovering=true;app.wheel_input=Some(services::wheel_input::Watch::start());app.handle_island(WM_MOUSELEAVE,WPARAM(0),LPARAM(0),&mut deferred);assert!(app.wheel_input.is_none());
            app.wheel_input=Some(services::wheel_input::Watch::start());app.display_off=true;app.sync_system_views();assert!(app.wheel_input.is_none());app.display_off=false;
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]
    #[ignore="requires a Windows desktop"]
    fn horizontal_gestures_route_left_to_ai_right_to_search_and_ignore_inertia() {
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();
        let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();
        app.settings_writable=false;app.st.settings.set_addon("ai",true);app.st.ai.loaded=true;let mut deferred=vec![];
        let wheel=|delta:i16|WPARAM(usize::from(delta as u16)<<16);
        for (i,delta) in [-120i16,120].into_iter().enumerate(){
            let now=10.0+i as f64*10.0;clock::hold(Some(now));
            app.expanded=false;app.st.search.active=false;app.st.search.view=None;app.st.notice=None;
            let before=app.st.page;let closed=app.geom_for(&app.target_mode());
            app.handle_island(WM_MOUSEHWHEEL,wheel(delta),LPARAM(0),&mut deferred);
            assert!(app.expanded&&app.text_focus);assert_eq!(app.st.search.active,delta>0);
            assert_eq!(app.st.page,if delta<0{PageKind::Ai}else{before},"each direction opens its own surface");
            let opened=app.geom_for(&app.target_mode());assert!(opened.w>closed.w && opened.h>closed.h);
            assert_eq!(app.hwnd,hwnd,"Search shares the island window");
            app.handle_island(WM_MOUSEHWHEEL,wheel(delta),LPARAM(0),&mut deferred);
            assert!(app.expanded,"opening inertia must not close either surface");
            clock::hold(Some(now+0.6));app.handle_island(WM_MOUSEHWHEEL,wheel(-delta),LPARAM(0),&mut deferred);
            assert!(!app.st.search.active && !app.expanded && !app.text_focus,"a new swipe collapses Search");
            clock::hold(Some(now+0.7));app.handle_island(WM_MOUSEHWHEEL,wheel(delta),LPARAM(0),&mut deferred);
            assert!(!app.st.search.active&&!app.expanded,"closing inertia cannot reopen either surface");
        }
        // Opening Search is independent from normal expanded-page navigation.
        clock::hold(Some(40.0));app.st.search.view=None;app.expanded=true;app.st.page=PageKind::Media;
        app.handle_island(WM_MOUSEHWHEEL,wheel(120),LPARAM(0),&mut deferred);
        assert!(!app.st.search.active);assert_ne!(app.st.page,PageKind::Media);
        app.st.settings.search.horizontal_scroll=false;app.expanded=false;
        clock::hold(Some(50.0));app.handle_island(WM_MOUSEHWHEEL,wheel(120),LPARAM(0),&mut deferred);assert!(!app.st.search.active);
        app.locked=true;app.st.settings.search.horizontal_scroll=true;
        clock::hold(Some(60.0));app.handle_island(WM_MOUSEHWHEEL,wheel(120),LPARAM(0),&mut deferred);assert!(!app.st.search.active);
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a Windows desktop; routes fictional file drops without reading files"]
    fn file_drags_attach_to_open_ai_hosts_and_ordinary_pages_keep_the_shelf(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        unsafe{OleInitialize(None).unwrap();}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;
        let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);app.st.settings.ai.keep_open=true;app.st.page=PageKind::Media;app.st.ai.draft="Keep this draft".into();app.open_ai(true);app.st.cmds.clear();
        app.on_event(AppEvent::DragHover{owner:SendHwnd::new(hwnd),on:true});
        assert!(app.expanded&&app.st.page==PageKind::Ai&&app.st.drag_hover);assert_eq!(app.st.ai.draft,"Keep this draft");
        let paths=vec![r"C:\fictional\Review.pdf".to_owned()];
        app.on_event(AppEvent::FilesDropped{owner:SendHwnd::new(hwnd),paths:paths.clone()});
        assert!(!app.st.drag_hover);assert_eq!(app.st.page,PageKind::Ai);assert!(app.st.store.shelf.items.is_empty());
        assert!(app.st.cmds.iter().any(|c|matches!(c,Command::Ai(crate::ai::Action::AttachFiles(p))if p==&paths)));app.st.cmds.clear();
        app.open_ai_window();let mini=app.ai_window.as_ref().unwrap().hwnd;app.st.cmds.clear();
        app.on_event(AppEvent::DragHover{owner:SendHwnd::new(mini),on:true});assert!(app.st.drag_hover&&app.ai_window.is_some());
        app.on_event(AppEvent::FilesDropped{owner:SendHwnd::new(mini),paths:paths.clone()});
        assert!(app.st.cmds.iter().any(|c|matches!(c,Command::Ai(crate::ai::Action::AttachFiles(p))if p==&paths)));assert!(app.st.store.shelf.items.is_empty());app.st.cmds.clear();
        app.on_event(AppEvent::DragHover{owner:SendHwnd::new(mini),on:true});
        app.ai_window=None;app.st.ai.full_open=false;
        app.on_event(AppEvent::DragHover{owner:SendHwnd::new(mini),on:false});assert!(!app.drag_hover&&!app.st.drag_hover);
        app.on_event(AppEvent::FilesDropped{owner:SendHwnd::new(mini),paths});assert!(app.st.cmds.is_empty());
        app.st.page=PageKind::Media;app.expanded=true;app.on_event(AppEvent::DragHover{owner:SendHwnd::new(hwnd),on:true});assert_eq!(app.st.page,PageKind::Shelf);
        app.on_event(AppEvent::DragHover{owner:SendHwnd::new(hwnd),on:false});assert!(!app.st.drag_hover);
        app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();OleUninitialize();}
    }
    #[test]#[ignore="requires a Windows desktop; uses an offline AI fixture"]
    fn ai_workspace_preserves_handoffs_resume_pin_and_pauses_while_away(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let home=std::env::var_os("LUMO_HOME");std::env::set_var("LUMO_HOME",std::env::current_dir().unwrap().join(".tools/ai-workspace-profile"));
        let output=std::env::var_os("LUMO_AI_QA").map(std::path::PathBuf::from);if let Some(out)=&output{std::fs::create_dir_all(out).unwrap();}
        for (i,theme) in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            clock::hold(Some(100.0+i as f64*100.0));let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);app.st.settings.theme=theme;app.st.page=PageKind::Media;app.st.pinned=false;app.st.notice=None;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.st.ai.draft="Keep this draft".into();app.fullscreen_on=true;app.st.settings.hide_in_fullscreen=true;app.ai_reply_peek=false;app.apply_fullscreen(app.st.now);assert!(app.fullscreen_hidden);app.open_ai(true);assert!(!app.fullscreen_hidden,"explicit AI opening is visible over fullscreen");
            assert!(!pages::tabs(&app.st).contains(&PageKind::Ai));assert!(!app.st.search.active&&app.expanded&&app.st.page==PageKind::Ai);
            app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);let body=app.island.body_rect_at(app.cx);assert!((body.w-crate::search::WIDTH).abs()<1.0&&body.h>400.0,"browser-sized standalone AI");
            for id in ["ai.prompt","ai.models","ai.open-full","ai.keep-open","ai.resume","ai.history","ai.new"]{assert!(app.hits.iter().any(|h|h.id==wid(id)),"workspace control {id}");}
            assert!(!app.hits.iter().any(|h|h.id==wid("header.settings")),"AI has its own header");
            if let Some(out)=&output{let now=app.st.now;for step in 1..=12{clock::hold(Some(now+step as f64*0.05));unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}let _=windows::Win32::Graphics::Dwm::DwmFlush();}app.frame(&mut deferred,None);}let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(out.join(format!("ai-workspace-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            let id=app.st.ai.current.id;app.st.settings.ai.resume=true;app.collapse();assert_eq!(app.st.page,PageKind::Media);app.open_ai(false);assert_eq!(app.st.ai.current.id,id);assert_eq!(app.st.ai.draft,"Keep this draft");
            app.st.settings.ai.keep_open=true;app.search_lost_activation(helper);assert!(app.expanded);app.st.settings.ai.keep_open=false;app.search_lost_activation(helper);assert!(!app.expanded);assert!(app.fullscreen_hidden,"closing AI restores the ordinary fullscreen visibility policy");
            app.open_ai(false);app.st.ai.pending.push(lumo_core::ai::Attachment{name:"Project notes.pdf".into(),mime:"application/pdf".into(),data:String::new()});let attachments=app.st.ai.pending.len();
            app.st.ai.scroll=12.0;app.open_ai_window();assert!(app.ai_window.is_some()&&app.st.ai.full_open&&!app.expanded);assert!(app.ai_window.as_ref().unwrap().prompt_focused(),"mini handoff is immediately ready for typing");assert_eq!(app.st.ai.current.id,id);assert_eq!(app.st.ai.draft,"Keep this draft");assert_eq!(app.st.ai.pending.len(),attachments);
            if let Some(out)=&output{app.ai_window.as_mut().unwrap().capture(&mut app.st,&out.join(format!("ai-mini-{theme:?}.png")));}
            let mini=app.ai_window.as_ref().unwrap().hwnd;app.st.settings.ai.keep_open=true;app.handle_ai_window(mini,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize));assert!(app.ai_window.is_some(),"mini pin prevents outside dismissal");app.st.settings.ai.keep_open=false;
            // The native procedure must preserve an activation delivered while
            // the app cannot be borrowed, just as the island does.
            unsafe{SendMessageW(mini,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize));let mut queued=MSG::default();assert!(PeekMessageW(&mut queued,mini,crate::ai_window::CHECK_ACTIVATION,crate::ai_window::CHECK_ACTIVATION,PM_REMOVE).as_bool());}
            MISSED.with(|messages|messages.borrow_mut().clear());APP.with(|slot|*slot.borrow_mut()=Some(app));
            with_app(|_|unsafe{SendMessageW(mini,crate::ai_window::CHECK_ACTIVATION,WPARAM(0),LPARAM(0));}).unwrap();
            app=APP.with(|slot|slot.borrow_mut().take().unwrap());
            unsafe{let mut queued=MSG::default();assert!(PeekMessageW(&mut queued,hwnd,crate::ai_window::CHECK_ACTIVATION,crate::ai_window::CHECK_ACTIVATION,PM_REMOVE).as_bool(),"nested activation checks replay after the App borrow ends");assert_eq!(queued.lParam.0,mini.0 as isize);}
            let s=app.gfx.scale();unsafe{SetWindowPos(mini,None,0,0,(420.0*s).round()as i32,(500.0*s).round()as i32,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE).unwrap();}
            app.handle_ai_window(mini,WM_EXITSIZEMOVE,WPARAM(0),LPARAM(0));assert_eq!((app.st.settings.ai.mini_width,app.st.settings.ai.mini_height),(420,500));
            app.locked=true;app.frame(&mut deferred,None);assert!(!unsafe{IsWindowVisible(mini)}.as_bool());app.locked=false;app.frame(&mut deferred,None);assert!(unsafe{IsWindowVisible(mini)}.as_bool());
            app.st.cmd(Command::ReturnAiToIsland);app.run_commands(&mut deferred);assert!(app.ai_window.is_none()&&!app.st.ai.full_open&&app.expanded&&app.st.page==PageKind::Ai);assert_eq!(app.st.ai.draft,"Keep this draft");assert_eq!(app.st.ai.current.id,id);
            app.st.settings.ai.resume=false;app.collapse();assert!(app.st.ai.draft.is_empty()&&app.st.ai.pending.is_empty(),"closing with Resume off releases unsent buffers immediately");app.open_ai(false);assert_ne!(app.st.ai.current.id,id);assert!(app.st.ai.draft.is_empty()&&app.st.ai.pending.is_empty());
            app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);assert!(!app.hits.iter().any(|h|h.id==wid("ai.send")),"empty prompt cannot send");
            if let Some(out)=&output{let now=app.st.now;for step in 1..=12{clock::hold(Some(now+step as f64*0.05));unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}let _=windows::Win32::Graphics::Dwm::DwmFlush();}app.frame(&mut deferred,None);}let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(out.join(format!("ai-empty-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.open_ai_window();let mini=app.ai_window.as_ref().unwrap().hwnd;app.handle_ai_window(mini,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize));assert!(app.ai_window.is_none()&&!app.expanded,"unpinned mini closes on outside activation");
            for field in [None,Some(wid("ai.history-filter"))]{app.open_ai(true);app.input.focused=field;app.handle_island(WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0),&mut deferred);assert!(!app.expanded,"Escape closes AI with any editor focus");}
            app.open_ai_window();let mini=app.ai_window.as_ref().unwrap().hwnd;app.st.ai.history=true;app.ai_window.as_mut().unwrap().focus_field(wid("ai.history-filter"),0);app.handle_ai_window(mini,WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0));assert!(app.ai_window.is_none(),"Escape closes mini AI from history");
            app.open_search();assert!(app.st.search.active&&app.st.page!=PageKind::Ai);app.close_search();
        }
        clock::hold(None);app.st.cmds.clear();app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
        if let Some(home)=home{std::env::set_var("LUMO_HOME",home);}else{std::env::remove_var("LUMO_HOME");}
    }

    #[test]#[ignore="requires a Windows desktop; uses an offline AI fixture"]
    fn ai_internal_mouse_presses_keep_workspace_focus(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let wheel_point=|p:Point,s:f32|LPARAM(((((p.y*s).round()as u16)as usize)<<16|(p.x*s).round()as u16 as usize)as isize);
        for theme in [Theme::Classic,Theme::Pill]{
            let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);app.st.settings.theme=theme;app.st.page=PageKind::Media;app.st.pinned=false;app.st.settings.ai.keep_open=false;app.st.ai.draft="Do not send this draft".into();app.st.notice=None;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.open_ai(true);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            for id in ["ai.history","ai.new","ai.temporary","ai.keep-open","ai.resume","ai.open-full","ai.close","ai.plus","ai.models","ai.send","ai.prompt"]{
                app.input.focused=Some(wid("ai.prompt"));app.text_focus=true;let at=app.hits.iter().find(|h|h.id==wid(id)).unwrap().rect.center();let at=wheel_point(at,app.gfx.scale());
                app.handle_island(WM_LBUTTONDOWN,WPARAM(0),at,&mut deferred);
                assert!(app.expanded&&app.st.page==PageKind::Ai&&app.text_focus,"pressing {id} must keep keyboard focus in the {theme:?} AI workspace");
                // Test the press of Close/Send/Settings without invoking those
                // actions. A real press is where the erroneous blur occurred.
                app.release_mouse();app.input.end_frame();
            }
            for (id,open) in [("ai.history",true),("ai.history",false),("ai.plus",true),("ai.plus",false),("ai.models",true)] {
                let at=app.hits.iter().find(|h|h.id==wid(id)).unwrap().rect.center();let at=wheel_point(at,app.gfx.scale());
                app.handle_island(WM_LBUTTONDOWN,WPARAM(0),at,&mut deferred);app.frame(&mut deferred,None);app.handle_island(WM_LBUTTONUP,WPARAM(0),at,&mut deferred);app.frame(&mut deferred,None);
                assert!(app.expanded&&app.text_focus,"clicking {id} must not dismiss AI");
                assert_eq!(if id=="ai.history"{app.st.ai.history}else if id=="ai.plus"{app.st.ai.attach_menu}else{app.st.ai.model_menu},open);
            }
            assert_eq!(app.st.ai.draft,"Do not send this draft");app.st.ai.close_popovers();app.search_lost_activation(helper);assert!(!app.expanded,"an actual outside activation must still dismiss unpinned AI");app.st.cmds.clear();
        }
        app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a Windows desktop; no injected user input"]
    fn wheel_workspaces_dismiss_on_background_press_without_activation_and_release_watch() {
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;app.st.power.saver=true;app.st.ai.loaded=true;app.st.settings.set_addon("ai",true);let mut deferred=vec![];
        let child=unsafe{CreateWindowExW(WINDOW_EX_STYLE::default(),w!("STATIC"),w!("Lumo child fixture"),WS_CHILD,0,0,10,10,hwnd,None,None,None).unwrap()};
        for theme in [Theme::Classic,Theme::Pill]{app.st.settings.theme=theme;
            for ai in [false,true]{
                if ai{app.open_ai(true);}else{app.open_search();}assert!(app.workspace_dismiss.is_some());app.text_focus=false;app.input.focused=None;
                app.handle_island(services::workspace_dismiss::OUTSIDE_PRESS,WPARAM(0),LPARAM(child.0 as isize),&mut deferred);assert!(app.expanded,"inside child click keeps the workspace");
                app.handle_island(services::workspace_dismiss::OUTSIDE_PRESS,WPARAM(0),LPARAM(helper.0 as isize),&mut deferred);
                assert!(!app.expanded&&!app.st.search.active&&app.workspace_dismiss.is_none(),"background press closes immediately without WM_ACTIVATE");
                assert_eq!(app.st.closed_view,lumo_core::settings::ClosedView::Time);
            }
            app.st.settings.ai.keep_open=true;app.open_ai(true);assert!(app.workspace_dismiss.is_none());app.handle_island(services::workspace_dismiss::OUTSIDE_PRESS,WPARAM(0),LPARAM(helper.0 as isize),&mut deferred);assert!(app.expanded);app.st.settings.ai.keep_open=false;app.sync_workspace_dismiss();assert!(app.workspace_dismiss.is_some());
            app.st.ai.attaching=true;app.sync_workspace_dismiss();assert!(app.workspace_dismiss.is_none());app.st.ai.attaching=false;
            app.locked=true;app.sync_workspace_dismiss();assert!(app.workspace_dismiss.is_none());app.locked=false;app.collapse();
            app.st.settings.search.keep_open=true;app.open_search();assert!(app.workspace_dismiss.is_none());app.handle_island(services::workspace_dismiss::OUTSIDE_PRESS,WPARAM(0),LPARAM(helper.0 as isize),&mut deferred);assert!(app.expanded);app.close_search();app.st.settings.search.keep_open=false;
            // One continuous contact: no 350-ms quiet gap anywhere in this
            // sequence, and no synthetic touch or mouse input is injected.
            app.search_gesture=Default::default();
            for (delta,t,open,search) in [(-80i16,300.0,true,false),(-40,300.08,true,false),(30,300.24,true,false),(40,300.26,false,false),(20,300.43,false,false),(20,300.57,false,false),(20,300.72,false,false),(80,300.92,false,false),(80,300.94,false,false),(40,300.96,true,true)] {
                clock::hold(Some(t));app.handle_island(WM_MOUSEHWHEEL,WPARAM((delta as u16 as usize)<<16),LPARAM(0),&mut deferred);
                assert_eq!((app.expanded,app.st.search.active),(open,search),"continuous-contact gesture at {t}");
            }
            app.close_search();
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a Windows desktop; uses only Lumo-owned windows"]
    fn search_outside_activation_closes_without_editor_focus_and_after_menu(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let child=unsafe{CreateWindowExW(WINDOW_EX_STYLE::default(),w!("STATIC"),w!("Lumo page focus fixture"),WS_CHILD,0,0,10,10,hwnd,None,None,None).unwrap()};
        for theme in [Theme::Classic,Theme::Pill]{for embedded in [false,true]{for editor_focus in [false,true]{
            app.st.settings.theme=theme;app.st.settings.search.keep_open=false;app.st.pinned=true;app.open_search();
            if embedded{app.st.search.web_url=Some("https://example.com/".into());}
            app.text_focus=editor_focus;app.input.focused=editor_focus.then_some(wid("search.query"));
            app.handle_island(WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(child.0 as isize),&mut deferred);
            assert!(app.st.search.active,"page focus remains inside Search");
            app.handle_island(WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize),&mut deferred);
            assert!(!app.st.search.active&&!app.expanded&&!app.text_focus,"outside activation closes {theme:?}, embedded={embedded}, editor={editor_focus}, independent of the ordinary island pin");
        }}}
        app.st.settings.search.keep_open=true;app.open_search();app.text_focus=false;
        app.handle_island(WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize),&mut deferred);assert!(app.st.search.active,"Search pin preserves the surface");
        app.st.settings.search.keep_open=false;app.st.search.context_menu=true;
        app.handle_island(WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize),&mut deferred);assert!(app.st.search.active,"a native menu defers dismissal");
        app.st.search.context_menu=false;app.search_lost_activation(helper);assert!(!app.st.search.active,"menu return reconciles the outside click even without another activation event");
        app.open_search();app.text_focus=false;app.search_lost_activation(hwnd);assert!(app.st.search.active,"a queued foreground event for Search cannot dismiss it");app.close_search();
        // A native call can dispatch deactivation while the application is
        // borrowed. Exercise the real window procedure and verify it defers
        // a foreground check instead of silently dropping that message.
        app.open_search();unsafe{SetWindowLongPtrW(hwnd,GWLP_WNDPROC,island_wndproc as *const () as isize);}
        APP.with(|slot|*slot.borrow_mut()=Some(app));
        with_app(|app|{unsafe{SendMessageW(hwnd,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(helper.0 as isize));}assert!(app.st.search.active);}).unwrap();
        let mut app=APP.with(|slot|slot.borrow_mut().take().unwrap());let mut queued=MSG::default();
        assert!(unsafe{PeekMessageW(&mut queued,hwnd,WM_SEARCH_ACTIVATION,WM_SEARCH_ACTIVATION,PM_REMOVE)}.as_bool(),"reentrant deactivation queues a final foreground check");
        unsafe{SetWindowLongPtrW(hwnd,GWLP_WNDPROC,test_proc as *const () as isize);}app.search_lost_activation(helper);assert!(!app.st.search.active);
        app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a native desktop; offline tool-result icon fixture"]
    fn search_tool_results_use_a_small_code_mark_and_powertoys_fallback() {
        use lumo_core::search::{Entry,Group,Target,Usage};
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;app.st.power.saver=true;app.st.settings.search.apps=false;app.st.settings.search.settings=false;app.st.settings.search.powertoys=true;app.st.settings.search.vscode_commands=true;
        app.st.search.integrations=vec![
            Entry::new("code-fixture".into(),"Command Palette".into(),"Project — Visual Studio Code".into(),"",Group::VSCode,Target::AppCommand{window:hwnd.0 as u64,pid:unsafe{windows::Win32::System::Threading::GetCurrentProcessId()},keys:vec![0x11,0x10,0x50]}),
            Entry::new("toy-ColorPicker".into(),"Color Picker".into(),"PowerToys · Open tool".into(),"",Group::PowerToys,Target::PowerToy{event:"offline-fixture".into()})];
        for e in &app.st.search.integrations{app.st.store.search_usage.insert(e.id.clone(),Usage{count:1,last:1});}
        let mut deferred=vec![];
        for theme in [Theme::Classic,Theme::Pill] {
            app.st.settings.theme=theme;app.open_search();app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.code-result",crate::ui::wid("code-fixture"))));
            if let Some(dir)=std::env::var_os("LUMO_SEARCH_QA"){let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(dir).join(format!("search-tool-icons-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.close_search();
        }
        assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::ExecuteSearch(_))));app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="native blank browser document in both hosts"]
    fn browser_opens_blank_without_page_content_or_a_web_renderer(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.st.settings.search.web=true;let mut deferred=vec![];
        for (index,theme)in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            clock::hold(Some(100.0+index as f64*100.0));app.st.now=clock::mono();app.st.settings.theme=theme;app.open_search();app.execute_search(crate::search::browser_target("",app.st.settings.search.engine));
            app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            assert!(crate::browser_home::is_home(&app.st)&&app.search_web.is_none(),"a blank document must not start WebView2 or load an external website");
            let body=app.island.body_rect_at(app.cx);let bar=crate::search::browser_toolbar(body,false).bar;let page=crate::search::web_rect(body);
            assert_eq!(bar.h,34.0);assert_eq!(bar.x-body.x,8.0);assert_eq!(page.y-bar.bottom(),6.0);assert_eq!(body.bottom()-page.bottom(),8.0);
            assert!(app.st.search.address.is_empty()&&app.st.search.navigation.title.is_empty());
            assert!(!app.hits.iter().any(|h|page.contains(h.rect.center())),"blank content must not expose page widgets");
            // Reproduce clicking the address, typing, clearing it, then clicking
            // the native background. The toolbar must leave editing immediately.
            let address=app.hits.iter().find(|h|h.id==wid("search.query")).unwrap().rect.center();
            for msg in [WM_LBUTTONDOWN,WM_LBUTTONUP]{let scale=app.gfx.scale();let at=LPARAM((((address.y*scale).round()as u16 as u32)<<16|(address.x*scale).round()as u16 as u32)as isize);app.handle_island(msg,WPARAM(0),at,&mut deferred);app.frame(&mut deferred,None);}
            app.frame(&mut deferred,None);assert!(app.st.search.address_editing&&app.st.search.address.is_empty(),"the idle prompt must never become editable text");
            assert!(!app.hits.iter().any(|h|h.id==wid("search.reload")));
            app.input.keys="example.com".chars().map(KeyInput::Char).collect();app.frame(&mut deferred,None);assert_eq!(app.st.search.address,"example.com");
            app.input.keys=vec![KeyInput::SelectAll,KeyInput::Backspace];app.frame(&mut deferred,None);assert!(app.st.search.address.is_empty()&&app.st.search.address_editing);
            assert!(app.input.caret_wake.is_some(),"an empty focused address still needs a visible caret");
            let background=page.center();for msg in [WM_LBUTTONDOWN,WM_LBUTTONUP]{let scale=app.gfx.scale();let at=LPARAM((((background.y*scale).round()as u16 as u32)<<16|(background.x*scale).round()as u16 as u32)as isize);app.handle_island(msg,WPARAM(0),at,&mut deferred);app.frame(&mut deferred,None);}
            assert!(!app.st.search.address_editing&&app.input.focused.is_none());assert!(app.st.search.address.is_empty());assert!(app.hits.iter().any(|h|h.id==wid("search.reload")),"background clicks must restore browser controls");
            if let Some(dir)=std::env::var_os("LUMO_BROWSER_HOME_QA"){let dir=std::path::PathBuf::from(dir);std::fs::create_dir_all(&dir).unwrap();let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(dir.join(format!("blank-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.toggle_mini_browser();app.frame(&mut deferred,None);assert!(app.st.search.detached&&app.search_web.is_none());
            app.st.settings.search.mini_width=280;app.st.settings.search.mini_height=200;app.browser_window.as_mut().unwrap().apply_size(&app.st.settings.search);app.browser_window.as_mut().unwrap().mark_dirty();app.frame(&mut deferred,None);
            assert!(app.browser_window.as_ref().unwrap().test_hit(wid("search.query")).is_some());
            let mini=app.browser_window.as_ref().unwrap().hwnd;let scale=app.browser_window.as_ref().unwrap().page_bounds().1;
            let address=app.browser_window.as_ref().unwrap().test_hit(wid("search.query")).unwrap().center();
            for msg in [WM_LBUTTONDOWN,WM_LBUTTONUP]{let at=LPARAM((((address.y*scale).round()as u16 as u32)<<16|(address.x*scale).round()as u16 as u32)as isize);app.handle_browser_window(mini,msg,WPARAM(0),at);app.frame(&mut deferred,None);}
            assert!(app.st.search.address_editing&&app.st.search.address.is_empty());
            for c in "test".chars(){app.handle_browser_window(mini,WM_CHAR,WPARAM(c as usize),LPARAM(0));}app.frame(&mut deferred,None);assert_eq!(app.st.search.address,"test");
            for _ in 0..4{app.handle_browser_window(mini,WM_KEYDOWN,WPARAM(VK_BACK.0 as usize),LPARAM(0));}app.frame(&mut deferred,None);assert!(app.st.search.address.is_empty()&&app.st.search.address_editing);
            let background=crate::search::web_rect(lumo_core::geom::Rect::new(0.0,0.0,280.0,200.0)).center();for msg in [WM_LBUTTONDOWN,WM_LBUTTONUP]{let at=LPARAM((((background.y*scale).round()as u16 as u32)<<16|(background.x*scale).round()as u16 as u32)as isize);app.handle_browser_window(mini,msg,WPARAM(0),at);app.frame(&mut deferred,None);}
            assert!(!app.st.search.address_editing&&app.st.search.address.is_empty());assert!(app.browser_window.as_ref().unwrap().test_hit(wid("search.reload")).is_some());
            app.toggle_mini_browser();app.frame(&mut deferred,None);assert!(app.st.search.embedded()&&crate::browser_home::is_home(&app.st));app.close_search();
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="native launcher layout, persistent app pins and drag reordering"]
    fn search_opens_as_a_bar_and_recent_icons_then_expands_only_for_text(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;app.st.settings.search.web=true;
        let all=services::search::scan(&std::sync::atomic::AtomicBool::new(false)).unwrap();app.st.search.apps=all.into_iter().take(20).collect();assert!(!app.st.search.apps.is_empty());
        let images=services::imaging::search_icons(app.st.search.apps.iter().rev().take(16).map(|e|(e.id.clone(),e.target.clone())).collect(),&std::sync::atomic::AtomicBool::new(false));app.st.search.icons.extend(images);
        for(i,e)in app.st.search.apps.iter().enumerate(){app.st.store.search_usage.insert(e.id.clone(),lumo_core::search::Usage{count:1,last:i as u64+1});}
        let first=app.st.search.apps[0].clone();let browser=lumo_core::search::BROWSER_ID;
        let packet=|p:Point,scale:f32|LPARAM((((p.y*scale).round()as u16 as u32)<<16|(p.x*scale).round()as u16 as u32)as isize);
        let mut deferred=vec![];
        for(index,theme)in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            app.st.store.search_pins=lumo_core::search::default_pins();assert!(lumo_core::search::set_pinned(&mut app.st.store.search_pins,&first,true));
            let base=100.0+index as f64*100.0;clock::hold(Some(base));app.st.now=base;app.st.settings.theme=theme;app.open_search();
            app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            let before=app.st.settings.clone();app.st.settings.search.web=false;app.settings_changed(&before);app.frame(&mut deferred,None);assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.recent",wid(browser))));assert!(app.st.store.search_pins.iter().any(|id|id==browser));
            let before=app.st.settings.clone();app.st.settings.search.web=true;app.settings_changed(&before);app.frame(&mut deferred,None);assert!(app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.recent",wid(browser))));
            let body=app.island.body_rect_at(app.cx);let width=app.hits.iter().find(|h|h.id==wid("search.query")).unwrap().rect.w;assert_eq!(width,body.w-48.0);
            let expected=crate::search::recent_capacity(body.w).min(app.st.search.apps.len()+1);
            assert_eq!(app.hits.iter().filter(|h|h.id==crate::ui::wid_n("search.recent",wid(browser))||app.st.search.apps.iter().any(|e|h.id==crate::ui::wid_n("search.recent",wid(&e.id)))).count(),expected);
            assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.result",0)||h.id==wid("search.web")));
            assert_eq!(app.st.search.view.as_ref().unwrap().selected().unwrap().id,browser);assert!((body.h-112.0).abs()<1.0);
            let browser_rect=app.hits.iter().find(|h|h.id==crate::ui::wid_n("search.recent",wid(browser))).unwrap().rect;
            let app_rect=app.hits.iter().find(|h|h.id==crate::ui::wid_n("search.recent",wid(&first.id))).unwrap().rect;assert!(browser_rect.x<app_rect.x);
            assert_eq!(app.hits.iter().filter(|h|h.id==crate::ui::wid_n("search.recent",wid(&first.id))).count(),1,"a frequent app must not duplicate a pin");
            app.search_context_menu(Some(browser_rect.center()),&mut deferred);assert!(matches!(deferred.last(),Some(Deferred::SearchMenu(e,_))if e.is_browser()));app.st.search.context_menu=false;deferred.clear();
            // Exercise real capture/move/release rather than injecting a reorder command.
            let scale=app.gfx.scale();app.handle_island(WM_LBUTTONDOWN,WPARAM(0),packet(browser_rect.center(),scale),&mut deferred);app.frame(&mut deferred,None);
            app.handle_island(WM_MOUSEMOVE,WPARAM(1),packet(app_rect.center(),scale),&mut deferred);app.frame(&mut deferred,None);
            app.handle_island(WM_LBUTTONUP,WPARAM(0),packet(app_rect.center(),scale),&mut deferred);app.frame(&mut deferred,None);app.frame(&mut deferred,None);
            assert_eq!(app.st.store.search_pins,[first.id.clone(),browser.into()]);assert!(app.st.search.active&&!app.st.search.embedded()&&app.search_web.is_none(),"dragging a browser pin must not launch it");
            let encoded=serde_json::to_string(&app.st.store).unwrap();let restarted=AppState::new(Settings::default(),serde_json::from_str(&encoded).unwrap());assert_eq!(restarted.store.search_pins,app.st.store.search_pins);
            let before=app.st.store.search_pins.clone();let source=app.hits.iter().find(|h|h.id==crate::ui::wid_n("search.recent",wid(browser))).unwrap().rect.center();let outside=Point::new(body.right()-12.0,body.bottom()-12.0);
            app.handle_island(WM_LBUTTONDOWN,WPARAM(0),packet(source,scale),&mut deferred);app.frame(&mut deferred,None);app.handle_island(WM_MOUSEMOVE,WPARAM(1),packet(outside,scale),&mut deferred);app.frame(&mut deferred,None);app.handle_island(WM_LBUTTONUP,WPARAM(0),packet(outside,scale),&mut deferred);app.frame(&mut deferred,None);
            assert_eq!(app.st.store.search_pins,before);assert!(app.st.search.active&&!app.st.search.embedded(),"dropping outside pins cancels without opening the app");

            app.input.mouse=None;app.frame(&mut deferred,None);
            if let Some(dir)=std::env::var_os("LUMO_SEARCH_REWORK_QA"){let dir=std::path::PathBuf::from(dir);std::fs::create_dir_all(&dir).unwrap();let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(dir.join(format!("pins-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            for _ in 0..40{app.handle_island(WM_KEYDOWN,WPARAM(VK_DOWN.0 as usize),LPARAM(0),&mut deferred);}app.frame(&mut deferred,None);
            let last=app.st.search.view.as_ref().unwrap().selected().unwrap().id;assert!(app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.recent",wid(&last))),"keyboard navigation must reveal apps beyond the visible strip");
            for _ in 0..40{app.handle_island(WM_KEYDOWN,WPARAM(VK_UP.0 as usize),LPARAM(0),&mut deferred);}app.frame(&mut deferred,None);
            app.input.keys="lumo browser".chars().map(KeyInput::Char).collect();app.frame(&mut deferred,None);clock::hold(Some(base+0.25));app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            assert!(app.st.search.view.as_ref().unwrap().selected().unwrap().is_browser());assert_eq!(app.hits.iter().find(|h|h.id==wid("search.query")).unwrap().rect.w,width);
            app.input.keys=vec![KeyInput::SelectAll,KeyInput::Backspace];app.frame(&mut deferred,None);clock::hold(Some(base+0.5));app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            assert!(app.st.search.view.as_ref().unwrap().query.is_empty());assert!(!app.hits.iter().any(|h|h.id==wid("search.browser")||h.id==crate::ui::wid_n("search.result",0)));
            assert!(app.st.search.web_url.is_none(),"showing and arranging Search must not start a browser");
            let browser_rect=app.hits.iter().find(|h|h.id==crate::ui::wid_n("search.recent",wid(browser))).unwrap().rect;
            for msg in [WM_LBUTTONDOWN,WM_LBUTTONUP]{app.handle_island(msg,WPARAM(0),packet(browser_rect.center(),scale),&mut deferred);app.frame(&mut deferred,None);}app.frame(&mut deferred,None);
            assert!(app.st.search.embedded()&&crate::browser_home::is_home(&app.st)&&app.search_web.is_none());assert!(app.st.store.search_usage.contains_key(browser));app.close_search();
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a Windows desktop and installed apps"]
    fn inline_search_renders_icons_keyboard_filters_and_collapses_in_both_themes() {
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        let all=services::search::scan(&std::sync::atomic::AtomicBool::new(false)).unwrap();
        let mut apps:Vec<_>=all.iter().filter(|e|["dia","media player","windows media","mediainfo"].iter().any(|n|e.title.to_lowercase().contains(n))).take(6).cloned().collect();
        for entry in &all{if apps.len()>=6{break;}if !apps.iter().any(|e|e.id==entry.id){apps.push(entry.clone());}}
        app.st.search.apps=apps;
        let items=app.st.search.apps.iter().map(|e|(e.id.clone(),e.target.clone())).collect();
        let images=services::imaging::search_icons(items,&std::sync::atomic::AtomicBool::new(false));
        assert!(images.iter().filter(|(_,i)|i.is_some()).count()>=3,"Windows supplies actual app icons");
        for (_,image) in &images{if let Some(image)=image{assert_eq!((image.width,image.height),(48,48));assert!(image.pixels.chunks_exact(4).any(|p|p[3]>0));assert!(image.pixels.chunks_exact(4).any(|p|p[3]==0),"icons preserve native transparency");}}
        app.st.search.icons.extend(images);
        for (i,entry) in app.st.search.apps.iter().enumerate(){app.st.store.search_usage.insert(entry.id.clone(),lumo_core::search::Usage{count:(20-i)as u32,last:i as u64});}
        for (index,theme) in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            let now=100.0+index as f64*100.0;clock::hold(Some(now));app.st.now=now;app.st.settings.theme=theme;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.st.notice=None;app.st.pinned=true;app.expanded=false;app.mode=activities::collapsed_mode(&app.st);app.island.snap(app.geom_for(&app.mode));
            app.st.settings.search.web=false;let initial=app.island.current();app.open_search();app.frame(&mut deferred,None);clock::hold(Some(now+0.05));app.frame(&mut deferred,None);let middle=app.island.current();assert!(middle.w>initial.w&&middle.h>initial.h);
            for step in 1..=70{clock::hold(Some(now+0.05+step as f64/60.0));app.frame(&mut deferred,None);}
            assert!(app.st.search.active&&app.expanded&&app.island.settled());assert!(app.hits.iter().any(|h|h.id==wid("search.query")));
            assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.filter",0)||h.id==wid("search.settings")),"no filter chips or footer gear");
            assert!(!app.hits.iter().any(|h|h.id==wid("search.close")),"local search has no close control");
            assert!(!app.hits.iter().any(|h|h.id==wid("search.keep_open")||h.id==wid("search.resume")),"page controls belong only to the browser");
            assert!(!app.hits.iter().any(|h|h.id==wid("search.browser")),"web search off must not expose an active browser button");
            assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.result",0)),"empty Search has no full result rows");
            assert!(app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.recent",crate::ui::wid(&app.st.search.apps[0].id))));
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(root.join(format!("island-search-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.handle_island(WM_KEYDOWN,WPARAM(VK_DOWN.0 as usize),LPARAM(0),&mut deferred);app.frame(&mut deferred,None);assert_eq!(app.st.search.view.as_ref().unwrap().selected,1);
            let id=app.st.search.view.as_ref().unwrap().selected().unwrap().id;let hit=app.hits.iter().find(|h|h.id==crate::ui::wid_n("search.recent",crate::ui::wid(&id))).unwrap().rect.center();app.search_context_menu(Some(hit),&mut deferred);assert!(matches!(deferred.last(),Some(Deferred::SearchMenu(entry,_))if matches!(entry.target,lumo_core::search::Target::App(_))));app.st.search.context_menu=false;deferred.clear();
            for c in "no-app-has-this-name-987".chars(){app.handle_island(WM_CHAR,WPARAM(c as usize),LPARAM(0),&mut deferred);}app.frame(&mut deferred,None);
            assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.result",0)),"no stale result after a query change");
            app.handle_island(WM_KEYDOWN,WPARAM(VK_RETURN.0 as usize),LPARAM(0),&mut deferred);app.frame(&mut deferred,None);assert!(app.st.search.active&&app.text_focus);
            app.st.settings.search.web=true;app.frame(&mut deferred,None);assert!(app.hits.iter().any(|h|h.id==wid("search.web")));
            assert!(!app.hits.iter().any(|h|h.id==wid("search.browser")),"typing hides the browser icon");
            for step in 1..=65{clock::hold(Some(now+1.3+step as f64/60.0));app.frame(&mut deferred,None);}
            assert!(app.island.settled());let empty_body=app.island.body_rect_at(app.cx);assert!((empty_body.h-204.0).abs()<1.0,"empty search shrinks to its content");let action=app.hits.iter().find(|h|h.id==wid("search.web")).unwrap().rect;assert!(action.bottom()<=empty_body.bottom()-16.0,"the whole web action remains clickable inside the island");
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(root.join(format!("search-empty-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            app.st.settings.search.google_suggestions=true;
            app.sync_search_suggestions();assert!(app.st.search.suggestions.due.is_some() && app.st.search.suggestions.pending.is_none(),"typing only arms one debounce wake");
            let query=app.st.search.view.as_ref().unwrap().query.trim().to_owned();let serial=app.st.search.suggestions.serial;
            app.st.search.suggestions.due=None;app.st.search.suggestions.pending=Some((serial,std::sync::Arc::new(services::http::Cancel::default())));
            crate::search::apply_suggestions(&mut app.st,crate::search::SuggestionsEvent{request:serial,query,items:vec!["Lumo desktop".into(),"Lumo Windows".into(),"Lumo mini browser".into()]});
            app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.suggestion",0)));
            app.handle_island(WM_KEYDOWN,WPARAM(VK_DOWN.0 as usize),LPARAM(0),&mut deferred);app.frame(&mut deferred,None);
            assert!(matches!(app.st.search.view.as_ref().unwrap().selected().unwrap().target,lumo_core::search::Target::Web{query,..}if query=="Lumo desktop"));
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(root.join(format!("search-suggestions-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            let before=app.st.settings.clone();app.st.settings.search.google_suggestions=false;app.settings_changed(&before);
            assert!(app.st.search.view.as_ref().unwrap().suggestions.is_empty() && app.st.search.suggestions.pending.is_none() && app.st.search.suggestions.due.is_none());
            // A single exact match must use one compact row, with no tall empty list.
            app.input.keys=vec![KeyInput::SelectAll];app.input.keys.extend("About your PC".chars().map(KeyInput::Char));app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.result",1)));assert!(app.island.body_rect_at(app.cx).h<130.0);
            assert!(!app.hits.iter().any(|h|h.id==wid("search.browser")),"nonempty query uses the whole field");
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(root.join(format!("search-single-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            // Ctrl+Enter explicitly searches even with a local app/settings match.
            {let body=app.island.body_rect_at(app.cx);let dirty=RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h};let mut canvas=app.gfx.begin(dirty).unwrap();let mut input=crate::ui::Input::default();input.ctrl=true;input.focused=Some(wid("search.query"));input.keys.push(KeyInput::Enter);let mut hits=vec![];let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,app.st.now);crate::search::draw(&mut ui,&mut app.st,body);drop(ui);drop(canvas);app.gfx.end().unwrap();}
            assert!(app.st.cmds.iter().any(|c|matches!(c,Command::ExecuteSearch(lumo_core::search::Target::Web{query,url})if query=="About your PC"&&Some(url)==app.st.settings.search.engine.url(query).as_ref())));app.st.cmds.clear();
            app.input.keys=vec![KeyInput::SelectAll];app.input.keys.extend("hirunews.lk".chars().map(KeyInput::Char));app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(root.join(format!("search-visit-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            let action=app.hits.iter().find(|h|h.id==wid("search.web")).unwrap().rect.center();app.search_context_menu(Some(action),&mut deferred);
            assert!(matches!(deferred.last(),Some(Deferred::SearchMenu(entry,_))if entry.title=="Visit" && matches!(&entry.target,lumo_core::search::Target::Web{url,..}if url=="https://hirunews.lk")));app.st.search.context_menu=false;deferred.clear();
            for action in [Some("search.web"),None] {
                let body=app.island.body_rect_at(app.cx);let dirty=RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h};let mut canvas=app.gfx.begin(dirty).unwrap();let mut input=crate::ui::Input::default();
                if let Some(id)=action{input.clicked=Some((wid(id),MouseButton::Left));}else{input.focused=Some(wid("search.query"));input.keys.push(KeyInput::Enter);}
                let mut hits=vec![];let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,app.st.now);crate::search::draw(&mut ui,&mut app.st,body);drop(ui);drop(canvas);app.gfx.end().unwrap();
                assert!(app.st.cmds.iter().any(|c|matches!(c,Command::ExecuteSearch(lumo_core::search::Target::Web{url,..})if url=="https://hirunews.lk")),"direct visit via {action:?}");app.st.cmds.clear();
            }
            app.st.settings.search.services=true;
            app.st.search.integrations=vec![lumo_core::search::Entry::new("service-demo".into(),"Lumo demo service".into(),"Running · Windows service".into(),"services",lumo_core::search::Group::Service,lumo_core::search::Target::Service(lumo_core::search::Service{name:"LumoDemoMissingService".into(),state:4,accepts_stop:true}))];
            app.input.keys=vec![KeyInput::SelectAll];app.input.keys.extend("Lumo demo service".chars().map(KeyInput::Char));app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            let stop_id=crate::search::service_action_id("LumoDemoMissingService",false);
            assert!(app.hits.iter().any(|h|h.id==stop_id));
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);let(w,h,p,_)=app.selftest_capture();crate::png::save_rgba(root.join(format!("search-service-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
            {let body=app.island.body_rect_at(app.cx);let dirty=RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h};let mut canvas=app.gfx.begin(dirty).unwrap();let mut input=crate::ui::Input::default();input.clicked=Some((stop_id,MouseButton::Left));let mut hits=vec![];let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,app.st.now);crate::search::draw(&mut ui,&mut app.st,body);drop(ui);drop(canvas);app.gfx.end().unwrap();}
            assert!(app.st.cmds.iter().any(|c|matches!(c,Command::SearchService{name,start:false}if name=="LumoDemoMissingService")));app.st.cmds.clear();
            let running=app.st.search.integrations[0].clone();
            if let lumo_core::search::Target::Service(service)=&mut app.st.search.integrations[0].target{service.state=1;service.accepts_stop=false;}
            crate::search::refresh(&mut app.st);
            // A Stop click captured before a state refresh cannot become Start.
            {let body=app.island.body_rect_at(app.cx);let dirty=RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h};let mut canvas=app.gfx.begin(dirty).unwrap();let mut input=crate::ui::Input::default();input.clicked=Some((stop_id,MouseButton::Left));let mut hits=vec![];let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,app.st.now);crate::search::draw(&mut ui,&mut app.st,body);drop(ui);drop(canvas);app.gfx.end().unwrap();}
            assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::SearchService{..})),"stale Stop click must not execute Start");app.st.cmds.clear();app.st.search.integrations[0]=running;
            // Disabling the source must reject a queued/stale service action.
            app.st.settings.search.services=false;app.search_service("LumoDemoMissingService",false);assert!(!app.st.search.action_pending);app.st.search.integrations.clear();
            app.handle_island(WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0),&mut deferred);assert!(!app.expanded&&!app.st.search.active);
            for step in 1..=90{clock::hold(Some(now+2.0+step as f64/60.0));app.frame(&mut deferred,None);}assert!(app.st.search.view.is_none());
            app.st.settings.search.web=false;
        }
        // No search history: opening the launcher must not invent suggestions.
        app.st.store.search_usage.clear();app.open_search();app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.result",0)));
        app.close_search();clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }

    #[test]#[ignore="requires a Windows desktop and installed WebView2"]
    fn ai_site_menu_handoffs_ready_notices_and_rest_are_offline(){
        use std::time::{Duration,Instant};
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        unsafe{OleInitialize(None).unwrap();}
        let home=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/ai-sites-app-qa-home");
        std::fs::create_dir_all(&home).unwrap();std::env::set_var("LUMO_HOME",home);
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();
        let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));
        let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;
        let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);
        app.monitor.rect.top+=80;app.monitor.work.top+=80;
        app.st.settings.ai.keep_open=true;app.st.settings.ai.resume=true;app.st.settings.ai.models_url.clear();app.st.ai.key_present=false;app.st.ai.loaded=true;app.st.ai.draft="Unsent native draft".into();clock::hold(None);
        let mut deferred=vec![];app.open_ai(true);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        app.input.clicked=Some((wid("ai.providers"),MouseButton::Left));app.frame(&mut deferred,None);app.frame(&mut deferred,None);
        assert!(app.st.ai.sites.menu);
        for i in 0..5{assert!(app.hits.iter().any(|h|h.id==crate::ui::wid_n("ai.provider",i)));}
        for i in [5,6,7]{assert!(!app.hits.iter().any(|h|h.id==crate::ui::wid_n("ai.provider",i)),"the AI picker must contain the remaining providers only, without Perplexity or settings entries");}
        // Render the real picker with Enter while withholding side effects;
        // choosing a website must not send the native composer's draft.
        {
            let dirty=RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h};let body=app.island.body_rect_at(app.cx);
            let mut canvas=app.gfx.begin(dirty).unwrap();let mut input=crate::ui::Input::default();input.keys=vec![KeyInput::Down,KeyInput::Enter];let mut hits=vec![];
            let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,app.st.now);crate::pages::ai::draw_chat(&mut ui,&mut app.st,crate::pages::ai::workspace_area(body),false);drop(ui);drop(canvas);app.gfx.end().unwrap();
            assert!(app.st.cmds.contains(&Command::SelectAiSite(Some(crate::ai_sites::Site::ChatGpt))));
            assert!(!app.st.cmds.contains(&Command::Ai(crate::ai::Action::Send)));app.st.cmds.clear();app.st.ai.sites.menu=true;
        }
        if let Some(dir)=std::env::var_os("LUMO_AI_SITES_QA"){
            // Let the existing finite opening fade settle before visual QA.
            std::thread::sleep(Duration::from_millis(350));app.frame(&mut deferred,None);
            let dir=std::path::PathBuf::from(dir);std::fs::create_dir_all(&dir).unwrap();let(w,h,p,_)=app.selftest_capture();
            crate::png::save_rgba(dir.join("ai-provider-menu.png"),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();
        }
        app.st.ai.sites=crate::ai_sites::State{selected:Some(crate::ai_sites::Site::ChatGpt),url:"about:blank".into(),..Default::default()};
        app.sync_ai_web();
        let pump=|app:&mut App,predicate:&dyn Fn(&App)->bool|{
            let end=Instant::now()+Duration::from_secs(20);
            while !predicate(app){assert!(Instant::now()<end,"AI website fixture timed out");unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}app.sync_ai_web();std::thread::sleep(Duration::from_millis(10));}
        };
        pump(&mut app,&|app|app.ai_web.as_ref().is_some_and(|web|web.status().0));
        let web=app.ai_web.as_ref().unwrap().test_web().unwrap();
        unsafe{web.NavigateToString(w!("<!doctype html><body style='margin:0;background:#18314a;color:white'><h1>Local website fixture</h1><p>Full client area; no address bar.</p></body>")).unwrap();}
        app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        let capture=|app:&mut App,name:&str|{
            let Some(dir)=std::env::var_os("LUMO_AI_SITES_QA")else{return;};
            for _ in 0..8{unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}app.frame(&mut deferred,None);unsafe{let _=windows::Win32::Graphics::Dwm::DwmFlush();}std::thread::sleep(Duration::from_millis(30));}
            let(x,y,w,h,radius)=if let Some(window)=app.ai_window.as_mut(){window.draw(&mut app.st).unwrap();let mut r=RECT::default();unsafe{GetWindowRect(window.hwnd,&mut r).unwrap();}(r.left,r.top,r.right-r.left,r.bottom-r.top,24.0*window.page_bounds().1)}else{let b=app.island.body_rect_at(app.cx);let s=app.gfx.scale();(app.surface.x+(b.x*s).round()as i32,app.surface.y+(b.y*s).round()as i32,(b.w*s).round()as i32,(b.h*s).round()as i32,app.island.current().r*s)};
            unsafe{
                use windows::Win32::Graphics::Gdi::*;
                let dc=GetDC(None);let mem=CreateCompatibleDC(dc);let mut bits=std::ptr::null_mut();let info=BITMAPINFO{bmiHeader:BITMAPINFOHEADER{biSize:std::mem::size_of::<BITMAPINFOHEADER>()as u32,biWidth:w,biHeight:-h,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()},..Default::default()};
                let bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&mut bits,None,0).unwrap();let old=SelectObject(mem,bitmap);BitBlt(mem,0,0,w,h,dc,x,y,SRCCOPY|CAPTUREBLT).unwrap();let mut pixels=std::slice::from_raw_parts(bits.cast::<u8>(),(w*h*4)as usize).to_vec();
                for(px,py)in[(w/2,h/2),(6,h/2),(w-6,h/2),(w/2,h-6)]{
                    let offset=((py*w+px)*4)as usize;
                    for(channel,expected)in [0x4a_u8,0x31,0x18].into_iter().enumerate(){assert!(pixels[offset+channel].abs_diff(expected)<=3,"{name}: the actual page must fill pixel ({px},{py}), got {:?}",&pixels[offset..offset+3]);}
                }
                for(i,p)in pixels.chunks_exact_mut(4).enumerate(){let px=(i%w as usize)as f32+0.5;let py=(i/w as usize)as f32+0.5;let dx=(radius-px).max(px-(w as f32-radius)).max(0.0);let dy=(radius-py).max(py-(h as f32-radius)).max(0.0);if dx*dx+dy*dy>radius*radius{p.fill(0);}else{p[3]=255;}}
                crate::png::save_rgba(std::path::PathBuf::from(dir).join(format!("{name}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();SelectObject(mem,old);let _=DeleteObject(bitmap);let _=DeleteDC(mem);ReleaseDC(None,dc);
            }
        };
        let mut capture=capture;capture(&mut app,"ai-website-island-offline");
        for id in ["ai.keep-open","ai.open-full","ai.close","ai.providers"]{assert!(app.hits.iter().any(|h|h.id==wid(id)));}
        let close=app.hits.iter().find(|h|h.id==wid("ai.close")).unwrap().rect;let popup=crate::ai_sites::toolbar(app.island.body_rect_at(app.cx));assert!((popup.right()-close.right()-8.0).abs()<0.1,"close has the same small right inset as the overlay");

        for id in ["ai.history","ai.new","ai.temporary","ai.prompt","search.query"]{assert!(!app.hits.iter().any(|h|h.id==wid(id)));}
        assert_eq!(app.ai_web.as_ref().unwrap().parent(),hwnd);
        app.open_ai_window();let mini=app.ai_window.as_ref().unwrap().hwnd;
        {let(client,_)=app.ai_window.as_ref().unwrap().page_bounds();let mut outer=RECT::default();unsafe{GetWindowRect(mini,&mut outer).unwrap();}assert_eq!((client.right,client.bottom),(outer.right-outer.left,outer.bottom-outer.top),"the website gets the entire frameless client area");}
        assert_eq!(app.ai_web.as_ref().unwrap().parent(),mini);assert_eq!(app.ai_web.as_ref().unwrap().test_web().unwrap(),web);
        app.ai_window.as_mut().unwrap().draw(&mut app.st).unwrap();
        app.st.settings.ai.keep_open=false;
        let dialog=unsafe{CreateWindowExW(WINDOW_EX_STYLE(0),w!("STATIC"),w!("Owned website dialog fixture"),WS_POPUP,0,0,80,80,mini,None,windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap(),None).unwrap()};
        app.handle_ai_window(mini,WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(dialog.0 as isize));assert!(app.ai_window.is_some(),"website-owned dialogs must not dismiss the mini window");unsafe{DestroyWindow(dialog).unwrap();}app.st.settings.ai.keep_open=true;
        capture(&mut app,"ai-website-mini-offline");
        app.return_ai_host();app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        app.st.settings.ai.keep_open=false;
        let dialog=unsafe{CreateWindowExW(WINDOW_EX_STYLE(0),w!("STATIC"),w!("Owned website dialog fixture"),WS_POPUP,0,0,80,80,hwnd,None,windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap(),None).unwrap()};
        app.search_lost_activation(dialog);assert!(app.expanded,"website-owned dialogs must not dismiss the island");unsafe{DestroyWindow(dialog).unwrap();}app.st.settings.ai.keep_open=true;
        assert!(!unsafe{IsWindow(mini)}.as_bool());assert_eq!(app.ai_web.as_ref().unwrap().parent(),hwnd);assert_eq!(app.ai_web.as_ref().unwrap().test_web().unwrap(),web);
        for reason in 0..3{
            app.locked=reason==0;app.display_off=reason==1;app.st.power.saver=reason==2;app.sync_ai_web();assert!(app.ai_web_resting);
            app.locked=false;app.display_off=false;app.st.power.saver=false;app.sync_ai_web();assert!(!app.ai_web_resting);
        }
        unsafe{web.ExecuteScript(w!("chrome.webview.postMessage({channel:'lumo.ai-site',kind:'working'})"),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|r,_|r))).unwrap();}
        pump(&mut app,&|app|app.st.ai.sites.working);let viewport=app.ai_web.as_ref().unwrap().test_viewport();app.collapse();app.island.snap(app.geom_for(&app.target_mode()));app.sync_ai_web();assert!(app.ai_web.is_some()&&!app.ai_web_resting,"an explicitly submitted reply can finish when the island closes");
        assert_eq!(app.ai_web.as_ref().unwrap().test_viewport(),viewport,"closing the island must not resize an in-flight website into a compact viewport");
        unsafe{web.ExecuteScript(w!("chrome.webview.postMessage({channel:'lumo.ai-site',kind:'ready',text:'Offline completed answer'})"),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|r,_|r))).unwrap();}
        pump(&mut app,&|app|app.st.ai.sites.ready.is_some());assert!(app.st.notice.as_ref().unwrap().is_ai_reply());assert!(app.ai_web_resting);
        app.run_inline_action(InlineAction::Hide);assert!(app.st.ai.sites.ready.is_none()&&app.st.notice.is_none());
        app.st.settings.ai.resume=false;app.sync_ai_web();assert_eq!(app.ai_web.as_ref().unwrap().test_web().unwrap(),web,"closing websites retains the exact page even when local-chat Resume is off");
        assert_eq!(app.st.ai.draft,"Unsent native draft","website lifetime must not discard the native chat draft");
        app.select_ai_site(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();OleUninitialize();}
    }

    #[test]#[ignore="requires a Windows desktop and installed WebView2"]
    fn web_results_render_inside_the_layered_island_and_close_with_it(){
        use std::{rc::Rc,cell::Cell,time::{Duration,Instant}};
        use windows::core::Interface;
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        unsafe{OleInitialize(None).unwrap();}
        let home=std::env::current_dir().unwrap().join(".tools/search-v2-web-island");std::fs::create_dir_all(&home).unwrap();std::env::set_var("LUMO_HOME",home);
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;app.st.settings.search.web=true;let mut deferred=vec![];clock::hold(None);
        app.open_search();app.execute_search(lumo_core::search::Target::Web{query:"fixture".into(),url:"about:blank".into()});
        let deadline=Instant::now()+Duration::from_secs(20);
        while !app.search_web.as_ref().unwrap().status().0{assert!(Instant::now()<deadline,"WebView2 startup timed out");unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}app.frame(&mut deferred,None);std::thread::sleep(Duration::from_millis(10));}
        let web=app.search_web.as_ref().unwrap().test_web().unwrap();let ready=Rc::new(Cell::new(false));let done=ready.clone();let mut token=windows::Win32::System::WinRT::EventRegistrationToken::default();
        unsafe{
            web.add_NavigationCompleted(&webview2_com::NavigationCompletedEventHandler::create(Box::new(move|web,_|{if let Some(web)=web{let done=done.clone();web.ExecuteScript(w!("document.body?.dataset.ready || ''"),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|code,value|{code?;if value=="\"yes\""{done.set(true);}Ok(())})))?;}Ok(())})),&mut token).unwrap();
            web.NavigateToString(w!("<!doctype html><html style='background:rgb(21,43,67)'><head><title>Lumo browser fixture</title></head><body data-ready='yes' style='margin:0;background:rgb(21,43,67);color:white'>Lumo embedded web results</body></html>")).unwrap();
        }
        let deadline=Instant::now()+Duration::from_secs(10);
        while !ready.get(){assert!(Instant::now()<deadline,"offline page did not render");unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}app.frame(&mut deferred,None);std::thread::sleep(Duration::from_millis(10));}
        app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        let body=app.island.body_rect_at(app.cx);let scale=app.gfx.scale();let at=POINT{x:app.surface.x+(body.center().x*scale)as i32,y:app.surface.y+((body.y+180.0)*scale)as i32};
        let deadline=Instant::now()+Duration::from_secs(3);let mut pixel=COLORREF(0);
        loop{app.frame(&mut deferred,None);unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}let _=windows::Win32::Graphics::Dwm::DwmFlush();let dc=windows::Win32::Graphics::Gdi::GetDC(None);pixel=windows::Win32::Graphics::Gdi::GetPixel(dc,at.x,at.y);windows::Win32::Graphics::Gdi::ReleaseDC(None,dc);}
            if (pixel.0&255).abs_diff(21)<8 && ((pixel.0>>8)&255).abs_diff(43)<8 && ((pixel.0>>16)&255).abs_diff(67)<8{break;}assert!(Instant::now()<deadline,"web content must be visible inside the island, pixel={:06x}",pixel.0);std::thread::sleep(Duration::from_millis(30));}
        unsafe{assert!(IsChild(hwnd,WindowFromPoint(at)).as_bool(),"web content must receive input through the island aperture");}
        for id in ["search.back","search.reload","search.external","search.query","search.keep_open","search.resume"]{assert!(app.hits.iter().any(|h|h.id==wid(id)),"browser toolbar: {id}");}
        assert!(!app.hits.iter().any(|h|h.id==wid("search.close")),"embedded browser has no close control");
        assert_eq!(app.st.search.navigation.title,"Lumo browser fixture","idle address bar receives the actual document title");
        assert!((app.island.body_rect_at(app.cx).w-crate::search::WIDTH).abs()<1.0,"tall browser preserves launcher width");
        {
            let result=Rc::new(std::cell::RefCell::new(None));let finished=result.clone();
            unsafe{web.ExecuteScript(w!("innerWidth >= 500 && innerWidth <= 560 && navigator.userAgent.includes('Windows') && !navigator.userAgent.includes('Mobile')"),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|code,value|{code?;*finished.borrow_mut()=Some(value);Ok(())}))).unwrap();}
            let deadline=Instant::now()+Duration::from_secs(5);while result.borrow().is_none(){assert!(Instant::now()<deadline);unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}std::thread::sleep(Duration::from_millis(10));}
            assert_eq!(result.borrow().as_deref(),Some("true"),"narrow window renders at normal scale with a desktop user agent");
        }
        assert!(!app.hits.iter().any(|h|h.id==wid("search.browser")),"the webpage uses its address toolbar instead of the local Search browser button");
        // Typing in the omnibox must preserve the current page/controller.
        let original=app.st.search.navigation.url.clone();
        app.search_web_action(crate::search_web::Action::Address);
        assert_eq!(unsafe{GetFocus()},hwnd);assert!(app.input.selection_all);
        app.frame(&mut deferred,None);assert!(!app.hits.iter().any(|h|matches!(h.id,id if id==wid("search.back")||id==wid("search.reload")||id==wid("search.external"))),"editing hides every browser icon");
        app.input.keys=vec![KeyInput::SelectAll,KeyInput::Char('x')];app.frame(&mut deferred,None);
        assert_eq!(app.st.search.address,"x");assert!(app.search_web.is_some());assert_eq!(app.st.search.navigation.url,original);
        app.st.search.address=original;app.st.search.address_dirty=false;app.input.focused=None;
        // Native DirectWrite pointer geometry must round-trip Unicode, and
        // pointer drags must select exact text rather than just move the caret.
        {
            let mut field="Wi😀fi".to_string();let field_id=wid("search.query");let mut input=crate::ui::Input{focused:Some(field_id),..Default::default()};let mut hits=vec![];
            let mut canvas=app.gfx.begin(RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h}).unwrap();let style=crate::gfx::TextStyle::body();let rect=Rect::new(20.0,20.0,500.0,42.0);let x=rect.x+43.0;
            let positions:Vec<_>=(0..=5).map(|i|canvas.text_caret(&field,&style,10000.0,i).0).collect();
            for i in 0..=5{assert_eq!(canvas.text_position(&field,&style,positions[i]+0.1),i,"Unicode pointer/caret round trip {i}");}
            input.just_pressed=true;input.down=true;input.pressed_id=Some(field_id);input.press_pos=Some(Point::new(x+positions[2]+0.1,40.0));input.mouse=input.press_pos;
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,0.0);ui.text_input_capsule(field_id,rect,&mut field,"Search",128,app.st.accent);}
            assert_eq!(input.caret,2);input.end_frame();
            input.just_pressed=true;input.press_pos=Some(Point::new(x+positions[1]+0.1,40.0));input.mouse=Some(Point::new(x+positions[3]+0.1,40.0));
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,0.0);ui.text_input_capsule(field_id,rect,&mut field,"Search",128,app.st.accent);}
            input.end_frame();
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,0.0);ui.text_input_capsule(field_id,rect,&mut field,"Search",128,app.st.accent);}
            assert_eq!((input.selection_anchor,input.caret),(Some(1),3));input.down=false;input.pressed_id=None;input.released=Some((field_id,Point::new(x+positions[4]+0.1,40.0)));input.keys=vec![KeyInput::Copy];
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,0.0);ui.text_input_capsule(field_id,rect,&mut field,"Search",128,app.st.accent);}
            assert_eq!(input.clipboard_write.as_deref(),Some("i😀f"));assert_eq!(input.caret,4);input.released=None;input.keys=vec![KeyInput::Char('o'),KeyInput::Char('k')];
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,0.0);ui.text_input_capsule(field_id,rect,&mut field,"Search",128,app.st.accent);}
            assert_eq!(field,"Woki");input.keys=vec![KeyInput::SelectHome,KeyInput::Cut];
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut input,&mut hits,0.0);ui.text_input_capsule(field_id,rect,&mut field,"Search",128,app.st.accent);}
            assert_eq!(field,"i");assert_eq!(input.clipboard_write.as_deref(),Some("Wok"));
            let mut url="https://example.com/one/two?q=three".to_owned();let mut address_input=crate::ui::Input::default();address_input.focused=Some(field_id);address_input.just_pressed=true;address_input.pressed_id=Some(field_id);address_input.press_pos=Some(Point::new(rect.x+120.0,rect.center().y));address_input.double_click=true;
            {let mut ui=crate::ui::Ui::new(&mut canvas,&mut address_input,&mut hits,0.0);ui.address_input(field_id,rect,&mut url,app.st.accent,None);}
            assert_eq!((address_input.selection_anchor,address_input.caret),(Some(0),url.chars().count()),"double-click selects the complete URL across slashes");drop(canvas);app.gfx.end().unwrap();
        }
        app.frame(&mut deferred,None);
        // Both native clipping and the upper visual must keep the corners out
        // of the page, while the middle remains an interactive WebView child.
        unsafe {use windows::Win32::Graphics::Gdi::*;let region=CreateRectRgn(0,0,0,0);assert!(GetWindowRgn(app.search_web.as_ref().unwrap().test_host(),region).0>0);assert!(!PtInRegion(region,0,0).as_bool());assert!(PtInRegion(region,50,50).as_bool());let _=DeleteObject(region);}
        {
            // Check the island's GPU aperture against the native clipping too.
            app.gfx.request_capture(RECT{left:0,top:0,right:app.surface.w,bottom:app.surface.h});app.frame(&mut deferred,None);
            let pixels=app.gfx.take_capture();let s=app.gfx.scale();let page=crate::search::snapped_web_rect(app.island.body_rect_at(app.cx),s);
            let(left,top,right,bottom)=((page.x*s).round() as i32,(page.y*s).round() as i32,(page.right()*s).round() as i32,(page.bottom()*s).round() as i32);
            unsafe{use windows::Win32::Graphics::Gdi::*;let region=CreateRectRgn(0,0,0,0);GetWindowRgn(app.search_web.as_ref().unwrap().test_host(),region);
                // An 8-DIP inset puts the page bounding box across the outer
                // rounded corners. Exclude the island's own transparent/AA
                // edge, and also ensure the child cannot bleed outside it.
                let body=app.island.body_rect_at(app.cx);let mask=HitMask{rect:None,notch:None,pill:Some((body.inset(2.0/s,2.0/s),app.island.current().r-2.0/s))};
                for y in top..bottom{for x in left..right{let inside=mask.contains(Point::new((x as f32+0.5)/s,(y as f32+0.5)/s));let child=PtInRegion(region,x-left,y-top).as_bool();if inside&&pixels[((y*app.surface.w+x)*4+3) as usize]<250{assert!(child,"uncovered island pixel ({x},{y})");}if child{assert!(inside,"web content outside island at ({x},{y})");}}}let _=DeleteObject(region);}
        }
        // Detach the live controller, rather than creating a second page.
        let kept=app.search_web.as_ref().unwrap().test_web().unwrap();
        app.switch_search_browser(false);app.sync_search_web();assert!(!app.st.search.embedded()&&!app.st.search.web_visible);assert_eq!(app.search_web.as_ref().unwrap().test_web().unwrap(),kept);
        app.switch_search_browser(true);app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);assert!(app.st.search.embedded());assert_eq!(app.search_web.as_ref().unwrap().test_web().unwrap(),kept);
        app.toggle_mini_browser();app.frame(&mut deferred,None);
        app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        let mini=app.browser_window.as_ref().unwrap().hwnd;
        assert!(app.st.search.detached && !app.st.search.active && !app.expanded);
        assert_eq!(app.search_web.as_ref().unwrap().test_web().unwrap(),kept);
        unsafe {assert!(IsChild(mini,app.search_web.as_ref().unwrap().test_host()).as_bool());}
        app.search_web_action(crate::search_web::Action::Address);assert_eq!(unsafe{GetFocus()},mini);
        app.handle_island(crate::search_web::PAGE_FOCUS,WPARAM(0),LPARAM(0),&mut deferred);
        for scale in [1.0,1.25,1.5,2.0] {
            app.browser_window.as_mut().unwrap().test_resize(scale);app.sync_search_web();
            let bounds=app.browser_window.as_ref().unwrap().page_bounds().0;
            app.st.search.navigation.loading=false;
            let (_,_,idle)=app.browser_window.as_mut().unwrap().capture(&mut app.st);
            app.st.search.navigation.loading=true;app.st.search.navigation.progress=0.5;
            let (w,h,half)=app.browser_window.as_mut().unwrap().capture(&mut app.st);
            app.st.search.navigation.progress=1.0;
            let (_,_,full)=app.browser_window.as_mut().unwrap().capture(&mut app.st);
            let px=|pixels:&[u8],x:f32,y:f32|pixels[((y*scale).round() as usize*w as usize+(x*scale).round() as usize)*4];
            // The progress tint reaches the field's far edge and full height,
            // including the area behind the trailing reload/stop control.
            for (x,y) in [(600.0,36.0),(450.0,22.0)] {assert!(px(&full,x,y)>px(&half,x,y)+8,"full-field progress at {scale}x ({x},{y})");}
            let bar=crate::search::browser_toolbar(Rect::new(0.0,0.0,w as f32/scale,h as f32/scale),true).bar;
            // Loading must never paint the square corners outside the field's
            // rounded outline (the island notch mask has a flat top).
            for (x,y) in [(bar.x+1.0,bar.y+1.0),(bar.right()-1.0,bar.y+1.0),(bar.x+1.0,bar.bottom()-1.0),(bar.right()-1.0,bar.bottom()-1.0)] {
                let index=((y*scale).round() as usize*w as usize+(x*scale).round() as usize)*4;
                for channel in 0..4 {assert!(full[index+channel].abs_diff(idle[index+channel])<=2,"loading corner at {scale}x ({x},{y}) channel {channel}");}
            }
            unsafe {
                use windows::Win32::Graphics::Gdi::*;
                let region=CreateRectRgn(0,0,0,0);assert!(GetWindowRgn(app.search_web.as_ref().unwrap().test_host(),region).0>0);
                // Every transparent aperture pixel must be backed by native
                // page pixels. A mismatch is a hole through to the desktop.
                for y in bounds.top..bounds.bottom {for x in bounds.left..bounds.right {
                    if full[((y*w+x)*4+3) as usize]<250 {assert!(PtInRegion(region,x-bounds.left,y-bounds.top).as_bool(),"uncovered aperture pixel at {scale}x ({x},{y})");}
                }}
                let _=DeleteObject(region);
            }
            if let Some(root)=std::env::var_os("LUMO_SEARCH_QA") {
                let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();
                crate::png::save_rgba(root.join(format!("mini-browser-loading-{scale}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&half)).unwrap();
            }
        }
        // Exercise real editor input and the debounce scope, using an injected
        // response so native QA never sends a query to Google.
        app.st.search.navigation.loading=false;let suggestions_before=app.st.settings.search.google_suggestions;app.st.settings.search.google_suggestions=true;
        app.search_web_action(crate::search_web::Action::Address);
        for ch in "lumo browser".chars(){app.handle_browser_window(mini,WM_CHAR,WPARAM(ch as usize),LPARAM(0));}
        app.frame(&mut deferred,None);app.sync_search_suggestions();
        assert_eq!(app.st.search.suggestions.query,"lumo browser");assert!(app.st.search.suggestions.address&&app.st.search.suggestions.due.is_some());
        let request=app.st.search.suggestions.serial;app.st.search.suggestions.due=None;app.st.search.suggestions.pending=Some((request,std::sync::Arc::new(services::http::Cancel::default())));
        crate::search::apply_suggestions(&mut app.st,crate::search::SuggestionsEvent{request,query:"lumo browser".into(),items:vec!["Lumo Windows".into(),"Lumo mini browser".into()]});
        app.browser_window.as_mut().unwrap().mark_dirty();app.frame(&mut deferred,None);
        // A real query dropdown must occlude the native page without resizing it.
        app.sync_search_web();let (bounds,scale)=app.browser_window.as_ref().unwrap().page_bounds();
        let cover=crate::search::address_cover(&app.st,Rect::new(0.0,0.0,(bounds.right+16) as f32/scale,680.0),scale);
        unsafe{use windows::Win32::Graphics::Gdi::*;let region=CreateRectRgn(0,0,0,0);GetWindowRgn(app.search_web.as_ref().unwrap().test_host(),region);assert!(!PtInRegion(region,30,2).as_bool(),"suggestions occlude native page pixels");let _=DeleteObject(region);}assert!(cover>0);
        if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){let root=std::path::PathBuf::from(root);let(w,h,p)=app.browser_window.as_mut().unwrap().capture(&mut app.st);crate::png::save_rgba(root.join("address-suggestions.png"),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
        app.handle_browser_window(mini,WM_KEYDOWN,WPARAM(VK_DOWN.0 as usize),LPARAM(0));app.frame(&mut deferred,None);assert_eq!(app.st.search.address_selected,Some(0));
        let page_before=app.st.search.navigation.url.clone();
        app.handle_browser_window(mini,WM_KEYDOWN,WPARAM(VK_TAB.0 as usize),LPARAM(0));
        assert_eq!(app.st.search.address,"Lumo Windows");assert!(app.st.search.address_suggestions.is_empty());assert_eq!(app.st.search.navigation.url,page_before);
        app.handle_browser_window(mini,WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0));app.frame(&mut deferred,None);
        assert!(app.browser_window.is_some()&&!app.st.search.address_editing&&app.st.search.address_suggestions.is_empty(),"Escape cancels address editing before closing the mini browser");
        app.search_web_action(crate::search_web::Action::Address);app.st.search.address="mouse query".into();app.st.search.address_dirty=true;
        app.st.search.suggestions.query="mouse query".into();app.st.search.suggestions.address=true;let request=app.st.search.suggestions.serial;
        app.st.search.suggestions.pending=Some((request,std::sync::Arc::new(services::http::Cancel::default())));
        crate::search::apply_suggestions(&mut app.st,crate::search::SuggestionsEvent{request,query:"mouse query".into(),items:vec!["mouse chosen".into()]});app.browser_window.as_mut().unwrap().mark_dirty();app.frame(&mut deferred,None);
        let rect=app.browser_window.as_ref().unwrap().test_hit(crate::ui::wid_n("search.address-suggestion",0)).unwrap();let scale=app.browser_window.as_ref().unwrap().page_bounds().1;
        let x=(rect.center().x*scale).round()as u16;let y=(rect.center().y*scale).round()as u16;let at=LPARAM((u32::from(x)|(u32::from(y)<<16))as isize);
        app.handle_browser_window(mini,WM_LBUTTONDOWN,WPARAM(1),at);app.frame(&mut deferred,None);assert!(app.st.search.address_editing&&!app.st.search.address_suggestions.is_empty());
        app.handle_browser_window(mini,WM_LBUTTONUP,WPARAM(0),at);app.browser_window.as_mut().unwrap().draw(&mut app.st).unwrap();
        assert!(app.st.cmds.contains(&Command::NavigateSearchWeb(app.st.settings.search.engine.url("mouse chosen").unwrap())),"a suggestion mouse click applies the chosen query");app.st.cmds.clear();
        app.browser_window.as_mut().unwrap().blur_address();app.st.search.address_dirty=false;app.st.search.suggestions.cancel();
        app.st.settings.search.google_suggestions=suggestions_before;app.sync_search_web();
        app.frame(&mut deferred,None);app.frame(&mut deferred,None);
        // A real screen pixel and HWND hit must both come from the detached
        // webpage, not just an empty transparent aperture in its chrome.
        let bounds=app.browser_window.as_ref().unwrap().page_bounds().0;
        let mut mini_rect=RECT::default();unsafe{GetWindowRect(mini,&mut mini_rect).unwrap();}
        let mini_at=POINT{x:mini_rect.left+(bounds.left+bounds.right)/2,y:mini_rect.top+(bounds.top+bounds.bottom)/2};
        let deadline=Instant::now()+Duration::from_secs(4);
        loop{unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}let _=windows::Win32::Graphics::Dwm::DwmFlush();let dc=windows::Win32::Graphics::Gdi::GetDC(None);let pixel=windows::Win32::Graphics::Gdi::GetPixel(dc,mini_at.x,mini_at.y);windows::Win32::Graphics::Gdi::ReleaseDC(None,dc);
            if (pixel.0&255).abs_diff(21)<8 && ((pixel.0>>8)&255).abs_diff(43)<8 && ((pixel.0>>16)&255).abs_diff(67)<8{break;}assert!(Instant::now()<deadline,"detached page must be visible, pixel={:06x}",pixel.0);}
            app.frame(&mut deferred,None);std::thread::sleep(Duration::from_millis(15));}
        unsafe{assert!(IsChild(mini,WindowFromPoint(mini_at)).as_bool(),"detached page receives pointer input");}
        let paints=app.browser_window.as_ref().unwrap().paints;
        app.frame(&mut deferred,None);app.frame(&mut deferred,None);
        assert_eq!(app.browser_window.as_ref().unwrap().paints,paints,"idle mini browser chrome must not repaint with island frames");
        app.locked=true;app.frame(&mut deferred,None);assert!(!app.st.search.web_visible && !unsafe{IsWindowVisible(mini)}.as_bool());
        app.locked=false;app.frame(&mut deferred,None);assert!(app.st.search.web_visible && unsafe{IsWindowVisible(mini)}.as_bool());
        assert_eq!(app.search_web.as_ref().unwrap().test_web().unwrap(),kept);
        app.st.cmd(Command::ToggleSearchResume);app.run_commands(&mut deferred);app.st.cmd(Command::ToggleSearchResume);app.run_commands(&mut deferred);
        assert!(app.browser_window.is_some(),"turning off Resume must not close an open mini browser");
        let scale=app.browser_window.as_ref().unwrap().page_bounds().1;
        unsafe{SetWindowPos(mini,None,0,0,(389.0*scale).round() as i32,(317.0*scale).round() as i32,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE).unwrap();}
        app.handle_browser_window(mini,WM_EXITSIZEMOVE,WPARAM(0),LPARAM(0));app.run_commands(&mut deferred);
        assert_eq!((app.st.settings.search.mini_width,app.st.settings.search.mini_height),(389,317),"arbitrary mini dimensions are saved");
        app.open_search();app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        assert!(app.st.search.active && app.st.search.detached && !app.st.search.embedded());
        assert!(app.hits.iter().any(|h|h.id==wid("search.query")) && app.hits.iter().any(|h|h.id==crate::ui::wid_n("search.recent",wid(lumo_core::search::BROWSER_ID))) && !app.hits.iter().any(|h|h.id==wid("search.mini")),"local app search and its browser action stay available beside the mini browser");
        app.close_search();assert!(app.search_web.is_some() && app.browser_window.is_some());
        app.toggle_mini_browser();assert!(app.browser_window.is_none() && !app.st.search.detached && app.st.search.active);
        assert_eq!(app.search_web.as_ref().unwrap().test_web().unwrap(),kept);
        unsafe{assert!(IsChild(hwnd,app.search_web.as_ref().unwrap().test_host()).as_bool());assert!(!IsWindow(mini).as_bool());}
        app.toggle_mini_browser();app.frame(&mut deferred,None);
        let reopened=app.browser_window.as_ref().unwrap();let s=reopened.page_bounds().1;let mut r=RECT::default();unsafe{GetClientRect(reopened.hwnd,&mut r).unwrap();}
        assert!((r.right as f32/s-389.0).abs()<1.0 && (r.bottom as f32/s-317.0).abs()<1.0,"reopening retains the chosen mini size");
        let before=app.st.settings.clone();app.st.settings.search.mini_width=560;app.st.settings.search.mini_height=680;app.settings_changed(&before);
        app.toggle_mini_browser();
        app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        if let Some(root)=std::env::var_os("LUMO_SEARCH_QA") {
            // Optional live visual QA uses an isolated test browser profile.
            if let Ok(url)=std::env::var("LUMO_BROWSER_QA_URL") {
                app.search_web.as_ref().unwrap().navigate(&url);
                let end=Instant::now()+Duration::from_secs(20);
                loop {unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}app.frame(&mut deferred,None);if !app.st.search.navigation.loading&&app.st.search.navigation.url.starts_with("https://"){break;}assert!(Instant::now()<end,"visual QA page timed out: {:?}",app.st.search.navigation);std::thread::sleep(Duration::from_millis(20));}
            }
            let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();
            // Move only this fixture below other always-on-top islands, so a
            // running user copy cannot appear over the native capture.
            app.monitor.rect.top+=80;app.monitor.work.top+=80;app.layout_surface_for(960.0,660.0);
            let qa_modes=(app.st.settings.search.keep_open,app.st.settings.search.resume_page);app.st.settings.search.keep_open=true;app.st.settings.search.resume_page=true;
            for theme in [Theme::Classic,Theme::Pill]{
                app.st.settings.theme=theme;app.island.set_floating(pill::on(theme).then_some(pill::TOP_GAP));app.island.snap(app.geom_for(&app.target_mode()));
                for _ in 0..4{app.frame(&mut deferred,None);unsafe{let _=windows::Win32::Graphics::Dwm::DwmFlush();}std::thread::sleep(Duration::from_millis(30));}
                let body=app.island.body_rect_at(app.cx);let scale=app.gfx.scale();let w=(body.w*scale).round()as i32;let h=(body.h*scale).round()as i32;let x=app.surface.x+(body.x*scale).round()as i32;let y=app.surface.y+(body.y*scale).round()as i32;
                unsafe {
                    use windows::Win32::Graphics::Gdi::*;
                    let dc=GetDC(None);let mem=CreateCompatibleDC(dc);let mut bits=std::ptr::null_mut();let info=BITMAPINFO{bmiHeader:BITMAPINFOHEADER{biSize:std::mem::size_of::<BITMAPINFOHEADER>()as u32,biWidth:w,biHeight:-h,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()},..Default::default()};
                    let bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&mut bits,None,0).unwrap();let old=SelectObject(mem,bitmap);BitBlt(mem,0,0,w,h,dc,x,y,SRCCOPY|CAPTUREBLT).unwrap();let mut pixels=std::slice::from_raw_parts(bits.cast::<u8>(),(w*h*4)as usize).to_vec();
                    let radius=app.island.current().r;let mask=HitMask{rect:None,notch:(!pill::on(theme)).then_some((body,radius,SHOULDER)),pill:pill::on(theme).then_some((body,radius))};
                    for (i,p) in pixels.chunks_exact_mut(4).enumerate(){p[3]=if mask.contains(Point::new(body.x+(i%w as usize)as f32/scale,body.y+(i/w as usize)as f32/scale)){255}else{0};if p[3]==0{p.fill(0);}}
                    crate::png::save_rgba(root.join(format!("search-browser-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();SelectObject(mem,old);let _=DeleteObject(bitmap);let _=DeleteDC(mem);ReleaseDC(None,dc);
                }
            }
            app.st.settings.search.keep_open=qa_modes.0;app.st.settings.search.resume_page=qa_modes.1;
            app.toggle_mini_browser();app.frame(&mut deferred,None);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
            let mini=app.browser_window.as_ref().unwrap().hwnd;
            unsafe{let mut r=RECT::default();GetWindowRect(mini,&mut r).unwrap();SetWindowPos(mini,HWND_TOPMOST,r.left,120,0,0,SWP_NOSIZE|SWP_NOACTIVATE).unwrap();}
            app.search_web.as_ref().unwrap().parent_moved();app.browser_window.as_mut().unwrap().mark_dirty();app.frame(&mut deferred,None);
            unsafe{let _=windows::Win32::Graphics::Dwm::DwmFlush();}
            app.browser_window.as_mut().unwrap().capture_screen(&mut app.st,&root.join("mini-browser.png"));
            // Capture an action hint over a real offline WebView, rather than
            // only the fallback page painted by the toolbar unit fixture.
            let mut client=RECT::default();unsafe{GetClientRect(mini,&mut client).unwrap();}
            let scale=app.browser_window.as_ref().unwrap().page_bounds().1;
            let toolbar=crate::search::browser_toolbar(Rect::new(0.0,0.0,client.right as f32/scale,client.bottom as f32/scale),true);
            let point=toolbar.pin.center();let x=(point.x*scale).round()as u32;let y=(point.y*scale).round()as u32;
            app.handle_browser_window(mini,WM_MOUSEMOVE,WPARAM(0),LPARAM(((y<<16)|x)as isize));
            app.frame(&mut deferred,None);unsafe{let _=windows::Win32::Graphics::Dwm::DwmFlush();}
            app.browser_window.as_mut().unwrap().capture_screen(&mut app.st,&root.join("mini-browser-action-hint.png"));
            app.handle_browser_window(mini,0x02a3,WPARAM(0),LPARAM(0));
            app.toggle_mini_browser();app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);
        }
        // Pin only prevents automatic dismissal; explicit close still works.
        app.st.cmd(Command::ToggleSearchKeepOpen);app.run_commands(&mut deferred);app.text_focus=true;app.hovering=false;
        app.handle_island(WM_ACTIVATE,WPARAM(WA_INACTIVE as usize),LPARAM(0),&mut deferred);app.pointer_left();
        assert!(app.st.search.active&&!app.collapse_due&&app.st.settings.search.keep_open);
        app.st.cmd(Command::ToggleSearchKeepOpen);app.run_commands(&mut deferred);
        app.st.cmd(Command::ToggleSearchResume);app.run_commands(&mut deferred);
        let retained=app.search_web.as_ref().unwrap().test_web().unwrap();let marked=Rc::new(Cell::new(false));let marked_done=marked.clone();
        unsafe{retained.ExecuteScript(w!("window.lumoRetainedPage='kept'; document.body.style.minHeight='2400px'; window.scrollTo(0,300);"),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|code,_|{code?;marked_done.set(true);Ok(())}))).unwrap();}
        let deadline=Instant::now()+Duration::from_secs(5);
        while !marked.get(){assert!(Instant::now()<deadline);unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}std::thread::sleep(Duration::from_millis(10));}
        app.close_search();assert!(!app.st.search.active&&!app.st.search.web_visible&&app.search_web.is_some());
        let suspended=retained.cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3>().unwrap();let deadline=Instant::now()+Duration::from_secs(5);
        loop{unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}let mut asleep=BOOL(0);unsafe{suspended.IsSuspended(&mut asleep).unwrap();}if asleep.as_bool(){break;}assert!(Instant::now()<deadline,"retained page must suspend while collapsed");std::thread::sleep(Duration::from_millis(10));}
        let deadline=Instant::now()+Duration::from_secs(4);while app.st.search.view.is_some(){app.frame(&mut deferred,None);assert!(Instant::now()<deadline,"search closing animation did not release the active view");std::thread::sleep(Duration::from_millis(15));}
        assert!(app.st.search.view.is_none()&&app.st.search.saved_view.is_some(),"retained view must not keep the closed frame clock running");
        app.open_search();assert!(!app.st.search.embedded(),"Search always opens as an empty launcher");app.open_browser();assert_eq!(app.search_web.as_ref().unwrap().test_web().unwrap(),retained,"explicit browser mode reuses the same page and history");
        let checked=Rc::new(RefCell::new(None));let checked_done=checked.clone();
        unsafe{retained.ExecuteScript(w!("window.lumoRetainedPage + ':' + window.scrollY"),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|code,value|{code?;*checked_done.borrow_mut()=Some(value);Ok(())}))).unwrap();}
        let deadline=Instant::now()+Duration::from_secs(5);while checked.borrow().is_none(){assert!(Instant::now()<deadline);unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}std::thread::sleep(Duration::from_millis(10));}
        assert_eq!(checked.borrow().as_deref(),Some("\"kept:300\""));
        app.st.cmd(Command::ToggleSearchResume);app.run_commands(&mut deferred);assert!(!app.st.settings.search.resume_page);app.island.snap(app.geom_for(&app.target_mode()));app.frame(&mut deferred,None);app.frame(&mut deferred,None);
        let deadline=Instant::now()+Duration::from_secs(4);while !app.st.search.web_visible{app.frame(&mut deferred,None);assert!(Instant::now()<deadline,"resumed browser did not become visible");std::thread::sleep(Duration::from_millis(15));}
        unsafe{let _=windows::Win32::Graphics::Dwm::DwmFlush();}let body=app.island.body_rect_at(app.cx);let at=POINT{x:app.surface.x+(body.center().x*app.gfx.scale())as i32,y:app.surface.y+((body.y+180.0)*app.gfx.scale())as i32};
        assert_eq!(app.search_web.as_ref().unwrap().test_swipe(at).0,1,"horizontal packets over browser content are forwarded");
        let mut packet=MSG::default();assert!(unsafe{PeekMessageW(&mut packet,hwnd,crate::search_web::SWIPE,crate::search_web::SWIPE,PM_REMOVE)}.as_bool());app.handle_island(packet.message,packet.wParam,packet.lParam,&mut deferred);assert!(!app.st.search.active && app.search_web.is_none(),"a browser swipe closes the same island and destroys its controller");
        app.open_search();app.execute_search(lumo_core::search::Target::Web{query:"fixture".into(),url:"about:blank".into()});
        app.handle_island(WM_SYSKEYDOWN,WPARAM(VK_LEFT.0 as usize),LPARAM(1<<29),&mut deferred);
        assert!(app.st.search.active&&crate::browser_home::is_home(&app.st),"Alt+Left from the first page returns to Home");app.close_search();
        // Route a real native close-button click through the app-owned popup
        // procedure, including a controller that is still being created.
        app.open_search();app.execute_search(lumo_core::search::Target::Web{query:"fixture".into(),url:"about:blank".into()});
        APP.with(|slot|*slot.borrow_mut()=Some(app));
        let(mini,at)=with_app(|app|{app.toggle_mini_browser();app.frame(&mut deferred,None);let w=app.browser_window.as_ref().unwrap();let scale=w.page_bounds().1;let mut r=RECT::default();unsafe{GetClientRect(w.hwnd,&mut r).unwrap();}let close=crate::search::browser_toolbar(Rect::new(0.0,0.0,r.right as f32/scale,r.bottom as f32/scale),true).close.unwrap().center();(w.hwnd,LPARAM((((close.y*scale).round() as u16 as usize)<<16 | (close.x*scale).round() as u16 as usize) as isize))}).unwrap();
        unsafe{SendMessageW(mini,WM_LBUTTONDOWN,WPARAM(0),at);}
        with_app(|app|app.frame(&mut deferred,None)).unwrap();
        unsafe{SendMessageW(mini,WM_LBUTTONUP,WPARAM(0),at);}
        with_app(|app|{app.frame(&mut deferred,None);assert!(app.browser_window.is_none() && app.search_web.is_none() && !app.st.search.detached);}).unwrap();
        let mut app=APP.with(|slot|slot.borrow_mut().take().unwrap());
        unsafe{assert!(!IsWindow(mini).as_bool());}
        drop(web);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();OleUninitialize();}
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
    fn disabling_ai_closes_surfaces_and_blocks_stale_commands_and_replies() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { DefWindowProcW(hwnd, msg, wp, lp) }
        window::register_classes(test_proc, test_proc).unwrap();
        let hwnd = window::create_island_window().unwrap();
        let helper = window::create_helper_window().unwrap();
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::new(tx, SendHwnd::new(hwnd));
        let mut app = App::new(hwnd, helper, sink, rx, Settings::default(), Store::default(), Some("unused".into()), false, false).unwrap();
        app.settings_writable = false;
        let sample = crate::ai_window::sample_state_for_selftest();
        app.st.ai = sample.ai;
        app.st.settings.ai = sample.settings.ai;
        app.st.settings.set_addon("ai", true);
        app.st.page = PageKind::Ai;
        app.st.ai.busy = true;
        app.st.ai.request = 10;
        app.st.ai.draft = "Keep this draft".into();
        app.open_ai_window();
        let before = app.st.settings.clone();
        let message_count = app.st.ai.current.messages.len();
        app.st.settings.set_addon("ai", false);
        app.settings_changed(&before);
        assert_eq!(app.st.settings.ai, before.ai);
        assert_eq!(app.st.ai.draft, "Keep this draft");
        assert!(!app.st.ai.busy && !app.st.ai.full_open);
        assert!(app.ai_window.is_none() && app.ai_ctrl_watch.is_none());
        assert_eq!(app.st.page, PageKind::Addons);
        app.on_event(AppEvent::Ai(crate::ai::Event::Reply { request: 10, chat: 1, result: Ok(lumo_core::ai::Message { role: "assistant".into(), text: "Late reply".into(), ..Default::default() }), images: vec![], unavailable: None }));
        assert!(app.st.ai.ready_reply.is_none());
        assert_eq!(app.st.ai.current.messages.len(), message_count);
        app.st.cmd(Command::OpenAiWindow);
        app.run_commands(&mut vec![]);
        assert!(app.ai_window.is_none());
        app.set_page(PageKind::Ai);
        assert_eq!(app.st.page, PageKind::Media);
        app.hovering = true;
        app.frame(&mut vec![], None);
        assert!(app.ai_ctrl_watch.is_none());
        app.st.cmds.clear();
        app.clock.set_active(false);
        app.gfx.set_shown(false);
        unsafe { DestroyWindow(hwnd).unwrap(); DestroyWindow(helper).unwrap(); }
    }
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
                let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);
                app.st.settings.theme=theme;app.st.settings.collapse_delay_ms=0;app.st.settings.open_on_hover=false;
                app.st.notice=None;app.st.hud=None;app.st.pinned=false;app.st.aside=false;app.fullscreen_on=false;app.widgets_open=false;
                app.st.media=if activity!=1 {Some(crate::state::MediaSession{app_id:"Spotify.exe".into(),app_name:"Spotify".into(),music_source:Some(lumo_core::nowplaying::MusicSource::Streaming),title:"Fixture".into(),artist:"Artist".into(),album:"Album".into(),art:None,art_color:None,timeline:lumo_core::timeline::Timeline::new(20.0,400.0,base,true),can_prev:true,can_next:true,can_play_pause:true,can_seek:true,track_key:"fixture".into()})}else{None};
                app.st.pomodoro.run=if activity!=0 {lumo_core::pomodoro::RunState::Running{ends_at:base+1500.0}}else{lumo_core::pomodoro::RunState::Idle};
                if activity==1 {app.st.ai.snapshot();app.st.ai.current=lumo_core::ai::Chat{id:77,..Default::default()};}
                app.st.page=if activity==1 {PageKind::Focus}else{PageKind::Media};app.expanded=true;app.hovering=true;app.mode=Mode::Expanded(app.st.page);
                app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
                app.st.ai.busy=true;app.st.ai.request=1;app.st.ai.running_chat=1;
                app.on_event(AppEvent::Ai(crate::ai::Event::Reply{request:1,chat:1,result:Ok(lumo_core::ai::Message{role:"assistant".into(),text:"# Answer ready\nThe result is here.".into(),..Default::default()}),images:vec![],unavailable:None}));
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
                app.st.settings.ai.keep_open=false;
                app.search_lost_activation(helper);assert!(!app.expanded,"outside activation closes the unpinned AI surface after reading its reply");
                app.fullscreen_on=false;app.apply_fullscreen(app.st.now);app.st.cmds.clear();
                if activity==0 {
                    let popup=crate::ai_window::AiWindow::new().unwrap();let popup_hwnd=popup.hwnd;
                    app.ai_window=Some(popup);app.st.ai.full_open=true;app.hovering=false;app.input.mouse=None;
                    app.handle_ai_window(popup_hwnd,WM_CLOSE,WPARAM(0),LPARAM(0));app.st.cmds.clear();app.on_event(AppEvent::Foreground);app.on_timer(TIMER_COLLAPSE);
                    app.frame(&mut deferred,None);assert!(!app.expanded&&app.ai_window.is_none(),"closing the mini window does not reopen the island");
                    app.collapse();
                }
            }
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }
    #[test]#[ignore="requires a Windows desktop"]
    fn markdown_hover_actions_ctrl_shortcut_and_browser_sized_workspace(){
        unsafe extern "system" fn test_proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{DefWindowProcW(hwnd,msg,wp,lp)}
        window::register_classes(test_proc,test_proc).unwrap();let hwnd=window::create_island_window().unwrap();let helper=window::create_helper_window().unwrap();let(tx,rx)=mpsc::channel();let sink=EventSink::new(tx,SendHwnd::new(hwnd));let mut app=App::new(hwnd,helper,sink,rx,Settings::default(),Store::default(),Some("unused".into()),false,false).unwrap();app.settings_writable=false;let mut deferred=vec![];
        for (index,theme)in [Theme::Classic,Theme::Pill].into_iter().enumerate(){
            clock::hold(Some(800.0+index as f64*20.0));let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.ai.current.messages[1].text=crate::ai_window::markdown_fixture().into();app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);app.st.settings.theme=theme;app.st.page=PageKind::Ai;app.st.pinned=true;app.st.notice=None;app.expanded=true;app.mode=Mode::Expanded(PageKind::Ai);app.hovering=true;app.input.mouse=None;app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            assert!(!app.hits.iter().any(|h|h.id==wid("ai.copy.1")),"message actions stay hidden without hover");
            let body=app.island.body_rect_at(app.cx);assert!((body.w-crate::search::WIDTH).abs()<1.0&&body.h>400.0,"AI uses browser geometry");
            assert!(!app.hits.iter().any(|h|h.id==wid("ai.resize-grip")),"old compact-panel grip is removed");
            let area=body.inset(20.0,16.0);app.input.mouse=Some(Point::new(area.x+110.0,area.y+145.0));app.frame(&mut deferred,None);
            app.st.ai.scroll=f32::MAX;app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==wid("ai.copy.1")&&h.rect.w<=34.0));assert!(app.hits.iter().any(|h|h.id==wid("ai.markdown.1")));
            app.st.cmds.clear();
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
        for (i,theme)in [Theme::Classic,Theme::Pill].into_iter().enumerate(){let base=500.0+i as f64*20.0;clock::hold(Some(base));let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);app.st.settings.theme=theme;app.st.page=PageKind::Ai;app.st.aside=false;app.st.notice=None;app.expanded=true;app.st.pinned=true;app.mode=Mode::Expanded(PageKind::Ai);app.layout_surface();app.island.snap(app.geom_for(&app.mode));app.frame(&mut deferred,None);
            assert!(app.hits.iter().any(|h|h.id==wid("ai.prompt")));assert!(!app.hits.iter().any(|h|h.id==wid("ai.send")),"empty composer disables Send");
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
            assert!(picker.y>=app.island.body_rect_at(app.cx).y&&picker.bottom()<=app.island.body_rect_at(app.cx).bottom(),"the model picker stays inside the workspace");
            assert!(picker.h>=240.0 && picker.w>=400.0,"the picker must have room to browse");
            assert!(app.hit_mask().contains(picker.center()),"the entire detached picker receives input");
            assert!(!app.hit_mask().contains(Point::new(app.island.body_rect_at(app.cx).right()+40.0,picker.center().y)),"desktop beyond the workspace stays click-through");
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
            app.focus_text();app.input.clicked=Some((wid("ai.model-filters"),MouseButton::Left));app.frame(&mut deferred,None);assert!(app.text_focus,"opening model filters must not blur the workspace window");
            assert!(app.st.ai.model_filters_open);
            assert!([1,2,4,8].into_iter().all(|mask|app.hits.iter().any(|h|h.id==crate::ui::wid_n("ai.model-filter",mask))));
            if let Some(root)=std::env::var_os("LUMO_AI_QA"){let(w,h,pixels,_)=app.selftest_capture();crate::png::save_rgba(std::path::PathBuf::from(root).join(format!("compact-filters-{theme:?}.png")),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
            let draft="Keep this unfinished message";app.st.ai.draft=draft.into();app.frame(&mut deferred,None);
            let send=app.hits.iter().find(|h|h.id==wid("ai.send")).unwrap().rect.center();
            let lp=point(send,app.gfx.scale());
            app.handle_island(WM_LBUTTONDOWN,WPARAM(0),lp,&mut deferred);app.frame(&mut deferred,None);
            assert!(!app.st.ai.model_menu && app.expanded && app.input.pressed_id.is_none(),"clicking chat dismisses the picker and consumes the press");
            assert!(app.text_focus&&app.input.focused==Some(wid("ai.prompt")),"dismissal returns to the composer without activating another app");
            app.handle_island(WM_LBUTTONUP,WPARAM(0),lp,&mut deferred);app.frame(&mut deferred,None);
            assert_eq!(app.st.ai.draft,draft,"dismissing above Send must never submit the message");
            app.st.pinned=true;app.st.ai.model_menu=true;app.frame(&mut deferred,None);
            app.handle_island(WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0),&mut deferred);assert!(app.expanded && !app.st.ai.model_menu,"Escape dismisses the picker without closing chat");
            let popup=crate::ai_window::AiWindow::new().unwrap();let popup_hwnd=popup.hwnd;app.ai_window=Some(popup);app.st.ai.full_open=true;app.st.ai.model_menu=true;
            app.handle_ai_window(popup_hwnd,WM_KEYDOWN,WPARAM(VK_ESCAPE.0 as usize),LPARAM(0));assert!(app.ai_window.is_some() && !app.st.ai.model_menu,"Escape keeps the full chat open");app.ai_window=None;app.st.ai.full_open=false;
            app.st.ai.busy=true;app.st.ai.request=8;app.st.ai.running_chat=1;app.expanded=false;app.st.pinned=false;app.fullscreen_on=true;app.st.settings.hide_in_fullscreen=true;app.st.notice=None;assert!(app.fullscreen_hides(app.st.now));app.on_event(AppEvent::Ai(crate::ai::Event::Reply{request:8,chat:1,result:Ok(lumo_core::ai::Message{role:"assistant".into(),text:"Ready".into(),..Default::default()}),images:vec![],unavailable:None}));assert!(!app.fullscreen_hides(app.st.now));assert!(!app.st.cmds.iter().any(|c|matches!(c,Command::Play(_))));app.widgets_open=true;assert!(app.fullscreen_hides(app.st.now));app.widgets_open=false;clock::hold(Some(base+10.0));app.st.now=base+10.0;app.tick();assert!(!app.fullscreen_hides(app.st.now));crate::ai::clear_ready(&mut app.st);assert!(app.fullscreen_hides(app.st.now));app.fullscreen_on=false;app.apply_fullscreen(app.st.now);
            // Do not execute fixture persistence commands against the user's profile.
            app.st.cmds.clear();
        }
        clock::hold(None);app.clock.set_active(false);app.gfx.set_shown(false);unsafe{DestroyWindow(hwnd).unwrap();DestroyWindow(helper).unwrap();}
    }
}
