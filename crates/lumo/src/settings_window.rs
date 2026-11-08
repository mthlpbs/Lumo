//! On-demand Settings window. All painting follows Windows/input messages;
//! one delayed tooltip wake is armed only while a model is hovered.
//! No idle worker, repeating timer or network request is owned by this window.
use crate::commands::Command;
use crate::gfx::{Gfx, TextStyle, Weight};
use crate::icons::Icon;
use crate::pages::settings::{draw_switch, picker};
use crate::pill::status;
use crate::state::AppState;
use crate::ui::{wid, wid_n, Hit, Input, MouseButton, PillStyle, Ui};
use lumo_core::geom::{palette, Color, Point, Rect};
use lumo_core::settings::{IdleStyle, MonitorChoice, Theme};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 650.0;
const SIDE: f32 = 226.0;
const BG: Color = Color::rgba(0.115, 0.115, 0.12, 1.0);
const SIDEBAR: Color = Color::rgba(0.075, 0.075, 0.08, 1.0);
const CARD: Color = Color::rgba(0.155, 0.155, 0.165, 1.0);
const GREEN: Color = Color::rgba(0.16, 0.77, 0.36, 1.0);
const WM_MOUSELEAVE_: u32 = 0x02a3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    General,
    Display,
    Appearance,
    Music,
    Ai,
    Focus,
    Notifications,
    Search,
    About,
}

impl Section {
    pub(crate) const ALL: [Self; 9] = [Self::General, Self::Display, Self::Appearance, Self::Music, Self::Ai, Self::Focus, Self::Notifications, Self::Search, Self::About];
    pub(crate) fn key(self)->&'static str {match self{Self::General=>"general",Self::Display=>"display",Self::Appearance=>"appearance",Self::Music=>"music",Self::Ai=>"ai",Self::Focus=>"focus",Self::Notifications=>"notifications",Self::Search=>"search",Self::About=>"about"}}
    pub(crate) fn visible(self, ai_on: bool) -> bool { self != Self::Ai || ai_on }
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Ai => "AI companion",
            Self::Search => "Search",
            Self::General => "General", Self::Display => "Display", Self::Appearance => "Appearance",
            Self::Music => "Music & lyrics", Self::Focus => "Focus", Self::Notifications => "Notifications", Self::About => "About",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Ai => "Your provider, models and local chat history.",
            Self::Search => "Find apps, Lumo actions and settings in one place.",
            Self::General => "Make Lumo fit your everyday routine.",
            Self::Display => "Choose where the island appears and when it stays visible.",
            Self::Appearance => "Give your island a look that feels like you.",
            Self::Music => "Choose how music and lyrics appear on your island.",
            Self::Focus => "Set a comfortable rhythm for focused work and breaks.",
            Self::Notifications => "Choose which updates appear on your island.",
            Self::About => "Lumo, a little space for your day.",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::Ai => Icon::Sparkles,
            Self::Search => Icon::Search,
            Self::General => Icon::Gear, Self::Display => Icon::App, Self::Appearance => Icon::Moon,
            Self::Music => Icon::Music, Self::Focus => Icon::Timer, Self::Notifications => Icon::Volume, Self::About => Icon::Circle,
        }
    }
    fn color(self) -> Color {
        match self {
            Self::Ai => Color::rgba(0.3,0.55,0.9,1.0),
            Self::Search => Color::rgba(0.42,0.62,0.92,1.0),
            Self::General | Self::About => Color::rgba(0.52, 0.53, 0.56, 1.0),
            Self::Display => Color::rgba(0.39, 0.35, 0.9, 1.0), Self::Appearance => Color::rgba(0.62, 0.38, 0.82, 1.0),
            Self::Music => Color::rgba(0.94, 0.22, 0.4, 1.0), Self::Focus => Color::rgba(0.96, 0.49, 0.18, 1.0),
            Self::Notifications => Color::rgba(0.16, 0.65, 0.82, 1.0),
        }
    }
}

pub struct SettingsWindow {
    pub hwnd: HWND,
    gfx: Gfx,
    input: Input,
    hits: Vec<Hit>,
    section: Section,
    ai_on: bool,
    keyboard_focus: Option<u64>,
    raised_for_open: bool,
    horizontal_pager: lumo_core::navigation::WheelPager,
    scroll: f32,
    max_scroll: f32,
    scroll_section: Section,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(Some(r)) = crate::app::with_app(|app| app.handle_settings(hwnd, msg, wp, lp)) {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

impl SettingsWindow {
    pub fn new(anchor: HWND) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("Lumo.Settings");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc), hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hIcon: LoadIconW(instance, windows::core::PCWSTR(1 as _)).unwrap_or_default(),
                lpszClassName: class, ..Default::default()
            };
            // The class stays registered after closing, so reopening is cheap.
            if RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(windows::core::Error::from_win32());
            }
            let monitor = crate::window::pick_monitor(MonitorChoice::Cursor);
            // Keep every category reachable on small screens and high scaling.
            let scale = (monitor.dpi as f32 / 96.0)
                .min((monitor.work.right - monitor.work.left - 24) as f32 / WIDTH)
                .min((monitor.work.bottom - monitor.work.top - 24) as f32 / HEIGHT);
            let width = (WIDTH * scale).round() as i32;
            let height = (HEIGHT * scale).round() as i32;
            let mut rect = RECT::default();
            let _ = GetWindowRect(anchor, &mut rect);
            let x = (rect.left + (rect.right - rect.left - width) / 2)
                .clamp(monitor.work.left, (monitor.work.right - width).max(monitor.work.left));
            let y = monitor.work.top + ((monitor.work.bottom - monitor.work.top - height) / 2).max(0);
            let hwnd = CreateWindowExW(WS_EX_LAYERED | WS_EX_APPWINDOW, class, w!("Lumo Settings"),
                WS_POPUP | WS_SYSMENU, x, y, width, height, None, None, instance, None)?;
            let gfx = match Gfx::static_window(scale * 96.0, hwnd) {
                Ok(gfx) => gfx,
                Err(e) => { let _ = DestroyWindow(hwnd); return Err(e); }
            };
            Ok(Self { hwnd, gfx, input: Input::default(), hits: Vec::new(), section: Section::General, ai_on: false, keyboard_focus: None, raised_for_open: false, horizontal_pager: Default::default(), scroll: 0.0, max_scroll: 0.0, scroll_section: Section::General })
        }
    }

    pub fn show_ai(&mut self) {self.section=Section::Ai;self.show();}
    pub fn show_section(&mut self,key:&str){self.section=Section::ALL.into_iter().find(|s|s.key()==key).unwrap_or(Section::General);self.scroll=0.0;self.show();}
    pub fn show_about(&mut self) { self.section = Section::About; self.scroll = 0.0; self.show(); }

    pub fn show(&mut self) {
        // The island is topmost. Give Settings priority for this explicit open,
        // then release it on deactivation instead of permanently pinning it.
        self.raise_for_open();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
        if unsafe { GetForegroundWindow() } != self.hwnd {
            self.release_open_priority();
        }
        self.invalidate();
    }

    fn raise_for_open(&mut self) {
        self.raised_for_open = unsafe {
            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER)
        }.is_ok();
    }

    fn release_open_priority(&mut self) {
        if !self.raised_for_open { return; }
        self.raised_for_open = false;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_NOTOPMOST, 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }

    pub fn has_open_priority(&self) -> bool {
        self.raised_for_open && unsafe { GetForegroundWindow() } == self.hwnd
    }

    pub fn invalidate(&self) {
        unsafe {
            if IsWindowVisible(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() {
                let _ = InvalidateRect(self.hwnd, None, false);
            }
        }
    }

    /// Capture the same native window used by the tray and island gear.
    pub fn selftest_capture(&mut self, st: &mut AppState, section: Section) -> Result<(i32, i32, Vec<u8>, (i32, i32), f32)> {
        self.section = section;
        let mut bounds = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut bounds)?; }
        let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        self.gfx.request_capture(RECT { left: 0, top: 0, right: w, bottom: h });
        self.draw(st)?;
        Ok((w, h, self.gfx.take_capture(), (bounds.left, bounds.top), self.gfx.scale()))
    }

    pub fn dismiss_model_picker(&mut self){self.input.focused=None;self.keyboard_focus=None;self.invalidate();}
    pub fn draw(&mut self, st: &mut AppState) -> Result<()> {
        self.ai_on = st.ai_on();
        if !self.section.visible(self.ai_on) {
            self.section = Section::General;
            self.input = Input::default();
            self.keyboard_focus = None;
        }
        let mut bounds = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut bounds)?; }
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        self.gfx.set_surface(width, height)?;
        let scale = self.gfx.scale();
        let area = Rect::new(0.0, 0.0, width as f32 / scale, height as f32 / scale);
        self.hits.clear();
        let mut c = self.gfx.begin(RECT { left: 0, top: 0, right: width, bottom: height })?;
        let mut ui = Ui::new(&mut c, &mut self.input, &mut self.hits, st.now);
        ui.c.fill_rrect(area.inset(1.0, 1.0), 16.0, BG);
        ui.c.clip(Rect::new(1.0, 1.0, SIDE, area.h - 2.0));
        ui.c.fill_rrect(area.inset(1.0, 1.0), 16.0, SIDEBAR);
        ui.c.unclip();
        ui.c.stroke_rrect(area.inset(1.0, 1.0), 16.0, Color::white(0.10), 1.0);
        ui.c.text("Lumo", Rect::new(24.0, 16.0, SIDE - 40.0, 24.0), &TextStyle::headline());
        ui.c.text("Settings", Rect::new(24.0, 40.0, SIDE - 40.0, 18.0), &TextStyle::footnote().color(palette::TEXT_DIM));
        for (row_index, (i, section)) in Section::ALL.iter().copied().enumerate().filter(|(_, s)| s.visible(self.ai_on)).enumerate() {
            let y = if section == Section::About { area.h - 62.0 } else { 82.0 + row_index as f32 * 46.0 };
            let row = Rect::new(12.0, y, SIDE - 24.0, 40.0);
            let response = ui.button(wid_n("preferences.section", i as u64), row);
            if response.clicked { self.section = section; st.update.sheet = false; }
            if self.section == section || response.hovered {
                ui.c.fill_rrect(row, 10.0, Color::white(if self.section == section { 0.09 } else { 0.04 }));
            }
            let icon = Rect::new(row.x + 10.0, row.y + 7.0, 26.0, 26.0);
            ui.c.fill_rrect(icon, 7.0, section.color());
            ui.c.icon(section.icon(), icon.inset(5.0, 5.0), palette::TEXT);
            ui.c.text(section.title(), Rect::new(row.x + 48.0, row.y, row.w - 54.0, row.h),
                &TextStyle::body().color(if self.section == section { palette::TEXT } else { palette::TEXT_DIM }));
        }
        let close = Rect::new(area.w - 48.0, 9.0, 38.0, 32.0);
        if ui.icon_button(wid("preferences.close"), close, Icon::Close, palette::TEXT_DIM, 15.0).clicked {
            unsafe { let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)); }
        }
        let content = Rect::new(SIDE + 28.0, 66.0, area.w - SIDE - 56.0, area.h - 90.0);
        ui.c.text(self.section.title(), Rect::new(content.x, content.y, content.w, 32.0), &TextStyle::new(25.0).weight(Weight::Semibold));
        ui.c.text(self.section.description(), Rect::new(content.x, content.y + 38.0, content.w, 22.0), &TextStyle::footnote().color(palette::TEXT_DIM));
        if self.scroll_section!=self.section {self.scroll=0.0;self.max_scroll=0.0;self.scroll_section=self.section;self.keyboard_focus=None;}
        let body=Rect::new(content.x,content.y+82.0,content.w,content.h-82.0);
        let fixed_hits=ui.hits.len();
        ui.c.clip(body);
        let bottom=draw_section(&mut ui,st,self.section,body.offset(0.0,-self.scroll));
        ui.c.unclip();
        self.max_scroll=(bottom+self.scroll-body.bottom()).max(0.0);
        self.scroll=self.scroll.clamp(0.0,self.max_scroll);
        let mut index=0;
        ui.hits.retain_mut(|hit| {
            index+=1;if index<=fixed_hits {return true;}
            let end=hit.rect.bottom().min(body.bottom());
            hit.rect.y=hit.rect.y.max(body.y);hit.rect.h=(end-hit.rect.y).max(0.0);hit.rect.h>1.0
        });
        if self.max_scroll>0.0 && !(self.section==Section::Ai && st.ai.model_menu && st.ai.model_settings) {
            let h=(body.h*body.h/(body.h+self.max_scroll)).max(28.0);
            let response=ui.button(wid("preferences.scrollbar"),Rect::new(body.right()+4.0,body.y,13.0,body.h));
            if response.pressed {
                if let Some(mouse)=ui.input.mouse {
                    self.scroll=((mouse.y-body.y-h/2.0)/(body.h-h)*self.max_scroll).clamp(0.0,self.max_scroll);
                    self.keyboard_focus=None;
                }
            }
            let y=body.y+(body.h-h)*self.scroll/self.max_scroll;
            ui.c.fill_rrect(Rect::new(body.right()+9.0,y,3.0,h),1.5,Color::white(0.28));
        }
        if self.section==Section::Ai && st.ai.model_menu && st.ai.model_settings {crate::pages::ai::model_picker(&mut ui,st,body,true);}
        if let Some(hit) = self.keyboard_focus.and_then(|id| ui.hits.iter().find(|hit| hit.id == id)) {
            ui.c.stroke_rrect(hit.rect.inset(2.0, 2.0), 7.0, Color::white(0.7), 1.0);
        }
        unsafe {let _=KillTimer(self.hwnd,71);if self.section==Section::Ai {if let Some(t)=crate::pages::ai::next_wake(st){SetTimer(self.hwnd,71,((t-st.now)*1000.0).ceil().max(1.0) as u32,None);}}}
        let repaint=ui.repaint_requested();
        self.input.end_frame();
        self.gfx.end()?;
        self.gfx.present(bounds.left, bounds.top)?;
        if repaint{self.invalidate();}
        Ok(())
    }

    pub fn message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        unsafe {
            match msg {
                WM_TIMER if wp.0==71 => {let _=KillTimer(self.hwnd,71);}
                WM_TIMER if wp.0==71 => {let _=KillTimer(self.hwnd,71);}
                WM_ACTIVATE => {
                    if (wp.0 & 0xffff) as u32 == WA_INACTIVE { self.release_open_priority(); }
                    // Keep the default activation/focus handling.
                    return None;
                }
                WM_ERASEBKGND => return Some(LRESULT(1)),
                WM_NCHITTEST => {
                    let mut p = POINT { x: lp.0 as i16 as i32, y: (lp.0 >> 16) as i16 as i32 };
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut p);
                    let s = self.gfx.scale();
                    let mut r = RECT::default();
                    let _ = GetClientRect(self.hwnd, &mut r);
                    if (p.y as f32) < 60.0 * s && (p.x as f32) < r.right as f32 - 56.0 * s {
                        return Some(LRESULT(HTCAPTION as isize));
                    }
                    return Some(LRESULT(HTCLIENT as isize));
                }
                WM_MOUSEWHEEL => {
                    let delta=(wp.0>>16) as u16 as i16;
                    let mut p=POINT{x:lp.0 as i16 as i32,y:(lp.0>>16) as i16 as i32};let _=windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd,&mut p);let p=Point::new(p.x as f32/self.gfx.scale(),p.y as f32/self.gfx.scale());self.input.mouse=Some(p);
                    if self.hits.iter().any(|h|h.id==wid("ai.model-surface") && h.rect.contains(p)){self.input.wheel+=f32::from(delta)/120.0;}else{self.scroll=(self.scroll-f32::from(delta)/120.0*48.0).clamp(0.0,self.max_scroll);}
                    self.keyboard_focus=None;
                }
                WM_MOUSEHWHEEL => {
                    let delta = (wp.0 >> 16) as u16 as i16;
                    if let Some(forward) = self.horizontal_pager.feed(delta, crate::clock::mono()) {
                        let sections: Vec<_> = Section::ALL.into_iter().filter(|s| s.visible(self.ai_on)).collect();
                        let current = sections.iter().position(|s| *s == self.section).unwrap_or(0);
                        let next = if forward { (current + 1).min(sections.len() - 1) }
                            else { current.saturating_sub(1) };
                        self.section = sections[next];
                        self.keyboard_focus = None;
                    }
                }
                WM_DPICHANGED => {
                    let r = &*(lp.0 as *const RECT);
                    let monitor = crate::window::pick_monitor(MonitorChoice::Cursor);
                    let scale = (GetDpiForWindow(self.hwnd) as f32 / 96.0)
                        .min((monitor.work.right - monitor.work.left - 24) as f32 / WIDTH)
                        .min((monitor.work.bottom - monitor.work.top - 24) as f32 / HEIGHT);
                    self.gfx.set_dpi(scale * 96.0);
                    let width = (WIDTH * scale).round() as i32;
                    let height = (HEIGHT * scale).round() as i32;
                    let x = r.left.clamp(monitor.work.left, (monitor.work.right - width).max(monitor.work.left));
                    let y = r.top.clamp(monitor.work.top, (monitor.work.bottom - height).max(monitor.work.top));
                    let _ = SetWindowPos(self.hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    let s = self.gfx.scale();
                    let p = Point::new(lp.0 as i16 as f32 / s, (lp.0 >> 16) as i16 as f32 / s);
                    self.input.mouse = Some(p);
                    if msg == WM_LBUTTONDOWN {
                        self.keyboard_focus = None;
                        self.input.down = true;
                        self.input.pressed_id = self.hits.iter().rev().find(|h| h.rect.contains(p)).map(|h| h.id);
                        if self.input.pressed_id!=self.input.focused{self.input.focused=None;}
                        SetCapture(self.hwnd);
                    } else if msg == WM_LBUTTONUP {
                        let id = self.hits.iter().rev().find(|h| h.rect.contains(p)).map(|h| h.id);
                        if id == self.input.pressed_id { self.input.clicked = id.map(|id| (id, MouseButton::Left)); }
                        self.input.down = false;
                        self.input.pressed_id = None;
                        let _ = ReleaseCapture();
                    }
                    let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, ..Default::default() };
                    let _ = TrackMouseEvent(&mut track);
                }
                WM_MOUSELEAVE_ | WM_CAPTURECHANGED => {
                    self.input.mouse = None;
                    self.input.down = false;
                    self.input.pressed_id = None;
                }
                WM_CHAR => {
                    if self.input.focused.is_some() {crate::ai_window::push_char(&mut self.input,wp.0 as u32);}
                }
                WM_KEYDOWN if self.input.focused.is_some() => {crate::ai_window::text_key(&mut self.input,self.hwnd,wp);}
                WM_KEYDOWN => {
                    match VIRTUAL_KEY(wp.0 as u16) {
                        VK_PRIOR => {self.scroll=(self.scroll-360.0).max(0.0);self.keyboard_focus=None;}
                        VK_NEXT => {self.scroll=(self.scroll+360.0).min(self.max_scroll);self.keyboard_focus=None;}
                        VK_HOME => {self.scroll=0.0;self.keyboard_focus=None;}
                        VK_END => {self.scroll=self.max_scroll;self.keyboard_focus=None;}
                        VK_ESCAPE => { let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)); }
                        VK_TAB if !self.hits.is_empty() => {
                            let backwards = GetKeyState(VK_SHIFT.0 as i32) < 0;
                            let next = match self.keyboard_focus.and_then(|id| self.hits.iter().position(|h| h.id == id)) {
                                Some(i) if backwards => (i + self.hits.len() - 1) % self.hits.len(),
                                Some(i) => (i + 1) % self.hits.len(),
                                None if backwards => self.hits.len() - 1,
                                None => 0,
                            };
                            self.keyboard_focus = Some(self.hits[next].id);
                        }
                        VK_RETURN | VK_SPACE => self.input.clicked = self.keyboard_focus.map(|id| (id, MouseButton::Left)),
                        _ => return None,
                    }
                }
                WM_SIZE => { if wp.0 as u32 == SIZE_MINIMIZED { self.release_open_priority(); } }
                WM_SHOWWINDOW => { if wp.0 == 0 { self.release_open_priority(); } }
                _ => return None,
            }
        }
        self.invalidate();
        Some(LRESULT(0))
    }
}

impl Drop for SettingsWindow {
    fn drop(&mut self) { unsafe { let _ = DestroyWindow(self.hwnd); } }
}

/// Shared row geometry keeps labels, descriptions and trailing controls aligned.
struct Form { x: f32, y: f32, w: f32 }
impl Form {
    fn new(area: Rect) -> Self { Self { x: area.x, y: area.y, w: area.w } }
    fn group(&mut self, ui: &mut Ui, title: &str, rows: usize) {
        ui.c.text(title, Rect::new(self.x + 4.0, self.y, self.w, 20.0), &TextStyle::footnote().color(palette::TEXT_DIM));
        self.y += 26.0;
        ui.c.fill_rrect(Rect::new(self.x, self.y, self.w, rows as f32 * 62.0), 13.0, CARD);
    }
    fn row(&mut self, ui: &mut Ui, label: &str, hint: &str) -> Rect {
        let r = Rect::new(self.x + 16.0, self.y, self.w - 32.0, 62.0);
        let y = if hint.is_empty() { r.y + 20.0 } else { r.y + 10.0 };
        ui.c.text(label, Rect::new(r.x, y, r.w - 66.0, 22.0), &TextStyle::body());
        if !hint.is_empty() {
            ui.c.text(hint, Rect::new(r.x, y + 23.0, r.w - 66.0, 18.0), &TextStyle::footnote().color(palette::TEXT_DIM));
        }
        self.y += 62.0;
        r
    }
    fn end(&mut self) { self.y += 18.0; }
    fn toggle(&mut self, ui: &mut Ui, id: &str, label: &str, hint: &str, on: Option<bool>) -> bool {
        let r = self.row(ui, label, hint);
        let resp = on.map(|_| ui.button(wid(id), r));
        if resp.is_some_and(|r| r.hovered) { ui.c.fill_rrect(r, 7.0, Color::white(0.025)); }
        ui.c.with_alpha(if on.is_some() { 1.0 } else { 0.4 }, |c| {
            draw_switch(c, Rect::new(r.right() - 46.0, r.y + 18.0, 46.0, 26.0), on == Some(true), GREEN);
        });
        resp.is_some_and(|r| r.clicked)
    }
    fn choices(&mut self, ui: &mut Ui, id: &str, label: &str, names: &[&str], selected: usize) -> Option<usize> {
        ui.c.text(label, Rect::new(self.x + 16.0, self.y + 8.0, self.w - 32.0, 20.0), &TextStyle::body());
        let result = picker(ui, id, self.x + 16.0, self.y + 30.0, 26.0, names, selected);
        self.y += 62.0;
        result
    }
}

fn draw_section(ui: &mut Ui, st: &mut AppState, section: Section, area: Rect) -> f32 {
    let before = st.settings.clone();
    let mut f = Form::new(area);
    match section {
        Section::Search => {
            f.group(ui,"Open Search",1);
            if f.toggle(ui,"preferences.search-gesture","Open with horizontal scroll","Swipe the closed island to open Search",Some(st.settings.search.horizontal_scroll)){st.settings.search.horizontal_scroll=!st.settings.search.horizontal_scroll;}
            f.end();
            if ui.pill(wid("preferences.open-search"),Rect::new(f.x+4.0,f.y,148.0,32.0),"Open Search",Some(Icon::Search),PillStyle::Primary(st.accent)).clicked {st.cmd(Command::OpenSearch);}
            f.y+=48.0;
            f.group(ui,"Search sources",2);
            if f.toggle(ui,"preferences.search-apps","Applications","Registered desktop and Microsoft Store apps",Some(st.settings.search.apps)){st.settings.search.apps=!st.settings.search.apps;}
            if f.toggle(ui,"preferences.search-settings","Windows settings","Display, Wi-Fi, sound, privacy and other system pages",Some(st.settings.search.settings)){st.settings.search.settings=!st.settings.search.settings;}
            f.end();
            if st.settings.search.apps && ui.pill(wid("preferences.refresh-search"),Rect::new(f.x+4.0,f.y,166.0,30.0),"Refresh applications",None,PillStyle::Utility).clicked {st.cmd(Command::RefreshSearchApps);}
            f.y+=44.0;
            f.group(ui,"Web search",if st.settings.search.web{2}else{1});
            if f.toggle(ui,"preferences.search-web","Search the web when nothing matches","Open results inside Lumo using installed Edge WebView2",Some(st.settings.search.web)){st.settings.search.web=!st.settings.search.web;}
            if st.settings.search.web {let engines=lumo_core::search::Engine::ALL;let labels=engines.map(|e|e.name());if let Some(i)=f.choices(ui,"preferences.search-engine","Search engine",&labels,engines.iter().position(|e|*e==st.settings.search.engine).unwrap_or(0)){st.settings.search.engine=engines[i];}}
            f.end();
            if ui.pill(wid("preferences.search-clear-history"),Rect::new(f.x+4.0,f.y,194.0,30.0),"Clear frequent results",None,PillStyle::Utility).clicked {st.store.search_usage.clear();st.save();crate::search::refresh(st);}
            f.y+=46.0;
            ui.c.text("Frequent results reflect what you open, not a saved query history. Web search sends a query only when you choose it. The browser closes with Search.",Rect::new(f.x+4.0,f.y,f.w-8.0,60.0),&TextStyle::footnote().color(palette::TEXT_DIM).wrap().top());f.y+=70.0;
        }
        Section::Ai => { f.y=crate::pages::ai::settings(ui,st,area); }
        Section::General => {
            f.group(ui, "Startup", 2);
            if f.toggle(ui, "preferences.autostart", "Start with Windows", "Ready when you sign in", st.ui.autostart) {
                st.cmd(Command::SetAutostart(!st.ui.autostart.unwrap_or(false)));
            }
            if f.toggle(ui, "preferences.remember", "Remember last page", "Start on your last page; reopening follows your activity", Some(st.settings.remember_page)) { st.settings.remember_page = !st.settings.remember_page; }
            f.end();
            f.group(ui, "Time & feedback", 2);
            if f.toggle(ui, "preferences.clock", "24-hour clock", "Use 14:00 instead of 2:00 PM", Some(st.settings.use_24h)) { st.settings.use_24h = !st.settings.use_24h; }
            if f.toggle(ui, "preferences.sounds", "Reminder sounds", "A gentle chime when a timer finishes", Some(st.settings.sounds)) { st.settings.sounds = !st.settings.sounds; }
            f.end();
            let hover = st.settings.open_on_hover;
            f.group(ui, "Interaction", if hover { 2 } else { 1 });
            if let Some(i) = f.choices(ui, "preferences.open-mode", "Open island with", &["Click", "Hover"], usize::from(hover)) { st.settings.open_on_hover = i == 1; }
            if hover {
                let values = [0, 140, 220, 400];
                if let Some(i) = f.choices(ui, "preferences.hover", "Hover delay", &["Instant", "Quick", "Normal", "Relaxed"], values.iter().position(|&v| v == st.settings.hover_delay_ms).unwrap_or(usize::MAX)) { st.settings.hover_delay_ms = values[i]; }
            }
            f.end();
        }
        Section::Display => {
            f.group(ui, "Visibility", 2);
            if f.toggle(ui, "preferences.fullscreen", "Hide in full screen", "Keep games and videos clear", Some(st.settings.hide_in_fullscreen)) { st.settings.hide_in_fullscreen = !st.settings.hide_in_fullscreen; }
            if f.toggle(ui, "preferences.maximized", "Reveal over maximized windows on hover", "Hover for the compact island, then click to open", Some(st.settings.hide_over_maximized)) { st.settings.hide_over_maximized = !st.settings.hide_over_maximized; }
            f.end();
            f.group(ui, "Size & screen", 2);
            let widths = [540.0, 620.0, 720.0];
            if let Some(i) = f.choices(ui, "preferences.width", "Expanded island width", &["Compact", "Default", "Roomy"], widths.iter().position(|&w| w == st.settings.expanded_width).unwrap_or(usize::MAX)) { st.settings.expanded_width = widths[i]; }
            if let Some(i) = f.choices(ui, "preferences.monitor", "Display on next start", &["Primary screen", "Screen under cursor"], usize::from(st.settings.monitor == MonitorChoice::Cursor)) { st.settings.monitor = if i == 0 { MonitorChoice::Primary } else { MonitorChoice::Cursor }; }
            f.end();

        }
        Section::Appearance => {
            f.group(ui, "Island style", 2);
            let themes = [Theme::Classic, Theme::Pill];
            if let Some(i) = f.choices(ui, "preferences.theme", "Theme", &["Classic", "Pill"], themes.iter().position(|&v| v == st.settings.theme).unwrap_or(0)) { st.settings.theme = themes[i]; }
            let looks = status::looks(st.settings.theme);
            let names: Vec<_> = looks.iter().map(|look| match look { IdleStyle::Clock => "Clock", IdleStyle::Eyes => "Eyes", IdleStyle::Minimal => "Minimal", IdleStyle::Hidden => "Hidden", IdleStyle::Status => "Status" }).collect();
            if let Some(i) = f.choices(ui, "preferences.idle", "When nothing is playing", &names, looks.iter().position(|&v| v == status::shown(st.settings.theme, st.settings.idle_style)).unwrap_or(0)) { st.settings.idle_style = looks[i]; }
            f.end();
            ui.c.text("Changes apply immediately.", Rect::new(f.x + 4.0, f.y, f.w - 8.0, 48.0), &TextStyle::footnote().color(palette::TEXT_DIM).wrap().top());
        }
        Section::Music => {
            f.group(ui, "On the island", 4);
            if f.toggle(ui, "preferences.media", "Show playing music", "Track details and playback controls", Some(st.settings.show_media_activity)) { st.settings.show_media_activity = !st.settings.show_media_activity; }
            if f.toggle(ui, "preferences.lyrics", "Lyrics on closed island", "Follow the current line while listening", Some(st.settings.island_lyrics)) { st.settings.island_lyrics = !st.settings.island_lyrics; }
            if f.toggle(ui, "preferences.lyrics-popup", "Lyrics window button", "Show the pop-out button in the Music player", Some(st.settings.lyrics_popup_button)) { st.settings.lyrics_popup_button = !st.settings.lyrics_popup_button; }
            if f.toggle(ui, "preferences.pronunciation", "Lyrics pronunciation", "Romanized Korean and Chinese lyrics", Some(st.settings.lyrics_pronunciation)) { st.settings.lyrics_pronunciation = !st.settings.lyrics_pronunciation; }
            f.end();
            f.group(ui, "Your music library", 1);
            if f.toggle(ui, "preferences.local", "Use local lyrics", "Sidecar and embedded lyrics first; LRCLIB for missing lyrics and streaming.", Some(st.settings.local_lyrics)) { st.settings.local_lyrics = !st.settings.local_lyrics; }
            f.end();
            if st.settings.local_lyrics {
                let path = crate::paths::music_dir(&st.settings).to_string_lossy().into_owned();
                ui.c.text(&path, Rect::new(f.x + 4.0, f.y, f.w - 8.0, 20.0), &TextStyle::footnote().color(palette::TEXT_DIM));
                f.y += 30.0;
                let mut x = f.x;
                for (id, label, cmd, enabled) in [
                    ("preferences.folder", if st.ui.choosing_music_dir { "Choosing…" } else { "Change folder" }, Command::ChooseMusicDirectory, !st.ui.choosing_music_dir),
                ] {
                    if !enabled { continue; }
                    let w = ui.pill_width(label, false, true);
                    if ui.pill(wid(id), Rect::new(x, f.y, w, 30.0), label, None, PillStyle::Utility).clicked { st.cmd(cmd); }
                    x += w + 8.0;
                }
                if !st.settings.music_directory.is_empty() && ui.pill(wid("preferences.folder-reset"), Rect::new(x, f.y, 66.0, 30.0), "Reset", None, PillStyle::Utility).clicked { st.settings.music_directory.clear(); }
                f.y+=44.0;
            }
            f.group(ui,"Downloaded lyrics",1);
            if f.toggle(ui,"preferences.lyrics-save","Save downloaded lyrics","Use the matching song's filename, with .lrc",Some(st.settings.lyrics_auto_save)) {st.settings.lyrics_auto_save=!st.settings.lyrics_auto_save;}
            f.end();
            if st.settings.lyrics_auto_save {
                let path=crate::paths::lyrics_download_dir(&st.settings).map(|p|p.to_string_lossy().into_owned())
                    .unwrap_or_else(||"Choose a Music folder above or a download folder below.".into());
                ui.c.text(&path,Rect::new(f.x+4.0,f.y,f.w-8.0,20.0),&TextStyle::footnote().color(palette::TEXT_DIM));
                f.y+=30.0;
                let label=if st.ui.choosing_lyrics_dir {"Choosing…"} else {"Change download folder"};
                let w=ui.pill_width(label,false,true);
                if ui.pill(wid("preferences.download-folder"),Rect::new(f.x,f.y,w,30.0),label,None,PillStyle::Utility).clicked && !st.ui.choosing_lyrics_dir {
                    st.cmd(Command::ChooseLyricsDownloadDirectory);
                }
                if !st.settings.lyrics_download_directory.is_empty() {
                    let label="Use music folder";let reset_w=ui.pill_width(label,false,true);
                    if ui.pill(wid("preferences.download-reset"),Rect::new(f.x+w+8.0,f.y,reset_w,30.0),label,None,PillStyle::Utility).clicked {st.settings.lyrics_download_directory.clear();}
                }
                f.y+=44.0;
            }
        }
        Section::Focus => {
            f.group(ui, "Session lengths", 3);
            let values = [15, 25, 45, 60];
            if let Some(i) = f.choices(ui, "preferences.work", "Focus time", &["15 min", "25 min", "45 min", "60 min"], values.iter().position(|&v| v == st.settings.pomodoro.focus_min).unwrap_or(usize::MAX)) { st.settings.pomodoro.focus_min = values[i]; }
            let values = [3, 5, 10, 15];
            if let Some(i) = f.choices(ui, "preferences.short-break", "Short break", &["3 min", "5 min", "10 min", "15 min"], values.iter().position(|&v| v == st.settings.pomodoro.short_break_min).unwrap_or(usize::MAX)) { st.settings.pomodoro.short_break_min = values[i]; }
            let values = [10, 15, 20, 30];
            if let Some(i) = f.choices(ui, "preferences.long-break", "Long break", &["10 min", "15 min", "20 min", "30 min"], values.iter().position(|&v| v == st.settings.pomodoro.long_break_min).unwrap_or(usize::MAX)) { st.settings.pomodoro.long_break_min = values[i]; }
        }
        Section::Notifications => {
            f.group(ui, "Popups", 3);
            if f.toggle(ui, "preferences.hud-volume", "Volume", "Show volume changes on the island", Some(st.settings.hud_volume)) { st.settings.hud_volume = !st.settings.hud_volume; }
            if f.toggle(ui, "preferences.hud-power", "Power & battery", "Charging and battery updates", Some(st.settings.hud_power)) { st.settings.hud_power = !st.settings.hud_power; }
            if f.toggle(ui, "preferences.hud-bluetooth", "Bluetooth", "Device connection updates", Some(st.settings.hud_bluetooth)) { st.settings.hud_bluetooth = !st.settings.hud_bluetooth; }
        }
        Section::About => {
            f.group(ui, "Lumo", 1);
            f.row(ui, &format!("Version {}", env!("CARGO_PKG_VERSION")), "A living island for Windows");
            f.end();
            draw_update_status(ui, st, &mut f);
            f.group(ui, "Update preferences", 2);
            let managed = st.update.managed;
            if f.toggle(ui, "preferences.auto-updates", "Automatically update Lumo", "Download now, install on the next launch", (!managed).then_some(st.settings.automatic_updates)) { st.settings.automatic_updates = !st.settings.automatic_updates; }
            if f.toggle(ui, "preferences.updates", "Tell me about new versions", "Show a notification when a new version is available", (!managed).then_some(st.settings.update_check)) { st.settings.update_check = !st.settings.update_check; }
            f.end();
            if st.update.checked != 0 {
                let minutes = (st.unix.max(0) as u64).saturating_sub(st.update.checked) / 60;
                let text = if minutes == 0 { "Last checked just now".into() } else if minutes < 60 { format!("Last checked {minutes} min ago") } else { format!("Last checked {} hours ago", minutes / 60) };
                ui.c.text(&text, Rect::new(f.x + 4.0, f.y, f.w, 18.0), &TextStyle::footnote().color(palette::TEXT_DIM));
                f.y += 22.0;
            }
        }
    }
    if st.settings != before && section!=Section::Ai { st.cmd(Command::SaveSettings); }
    f.y
}

fn draw_update_status(ui: &mut Ui, st: &mut AppState, f: &mut Form) {
    use crate::state::UpdateStatus;
    ui.c.text("Software updates", Rect::new(f.x + 4.0, f.y, f.w, 20.0), &TextStyle::footnote().color(palette::TEXT_DIM));
    f.y += 26.0;
    let card = Rect::new(f.x, f.y, f.w, 134.0);
    ui.c.fill_rrect(card, 13.0, CARD);
    let busy = matches!(st.update.status, UpdateStatus::Checking | UpdateStatus::Downloading | UpdateStatus::Installing);
    let version = st.update.offer.as_ref().map(|r| r.version.as_str()).unwrap_or_default();
    let (title, detail, icon, color) = if st.update.managed {
        ("Managed by Microsoft Store".to_string(), "Windows keeps your Store copy up to date.".to_string(), Icon::CheckCircle, GREEN)
    } else {
        match &st.update.status {
            UpdateStatus::Checking => ("Checking for updates…".into(), "Looking for a newer Windows release.".into(), Icon::Reset, palette::TEXT_DIM),
            UpdateStatus::Downloading => (format!("Downloading {version}…"), "You can keep using Lumo while the update downloads.".into(), Icon::Download, st.accent),
            UpdateStatus::Installing => ("Installing update…".into(), "Lumo will reopen when the update is installed.".into(), Icon::Download, st.accent),
            UpdateStatus::Failed(error) => ("Update couldn't finish".into(), error.chars().take(125).collect(), Icon::Circle, palette::ORANGE),
            _ if st.update.ready => (format!("Version {version} is ready"), if st.update.next_start { "It will install automatically the next time Lumo starts.".into() } else { "Install the update and restart when you're ready.".into() }, Icon::CheckCircle, GREEN),
            _ if st.update.offer.is_some() => (format!("Version {version} is available"), "Download the update here. Your settings will stay.".into(), Icon::Download, st.accent),
            UpdateStatus::Current => ("You're up to date".into(), "You have the newest available Windows build.".into(), Icon::CheckCircle, GREEN),
            _ => ("Keep Lumo up to date".into(), "Check for a newer version whenever you're ready.".into(), Icon::Reset, palette::TEXT_DIM),
        }
    };
    ui.c.icon(icon, Rect::new(card.x + 16.0, card.y + 17.0, 22.0, 22.0), color);
    ui.c.text(&title, Rect::new(card.x + 50.0, card.y + 13.0, card.w - 66.0, 25.0), &TextStyle::body().weight(Weight::Semibold));
    ui.c.text(&detail, Rect::new(card.x + 50.0, card.y + 41.0, card.w - 66.0, 36.0), &TextStyle::footnote().color(palette::TEXT_DIM));
    if !st.update.managed {
        let check_label = if busy { "Please wait…" } else { "Check for updates" };
        let check_w = ui.pill_width(check_label, false, true);
        let check = Rect::new(card.right() - 16.0 - check_w, card.y + 88.0, check_w, 30.0);
        if !busy {
            if ui.pill(wid("preferences.check-updates"), check, check_label, None, PillStyle::Utility).clicked { st.cmd(Command::CheckUpdates); }
            if st.update.offer.is_some() {
                let label = if st.update.ready { "Install and restart" } else if matches!(st.update.status, UpdateStatus::Failed(_)) { "Retry download" } else { "Download update" };
                let width = ui.pill_width(label, false, true);
                if ui.pill(wid("preferences.update"), Rect::new(card.x + 16.0, check.y, width, 30.0), label, None, PillStyle::Primary(st.accent)).clicked { st.cmd(Command::Update); }
            }
        } else {
            ui.c.text(check_label, check, &TextStyle::footnote().color(palette::TEXT_DIM));
        }
    }
    f.y += 152.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumo_core::{settings::Settings, store::Store};

    #[test] #[ignore="requires a Windows desktop"]
    fn search_preferences_configure_gestures_sources_and_explicit_actions(){
        let mut st=AppState::new(Settings::default(),Store::default());let mut window=SettingsWindow::new(HWND::default()).unwrap();window.section=Section::Search;window.draw(&mut st).unwrap();
        assert!(st.cmds.is_empty(),"opening preferences does not start discovery");
        for id in ["preferences.search-gesture","preferences.search-apps","preferences.search-web","preferences.search-settings"]{window.input.clicked=Some((wid(id),MouseButton::Left));window.draw(&mut st).unwrap();}
        assert!(!st.settings.search.horizontal_scroll&&!st.settings.search.apps&&st.settings.search.web&&!st.settings.search.settings);
        window.input.clicked=Some((wid("preferences.open-search"),MouseButton::Left));window.draw(&mut st).unwrap();assert!(st.cmds.contains(&Command::OpenSearch));
        st.settings.search=Default::default();window.draw(&mut st).unwrap();window.input.clicked=Some((wid("preferences.refresh-search"),MouseButton::Left));window.draw(&mut st).unwrap();assert!(st.cmds.contains(&Command::RefreshSearchApps));
        if let Some(dir)=std::env::var_os("LUMO_SEARCH_QA"){let dir=std::path::PathBuf::from(dir);std::fs::create_dir_all(&dir).unwrap();window.scroll=0.0;let(w,h,p,_,_)=window.selftest_capture(&mut st,Section::Search).unwrap();crate::png::save_rgba(dir.join("search-preferences.png"),w as u32,h as u32,&crate::png::pbgra_to_rgba(&p)).unwrap();}
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn disabled_ai_is_hidden_from_settings_and_swipe_navigation() {
        let mut st = AppState::new(Settings::default(), Store::default());
        let mut window = SettingsWindow::new(HWND::default()).unwrap();
        window.section = Section::Music;
        window.draw(&mut st).unwrap();
        assert!(!window.hits.iter().any(|h| h.id == wid_n("preferences.section", 4)));
        window.message(WM_MOUSEHWHEEL, WPARAM(120usize << 16), LPARAM(0));
        assert!(window.section == Section::Focus);
        st.settings.set_addon("ai", true);
        st.ai.loaded = true;
        window.show_ai();
        window.draw(&mut st).unwrap();
        assert!(window.section == Section::Ai);
        assert!(window.hits.iter().any(|h| h.id == wid_n("preferences.section", 4)));
        st.settings.set_addon("ai", false);
        window.draw(&mut st).unwrap();
        assert!(window.section == Section::General);
        assert!(!window.hits.iter().any(|h| h.id == wid_n("preferences.section", 4)));
        assert!(!window.hits.iter().any(|h| h.id == wid("ai.key")));
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn about_update_controls_and_states() {
        use crate::state::{UpdateStatus, UpdateView};
        let mut st = AppState::new(Settings::default(), Store::default());
        let mut window = SettingsWindow::new(HWND::default()).unwrap();
        window.show_about();
        window.draw(&mut st).unwrap();
        assert!(window.hits.iter().any(|h| h.id == wid("preferences.check-updates")));
        assert!(!window.hits.iter().any(|h| h.id == wid_n("preferences.about-link", 0) || h.id == wid_n("preferences.about-link", 1)));
        st.cmds.clear();
        window.input.clicked = Some((wid("preferences.check-updates"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&Command::CheckUpdates));
        assert!(!st.settings.automatic_updates && !st.settings.update_check);
        for id in ["preferences.auto-updates", "preferences.updates"] {
            window.input.clicked = Some((wid(id), MouseButton::Left));
            window.draw(&mut st).unwrap();
        }
        assert!(st.settings.automatic_updates && st.settings.update_check);
        let output = std::env::var_os("LUMO_SETTINGS_QA").map(std::path::PathBuf::from);
        let release = lumo_core::update::Release { version: "2026.1107.3".into(), date: "2026-11-07".into(), headline: "A new version".into(), notes: String::new(), windows: None };
        for (name, status, offer, ready) in [
            ("idle", UpdateStatus::Idle, None, false),
            ("checking", UpdateStatus::Checking, None, false),
            ("current", UpdateStatus::Current, None, false),
            ("available", UpdateStatus::Available, Some(release.clone()), false),
            ("downloading", UpdateStatus::Downloading, Some(release.clone()), false),
            ("ready", UpdateStatus::Ready, Some(release.clone()), true),
            ("error", UpdateStatus::Failed("Couldn't connect to GitHub. Check your connection and try again.".into()), Some(release), false),
        ] {
            let busy = matches!(status, UpdateStatus::Checking | UpdateStatus::Downloading);
            st.update = UpdateView { status, offer, ready, ..Default::default() };
            window.draw(&mut st).unwrap();
            assert_eq!(window.hits.iter().any(|h| h.id == wid("preferences.check-updates")), !busy);
            if ready || name == "available" || name == "error" {
                st.cmds.clear();
                window.input.clicked = Some((wid("preferences.update"), MouseButton::Left));
                window.draw(&mut st).unwrap();
                assert!(st.cmds.contains(&Command::Update));
            }
            assert_eq!(window.max_scroll, 0.0, "About must fit without scrolling: {name}");
            if let Some(dir) = &output {
                std::fs::create_dir_all(dir).unwrap();
                let (w, h, pixels, _, _) = window.selftest_capture(&mut st, Section::About).unwrap();
                crate::png::save_rgba(dir.join(format!("about-{name}.png")), w as u32, h as u32, &crate::png::pbgra_to_rgba(&pixels)).unwrap();
            }
        }
        st.update.managed = true;
        window.draw(&mut st).unwrap();
        assert!(!window.hits.iter().any(|h| h.id == wid("preferences.check-updates") || h.id == wid("preferences.auto-updates")));
    }

    #[test]
    #[ignore = "requires a Windows desktop"]
    fn settings_open_priority_is_temporary_and_does_not_move_or_resize() {
        unsafe {
            let island = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"), w!("Lumo stacking test"), WS_POPUP, 0, 0, 200, 80,
                None, None, GetModuleHandleW(None).unwrap(), None).unwrap();
            let mut window = SettingsWindow::new(island).unwrap();
            let topmost = |hwnd| GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
            let mut before = RECT::default();
            GetWindowRect(window.hwnd, &mut before).unwrap();
            assert!(!topmost(window.hwnd));
            window.raise_for_open();
            assert!(topmost(window.hwnd));
            assert!(window.raised_for_open);
            // A subsequently raised Settings window must precede the island.
            let mut previous = GetWindow(window.hwnd, GW_HWNDPREV).unwrap_or_default();
            while !previous.is_invalid() {
                assert_ne!(previous, island, "island still covers Settings");
                previous = GetWindow(previous, GW_HWNDPREV).unwrap_or_default();
            }
            window.message(WM_ACTIVATE, WPARAM(WA_INACTIVE as usize), LPARAM(0));
            assert!(!topmost(window.hwnd));
            assert!(!window.raised_for_open);
            assert!(topmost(island));
            let mut after = RECT::default();
            GetWindowRect(window.hwnd, &mut after).unwrap();
            assert_eq!((before.left, before.top, before.right, before.bottom),
                       (after.left, after.top, after.right, after.bottom));
            // Opening again repeats the one-shot lift; hiding/minimizing clears it.
            window.raise_for_open();
            window.message(WM_SIZE, WPARAM(SIZE_MINIMIZED as usize), LPARAM(0));
            assert!(!topmost(window.hwnd));
            window.raise_for_open();
            window.message(WM_SHOWWINDOW, WPARAM(0), LPARAM(0));
            assert!(!topmost(window.hwnd));
            DestroyWindow(island).unwrap();
        }
    }

    /// Exercises real HWND creation, layered rendering, input and destruction.
    /// Explicit because it needs a Windows desktop.
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn settings_window_renders_and_reopens() {
        let mut st = AppState::new(Settings::default(), Store::default());
        st.settings.set_addon("ai", true);
        st.ui.autostart = Some(false);
        st.ai.loaded=true;
        let mut window = SettingsWindow::new(HWND::default()).unwrap();
        window.show();
        let output = std::env::var_os("LUMO_SETTINGS_QA").map(std::path::PathBuf::from);
        if let Some(dir) = &output { std::fs::create_dir_all(dir).unwrap(); }
        for (i, section) in Section::ALL.iter().copied().enumerate() {
            window.input.clicked = Some((wid_n("preferences.section", i as u64), MouseButton::Left));
            window.draw(&mut st).unwrap();
            assert!(window.section == section);
            // Controls must remain inside the window, including at display scaling.
            for hit in &window.hits {
                assert!(hit.rect.y >= 0.0 && hit.rect.bottom() <= HEIGHT + 1.0, "{}: {:?}", section.title(), hit.rect);
                assert!(hit.rect.x >= 0.0 && hit.rect.right() <= WIDTH + 1.0);
            }
            if let Some(dir) = &output {
                let mut bounds = RECT::default();
                unsafe { GetWindowRect(window.hwnd, &mut bounds).unwrap(); }
                let w = bounds.right - bounds.left;
                let h = bounds.bottom - bounds.top;
                window.gfx.request_capture(RECT { left: 0, top: 0, right: w, bottom: h });
                window.draw(&mut st).unwrap();
                let rgba = crate::png::pbgra_to_rgba(&window.gfx.take_capture());
                crate::png::save_rgba(&dir.join(format!("settings-{i}.png")), w as u32, h as u32, &rgba).unwrap();
            }
        }
        window.section=Section::Ai;st.ai.models=(0..80).map(|i|lumo_core::ai::Model::manual(&format!("provider/model-{i}"))).collect();st.ai.model_menu=true;st.ai.model_settings=true;window.scroll=0.0;window.draw(&mut st).unwrap();
        let model_surface=window.hits.iter().find(|h|h.id==wid("ai.model-surface")).unwrap().rect;let mouse=model_surface.center();let mut bounds=RECT::default();unsafe{GetWindowRect(window.hwnd,&mut bounds).unwrap();}let mx=bounds.left+(mouse.x*window.gfx.scale()) as i32;let my=bounds.top+(mouse.y*window.gfx.scale()) as i32;let position=LPARAM(((my as u32 as usize)<<16 | (mx as u16 as usize)) as isize);
        window.message(WM_MOUSEWHEEL,WPARAM((-120i16 as u16 as usize)<<16),position);window.draw(&mut st).unwrap();assert!(st.ai.model_scroll>0.0,"Settings model list must receive the wheel");assert_eq!(window.scroll,0.0,"Settings behind the model selector must not scroll");
        if let Some(dir)=&output{let(w,h,pixels,_,_)=window.selftest_capture(&mut st,Section::Ai).unwrap();crate::png::save_rgba(dir.join("settings-ai-models.png"),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
        st.ai.model_menu=false;
        window.section = Section::General;
        assert!(!st.settings.open_on_hover);
        window.input.clicked = Some((wid_n("preferences.open-mode", 1), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.draw(&mut st).unwrap();
        assert!(st.settings.open_on_hover);
        assert!(window.hits.iter().any(|hit| hit.id == wid_n("preferences.hover", 0)));
        for hit in &window.hits { assert!(hit.rect.bottom() <= HEIGHT + 1.0); }
        window.input.clicked = Some((wid_n("preferences.open-mode", 0), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.draw(&mut st).unwrap();
        assert!(!st.settings.open_on_hover);
        assert!(!window.hits.iter().any(|hit| hit.id == wid_n("preferences.hover", 0)));
        window.section = Section::Music;
        st.cmds.clear();
        window.input.clicked = Some((wid("preferences.local"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(st.settings.local_lyrics);
        assert!(st.cmds.contains(&Command::SaveSettings));
        assert!(!window.hits.iter().any(|hit|hit.id==wid("preferences.lyrics-file")));
        window.draw(&mut st).unwrap();
        assert!(window.max_scroll>0.0,"Music settings must make room for download options");
        window.message(WM_MOUSEWHEEL,WPARAM(((-360i16 as u16) as usize)<<16),LPARAM(0));
        window.draw(&mut st).unwrap();
        window.scroll=window.max_scroll;window.draw(&mut st).unwrap();
        assert!(window.scroll>0.0);
        assert!(window.hits.iter().any(|hit|hit.id==wid("preferences.download-folder")));
        st.cmds.clear();window.input.clicked=Some((wid("preferences.download-folder"),MouseButton::Left));window.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&Command::ChooseLyricsDownloadDirectory));
        if let Some(dir)=&output {
            let (w,h,pixels,_,_)=window.selftest_capture(&mut st,Section::Music).unwrap();
            crate::png::save_rgba(dir.join("settings-music-downloads.png"),w as u32,h as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();
        }
        window.input.clicked=Some((wid("preferences.lyrics-save"),MouseButton::Left));window.draw(&mut st).unwrap();
        assert!(!st.settings.lyrics_auto_save);
        window.input.clicked=Some((wid("preferences.lyrics-save"),MouseButton::Left));window.draw(&mut st).unwrap();
        assert!(st.settings.lyrics_auto_save);
        // Keyboard navigation can reach the same controls as the mouse.
        window.message(WM_KEYDOWN, WPARAM(VK_TAB.0 as usize), LPARAM(0));
        assert!(window.keyboard_focus.is_some());
        assert!(!window.hits.iter().any(|hit| hit.id == wid("preferences.minimize")));
        assert_eq!(unsafe { GetWindowLongPtrW(window.hwnd, GWL_STYLE) } as u32 & WS_MINIMIZEBOX.0, 0);
        window.message(WM_MOUSEHWHEEL, WPARAM(120usize << 16), LPARAM(0));
        window.draw(&mut st).unwrap();
        assert!(window.section == Section::Ai);
        window.message(WM_MOUSEHWHEEL, WPARAM(120usize << 16), LPARAM(0));
        window.draw(&mut st).unwrap();
        assert!(window.section == Section::Ai, "inertial packets in the same swipe must not skip sections");
        window.show();
        assert!(!unsafe { IsIconic(window.hwnd) }.as_bool());
        let hwnd = window.hwnd;
        drop(window);
        assert!(!unsafe { IsWindow(hwnd) }.as_bool());
        let reopened = SettingsWindow::new(HWND::default()).unwrap();
        assert!(unsafe { IsWindow(reopened.hwnd) }.as_bool());
    }
}



