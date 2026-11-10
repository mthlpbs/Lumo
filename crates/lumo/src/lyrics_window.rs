//! On-demand lyrics player. It shares the island's frame clock and media
//! commands; closing it releases its renderer, artwork and text caches.
use crate::commands::{Command, MediaCmd};
use crate::gfx::{Gfx, Image, ImageData, TextStyle, Weight};
use crate::icons::Icon;
use crate::pages::media::{self, TrackFade};
use crate::state::{AppState, LyricsStatus};
use crate::ui::{wid, wid_n, Hit, Input, MouseButton, Ui};
use lumo_core::datetime::fmt_duration;
use lumo_core::geom::{Color, Point, Rect};
use lumo_core::nowplaying::lyrics_lead_s;
use lumo_core::timeline::{LyricClock, Timeline};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, ScreenToClient, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const OPEN_S: f64 = 0.18;
const CLOSE_S: f64 = 0.16;
const SCROLL_S: f64 = 0.42;
const TRACK_S: f64 = 0.22;
const PAD: f32 = 26.0;
const MAX_LINES: usize = 4096;
const WM_MOUSELEAVE_: u32 = 0x02a3;

pub fn available(st: &AppState) -> bool {
    st.settings.lyrics_popup_button && st.settings.lyrics && st.media.as_ref().is_some_and(|m|
        m.music_source.is_some() && st.lyrics.track_key == m.track_key && st.lyrics.status == LyricsStatus::Found
            && st.lyrics.lyrics.as_ref().is_some_and(|lyrics| !lyrics.is_empty()))
}

fn ease(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

fn fit_scale(monitor: crate::window::MonitorInfo) -> f32 {
    monitor.scale().min((monitor.work.right-monitor.work.left-24) as f32/340.0)
        .min((monitor.work.bottom-monitor.work.top-24) as f32/420.0).max(0.25)
}

#[derive(Clone, Copy)]
struct Layout { art: Rect, meta: Rect, lyrics: Rect, progress: Rect, controls: Point }
impl Layout {
    fn new(area: Rect) -> Self {
        if area.w >= 640.0 {
            let side = (area.w * 0.35).min((area.h - 270.0).max(120.0)).min(300.0);
            let x = PAD + side + 42.0;
            Self { art: Rect::new(PAD, 65.0, side, side), meta: Rect::new(PAD, 85.0 + side, side, 64.0),
                lyrics: Rect::new(x, 62.0, area.w - x - PAD, area.h - 106.0),
                progress: Rect::new(PAD, area.h - 112.0, side, 4.0), controls: Point::new(PAD + side / 2.0, area.h - 48.0) }
        } else {
            let compact = area.h < 560.0;
            let (art_y, art, meta_x, lyric_y, footer) = if compact { (46.0,52.0,68.0,116.0,96.0) } else { (54.0,64.0,82.0,147.0,112.0) };
            Self { art: Rect::new(PAD, art_y, art, art), meta: Rect::new(PAD + meta_x, art_y+3.0, area.w - 2.0 * PAD - meta_x, 64.0),
                lyrics: Rect::new(PAD, lyric_y, area.w - 2.0 * PAD, area.h-footer-22.0-lyric_y),
                progress: Rect::new(PAD, area.h-footer, area.w - 2.0 * PAD, 4.0), controls: Point::new(area.w / 2.0, area.h-if compact {40.0} else {48.0}) }
        }
    }
}

#[derive(Default)]
struct Scene {
    track: TrackFade,
    track_key: String,
    clock: LyricClock,
    art_id: Option<u64>,
    art_scale: f32,
    art_from: f32,
    art_to: f32,
    art_at: f64,
    backdrop: Option<Image>,
    old_backdrop: Option<Image>,
    backdrop_at: f64,
    layout_key: Option<(u64, usize, u32, u32)>,
    rows: Vec<(f32, f32)>,
    active: Option<usize>,
    old_active: Option<usize>,
    active_at: f64,
    scroll: f32,
    scroll_from: f32,
    scroll_at: f64,
    scroll_target: f32,
    manual: bool,
    keyboard_scroll: f32,
}

#[derive(Clone, Copy, Default)]
pub struct Frame { pub moving: bool, pub live: bool, pub closed: bool }

pub struct LyricsWindow {
    pub hwnd: HWND,
    gfx: Gfx,
    input: Input,
    hits: Vec<Hit>,
    scene: Scene,
    opened_at: f64,
    closing_at: Option<f64>,
    closing_from: f32,
    last_draw: f64,
    shown: bool,
    activated: bool,
    focus: Option<u64>,
    dirty: bool,
    last_frame: Frame,
    last_animate: bool,
    last_timeline: Option<Timeline>,
    last_revision: u64,
    last_status: LyricsStatus,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // These also arrive during CreateWindowEx, before App owns the window.
    if msg == WM_NCCALCSIZE { return LRESULT(0); }
    if matches!(msg, WM_NCACTIVATE | WM_ERASEBKGND) { return LRESULT(1); }
    if let Some(Some(result)) = crate::app::with_app(|app| app.handle_lyrics_window(hwnd, msg, wp, lp)) { return result; }
    DefWindowProcW(hwnd, msg, wp, lp)
}

impl LyricsWindow {
    pub fn new(anchor: HWND, st: &AppState) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("Lumo.Lyrics");
            let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc),
                hInstance: instance.into(), hCursor: LoadCursorW(None, IDC_ARROW)?,
                hIcon: LoadIconW(instance, windows::core::PCWSTR(1 as _)).unwrap_or_default(), lpszClassName: class, ..Default::default() };
            if RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS { return Err(windows::core::Error::from_win32()); }
            let monitor = crate::window::monitor_info(MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST))
                .unwrap_or_else(|| crate::window::pick_monitor(st.settings.monitor));
            let scale = fit_scale(monitor);
            let margin = (16.0 * scale).round() as i32;
            let width = (st.settings.lyrics_popup_width * scale).round() as i32;
            let height = (st.settings.lyrics_popup_height * scale).round() as i32;
            let width = width.min(monitor.work.right - monitor.work.left - 2 * margin).max(1);
            let height = height.min(monitor.work.bottom - monitor.work.top - 2 * margin).max(1);
            let x = monitor.work.right - margin - width;
            let y = monitor.work.top + margin;
            let hwnd = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW, class, w!("Lumo Lyrics"),
                WS_POPUP | WS_THICKFRAME | WS_SYSMENU, x, y, width, height, None, None, instance, None)?;
            let gfx = match Gfx::static_window(scale*96.0, hwnd) {
                Ok(gfx) => gfx, Err(error) => { let _ = DestroyWindow(hwnd); return Err(error); }
            };
            Ok(Self { hwnd, gfx, input: Input::default(), hits: Vec::new(), scene: Scene::default(), opened_at: st.now,
                closing_at: None, closing_from: 1.0, last_draw: st.now, shown: false, activated: false, focus: None,
                dirty: true, last_frame: Frame::default(), last_animate: true, last_timeline: None, last_revision: 0, last_status: LyricsStatus::Idle })
        }
    }

    pub fn close(&mut self, now: f64) {
        if self.closing_at.is_none() {
            self.closing_from = ease((now-self.opened_at)/OPEN_S);
            self.closing_at = Some(now);
        }
    }
    pub fn is_closing(&self) -> bool { self.closing_at.is_some() }
    pub fn mark_dirty(&mut self) { self.dirty = true; }
    pub fn keep_on_top(&self) {
        if self.shown { unsafe { let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0,0,0,0, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE); } }
    }
    pub fn focus(&mut self) { self.dirty = true; self.keep_on_top(); unsafe { let _ = SetForegroundWindow(self.hwnd); } }

    /// The compact island can animate at 60 fps. Redraw this larger window
    /// only for its own input/motion, or ~30 fps while its progress bar drifts.
    /// A paused popup keeps its bitmap even when the island is animating.
    pub fn frame(&mut self, st: &mut AppState, animate: bool) -> Result<Frame> {
        let timeline = st.media.as_ref().map(|m| m.timeline);
        let changed = st.media.as_ref().is_some_and(|m| m.track_key != self.scene.track_key || m.art.as_ref().map(|art| art.id) != self.scene.art_id)
            || timeline != self.last_timeline || st.lyrics.request_id != self.last_revision || st.lyrics.status != self.last_status;
        if !self.dirty && !changed && self.shown && self.last_animate == animate && !self.is_closing() && !self.last_frame.moving
            && (!self.last_frame.live || st.now-self.last_draw < 1.0/30.0-0.001) { return Ok(self.last_frame); }
        let frame = self.draw(st, animate)?;
        self.dirty = false; self.last_frame = frame; self.last_animate = animate;
        self.last_timeline = st.media.as_ref().map(|m| m.timeline);
        self.last_revision = st.lyrics.request_id; self.last_status = st.lyrics.status.clone();
        Ok(frame)
    }

    pub fn hide(&mut self) {
        if self.shown { unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); } self.shown = false; }
    }

    pub fn draw(&mut self, st: &mut AppState, animate: bool) -> Result<Frame> {
        let now = st.now;
        self.last_draw = now;
        let closing = self.closing_at.map(|t| (now - t) / CLOSE_S);
        if closing.is_some_and(|t| !animate || t >= 1.0) { return Ok(Frame { moving: false, live: false, closed: true }); }
        let alpha = if !animate { 1.0 } else {
            closing.map_or_else(|| ease((now-self.opened_at)/OPEN_S), |t| self.closing_from*(1.0-ease(t)))
        };
        let mut bounds = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut bounds)?; }
        let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        self.gfx.set_surface(width, height)?;
        let area = Rect::new(0.0, 0.0, width as f32 / self.gfx.scale(), height as f32 / self.gfx.scale());
        self.hits.clear();
        let mut c = self.gfx.begin(RECT { left: 0, top: 0, right: width, bottom: height })?;
        let scene = &mut self.scene;
        let (input, hits) = (&mut self.input, &mut self.hits);
        let (moving, live, repaint) = c.with_alpha(alpha, |c| {
            let mut ui = Ui::new(c, input, hits, now);
            let result = scene.paint(&mut ui, st, area, animate);
            if let Some(hit) = self.focus.and_then(|id| ui.hits.iter().find(|hit| hit.id == id)) {
                ui.c.stroke_rrect(hit.rect.inset(-2.0, -2.0), 8.0, Color::white(0.65), 1.0);
            }
            result
        });
        drop(c);
        self.gfx.end()?;
        self.gfx.present(bounds.left, bounds.top)?;
        self.input.end_frame();
        if !self.shown {
            unsafe { let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE); }
            self.shown = true;
            if !self.activated { unsafe { let _ = SetForegroundWindow(self.hwnd); } self.activated = true; }
        }
        Ok(Frame { moving: repaint || (animate && (moving || now - self.opened_at < OPEN_S || closing.is_some())), live, closed: false })
    }

    pub fn next_wake(&self, st: &AppState) -> Option<f64> {
        let m = st.media.as_ref()?;
        if !m.is_playing() || !m.timeline.has_duration() { return None; }
        let pos = m.timeline.position_at(st.now);
        let mut wait = (1.0 - pos.fract()).max(0.02);
        if let Some(lyrics) = st.lyrics.lyrics.as_ref().filter(|l| l.synced && st.lyrics.track_key == m.track_key) {
            let lead = lyrics_lead_s(&m.app_id) + f64::from(st.settings.lyrics_offset_ms) / 1000.0;
            if let Some(line) = lyrics.lines.iter().find(|line| line.time_s > pos + lead + 0.001) {
                wait = wait.min((line.time_s - pos - lead) / m.timeline.rate.max(0.01));
            }
        }
        Some(st.now + wait.max(0.02))
    }

    pub fn message(&mut self, st: &mut AppState, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        unsafe {
            match msg {
                WM_NCCALCSIZE => return Some(LRESULT(0)),
                WM_NCACTIVATE | WM_ERASEBKGND => return Some(LRESULT(1)),
                WM_RBUTTONUP | WM_CONTEXTMENU => return Some(LRESULT(0)),
                WM_NCHITTEST => {
                    let mut p = POINT { x: lp.0 as i16 as i32, y: (lp.0 >> 16) as i16 as i32 };
                    let _ = ScreenToClient(self.hwnd, &mut p);
                    let mut r = RECT::default(); let _ = GetClientRect(self.hwnd, &mut r);
                    if let Some(hit) = crate::window::mini_resize_hit(p,r,self.gfx.scale()) { return Some(LRESULT(hit as isize)); }
                    let hit = match () {
                        _ if p.y < (44.0 * self.gfx.scale()) as i32 && p.x < r.right - (54.0 * self.gfx.scale()) as i32 => HTCAPTION,
                        _ => HTCLIENT,
                    };
                    return Some(LRESULT(hit as isize));
                }
                WM_GETMINMAXINFO => {
                    let monitor = crate::window::monitor_info(MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST))?;
                    let s = self.gfx.scale();
                    let info = &mut *(lp.0 as *mut MINMAXINFO);
                    info.ptMaxTrackSize = POINT { x: monitor.work.right - monitor.work.left, y: monitor.work.bottom - monitor.work.top };
                    info.ptMinTrackSize = POINT { x: (340.0 * s).round().min(info.ptMaxTrackSize.x as f32) as i32, y: (420.0 * s).round().min(info.ptMaxTrackSize.y as f32) as i32 };
                    return Some(LRESULT(0));
                }
                WM_DPICHANGED => {
                    let r = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(self.hwnd, None, r.left, r.top, r.right-r.left, r.bottom-r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                    let dpi = crate::window::monitor_info(MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST)).map_or((wp.0 & 0xffff) as f32, |monitor| fit_scale(monitor)*96.0);
                    self.gfx.set_dpi(dpi);
                    self.scene.layout_key = None;
                }
                WM_SIZE => self.scene.layout_key = None,
                WM_EXITSIZEMOVE => {
                    let mut r = RECT::default(); let _ = GetWindowRect(self.hwnd, &mut r);
                    st.settings.lyrics_popup_width = (r.right-r.left) as f32 / self.gfx.scale();
                    st.settings.lyrics_popup_height = (r.bottom-r.top) as f32 / self.gfx.scale();
                    st.cmd(Command::SaveSettings);
                }
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    let p = Point::new(lp.0 as i16 as f32 / self.gfx.scale(), (lp.0 >> 16) as i16 as f32 / self.gfx.scale());
                    self.input.mouse = Some(p);
                    if msg == WM_LBUTTONDOWN {
                        self.input.down = true; self.input.just_pressed = true; self.input.press_pos = Some(p);
                        self.input.pressed_id = self.hits.iter().rev().find(|hit| hit.rect.contains(p)).map(|hit| hit.id);
                        SetCapture(self.hwnd);
                    } else if msg == WM_LBUTTONUP {
                        let hit = self.hits.iter().rev().find(|hit| hit.rect.contains(p)).map(|hit| hit.id);
                        if hit == self.input.pressed_id { self.input.clicked = hit.map(|id| (id, MouseButton::Left)); }
                        self.input.released = self.input.pressed_id.map(|id| (id, p));
                        self.input.down = false; self.input.pressed_id = None; let _ = ReleaseCapture();
                    }
                    let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, ..Default::default() };
                    let _ = TrackMouseEvent(&mut track);
                }
                WM_MOUSELEAVE_ | WM_CAPTURECHANGED => { self.input.mouse = None; self.input.down = false; self.input.pressed_id = None; }
                WM_SETCURSOR if lp.0 as u16 as u32 == HTCLIENT => {
                    let hand = self.input.mouse.is_some_and(|p| self.hits.iter().any(|hit| hit.rect.contains(p)));
                    SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).unwrap_or_default());
                    return Some(LRESULT(1));
                }
                WM_MOUSEWHEEL => self.input.wheel += (wp.0 >> 16) as u16 as i16 as f32 / 120.0,
                WM_KEYDOWN => match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => st.cmd(Command::CloseLyricsWindow),
                    VK_SPACE if self.focus.is_none() => media::push_media(st, MediaCmd::PlayPause),
                    VK_LEFT if GetKeyState(VK_CONTROL.0 as i32) < 0 => media::push_media(st, MediaCmd::Prev),
                    VK_RIGHT if GetKeyState(VK_CONTROL.0 as i32) < 0 => media::push_media(st, MediaCmd::Next),
                    VK_UP => self.scene.keyboard_scroll += 1.0, VK_DOWN => self.scene.keyboard_scroll -= 1.0,
                    VK_HOME => self.scene.manual = false,
                    VK_TAB if !self.hits.is_empty() => {
                        let backwards = GetKeyState(VK_SHIFT.0 as i32) < 0;
                        let next = match self.focus.and_then(|id| self.hits.iter().position(|hit| hit.id == id)) {
                            Some(i) if backwards => (i + self.hits.len() - 1) % self.hits.len(), Some(i) => (i + 1) % self.hits.len(),
                            None if backwards => self.hits.len()-1, None => 0,
                        };
                        self.focus = Some(self.hits[next].id);
                    }
                    VK_RETURN | VK_SPACE => self.input.clicked = self.focus.map(|id| (id, MouseButton::Left)),
                    _ => return None,
                },
                _ => return None,
            }
        }
        self.dirty = true;
        Some(LRESULT(0))
    }

    #[cfg(test)]
    pub(crate) fn capture(&mut self, st: &mut AppState) -> (i32, i32, Vec<u8>) {
        let mut r = RECT::default(); unsafe { GetWindowRect(self.hwnd, &mut r).unwrap(); }
        self.gfx.request_capture(RECT { left: 0, top: 0, right: r.right-r.left, bottom: r.bottom-r.top });
        self.draw(st, true).unwrap();
        (r.right-r.left, r.bottom-r.top, self.gfx.take_capture())
    }
}

impl Drop for LyricsWindow { fn drop(&mut self) { unsafe { let _ = DestroyWindow(self.hwnd); } } }

impl Scene {
    fn scroll_at(&self, now: f64) -> f32 {
        self.scroll_from + (self.scroll_target - self.scroll_from) * ease((now-self.scroll_at) / SCROLL_S)
    }
    fn aim(&mut self, target: f32, now: f64, animate: bool) {
        if (target-self.scroll_target).abs() < 0.1 { return; }
        self.scroll_from = if animate { self.scroll_at(now) } else { target };
        self.scroll = self.scroll_from; self.scroll_target = target; self.scroll_at = now;
    }
    fn paint(&mut self, ui: &mut Ui, st: &mut AppState, area: Rect, animate: bool) -> (bool, bool, bool) {
        let now = st.now;
        let m = st.media.as_ref();
        if let Some(m) = m {
            self.track.observe(m, now, !animate);
            if self.track_key != m.track_key {
                self.track_key = m.track_key.clone(); self.layout_key = None; self.rows.clear(); self.clock.reset();
                self.manual = false; self.keyboard_scroll = 0.0; self.active = None; self.old_active = None;
            }
        } else {
            self.track = TrackFade::default(); self.track_key.clear(); self.clock.reset();
            self.rows.clear(); self.layout_key = None; self.active = None; self.old_active = None; self.manual = false; self.keyboard_scroll = 0.0;
        }
        let art = m.and_then(|m| m.art.as_ref());
        if self.art_id != art.map(|art| art.id) {
            self.art_id = art.map(|art| art.id); self.old_backdrop = self.backdrop.take();
            self.backdrop = art.map(backdrop); self.backdrop_at = now;
        }
        let blend = if animate { ease((now-self.backdrop_at)/TRACK_S) } else { 1.0 };
        let surface = area.inset(1.0, 1.0);
        ui.c.fill_rrect(surface, 24.0, Color::hex(0x242432));
        if let Some(image) = self.old_backdrop.as_ref() { ui.c.with_alpha(1.0-blend, |c| c.image(image, surface, 24.0)); }
        if let Some(image) = self.backdrop.as_ref() { ui.c.with_alpha(blend, |c| c.image(image, surface, 24.0)); }
        ui.c.fill_rrect(surface, 24.0, Color::black(0.42));
        if blend >= 1.0 { self.old_backdrop = None; }
        let layout = Layout::new(area);
        let mut acts = Vec::new();
        let mut moving = self.track.moving() || blend < 1.0;
        let mut live = false;
        if let Some(m) = m {
            let size = if m.is_playing() { 1.0 } else { media::ART_PAUSED_SCALE };
            self.art_scale = self.art_from+(self.art_to-self.art_from)*ease((now-self.art_at)/TRACK_S);
            if self.art_to <= 0.0 || !animate { self.art_scale = size; self.art_from = size; self.art_to = size; }
            else if self.art_to != size { self.art_from = self.art_scale; self.art_to = size; self.art_at = now; }
            moving |= (self.art_scale-size).abs() > 0.0001;
            let art_scale = self.art_scale;
            self.track.paint_art(ui, now, layout.art, |ui, art, rect| {
                let rect = Rect::from_center(rect.center(), rect.w*art_scale, rect.h*art_scale);
                ui.c.fill_rrect(rect, 12.0, Color::white(0.08));
                if let Some(art) = art { ui.c.image(art, rect, 12.0); }
                else { ui.c.icon(Icon::Music, rect.inset(rect.w * 0.27, rect.h * 0.27), Color::white(0.55)); }
            });
            self.track.paint_words(ui, now, |ui, title, artist, dx| {
                ui.c.text(title, Rect::new(layout.meta.x+dx, layout.meta.y, layout.meta.w, 27.0), &TextStyle::new(18.0).weight(Weight::Semibold));
                ui.c.text(artist, Rect::new(layout.meta.x+dx, layout.meta.y+29.0, layout.meta.w, 22.0), &TextStyle::new(14.0).color(Color::white(0.65)));
            });
            let tl = m.timeline;
            let playing = m.is_playing();
            let position = tl.position_at(now);
            let lead = lyrics_lead_s(&m.app_id) + f64::from(st.settings.lyrics_offset_ms)/1000.0;
            let lyric_pos = self.clock.tick_with_seek(now, position+lead, playing && tl.has_duration(), tl.rate, st.media_pending_seek);
            let lyrics = st.lyrics.lyrics.as_ref().filter(|lyrics| st.lyrics.status == LyricsStatus::Found && st.lyrics.track_key == m.track_key && !lyrics.is_empty());
            if let Some(lyrics) = lyrics {
                // Fit context as well as the current cue in short windows.
                // Three wrapped cues need more than three single text rows.
                let font = (layout.lyrics.w/11.8).clamp(22.0, 38.0)
                    .min(((layout.lyrics.h-24.0)/9.0).clamp(18.0,38.0));
                let style = TextStyle::new(font).weight(Weight::Bold).wrap().top();
                let key = (st.lyrics.request_id, lyrics.lines.as_ptr() as usize, layout.lyrics.w.to_bits(), font.to_bits());
                let fresh = self.layout_key != Some(key);
                if fresh {
                    self.layout_key = Some(key); self.rows.clear();
                    let mut y = 0.0;
                    for line in lyrics.lines.iter().take(MAX_LINES) {
                        let h = if line.text.trim().is_empty() { font } else { ui.c.measure(&line.text, &style, layout.lyrics.w).1.max(font*1.3) };
                        self.rows.push((y, h)); y += h + (font*0.55).clamp(10.0,21.0);
                    }
                }
                let active = lyrics.line_index_at(lyric_pos).filter(|&i| i < self.rows.len());
                if active != self.active { self.old_active = self.active; self.active = active; self.active_at = now; }
                let wheel = if ui.hovered(layout.lyrics) { ui.input.wheel } else { 0.0 } + std::mem::take(&mut self.keyboard_scroll);
                if wheel != 0.0 {
                    self.manual = true;
                    let total = self.rows.last().map_or(0.0, |(y,h)| y+h);
                    let target = (self.scroll_target-wheel*52.0).clamp(-layout.lyrics.h*0.38, (total-layout.lyrics.h*0.45).max(0.0));
                    self.aim(target, now, animate);
                }
                if lyrics.synced && !self.manual {
                    let (y,height) = active.and_then(|i| self.rows.get(i)).copied().unwrap_or((0.0,font*1.3));
                    self.aim(y-(layout.lyrics.h-height).max(0.0)*0.5, now, animate && !fresh);
                } else if fresh { self.aim(-(layout.lyrics.h*0.16).clamp(18.0,54.0), now, false); }
                self.scroll = self.scroll_at(now);
                if !animate { self.scroll = self.scroll_target; self.scroll_from = self.scroll_target; }
                let highlight = if animate { ease((now-self.active_at)/0.18) } else { 1.0 };
                moving |= (self.scroll-self.scroll_target).abs() > 0.1 || highlight < 1.0;
                ui.c.clip(layout.lyrics);
                let feather = (layout.lyrics.h*0.16).clamp(18.0,54.0);
                ui.c.push_feather_mask(layout.lyrics,feather);
                let snap = ui.c.set_text_snap(false);
                for (i, &(y, h)) in self.rows.iter().enumerate() {
                    let r = Rect::new(layout.lyrics.x, layout.lyrics.y+y-self.scroll, layout.lyrics.w, h);
                    if r.bottom() < layout.lyrics.y || r.y > layout.lyrics.bottom() { continue; }
                    let line = &lyrics.lines[i];
                    let base = if !lyrics.synced { 0.85 } else { 0.45 };
                    let alpha = if active == Some(i) { base+(1.0-base)*highlight }
                        else if self.old_active == Some(i) { base+(1.0-base)*(1.0-highlight) } else { base };
                    let hit = Rect::new(r.x, r.y.max(layout.lyrics.y), r.w, r.bottom().min(layout.lyrics.bottom())-r.y.max(layout.lyrics.y));
                    let response = if lyrics.synced && m.can_seek { Some(ui.button(wid_n("lyrics.line", i as u64), hit)) } else { None };
                    if response.is_some_and(|response| response.clicked) { acts.push(MediaCmd::Seek((line.time_s-lead).max(0.0))); self.manual = false; }
                    let distance = (r.center().y-layout.lyrics.y).min(layout.lyrics.bottom()-r.center().y);
                    let blur = if active == Some(i) { 0.0 } else { ((feather-distance)/feather).clamp(0.0,1.0)*3.5 };
                    ui.c.soft_text(&line.text,r,&style.color(Color::white(if response.is_some_and(|r| r.hovered) { alpha.max(0.8) } else { alpha })),blur);
                }
                ui.c.set_text_snap(snap);
                ui.c.pop_shape_mask();
                ui.c.unclip();
                if self.manual && lyrics.synced {
                    let r = Rect::new(layout.lyrics.right()-116.0, layout.lyrics.y-28.0, 116.0, 25.0);
                    if text_button(ui, "lyrics.follow", r, "Follow song") { self.manual = false; }
                }
            } else {
                self.rows.clear(); self.layout_key = None;
                let message = match &st.lyrics.status {
                    LyricsStatus::Loading => "Finding lyrics…", LyricsStatus::Instrumental => "Instrumental",
                    _ => "Lyrics is not found",
                };
                ui.c.text(message, layout.lyrics, &TextStyle::new(24.0).weight(Weight::Semibold).wrap().color(Color::white(0.6)));
                if let LyricsStatus::Error(error)=&st.lyrics.status {if ui.hovered(layout.lyrics){crate::pages::ai::tooltip(ui,error,layout.lyrics);}}
            }
            if tl.has_duration() {
                let scrub = ui.progress_bar(wid("lyrics.seek"), layout.progress, tl.progress_at(now), Color::white(0.85), m.can_seek);
                if scrub.began { acts.push(MediaCmd::Prepare); }
                if let Some(fraction) = scrub.released { acts.push(MediaCmd::Seek(f64::from(fraction)*tl.duration_s)); }
                let pos = scrub.held.or(scrub.released).map_or(position, |fraction| f64::from(fraction)*tl.duration_s);
                let style = TextStyle::footnote().tabular().color(Color::white(0.6));
                ui.c.text(&fmt_duration(pos), Rect::new(layout.progress.x, layout.progress.y+10.0, 80.0, 20.0), &style);
                ui.c.text(&format!("−{}", fmt_duration((tl.duration_s-pos).max(0.0))), Rect::new(layout.progress.right()-80.0, layout.progress.y+10.0, 80.0, 20.0), &style.right());
            }
            for (id, dx, icon, enabled, command) in [
                ("lyrics.prev", -66.0, Icon::Backward, m.can_prev, MediaCmd::Prev),
                ("lyrics.play", 0.0, if playing { Icon::Pause } else { Icon::Play }, m.can_play_pause, MediaCmd::PlayPause),
                ("lyrics.next", 66.0, Icon::Forward, m.can_next, MediaCmd::Next),
            ] {
                let rect = Rect::from_center(Point::new(layout.controls.x+dx, layout.controls.y), 42.0, 42.0);
                if transport(ui, id, rect, icon, enabled) { acts.push(command); }
            }
            live = playing && tl.has_duration();
        } else {
            ui.c.text("Nothing playing", layout.lyrics, &TextStyle::new(28.0).weight(Weight::Semibold).color(Color::white(0.7)));
        }
        let close = Rect::new(area.w-44.0, 10.0, 34.0, 30.0);
        // Always show the return action; a hover must not be needed to find it.
        let response = ui.icon_button(wid("lyrics.close"), close, Icon::ExitMiniPlayer, Color::white(0.85), 20.0);
        if response.clicked { st.cmd(Command::CloseLyricsWindow); }
        if response.hovered {
            let hint = Rect::new(close.right()-138.0, close.bottom()+6.0, 138.0, 26.0);
            ui.c.fill_rrect(hint, 7.0, Color::hex(0x303030));
            ui.c.text("Exit mini player", hint, &TextStyle::footnote().center());
        }
        // An understated grip, in addition to resizing from all window edges.
        for n in 0..3 { let d = 4.0+n as f32*4.0; ui.c.line(Point::new(area.w-9.0-d, area.h-9.0), Point::new(area.w-9.0, area.h-9.0-d), Color::white(0.3), 1.0); }
        ui.c.stroke_rrect(area.inset(1.0, 1.0), 24.0, Color::white(0.14), 1.0);
        for action in acts { media::push_media(st, action); }
        (moving, live, ui.repaint_requested())
    }
}

fn transport(ui: &mut Ui, id: &str, rect: Rect, icon: Icon, enabled: bool) -> bool {
    let response = if enabled { Some(ui.button(wid(id), rect)) } else { None };
    if response.is_some_and(|r| r.hovered || r.pressed) { ui.c.fill_circle(rect.center(), rect.w/2.0, Color::white(0.1)); }
    ui.c.icon(icon, rect.inset(8.0, 8.0), Color::white(if enabled { 0.95 } else { 0.25 }));
    response.is_some_and(|r| r.clicked)
}
fn text_button(ui: &mut Ui, id: &str, rect: Rect, label: &str) -> bool {
    let response = ui.button(wid(id), rect);
    ui.c.fill_rrect(rect, rect.h/2.0, Color::white(if response.hovered { 0.2 } else { 0.13 }));
    ui.c.text(label, rect, &TextStyle::footnote().center()); response.clicked
}

/// A tiny, cached blurred colour field from the actual cover. At most 4096
/// source samples and 64 output pixels, only when artwork changes. No filter
/// library, full-size blurred bitmap or continuously moving background.
fn backdrop(art: &Image) -> Image {
    const N: usize = 8;
    let mut pixels = vec![0u8; N*N*4];
    for y in 0..N { for x in 0..N {
        let mut sum = [0u32; 3]; let mut count = 0;
        for sy in 0..8 { for sx in 0..8 {
            let px = ((x*8+sx)*art.width as usize/(N*8)).min(art.width.saturating_sub(1) as usize);
            let py = ((y*8+sy)*art.height as usize/(N*8)).min(art.height.saturating_sub(1) as usize);
            let index = (py*art.width as usize+px)*4;
            if let Some(pixel) = art.pixels.get(index..index+4) { for c in 0..3 { sum[c] += pixel[c] as u32; } count += 1; }
        }}
        let index=(y*N+x)*4;
        for c in 0..3 { pixels[index+c]=(sum[c]/count.max(1)).min(255) as u8; }
        pixels[index+3]=255;
    }}
    for _ in 0..3 {
        let old=pixels.clone();
        for y in 0..N { for x in 0..N { for c in 0..3 {
            let mut sum=0u32; let mut count=0;
            for yy in y.saturating_sub(1)..=(y+1).min(N-1) { for xx in x.saturating_sub(1)..=(x+1).min(N-1) { sum+=old[(yy*N+xx)*4+c] as u32; count+=1; }}
            pixels[(y*N+x)*4+c]=(sum/count) as u8;
        }}}
    }
    ImageData::new(N as u32, N as u32, pixels)
}

#[cfg(test)]
pub(crate) fn sample_state() -> AppState {
    let mut st = AppState::new(Default::default(), Default::default());
    st.now = 10.0;
    let mut pixels = Vec::new();
    for y in 0..128 { for x in 0..128 {
        let warm = ((x as f32/127.0)*0.65+(1.0-y as f32/127.0)*0.35).clamp(0.0, 1.0);
        pixels.extend_from_slice(&[(95.0-40.0*warm) as u8, (125.0-30.0*warm) as u8, (65.0+170.0*warm) as u8, 255]);
    }}
    st.media = Some(crate::state::MediaSession {
        app_id: "Spotify.exe".into(), app_name: "Spotify".into(), music_source: Some(lumo_core::nowplaying::MusicSource::Streaming),
        title: "After the rain".into(), artist: "Lumo Sessions".into(), album: "Evening light".into(),
        art: Some(ImageData::new(128, 128, pixels)), art_color: None,
        timeline: lumo_core::timeline::Timeline::new(19.0, 210.0, 10.0, false),
        can_prev: true, can_next: true, can_play_pause: true, can_seek: true, track_key: "lyrics-window-fixture".into(),
    });
    st.lyrics = crate::state::LyricsState { track_key: "lyrics-window-fixture".into(), request_id: 1,
        status: LyricsStatus::Found, source: "LRCLIB".into(),
        lyrics: Some(lumo_core::lrc::parse_lrc("[00:00]Let the city settle down\n[00:08]Watch the lights turn into gold\n[00:16]Every sound is a little closer\n[00:24]And the night is ours to hold\n[00:32]We can take the long way home\n[00:40]With the music playing low\n[00:48]After the rain, we start again")), ..Default::default() };
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]#[ignore="requires a Windows desktop; offline mini-player corner resize"]
    fn mini_corners_resize_diagonally_without_changing_playback() {
        let mut st=sample_state();let mut w=LyricsWindow::new(unsafe{GetDesktopWindow()},&st).unwrap();
        for scale in [1.0,1.25,1.5,2.0] {w.gfx.set_dpi(scale*96.0);let hwnd=w.hwnd;crate::window::check_mini_resize_corners(hwnd,scale,|lp|w.message(&mut st,WM_NCHITTEST,WPARAM(0),lp));}
        for (width,height) in [(420,460),(680,740)] {unsafe{SetWindowPos(w.hwnd,None,0,0,width,height,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE).unwrap();}w.message(&mut st,WM_SIZE,WPARAM(0),LPARAM(0));}
        assert!(st.cmds.is_empty());
    }

    #[test]
    fn popup_eligibility_tracks_music_lyrics_and_the_saved_switch() {
        let mut st = sample_state();
        assert!(available(&st));
        st.lyrics.track_key = "previous song".into(); assert!(!available(&st));
        st.lyrics.track_key = "lyrics-window-fixture".into();
        st.media.as_mut().unwrap().music_source = None; assert!(!available(&st));
        st.media.as_mut().unwrap().music_source = Some(lumo_core::nowplaying::MusicSource::Local);
        st.settings.lyrics_popup_button = false; assert!(!available(&st));
        st.settings.lyrics_popup_button = true; st.lyrics.status = LyricsStatus::Loading; assert!(!available(&st));
    }

    #[test]
    fn colour_field_is_small_and_keeps_a_uniform_covers_colour() {
        let image = ImageData::new(64, 64, [80, 120, 200, 255].repeat(64*64));
        let blurred = backdrop(&image);
        assert_eq!((blurred.width, blurred.height, blurred.pixels.len()), (8, 8, 256));
        assert!(blurred.pixels.chunks_exact(4).all(|pixel| pixel == [80, 120, 200, 255]));
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn lyrics_player_renders_resizes_controls_scrolls_and_follows_the_current_track() {
        // Closing during the opening fade must not brighten the window first.
        {
            let mut state = sample_state();
            let mut quick = LyricsWindow::new(unsafe { GetDesktopWindow() }, &state).unwrap();
            state.now += 0.05;
            let (_,_,before) = quick.capture(&mut state);
            quick.close(state.now);
            state.now += 0.03;
            let (_,_,after) = quick.capture(&mut state);
            let alpha = |pixels: &[u8]| pixels.chunks_exact(4).map(|pixel| pixel[3]).max().unwrap_or(0);
            assert!(alpha(&after) <= alpha(&before));
        }
        let mut st = sample_state();
        let mut window = LyricsWindow::new(unsafe { GetDesktopWindow() }, &st).unwrap();
        // Verify the actual native opacity layer: a hard clip would leave
        // all these interior rows equally opaque.
        {
            let g=&mut window.gfx; let dpi=g.dpi;g.set_dpi(96.0);let r=RECT {left:0,top:0,right:100,bottom:100};
            g.set_surface(100,100).unwrap();g.request_capture(r);
            let mut c=g.begin(r).unwrap();
            c.push_feather_mask(Rect::new(0.0,0.0,80.0,80.0),16.0);
            c.fill_rect(Rect::new(0.0,0.0,80.0,80.0),Color::white(1.0));c.pop_shape_mask();
            drop(c);g.end().unwrap();let pixels=g.take_capture();
            let alpha=|y:usize|pixels[(y*100+40)*4+3];
            let at=|y:f32|alpha(y.round() as usize);
            assert!(at(1.0)<at(5.0) && at(5.0)<at(12.0) && at(12.0)<at(30.0));
            assert!(at(30.0)>245 && at(78.0)<40);
            g.set_dpi(dpi);
        }
        let output = std::env::var_os("LUMO_LYRICS_QA").map(std::path::PathBuf::from);
        for (i, (width, height)) in [(430.0,640.0),(760.0,540.0),(340.0,420.0)].into_iter().enumerate() {
            let scale = window.gfx.scale();
            unsafe { SetWindowPos(window.hwnd, HWND_TOPMOST, 20, 20, (width*scale) as i32, (height*scale) as i32, SWP_NOACTIVATE).unwrap(); }
            window.scene.layout_key = None;
            st.now = 11.0+i as f64;
            window.draw(&mut st, true).unwrap();
            st.now += 0.5;
            let (w,h,pixels) = window.capture(&mut st);
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 250));
            let cues=(0..7).filter(|&i|window.hits.iter().any(|hit|hit.id==wid_n("lyrics.line",i))).count();
            assert!(cues>=3,"{width}x{height}: current and neighbouring cues must fit, got {cues}");
            for hit in &window.hits {
                assert!(hit.rect.x >= 0.0 && hit.rect.y >= 0.0 && hit.rect.right() <= width+1.0 && hit.rect.bottom() <= height+1.0, "{width}x{height}: {hit:?}");
            }
            if let Some(dir) = output.as_ref() {
                std::fs::create_dir_all(dir).unwrap();
                crate::png::save_rgba(dir.join(format!("lyrics-player-{width:.0}x{height:.0}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
        }
        assert_eq!(window.scene.active, Some(2));
        let was = window.scene.scroll_target;
        window.message(&mut st, WM_KEYDOWN, WPARAM(VK_DOWN.0 as usize), LPARAM(0));
        window.draw(&mut st, true).unwrap();
        assert!(window.scene.manual && window.scene.scroll_target > was);
        window.message(&mut st, WM_KEYDOWN, WPARAM(VK_HOME.0 as usize), LPARAM(0));
        st.now += 0.5; window.draw(&mut st, true).unwrap(); assert!(!window.scene.manual);
        st.now += 0.5; window.draw(&mut st, true).unwrap();
        for (id, command) in [("lyrics.next", MediaCmd::Next), ("lyrics.prev", MediaCmd::Prev), ("lyrics.play", MediaCmd::PlayPause)] {
            st.cmds.clear();
            window.input.clicked = Some((wid(id), MouseButton::Left));
            window.draw(&mut st, true).unwrap();
            assert!(st.cmds.contains(&Command::Media(command)));
        }
        st.cmds.clear();
        let line_id = window.hits.iter().find(|hit| hit.id == wid_n("lyrics.line", 1)).unwrap().id;
        window.input.clicked = Some((line_id, MouseButton::Left));
        window.draw(&mut st, true).unwrap();
        assert_eq!(st.media.as_ref().unwrap().timeline.position_s, 8.0);
        assert!(st.media_pending_seek.is_some());
        st.now += 0.5; window.draw(&mut st, true).unwrap();
        assert_eq!(window.scene.active, Some(1));
        window.message(&mut st, WM_EXITSIZEMOVE, WPARAM(0), LPARAM(0));
        assert!((st.settings.lyrics_popup_width-340.0).abs() < 1.0);
        assert!((st.settings.lyrics_popup_height-420.0).abs() < 1.0);
        st.media.as_mut().unwrap().track_key = "next song".into();
        window.draw(&mut st, true).unwrap();
        assert!(window.scene.rows.is_empty(), "old lyrics must never remain under a new track");
        st.lyrics.track_key = "next song".into(); st.lyrics.request_id += 1;
        st.lyrics.lyrics = Some(lumo_core::lrc::Lyrics::plain("A line without timing\nAnother line to scroll"));
        window.draw(&mut st, true).unwrap();
        assert!(!window.hits.iter().any(|hit| hit.id == wid_n("lyrics.line", 0)), "unsynced lyrics cannot seek");
        window.input.mouse = Some(Point::new(313.0,25.0));
        for _ in 0..3 { st.now += 0.05; window.draw(&mut st, true).unwrap(); }
        window.input.clicked = Some((wid("lyrics.close"), MouseButton::Left));
        window.draw(&mut st, true).unwrap();
        assert!(st.cmds.contains(&Command::CloseLyricsWindow));
        st.media = None; window.draw(&mut st, true).unwrap();
        st.now += 0.5;
        let idle = window.draw(&mut st, true).unwrap();
        assert!(!idle.moving && !idle.live, "a disconnected player must not leave an animation running");
        window.frame(&mut st,true).unwrap();let last_draw=window.last_draw;
        st.now+=0.1;window.frame(&mut st,true).unwrap();
        assert_eq!(window.last_draw,last_draw,"another window's frames must not repaint a resting popup");
        window.close(st.now);
        st.now += CLOSE_S+0.01;
        assert!(window.draw(&mut st, true).unwrap().closed);
    }
}
