//! Full chat is a native, resizable window, constructed only when opened.
use crate::{
    gfx::Gfx,
    state::AppState,
    ui::{wid, Hit, Input, KeyInput, MouseButton},
};
use lumo_core::geom::{Color, Point, Rect};
use windows::{
    core::{w, Result},
    Win32::{
        Foundation::*,
        Graphics::Gdi::{InvalidateRect, ScreenToClient},
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};
pub struct AiWindow {
    pub hwnd: HWND,
    gfx: Gfx,
    input: Input,
    hits: Vec<Hit>,
    resting: bool,
    restore_after_rest: bool,
    drop_registered: bool,
    double_press: bool,
}
/// A native focus change can arrive while App is borrowed by a window call.
pub const CHECK_ACTIVATION: u32 = WM_APP + 98;
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match crate::app::with_app(|app| app.handle_ai_window(hwnd, msg, wp, lp)) {
        Some(Some(r)) => return r,
        None if msg == CHECK_ACTIVATION => {
            crate::app::note_missed(msg, WPARAM(0), LPARAM(hwnd.0 as isize));
            return LRESULT(0);
        }
        _ => {}
    }
    if msg == WM_ACTIVATE && (wp.0 & 0xffff) as u32 == WA_INACTIVE {
        let _ = PostMessageW(hwnd, CHECK_ACTIVATION, WPARAM(0), LPARAM(0));
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
impl AiWindow {
    pub fn new() -> Result<Self> {
        Self::with_config(&lumo_core::ai::Config::default())
    }
    pub fn with_config(config: &lumo_core::ai::Config) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("Lumo.AiChat");
            let wc = WNDCLASSEXW {
                style: CS_DBLCLKS,
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(windows::core::Error::from_win32());
            }
            let monitor = crate::window::pick_monitor(lumo_core::settings::MonitorChoice::Cursor);
            let scale = monitor.dpi as f32 / 96.0;
            let w = (f32::from(config.mini_width.clamp(360, 1920)) * scale)
                .min((monitor.work.right - monitor.work.left - 24) as f32)
                as i32;
            let h = (f32::from(config.mini_height.clamp(360, 1600)) * scale)
                .min((monitor.work.bottom - monitor.work.top - 24) as f32)
                as i32;
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class,
                w!("Ask AI — Lumo"),
                WS_POPUP | WS_THICKFRAME | WS_SYSMENU,
                monitor.work.right - w - 12,
                monitor.work.top + 12,
                w,
                h,
                None,
                None,
                instance,
                None,
            )?;
            let gfx = match Gfx::static_window(scale * 96.0, hwnd) {
                Ok(g) => g,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(e);
                }
            };
            Ok(Self {
                hwnd,
                gfx,
                input: Input::default(),
                hits: vec![],
                resting: false,
                restore_after_rest: false,
                drop_registered: false,
                double_press: false,
            })
        }
    }
    pub fn enable_file_drop(&mut self,sink:crate::events::EventSink)->Result<()> {
        if !self.drop_registered {crate::dragdrop::register_target(self.hwnd,sink)?;self.drop_registered=true;}
        Ok(())
    }
    pub fn show(&self) {
        unsafe {
            let monitor = crate::window::pick_monitor(lumo_core::settings::MonitorChoice::Cursor);
            let mut bounds = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut bounds);
            let gap = (12.0 * monitor.dpi as f32 / 96.0) as i32;
            let width =
                (bounds.right - bounds.left).min(monitor.work.right - monitor.work.left - 2 * gap);
            let height =
                (bounds.bottom - bounds.top).min(monitor.work.bottom - monitor.work.top - 2 * gap);
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                bounds
                    .left
                    .clamp(monitor.work.left, monitor.work.right - width),
                bounds
                    .top
                    .clamp(monitor.work.top, monitor.work.bottom - height),
                width,
                height,
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.invalidate();
    }
    pub fn size(&self) -> (u16, u16) {
        unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            (
                ((r.right - r.left) as f32 / self.gfx.scale())
                    .round()
                    .clamp(360.0, 1920.0) as u16,
                ((r.bottom - r.top) as f32 / self.gfx.scale())
                    .round()
                    .clamp(360.0, 1600.0) as u16,
            )
        }
    }
    pub fn apply_size(&self, config: &lumo_core::ai::Config) {
        unsafe {
            let monitor =
                crate::window::monitor_info(windows::Win32::Graphics::Gdi::MonitorFromWindow(
                    self.hwnd,
                    windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
                ));
            if let Some(m) = monitor {
                let mut r = RECT::default();
                let _ = GetWindowRect(self.hwnd, &mut r);
                let w = (f32::from(config.mini_width.clamp(360, 1920)) * self.gfx.scale())
                    .min((m.work.right - m.work.left - 16) as f32) as i32;
                let h = (f32::from(config.mini_height.clamp(360, 1600)) * self.gfx.scale())
                    .min((m.work.bottom - m.work.top - 16) as f32) as i32;
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    r.left.clamp(m.work.left, m.work.right - w),
                    r.top.clamp(m.work.top, m.work.bottom - h),
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                self.invalidate();
            }
        }
    }
    /// No repeating animation survives a locked/off/saver session. Restore
    /// only a window that was visible before the rest state, without focus.
    pub fn rest(&mut self, on: bool) {
        if self.resting == on {
            return;
        }
        self.resting = on;
        unsafe {
            if on {
                self.restore_after_rest = IsWindowVisible(self.hwnd).as_bool();
                let _ = KillTimer(self.hwnd, 71);
                let _ = KillTimer(self.hwnd, 73);
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            } else if self.restore_after_rest {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.invalidate();
            }
        }
    }
    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, false);
        }
    }
    pub fn focus_prompt(&mut self, len: usize) {
        self.focus_field(wid("ai.prompt"), len);
    }
    pub fn focus_field(&mut self, id: crate::ui::WidgetId, len: usize) {
        self.input.focused = Some(id);
        self.input.caret = len;
        self.input.selection_all=false;
        self.input.selection_anchor=None;
        self.input.selection_owner=Some(id);
        self.input.text_scroll=0.0;
        self.invalidate();
    }
    pub fn prompt_focused(&self) -> bool {
        self.input.focused == Some(wid("ai.prompt"))
    }
    pub fn draw(&mut self, st: &mut AppState) -> Result<()> {
        self.input.begin_frame();
        if self.resting {
            return Ok(());
        }
        self.gfx.set_look(lumo_core::look::of(st.settings.theme));
        let mut bounds = RECT::default();
        unsafe {
            GetWindowRect(self.hwnd, &mut bounds)?;
        }
        let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        self.gfx.set_surface(w, h)?;
        let area = Rect::new(
            0.0,
            0.0,
            w as f32 / self.gfx.scale(),
            h as f32 / self.gfx.scale(),
        );
        self.hits.clear();
        let mut c = self.gfx.begin(RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        })?;
        let mut ui = crate::ui::Ui::new(&mut c, &mut self.input, &mut self.hits, st.now);
        ui.c.fill_rrect(area.inset(1.0, 1.0), 24.0, Color::black(1.0));
        ui.c.stroke_rrect(area.inset(1.0, 1.0), 24.0, Color::white(0.13), 1.0);
        crate::pages::ai::draw_chat(&mut ui, st, crate::pages::ai::workspace_area(area), true);
        // A model description needs one delayed paint, never a repeating timer.
        unsafe {
            let _ = KillTimer(self.hwnd, 71);
            if let Some(t) = crate::pages::ai::next_wake(st) {
                SetTimer(
                    self.hwnd,
                    71,
                    ((t - st.now) * 1000.0).ceil().max(1.0) as u32,
                    None,
                );
            }
        }
        let moving = ui.anim_requested() && !st.power.saver;
        let live = ui.live_requested() && !st.power.saver;
        let repaint = ui.repaint_requested();
        let caret=ui.input.caret_wake.filter(|_|!st.power.saver&&unsafe{GetForegroundWindow()}==self.hwnd);
        drop(ui);
        if let Some(text)=self.input.clipboard_write.take(){st.cmd(crate::commands::Command::CopyText(text));}
        unsafe {
            let _ = KillTimer(self.hwnd, 73);
            if moving || live {
                // Pending reply dots need only 30 fps; finite transitions
                // retain their usual cadence. No timer while idle.
                SetTimer(self.hwnd, 73, if moving { 16 } else { 33 }, None);
            }else if let Some(t)=caret{
                // One paint per Windows caret boundary, only in a focused editor.
                SetTimer(self.hwnd,73,((t-st.now)*1000.0).ceil().max(10.0)as u32,None);
            }
        }
        drop(c);
        self.input.end_frame();
        self.gfx.end()?;
        self.gfx.present(bounds.left, bounds.top)?;
        if repaint {
            self.invalidate();
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn capture(&mut self, st: &mut AppState, path: &std::path::Path) {
        let mut r = RECT::default();
        unsafe {
            GetWindowRect(self.hwnd, &mut r).unwrap();
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        self.gfx.request_capture(RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        });
        self.draw(st).unwrap();
        let rgba = crate::png::pbgra_to_rgba(&self.gfx.take_capture());
        crate::png::save_rgba(path, w as u32, h as u32, &rgba).unwrap();
    }
    pub fn message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        unsafe {
            match msg {
                WM_ERASEBKGND => return Some(LRESULT(1)),
                WM_TIMER if wp.0 == 71 || wp.0 == 73 => {
                    let _ = KillTimer(self.hwnd, wp.0);
                }
                WM_GETMINMAXINFO => {
                    let m = &mut *(lp.0 as *mut MINMAXINFO);
                    m.ptMinTrackSize = POINT {
                        x: (360.0 * self.gfx.scale()) as i32,
                        y: (360.0 * self.gfx.scale()) as i32,
                    };
                    return Some(LRESULT(0));
                }
                WM_NCCALCSIZE => return Some(LRESULT(0)),
                WM_SETCURSOR if lp.0 as u16 as u32 == HTCLIENT => {
                    let cursor = self
                        .input
                        .mouse
                        .and_then(|p| self.hits.iter().rev().find(|h| h.rect.contains(p)))
                        .map(|h| h.cursor);
                    let cursor = match cursor {
                        Some(crate::ui::CursorKind::Text) => IDC_IBEAM,
                        Some(crate::ui::CursorKind::Hand) => IDC_HAND,
                        _ => IDC_ARROW,
                    };
                    SetCursor(LoadCursorW(None, cursor).unwrap_or_default());
                    return Some(LRESULT(1));
                }
                WM_NCHITTEST => {
                    let mut p = POINT {
                        x: lp.0 as i16 as i32,
                        y: (lp.0 >> 16) as i16 as i32,
                    };
                    let _ = ScreenToClient(self.hwnd, &mut p);
                    let mut r = RECT::default();
                    let _ = GetClientRect(self.hwnd, &mut r);
                    if let Some(hit) = crate::window::mini_resize_hit(p,r,self.gfx.scale()) { return Some(LRESULT(hit as isize)); }
                    let hit = match () {
                        _ if p.y < (52.0 * self.gfx.scale()) as i32
                            && !self.hits.iter().any(|h| {
                                h.rect.contains(Point::new(
                                    p.x as f32 / self.gfx.scale(),
                                    p.y as f32 / self.gfx.scale(),
                                ))
                            }) =>
                        {
                            HTCAPTION
                        }
                        _ => HTCLIENT,
                    };
                    return Some(LRESULT(hit as isize));
                }
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP => {
                    let scale = self.gfx.scale();
                    let p = Point::new(
                        lp.0 as i16 as f32 / scale,
                        (lp.0 >> 16) as i16 as f32 / scale,
                    );
                    self.input.mouse = Some(p);
                    if msg == WM_LBUTTONDOWN || msg == WM_LBUTTONDBLCLK {
                        self.double_press=msg==WM_LBUTTONDBLCLK;
                        self.input.double_click=self.double_press;
                        self.input.shift=GetKeyState(VK_SHIFT.0 as i32)<0;
                        self.input.down = true;
                        self.input.just_pressed = true;
                        self.input.press_pos = Some(p);
                        self.input.pressed_id = self
                            .hits
                            .iter()
                            .rev()
                            .find(|h| h.rect.contains(p))
                            .map(|h| h.id);
                        if self.input.pressed_id != self.input.focused {
                            self.input.focused = None;
                        }
                        SetCapture(self.hwnd);
                    } else if msg == WM_LBUTTONUP {
                        let id = self
                            .hits
                            .iter()
                            .rev()
                            .find(|h| h.rect.contains(p))
                            .map(|h| h.id);
                        if id == self.input.pressed_id && !self.double_press {
                            self.input.clicked = id.map(|id| (id, MouseButton::Left));
                        }
                        if self.input.down {
                            self.input.released = self.input.pressed_id.map(|id| (id, p));
                        }
                        self.input.down = false;
                        self.input.pressed_id = None;
                        self.double_press=false;
                        let _ = ReleaseCapture();
                    }
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        ..Default::default()
                    };
                    let _ = TrackMouseEvent(&mut t);
                }
                0x02a3 => {
                    self.input.mouse = None;
                }
                WM_MOUSEWHEEL => self.input.wheel += (wp.0 >> 16) as u16 as i16 as f32 / 120.0,
                WM_CHAR => push_char(&mut self.input, wp.0 as u32),
                WM_PASTE => paste_text(&mut self.input, self.hwnd),
                WM_KEYDOWN => {
                    if VIRTUAL_KEY(wp.0 as u16) == VK_ESCAPE && self.input.focused.is_none() {
                        let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    } else {
                        text_key(&mut self.input, self.hwnd, wp);
                    }
                }
                WM_DPICHANGED => {
                    self.gfx.set_dpi(GetDpiForWindow(self.hwnd) as f32);
                    let r = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                WM_SIZE | WM_EXITSIZEMOVE => {}
                _ => return None,
            }
        }
        self.invalidate();
        Some(LRESULT(0))
    }
}
impl Drop for AiWindow {
    fn drop(&mut self) {
        if self.drop_registered {crate::dragdrop::revoke(self.hwnd);}
        unsafe {
            let _ = KillTimer(self.hwnd, 71);
            let _ = KillTimer(self.hwnd, 73);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
pub fn text_key(input: &mut Input, hwnd: HWND, wp: WPARAM) {
    unsafe {
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        if ctrl && wp.0 == b'A' as usize {
            input.keys.push(KeyInput::SelectAll);
            return;
        }
        if ctrl && matches!(wp.0, x if x==b'C' as usize || x==b'X' as usize){input.keys.push(if wp.0==b'C' as usize{KeyInput::Copy}else{KeyInput::Cut});return;}
        if ctrl && wp.0 == b'V' as usize {
            paste_text(input, hwnd);
            return;
        }
        let key = match VIRTUAL_KEY(wp.0 as u16) {
            VK_BACK => Some(KeyInput::Backspace),
            VK_DELETE => Some(KeyInput::Delete),
            VK_LEFT => Some(if shift{KeyInput::SelectLeft}else{KeyInput::Left}),
            VK_RIGHT => Some(if shift{KeyInput::SelectRight}else{KeyInput::Right}),
            VK_HOME => Some(if shift{KeyInput::SelectHome}else{KeyInput::Home}),
            VK_END => Some(if shift{KeyInput::SelectEnd}else{KeyInput::End}),
            VK_RETURN => Some(if shift {
                KeyInput::Char('\n')
            } else {
                KeyInput::Enter
            }),
            VK_ESCAPE => Some(KeyInput::Escape),
            VK_TAB => {
                input.focused = None;
                None
            }
            _ => None,
        };
        if let Some(k) = key {
            input.keys.push(k);
        }
    }
}
pub fn paste_text(input: &mut Input, hwnd: HWND) {
    if let Some(s) = crate::services::clipboard::read_text(hwnd) {
        input.keys.extend(
            s.chars()
                .take(16000)
                .filter(|c| !c.is_control() || *c == '\n')
                .map(KeyInput::Char),
        );
    }
}

pub fn push_char(input: &mut Input, u: u32) {
    let ch = if (0xD800..0xDC00).contains(&u) {
        input.high_surrogate = Some(u as u16);
        None
    } else if (0xDC00..0xE000).contains(&u) {
        input.high_surrogate.take().and_then(|h| {
            char::decode_utf16([h, u as u16])
                .next()
                .and_then(|r| r.ok())
        })
    } else {
        input.high_surrogate = None;
        char::from_u32(u)
    };
    if let Some(c) = ch.filter(|c| !c.is_control()) {
        input.keys.push(KeyInput::Char(c));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]#[ignore="requires a Windows desktop; offline native corner hit targets"]
    fn mini_corners_use_diagonal_resize_and_keep_the_chat() {
        let mut st=sample_state_for_selftest();st.ai.draft="Keep this resize draft".into();let mut w=AiWindow::new().unwrap();
        for scale in [1.0,1.25,1.5,2.0] {
            w.gfx.set_dpi(scale*96.0);let hwnd=w.hwnd;
            crate::window::check_mini_resize_corners(hwnd,scale,|lp|w.message(WM_NCHITTEST,WPARAM(0),lp));
        }
        w.gfx.set_dpi(96.0);
        for (width,height) in [(420,460),(680,740),(360,360)] {
            unsafe{SetWindowPos(w.hwnd,None,0,0,width,height,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE).unwrap();}
            w.message(WM_SIZE,WPARAM(0),LPARAM(0));w.draw(&mut st).unwrap();
            assert_eq!(st.ai.draft,"Keep this resize draft");
        }
    }
    #[test]#[ignore="requires a Windows desktop; offline model preview and filter popover"]
    fn model_info_hover_previews_capabilities_and_filters_dismiss_locally() {
        use crate::ui::wid_n;
        let mut st=sample_state_for_selftest();
        for i in 0..30 {st.ai.models.push(lumo_core::ai::Model::manual(&format!("zz-extra/model-{i}")));}
        st.ai.model_menu=true;st.ai.draft="Keep the prompt".into();
        let selected=st.ai.selected.clone();let mut w=AiWindow::new().unwrap();w.draw(&mut st).unwrap();
        let button=w.hits.iter().find(|h|h.id==wid_n("ai.model-info",0)).unwrap().rect;
        let mut rect=RECT::default();unsafe{GetClientRect(w.hwnd,&mut rect).unwrap();}
        w.input.mouse=None;w.gfx.request_capture(rect);w.draw(&mut st).unwrap();let quiet=w.gfx.take_capture();
        w.input.mouse=Some(button.center());w.gfx.request_capture(rect);w.draw(&mut st).unwrap();let hovered=w.gfx.take_capture();
        assert_ne!(quiet,hovered);assert!(st.ai.model_detail.is_none());assert_eq!(st.ai.selected,selected);assert_eq!(st.ai.draft,"Keep the prompt");
        if let Some(dir)=std::env::var_os("LUMO_AI_QA"){w.capture(&mut st,&std::path::PathBuf::from(dir).join("model-quick-preview.png"));}
        w.input.mouse=None;w.input.clicked=Some((wid("ai.model-filters"),MouseButton::Left));w.draw(&mut st).unwrap();
        let parent=w.hits.iter().find(|h|h.id==wid("ai.model-surface")).unwrap().rect;
        let child=w.hits.iter().find(|h|h.id==wid("ai.model-filter-surface")).unwrap().rect;
        assert!(child.w<parent.w&&child.h<500.0,"filter has its own compact surface");
        w.input.clicked=Some((wid_n("ai.model-filter",1),MouseButton::Left));w.draw(&mut st).unwrap();w.draw(&mut st).unwrap();
        assert_eq!(w.hits.iter().find(|h|h.id==wid("ai.model-filter-surface")).unwrap().rect,child,"filtering a long catalog keeps the popup anchored");
        w.input.clicked=Some((wid("ai.model-filter-clear"),MouseButton::Left));w.draw(&mut st).unwrap();
        w.input.mouse=Some(Point::new(parent.x+2.0,parent.y+3.0));w.input.press_pos=w.input.mouse;w.input.just_pressed=true;w.draw(&mut st).unwrap();
        assert!(!st.ai.model_filters_open&&st.ai.model_menu,"outside press dismisses only the filter");
        w.input.clicked=Some((wid("ai.model-filters"),MouseButton::Left));w.draw(&mut st).unwrap();
        w.input.keys=vec![KeyInput::Escape];w.draw(&mut st).unwrap();assert!(!st.ai.model_filters_open&&st.ai.model_menu);
        for (width,height) in [(360,360),(420,460),(560,680)] {
            unsafe{SetWindowPos(w.hwnd,None,0,0,width,height,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE).unwrap();}w.gfx.set_dpi(96.0);
            st.ai.model_filters_open=true;w.draw(&mut st).unwrap();
            for h in w.hits.iter().filter(|h|h.id==wid("ai.model-filter-surface")||(1..=8).any(|n|h.id==wid_n("ai.model-filter",n))) {
                assert!(h.rect.x>=0.0&&h.rect.y>=0.0&&h.rect.right()<=width as f32&&h.rect.bottom()<=height as f32,"{width}x{height}: {h:?}");
            }
            if width==360 {if let Some(dir)=std::env::var_os("LUMO_AI_QA"){w.capture(&mut st,&std::path::PathBuf::from(dir).join("model-filter-small.png"));}}
            st.ai.model_filters_open=false;
        }
        assert_eq!(st.ai.selected,selected);assert_eq!(st.ai.draft,"Keep the prompt");
        assert!(!st.cmds.iter().any(|c|matches!(c,crate::commands::Command::Ai(crate::ai::Action::Send))));
    }
    #[test]#[ignore="requires a Windows desktop; offline unavailable-model interaction fixture"]
    fn unavailable_models_cannot_select_or_send_and_hackclub_can_hide_them() {
        use crate::ui::wid_n;
        let mut st=sample_state_for_selftest();st.ai.draft="Preserve my draft".into();
        st.settings.ai.provider=lumo_core::ai::Provider::Compatible;
        st.settings.ai.completion_url="https://ai.hackclub.com/proxy/v1/chat/completions".into();
        let mut old=lumo_core::ai::Model::manual("old");old.unavailable=Some("Deprecated · retirement date 2026-11-01".into());
        st.ai.models=vec![old,lumo_core::ai::Model::manual("good")];st.ai.selected="good".into();st.ai.catalog_revision+=1;
        st.ai.model_menu=true;let mut w=AiWindow::new().unwrap();w.draw(&mut st).unwrap();
        assert!(!w.hits.iter().any(|h|h.id==wid_n("ai.model-info",0)));
        st.settings.ai.hide_unavailable_models=false;w.draw(&mut st).unwrap();
        assert!(w.hits.iter().any(|h|h.id==wid_n("ai.model-info",0)));
        assert!(!w.hits.iter().any(|h|h.id==wid_n("ai.model",0)));
        w.input.clicked=Some((wid_n("ai.model",0),MouseButton::Left));w.draw(&mut st).unwrap();assert_eq!(st.ai.selected,"good");
        w.input.clicked=Some((wid_n("ai.model-info",0),MouseButton::Left));w.draw(&mut st).unwrap();
        assert_eq!(st.ai.model_detail.as_deref(),Some("old"));
        assert!(!w.hits.iter().any(|h|h.id==wid("ai.model-detail-select")));
        if let Some(dir)=std::env::var_os("LUMO_AI_QA"){w.capture(&mut st,&std::path::PathBuf::from(dir).join("ai-unavailable-model.png"));}
        w.input.clicked=Some((wid("ai.model-detail-select"),MouseButton::Left));w.draw(&mut st).unwrap();assert_eq!(st.ai.selected,"good");
        st.ai.close_popovers();st.ai.selected="old".into();st.cmds.clear();w.draw(&mut st).unwrap();
        assert!(!w.hits.iter().any(|h|h.id==wid("ai.send")));
        w.focus_prompt(st.ai.draft.chars().count());w.input.keys=vec![KeyInput::Enter];w.draw(&mut st).unwrap();
        assert_eq!(st.ai.draft,"Preserve my draft");assert!(!st.cmds.iter().any(|c|matches!(c,crate::commands::Command::Ai(crate::ai::Action::Send))));
        st.ai.busy=true;w.draw(&mut st).unwrap();assert!(w.hits.iter().any(|h|h.id==wid("ai.send")),"Stop stays reachable");
    }
    #[test]#[ignore="requires a Windows desktop; offline settings visibility fixture"]
    fn hide_unavailable_setting_is_only_offered_for_hackclub_urls() {
        let mut st=sample_state_for_selftest();let mut w=AiWindow::new().unwrap();w.gfx.set_dpi(96.0);w.gfx.set_surface(640,2400).unwrap();
        for (url,visible) in [("https://ai.hackclub.com/proxy/v1/chat/completions",true),("https://example.org/chat",false),("https://ai.hackclub.com.evil.org/chat",false)] {
            st.settings.ai.completion_url=url.into();st.settings.ai.models_url.clear();let mut hits=vec![];let mut input=Input::default();
            let mut c=w.gfx.begin(RECT{left:0,top:0,right:640,bottom:2400}).unwrap();
            {let mut ui=crate::ui::Ui::new(&mut c,&mut input,&mut hits,st.now);crate::pages::ai::settings(&mut ui,&mut st,Rect::new(20.0,20.0,600.0,2300.0));}
            drop(c);w.gfx.end().unwrap();
            assert_eq!(hits.iter().any(|h|h.id==wid("ai.hide-unavailable-models")),visible,"{url}");
        }
    }
    #[test]#[ignore="requires a Windows desktop; offline passive action-hint fixture"]
    fn hover_hints_draw_for_ai_controls_without_changing_layout_focus_or_sending(){
        let mut st=sample_state_for_selftest();st.ai.draft="Keep my draft".into();st.power.saver=true;let mut w=AiWindow::new().unwrap();w.focus_prompt(st.ai.draft.chars().count());
        let mut bounds=RECT::default();unsafe{GetClientRect(w.hwnd,&mut bounds).unwrap();}
        w.input.mouse=None;w.gfx.request_capture(bounds);w.draw(&mut st).unwrap();let quiet=w.gfx.take_capture();let initial=w.hits.clone();
        for id in ["ai.history","ai.new","ai.temporary","ai.keep-open","ai.resume","ai.open-full","ai.close","ai.plus","ai.models","ai.send"]{
            let button=initial.iter().find(|h|h.id==wid(id)).unwrap().rect;w.input.mouse=Some(button.center());w.gfx.request_capture(bounds);w.draw(&mut st).unwrap();let hovered=w.gfx.take_capture();
            // A tooltip paints outside its button, without moving any target.
            let scale=w.gfx.scale();let width=bounds.right as usize;
            let changed=quiet.chunks_exact(4).zip(hovered.chunks_exact(4)).enumerate().filter(|(n,(a,b))|a!=b&&!button.inset(-2.0,-2.0).contains(Point::new((n%width)as f32/scale,(n/width)as f32/scale))).count();
            if matches!(id,"ai.plus"|"ai.models"|"ai.send") { assert!(changed<20,"{id} must not paint a composer tooltip: {changed}"); } else { assert!(changed>100,"{id} must paint an action hint outside the button"); }
            for h in &initial{assert_eq!(w.hits.iter().find(|v|v.id==h.id).unwrap().rect,h.rect);}
            assert!(w.prompt_focused());assert_eq!(st.ai.draft,"Keep my draft");
            assert!(!st.cmds.iter().any(|c|matches!(c,crate::commands::Command::Ai(crate::ai::Action::Send|crate::ai::Action::FetchModels))));
            if id=="ai.keep-open"{if let Some(dir)=std::env::var_os("LUMO_AI_QA"){w.capture(&mut st,&std::path::PathBuf::from(dir).join("ai-action-hint.png"));}}
        }
    }
    #[test]#[ignore="requires a Windows desktop; deterministic caret pixel fixture"]
    fn caret_hides_when_empty_blinks_after_typing_and_stops_when_cleared(){
        let mut st=sample_state_for_selftest();st.now=100.0;st.ai.draft.clear();
        let mut w=AiWindow::new().unwrap();w.focus_prompt(0);
        let mut bounds=RECT::default();unsafe{GetClientRect(w.hwnd,&mut bounds).unwrap();}
        w.gfx.request_capture(bounds);w.draw(&mut st).unwrap();let empty=w.gfx.take_capture();
        assert!(w.input.caret_wake.is_none());assert!(w.prompt_focused());
        st.now=100.51;w.gfx.request_capture(bounds);w.draw(&mut st).unwrap();
        assert_eq!(empty,w.gfx.take_capture(),"an empty focused composer has no blinking pixels");
        w.input.keys.push(crate::ui::KeyInput::Char('a'));st.now=101.0;w.draw(&mut st).unwrap();
        assert_eq!(st.ai.draft,"a");
        w.input.caret_key=Some((wid("ai.prompt"),1,None));w.input.caret_period=Some(0.5);w.input.caret_epoch=101.0;
        w.gfx.request_capture(bounds);w.draw(&mut st).unwrap();let on=w.gfx.take_capture();
        assert_eq!(w.input.caret_wake,Some(101.5));
        st.now=101.51;w.gfx.request_capture(bounds);w.draw(&mut st).unwrap();
        assert_ne!(on,w.gfx.take_capture(),"a nonempty composer blinks without a continuous frame clock");
        w.input.keys=vec![crate::ui::KeyInput::SelectAll,crate::ui::KeyInput::Backspace];st.now=102.0;w.draw(&mut st).unwrap();
        assert!(st.ai.draft.is_empty());assert!(w.input.caret_wake.is_none());assert!(w.prompt_focused());
        w.input.focused=None;w.draw(&mut st).unwrap();assert!(w.input.caret_wake.is_none());
    }
    #[test]
    #[ignore="requires a Windows desktop; no file picker or provider request"]
    fn attachment_background_press_dismisses_without_sending_and_keeps_composer_focus(){
        let mut st=sample_state_for_selftest();st.ai.draft="Keep my draft".into();let mut w=AiWindow::new().unwrap();w.draw(&mut st).unwrap();
        st.ai.attach_menu=true;w.draw(&mut st).unwrap();
        let r=w.hits.iter().find(|h|h.id==wid("ai.send")).unwrap().rect;let scale=w.gfx.scale();
        let p=LPARAM(((((r.center().y*scale)as u16 as usize)<<16)|((r.center().x*scale)as u16 as usize))as isize);
        w.message(WM_LBUTTONDOWN,WPARAM(0),p);w.message(WM_LBUTTONUP,WPARAM(0),p);w.draw(&mut st).unwrap();
        assert!(!st.ai.attach_menu);assert!(w.prompt_focused());assert_eq!(st.ai.draft,"Keep my draft");
        assert!(!st.cmds.iter().any(|c|matches!(c,crate::commands::Command::Ai(crate::ai::Action::Send|crate::ai::Action::Attach(_)))));
        assert!(!w.hits.iter().any(|h|h.id==wid("ai.settings")));
        assert!(crate::pages::ai::next_wake(&st).is_none());
        st.ai.draft.clear();w.input.mouse=None;
        if let Some(dir)=std::env::var_os("LUMO_AI_QA"){w.capture(&mut st,&std::path::PathBuf::from(dir).join("ai-blank-composer.png"));}
    }
    #[test]
    #[ignore = "requires Windows DirectWrite and a desktop; never changes the clipboard"]
    fn mouse_positions_text_and_double_click_selects_entire_prompt_and_search() {
        let mut st=sample_state_for_selftest();
        st.ai.draft="First line\nSecond 🙂 line\nThird line".into();
        let mut w=AiWindow::new().unwrap();w.draw(&mut st).unwrap();
        let r=w.hits.iter().find(|h|h.id==wid("ai.prompt")).unwrap().rect;
        let inner=r.inset(3.0,0.0);let style=crate::ui::text_area_style();
        let target=15;let scale=w.gfx.scale();let bounds=unsafe{let mut b=RECT::default();GetClientRect(w.hwnd,&mut b).unwrap();b};
        let mut c=w.gfx.begin(bounds).unwrap();
        let(x,y,h)=c.text_caret(&st.ai.draft,&style,inner.w,target);
        let(_,last_y,last_h)=c.text_caret(&st.ai.draft,&style,inner.w,st.ai.draft.chars().count());
        let text_h=c.measure(&st.ai.draft,&style,inner.w).1;
        let at=Point::new(inner.x+x,inner.y+((inner.h-text_h)/2.0).max(0.0)-(last_y+last_h-inner.h+3.0).max(0.0)+y+h*0.45);
        drop(c);w.gfx.end().unwrap();
        let point=|p:Point|LPARAM((((p.y*scale).round()as u16 as usize)<<16|(p.x*scale).round()as u16 as usize)as isize);
        // A fast click can finish before Windows delivers the next paint.
        w.message(WM_LBUTTONDOWN,WPARAM(0),point(at));w.message(WM_LBUTTONUP,WPARAM(0),point(at));w.draw(&mut st).unwrap();
        assert_eq!(w.input.caret,target,"mouse click hits the second line, including UTF-16 glyph clusters");
        w.input.keys=vec![KeyInput::SelectRight,KeyInput::SelectRight,KeyInput::Cut];
        w.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&crate::commands::Command::CopyText("nd".into())));
        assert!(st.ai.draft.contains("Seco 🙂 line"));st.cmds.clear();
        w.message(WM_LBUTTONDBLCLK,WPARAM(0),point(at));w.message(WM_LBUTTONUP,WPARAM(0),point(at));w.draw(&mut st).unwrap();
        assert!(w.input.selection_all);assert_eq!(w.input.selection_anchor,Some(0));
        assert_eq!(w.input.caret,st.ai.draft.chars().count());
        w.input.keys=vec![KeyInput::Char('R')];w.draw(&mut st).unwrap();assert_eq!(st.ai.draft,"R");
        let field=wid("test.search");let rect=Rect::new(20.0,20.0,320.0,45.0);let mut text="Find this whole query 🙂".to_owned();let mut hits=vec![];
        let mut input=Input::default();let click=Point::new(rect.x+44.0,rect.center().y);
        input.just_pressed=true;input.press_pos=Some(click);input.released=Some((field,click));
        let mut c=w.gfx.begin(bounds).unwrap();
        {let mut ui=crate::ui::Ui::new(&mut c,&mut input,&mut hits,0.0);ui.text_input_capsule(field,rect,&mut text,"Search",100,st.accent);}
        assert_eq!(input.focused,Some(field));assert_eq!(input.caret,0);
        input.double_click=true;
        {let mut ui=crate::ui::Ui::new(&mut c,&mut input,&mut hits,0.0);ui.text_input_capsule(field,rect,&mut text,"Search",100,st.accent);}
        assert!(input.selection_all);assert_eq!((input.selection_anchor,input.caret),(Some(0),text.chars().count()));
        input.end_frame();input.keys=vec![KeyInput::Char('Q')];
        {let mut ui=crate::ui::Ui::new(&mut c,&mut input,&mut hits,0.0);ui.text_input_capsule(field,rect,&mut text,"Search",100,st.accent);}
        assert_eq!(text,"Q");
        input.end_frame();input.keys=vec![KeyInput::SelectAll,KeyInput::Backspace];
        {let mut ui=crate::ui::Ui::new(&mut c,&mut input,&mut hits,1.0);ui.text_input_capsule(field,rect,&mut text,"",100,st.accent);}
        assert!(text.is_empty());assert_eq!(input.focused,Some(field));assert!(input.caret_wake.is_none());assert!(input.caret_key.is_none());
        input.end_frame();input.keys=vec![KeyInput::Char('R')];
        {let mut ui=crate::ui::Ui::new(&mut c,&mut input,&mut hits,2.0);ui.text_input_capsule(field,rect,&mut text,"",100,st.accent);}
        assert_eq!(text,"R");assert!(input.caret_key.is_some(),"typing restores the Search caret");
        drop(c);w.gfx.end().unwrap();
    }
    #[test]
    #[ignore = "requires a Windows desktop and the system Segoe UI Emoji font"]
    fn color_emojis_render_in_plain_text_and_markdown() {
        let mut w=AiWindow::new().unwrap();w.gfx.set_dpi(96.0);w.gfx.set_surface(360,150).unwrap();
        let rect=RECT{left:0,top:0,right:360,bottom:150};w.gfx.request_capture(rect);
        let mut c=w.gfx.begin(rect).unwrap();
        c.fill_rect(Rect::new(0.0,0.0,360.0,150.0),Color::black(1.0));
        let style=crate::gfx::TextStyle::new(32.0).top();
        c.text("🙂 😊 🎉 👍🏽",Rect::new(12.0,8.0,336.0,56.0),&style);
        c.rich_text(&lumo_core::markdown::inline("**🙂** 😊 🎉 👍🏽"),Rect::new(12.0,78.0,336.0,56.0),&style);
        drop(c);w.gfx.end().unwrap();let pixels=w.gfx.take_capture();
        for (start,end) in [(0,75),(75,150)] {
            let colored=pixels[start*360*4..end*360*4].chunks_exact(4).filter(|p|p[3]>200&&p[2].abs_diff(p[0])>40&&p[2]>100).count();
            assert!(colored>100,"emoji row {start} must contain colored glyph pixels; found {colored}");
        }
        if let Some(dir)=std::env::var_os("LUMO_AI_QA"){crate::png::save_rgba(std::path::PathBuf::from(dir).join("color-emojis.png"),360,150,&crate::png::pbgra_to_rgba(&pixels)).unwrap();}
    }
    #[test]
    #[ignore = "requires a Windows desktop; offline composer preview fixture"]
    fn composer_previews_temporary_mode_and_right_aligned_model_fit_both_hosts() {
        let mut st = sample_state_for_selftest();
        st.ai.current.messages.clear();
        let mut w = AiWindow::new().unwrap();
        w.draw(&mut st).unwrap();
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(&mut st, &std::path::PathBuf::from(dir).join("ai-empty.png"));
        }
        crate::ai::local(&mut st, &crate::ai::Action::ToggleTemporary);
        w.draw(&mut st).unwrap();
        assert!(!w.hits.iter().any(|h| h.id == wid("ai.starter.0")));
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(&mut st, &std::path::PathBuf::from(dir).join("ai-temporary.png"));
        }
        let mut pixels = vec![0; 160 * 100 * 4];
        for y in 0..100 { for x in 0..160 {
            let n = (y * 160 + x) * 4;
            pixels[n..n+4].copy_from_slice(&[80 + y as u8, 100 + x as u8 / 2, 190, 255]);
        }}
        let image = crate::gfx::ImageData::new(160, 100, pixels);
        crate::ai::apply(&mut st, crate::ai::Event::Attached {
            result: Ok(vec![
                lumo_core::ai::Attachment { name: "Design reference.png".into(), mime: "image/png".into(), data: String::new() },
                lumo_core::ai::Attachment { name: "SHA256SUMS.txt".into(), mime: "text/plain".into(), data: String::new() },
            ]), previews: vec![Some(image), None],
        }, false);
        st.ai.draft = "Review this image and document together.".into();
        w.draw(&mut st).unwrap();
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(&mut st, &std::path::PathBuf::from(dir).join("ai-attachments.png"));
        }
        for n in 3..=4 {
            st.ai.pending.push(lumo_core::ai::Attachment { name: format!("Notes {n}.pdf"), mime: "application/pdf".into(), data: String::new() });
            st.ai.pending_previews.push(None);
        }
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for (width,height) in [(360,360),(420,500),(560,680)] {
                w.gfx.set_dpi(scale * 96.0);
                unsafe { SetWindowPos(w.hwnd,None,0,0,(width as f32 * scale) as i32,(height as f32 * scale) as i32,SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE).unwrap(); }
                w.draw(&mut st).unwrap();
                let find = |id| w.hits.iter().find(|h|h.id == id).unwrap().rect;
                let prompt = find(wid("ai.prompt"));
                let model = find(wid("ai.models"));
                let plus = find(wid("ai.plus"));
                let send = find(wid("ai.send"));
                assert!(model.x > plus.right() && model.right() < send.x);
                assert!((model.right() - (send.x - 8.0)).abs() < 2.0);
                for i in 0..4 { assert!(find(crate::ui::wid_n("ai.remove-attachment", i)).bottom() <= prompt.y); }
                assert!(w.hits.iter().all(|h|h.rect.x >= 0.0 && h.rect.y >= 0.0 && h.rect.right() <= width as f32 + 1.0 && h.rect.bottom() <= height as f32 + 1.0));
            }
        }
        w.input.clicked = Some((crate::ui::wid_n("ai.remove-attachment",0),MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert_eq!(st.ai.pending[0].name,"SHA256SUMS.txt");
        assert_eq!(st.ai.pending_previews.len(),3);
        assert!(st.ai.pending_previews[0].is_none());
        assert!(!st.cmds.iter().any(|c|matches!(c,crate::commands::Command::Ai(crate::ai::Action::Send))));
    }
    #[test]
    #[ignore = "requires a Windows desktop; uses an offline UI fixture"]
    fn workspace_starters_history_and_small_window_layout_remain_local() {
        let mut st = sample_state_for_selftest();
        st.ai.current.messages.clear();
        let mut window = AiWindow::new().unwrap();
        let configured = std::mem::take(&mut st.settings.ai.default_model);
        window.draw(&mut st).unwrap();
        assert!(window.hits.iter().any(|h| h.id == wid("ai.setup")));
        st.settings.ai.default_model = configured;
        window.draw(&mut st).unwrap();
        st.cmds.clear();
        window.input.clicked = Some((wid("ai.starter.0"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert_eq!(st.ai.draft, "Explain this in simple terms: ");
        assert!(window.prompt_focused());
        assert!(!st.cmds.iter().any(|c| matches!(
            c,
            crate::commands::Command::Ai(crate::ai::Action::Send | crate::ai::Action::Retry)
        )));
        st.ai.chats.push(st.ai.current.clone());
        st.ai.history = true;
        st.ai.history_query = "Oracle".into();
        window.draw(&mut st).unwrap();
        assert!(window
            .hits
            .iter()
            .any(|h| h.id == crate::ui::wid_n("ai.chat", st.ai.current.id)));
        st.ai.history_query = "no such chat 987".into();
        window.draw(&mut st).unwrap();
        assert!(!window
            .hits
            .iter()
            .any(|h| h.id == crate::ui::wid_n("ai.chat", st.ai.current.id)));
        st.ai.history = false;
        st.ai.error = "A sample provider error".into();
        st.ai.pending.push(lumo_core::ai::Attachment {
            name: "Notes.pdf".into(),
            mime: "application/pdf".into(),
            data: String::new(),
        });
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for (width, height) in [(360, 360), (420, 500), (560, 680)] {
                window.gfx.set_dpi(scale * 96.0);
                unsafe {
                    SetWindowPos(
                        window.hwnd,
                        None,
                        0,
                        0,
                        (width as f32 * scale).round() as i32,
                        (height as f32 * scale).round() as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .unwrap();
                }
                window.draw(&mut st).unwrap();
                for hit in &window.hits {
                    assert!(
                        hit.rect.x >= 0.0
                            && hit.rect.y >= 0.0
                            && hit.rect.right() <= width as f32 + 1.0
                            && hit.rect.bottom() <= height as f32 + 1.0,
                        "control exceeds {width}×{height} at {scale}x: {:?}",
                        hit.rect
                    );
                }
                for id in [
                    "ai.prompt",
                    "ai.open-full",
                    "ai.keep-open",
                    "ai.resume",
                    "ai.close",
                ] {
                    assert!(window.hits.iter().any(|h| h.id == wid(id)));
                }
                if width == 360 && scale == 1.0 {
                    if let Some(out) = std::env::var_os("LUMO_AI_QA") {
                        window.capture(
                            &mut st,
                            &std::path::PathBuf::from(out).join("ai-mini-small.png"),
                        );
                    }
                }
            }
        }
    }
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn starred_model_uses_default_then_reselection_opens_original_slider() {
        use crate::{commands::Command, ui::wid_n};
        let mut st = sample_state_for_selftest();
        st.settings.ai.provider = lumo_core::ai::Provider::Compatible;
        st.settings.ai.completion_url = "https://example.org/v1/chat/completions".into();
        st.ai.models[0].reasoning = lumo_core::ai_reasoning::Reasoning::parse(&serde_json::json!({
            "reasoning": {"supported_efforts": ["high", "low", "max"], "default_effort": "low"}
        }))
        .unwrap();
        let model_id = st.ai.models[0].id.clone();
        st.ai.selected = st.ai.models[1].id.clone();
        st.settings
            .ai
            .reasoning_efforts
            .insert(model_id.clone(), "max".into());
        let mut window = AiWindow::new().unwrap();
        window.input.clicked = Some((wid("ai.models"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.input.clicked = Some((wid_n("ai.model", 0), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert_eq!(st.ai.selected, model_id);
        assert!(
            !st.ai.model_menu && !st.ai.reasoning_menu,
            "switching to a supported model selects it directly"
        );
        assert!(
            !st.settings.ai.reasoning_efforts.contains_key(&model_id),
            "a new selection uses the model default even after an earlier override"
        );
        let (_, body) = lumo_core::ai::request(
            &st.settings.ai,
            &st.ai.model(&st.settings.ai),
            &st.ai.current.messages,
        )
        .unwrap();
        assert!(
            body.get("reasoning_effort").is_none(),
            "default means no forced effort in the request"
        );
        window.input.clicked = Some((wid("ai.models"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.input.clicked = Some((wid_n("ai.model", 0), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.draw(&mut st).unwrap();
        assert!(
            st.ai.model_menu && st.ai.reasoning_menu,
            "reselecting the active starred model opens its slider"
        );
        let slider = window
            .hits
            .iter()
            .find(|h| h.id == wid("ai.effort-slider"))
            .unwrap()
            .rect;
        let popup = window
            .hits
            .iter()
            .find(|h| h.id == wid("ai.model-surface"))
            .unwrap()
            .rect;
        assert!(
            popup.w <= 280.0 && popup.h <= 132.0,
            "restore the earlier compact popup"
        );
        let point = |p: Point, scale: f32| {
            LPARAM(
                (((p.y * scale).round() as u16 as usize) << 16
                    | (p.x * scale).round() as u16 as usize) as isize,
            )
        };
        let start = Point::new(slider.x + 12.0, slider.center().y);
        let end = Point::new(slider.right() - 12.0, slider.center().y);
        st.cmds.clear();
        window.message(WM_LBUTTONDOWN, WPARAM(0), point(start, window.gfx.scale()));
        window.draw(&mut st).unwrap();
        window.message(WM_MOUSEMOVE, WPARAM(0), point(end, window.gfx.scale()));
        window.draw(&mut st).unwrap();
        assert_eq!(
            st.settings
                .ai
                .reasoning_efforts
                .get(&model_id)
                .map(String::as_str),
            Some("max")
        );
        assert!(
            !st.cmds.contains(&Command::SaveSettings),
            "dragging does not write settings every frame"
        );
        window.message(WM_LBUTTONUP, WPARAM(0), point(end, window.gfx.scale()));
        window.draw(&mut st).unwrap();
        assert_eq!(
            st.cmds
                .iter()
                .filter(|c| **c == Command::SaveSettings)
                .count(),
            1
        );
        let (_, body) = lumo_core::ai::request(
            &st.settings.ai,
            &st.ai.model(&st.settings.ai),
            &st.ai.current.messages,
        )
        .unwrap();
        assert_eq!(body["reasoning_effort"], "max");
        if let Some(root) = std::env::var_os("LUMO_AI_QA") {
            let root = std::path::PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            window.capture(&mut st, &root.join("effort-slider.png"));
        }
        window.input.keys.push(KeyInput::Home);
        window.draw(&mut st).unwrap();
        assert_eq!(
            st.settings
                .ai
                .reasoning_efforts
                .get(&model_id)
                .map(String::as_str),
            Some("low")
        );
        window.input.clicked = Some((wid("ai.effort-reset"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(!st.settings.ai.reasoning_efforts.contains_key(&model_id));
        window.input.clicked = Some((wid("ai.effort-model"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(st.ai.model_menu && !st.ai.reasoning_menu);
        window.input.clicked = Some((wid_n("ai.model", 1), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(
            !st.ai.model_menu && !st.ai.reasoning_menu,
            "plain models still select directly"
        );
    }
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn model_picker_selection_filters_details_and_background_dismissal_work() {
        use crate::{ai::ModelRow, commands::Command};
        let mut st = sample_state_for_selftest();
        st.settings.ai.provider = lumo_core::ai::Provider::Compatible;
        st.settings.ai.completion_url = "https://example.org/v1/chat/completions".into();
        st.settings.ai.default_model = "openai/gpt-6.1-sol".into();
        st.ai.selected.clear();
        st.ai.models=lumo_core::ai::parse_models(&serde_json::json!({"data":[
            {"id":"openai/gpt-6.1-sol","name":"OpenAI: GPT-6.1 Sol","description":"A capable reasoning model for complex writing and code.","supported_parameters":["reasoning","tools","structured_outputs"],"reasoning":{"mandatory":true,"supported_efforts":["max","xhigh","high","medium","low"],"default_effort":"medium"},"architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]},"context_length":1050000,"pricing":{"prompt":"0.000002","completion":"0.000012"}},
            {"id":"liquid/lfm-2.5-2.6b:free","name":"LiquidAI: LFM2.5-2.6B (free)","description":"A compact model that reasons automatically. Adjustable effort is not supplied by this endpoint.","reasoning":{"mandatory":true},"architecture":{"input_modalities":["text"],"output_modalities":["text"]},"context_length":65536,"pricing":{"prompt":"0","completion":"0"}},
            {"id":"openai/free-effort:free","name":"OpenAI: Free effort model","architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]},"reasoning":{"supported_efforts":["low","high"]},"pricing":{"prompt":"0","completion":"0"}},
            {"id":"google/plain","name":"Google: Plain model","description":"A fast language model.","architecture":{"input_modalities":["text"],"output_modalities":["text"]}}
        ]})).unwrap();
        st.ai.catalog_revision += 1;
        let mut w = AiWindow::new().unwrap();
        w.draw(&mut st).unwrap();
        w.input.clicked = Some((wid("ai.models"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        w.draw(&mut st).unwrap();
        assert!(st.ai.model_menu && !st.ai.reasoning_menu);
        assert!(!w.hits.iter().any(|h| h.id == wid("ai.effort-slider")));
        assert!(
            w.hits
                .iter()
                .find(|h| h.id == wid("ai.model-surface"))
                .unwrap()
                .rect
                .w
                >= 450.0
        );
        let names = st
            .ai
            .model_rows
            .iter()
            .map(|r| {
                let ModelRow::Model(i) = r;
                st.ai.models[*i].display_name()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["Free effort model", "GPT-6.1 Sol", "LFM2.5-2.6B", "Plain model"]);
        assert!(!st.ai.models.iter().any(|m| m.id == "default"));
        let liquid = st
            .ai
            .models
            .iter()
            .position(|m| m.id.starts_with("liquid/"))
            .unwrap();
        assert_eq!(st.ai.models[liquid].free, Some(true));
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("provider-model-list.png"),
            );
        }
        w.input.mouse = None;
        w.input.clicked = Some((wid("ai.model-filters"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st.ai.model_filters_open);
        assert!([1, 2, 4, 8].into_iter().all(|mask| w
            .hits
            .iter()
            .any(|h| h.id == crate::ui::wid_n("ai.model-filter", mask))));
        assert!(
            !w.hits
                .iter()
                .any(|h| h.id == crate::ui::wid_n("ai.model", liquid as u64)),
            "the filter menu blocks clicks on models behind it"
        );
        for mask in [1, 2, 4] {
            w.input.clicked = Some((crate::ui::wid_n("ai.model-filter", mask), MouseButton::Left));
            w.draw(&mut st).unwrap();
        }
        w.draw(&mut st).unwrap();
        assert_eq!(st.ai.model_filters, 7);
        let visible = st
            .ai
            .model_rows
            .iter()
            .map(|row| {
                let ModelRow::Model(i) = row;
                st.ai.models[*i].id.as_str()
            })
            .collect::<Vec<_>>();
        assert_eq!(visible, ["openai/free-effort:free"]);
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("model-filters.png"),
            );
        }
        w.input.clicked = Some((wid("ai.model-filter-done"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        w.draw(&mut st).unwrap();
        assert!(!st.ai.model_filters_open);
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("models-filtered.png"),
            );
        }
        w.input.clicked = Some((wid("ai.model-filters"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        w.input.clicked = Some((wid("ai.model-filter-clear"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert_eq!(st.ai.model_filters, 0);
        w.input.clicked = Some((wid("ai.model-filter-done"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        w.draw(&mut st).unwrap();
        w.input.clicked = Some((
            crate::ui::wid_n("ai.model-info", liquid as u64),
            MouseButton::Left,
        ));
        w.draw(&mut st).unwrap();
        w.draw(&mut st).unwrap();
        assert!(st.ai.model_detail.is_some());
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("model-details.png"),
            );
        }
        w.input.mouse = Some(Point::new(490.0, 350.0));
        w.input.wheel = -8.0;
        w.draw(&mut st).unwrap();
        assert!(st.ai.model_detail_scroll > 0.0);
        w.input.clicked = Some((wid("ai.model-detail-select"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert_eq!(st.ai.selected, st.ai.models[liquid].id);
        assert!(!st.ai.model_menu);
        w.input.clicked = Some((wid("ai.models"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(
            st.ai.model_menu && !st.ai.reasoning_menu,
            "fixed reasoning opens model selection directly"
        );
        st.ai.model_menu = false;
        st.ai.selected.clear();
        assert_eq!(
            st.ai.model(&st.settings.ai).id,
            st.settings.ai.default_model,
            "the configured default is the actual initial model"
        );
        // Background dismissal consumes the press, including over Send.
        st.ai.draft = "Do not send this".into();
        st.ai.model_menu = true;
        w.draw(&mut st).unwrap();
        let send = w.hits.iter().find(|h| h.id == wid("ai.send")).unwrap().rect;
        w.input.mouse = Some(send.center());
        w.input.just_pressed = true;
        w.input.pressed_id = Some(wid("ai.send"));
        w.input.down = true;
        st.cmds.clear();
        w.draw(&mut st).unwrap();
        assert!(!st.ai.model_menu && w.input.pressed_id.is_none());
        assert!(!st.cmds.contains(&Command::Ai(crate::ai::Action::Send)));
        w.input.clicked = Some((wid("ai.plus"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st.ai.attach_menu && !st.ai.model_menu);
        assert!((0..3).all(|i| w
            .hits
            .iter()
            .any(|h| h.id == crate::ui::wid_n("ai.attach", i))));
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("attachments-menu.png"),
            );
        }
        st.cmds.clear();
        w.input.clicked = Some((crate::ui::wid_n("ai.attach", 1), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st
            .cmds
            .contains(&Command::Ai(crate::ai::Action::Attach(false))));
        assert!(!st.ai.attach_menu);
    }
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn full_chat_renders_and_accepts_keyboard_input() {
        let mut st = sample_state_for_selftest();
        let mut w = AiWindow::new().unwrap();
        let monitor = crate::window::pick_monitor(lumo_core::settings::MonitorChoice::Cursor);
        let mut bounds = RECT::default();
        unsafe {
            GetWindowRect(w.hwnd, &mut bounds).unwrap();
        }
        assert!(
            bounds.right <= monitor.work.right && monitor.work.right - bounds.right <= 24,
            "mini window opens at the right edge"
        );
        assert!(
            bounds.top >= monitor.work.top && bounds.top - monitor.work.top <= 24,
            "mini window opens at the top of the work area"
        );
        w.draw(&mut st).unwrap();
        assert!(w.hits.iter().any(|h| h.id == wid("ai.prompt")));
        assert!(
            !w.hits.iter().any(|h| h.id == wid("ai.send")),
            "empty draft does not offer Send"
        );
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            w.capture(&mut st, &dir.join("full-chat.png"));
            st.ai.model_menu = true;
            w.capture(&mut st, &dir.join("full-models.png"));
            st.ai.model_menu = false;
        }
        st.ai.current.messages[1].text = markdown_fixture().into();
        st.ai.scroll = 0.0;
        w.draw(&mut st).unwrap();
        assert!(!w.hits.iter().any(|h| h.id == wid("ai.copy.1")));
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("full-markdown-top.png"),
            );
        }
        w.input.mouse = Some(Point::new(90.0, 230.0));
        st.ai.scroll = f32::MAX;
        w.draw(&mut st).unwrap();
        assert!(w.hits.iter().any(|h| h.id == wid("ai.copy.1")));
        st.cmds.clear();
        w.input.clicked = Some((wid("ai.markdown.1"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&crate::commands::Command::CopyText(
            markdown_fixture().into()
        )));
        let block_copy = crate::ui::wid_n("ai.markdown.copy", 2 * 8191);
        assert!(w.hits.iter().any(|h| h.id == block_copy));
        st.cmds.clear();
        w.input.clicked = Some((block_copy, MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&crate::commands::Command::CopyText(
            "Pads → gentle sidechain\nPiano → short room reverb".into()
        )));
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("full-markdown.png"),
            );
        }
        let link = w
            .hits
            .iter()
            .find(|h| h.id == crate::ui::wid_n("ai.markdown.link", 2 * 8191))
            .unwrap()
            .rect;
        w.input.mouse = Some(link.center());
        w.input.clicked = Some((
            crate::ui::wid_n("ai.markdown.link", 2 * 8191),
            MouseButton::Left,
        ));
        st.cmds.clear();
        w.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&crate::commands::Command::OpenUrl(
            "https://example.org/session".into()
        )));
        st.cmds.clear();
        w.input.mouse = None;
        let long_code = (0..100)
            .map(|n| format!("    value_{n} = {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        st.ai.current.messages[1].text = format!("```python\n{long_code}\n```");
        st.ai.scroll = 500.0;
        w.draw(&mut st).unwrap();
        let copy = w.hits.iter().find(|h| h.id == block_copy).unwrap().rect;
        assert!(
            copy.y >= w.hits.iter().find(|h|h.id==wid("ai.history")).unwrap().rect.bottom()+10.0 && copy.y < 160.0,
            "long blocks keep Copy at the viewport top"
        );
        w.input.clicked = Some((block_copy, MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st
            .cmds
            .contains(&crate::commands::Command::CopyText(long_code)));
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("long-code-copy.png"),
            );
        }
        st.cmds.clear();
        st.ai.current.messages[1].text = "$$\\frac{a_b}{c^2}$$".into();
        st.ai.scroll = 0.0;
        w.draw(&mut st).unwrap();
        w.input.clicked = Some((block_copy, MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&crate::commands::Command::CopyText(
            r"\frac{a_b}{c^2}".into()
        )));
        st.cmds.clear();
        st.ai.busy = false;
        st.settings.ai = Default::default();
        w.draw(&mut st).unwrap();
        for id in ["ai.new", "ai.temporary"] {
            let r = w.hits.iter().find(|h| h.id == wid(id)).unwrap().rect;
            assert!(r.w <= 32.0, "chat actions use compact icon-only targets");
            w.input.mouse = Some(r.center());
            if id == "ai.temporary" {
                st.ai.temporary = true;
            }
            if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
                w.capture(
                    &mut st,
                    &std::path::PathBuf::from(dir).join(format!("{id}-hover.png")),
                );
            }
            w.input.clicked = Some((wid(id), MouseButton::Left));
            w.draw(&mut st).unwrap();
            let expected = if id == "ai.temporary" {
                crate::ai::Action::ToggleTemporary
            } else {
                crate::ai::Action::New(false)
            };
            assert!(st.cmds.contains(&crate::commands::Command::Ai(expected)));
            st.cmds.clear();
        }
        st.ai.temporary = false;
        let previous = st.ai.current.clone();
        for enabled in [true, false] {
            w.input.clicked = Some((wid("ai.temporary"), MouseButton::Left));
            w.draw(&mut st).unwrap();
            assert!(st.cmds.contains(&crate::commands::Command::Ai(
                crate::ai::Action::ToggleTemporary
            )));
            st.cmds.clear();
            crate::ai::local(&mut st, &crate::ai::Action::ToggleTemporary);
            st.cmds.clear();
            assert_eq!(
                st.ai.temporary, enabled,
                "each click switches the temporary mode"
            );
            w.input.mouse = Some(
                w.hits
                    .iter()
                    .find(|h| h.id == wid("ai.temporary"))
                    .unwrap()
                    .rect
                    .center(),
            );
            if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
                w.capture(
                    &mut st,
                    &std::path::PathBuf::from(dir).join(if enabled {
                        "temporary-checked.png"
                    } else {
                        "temporary-unchecked.png"
                    }),
                );
            }
        }
        st.ai.current = previous;
        w.input.mouse = None;
        st.ai.busy = true;
        st.ai.generation = crate::ai::Generation::Working;
        st.ai.running_chat = st.ai.current.id;
        st.ai.generation_started = 90.0;
        w.draw(&mut st).unwrap();
        w.draw(&mut st).unwrap();
        assert!(!w.hits.iter().any(|h|h.id==wid("ai.generation")));
        w.input.clicked=Some((wid("ai.send"),MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st.cmds.contains(&crate::commands::Command::Ai(crate::ai::Action::Stop)));
        st.cmds.clear();
        st.ai.busy=false;
        st.ai.generation = crate::ai::Generation::Failed;
        st.ai.error = "HTTP 503: Service unavailable".into();
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(&dir).join("generation-failed.png"),
            );
            st.ai.error = "HTTP 429: Provider returned error".into();
            let prompt = w
                .hits
                .iter()
                .find(|h| h.id == wid("ai.prompt"))
                .unwrap()
                .rect;
            w.input.mouse = Some(Point::new(prompt.x, prompt.y - 24.0));
            w.capture(
                &mut st,
                &std::path::PathBuf::from(&dir).join("provider-error-near-message.png"),
            );
            w.input.mouse = Some(
                w.hits
                    .iter()
                    .find(|h| h.id == wid("ai.temporary"))
                    .unwrap()
                    .rect
                    .center(),
            );
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("temporary-hover-with-provider-error.png"),
            );
        }
        st.ai.error.clear();
        let mut other = st.ai.current.clone();
        other.id = other.id + 100;
        st.ai.running_chat = other.id;
        st.ai.chats.push(other.clone());
        st.ai.busy = true;
        w.draw(&mut st).unwrap();
        assert!(!w.hits.iter().any(|h|h.id==wid("ai.generation")));
        assert!(!st.cmds.iter().any(|c|matches!(c,crate::commands::Command::Ai(crate::ai::Action::Select(_)))));
        st.cmds.clear();
        st.ai.busy = false;
        st.ai.generation = crate::ai::Generation::Idle;
        w.input.mouse = None;
        let base = st.ai.models[0].clone();
        st.ai.models = (0..80)
            .map(|i| {
                let mut m = base.clone();
                m.id = format!("sample/{i}");
                m.name = format!("Model {i}");
                m
            })
            .collect();
        st.ai.model_menu = true;
        st.ai.model_settings = false;
        st.ai.scroll = 0.0;
        w.draw(&mut st).unwrap();
        w.input.mouse = Some(
            w.hits
                .iter()
                .find(|h| h.id == wid("ai.model-surface"))
                .unwrap()
                .rect
                .center(),
        );
        w.input.wheel = -3.0;
        w.draw(&mut st).unwrap();
        assert!(st.ai.model_scroll > 0.0, "model lists must scroll");
        assert_eq!(
            st.ai.scroll, 0.0,
            "the transcript behind the selector must stay still"
        );
        st.ai.model_menu = false;
        w.input.focused = Some(wid("ai.prompt"));
        w.input.keys = vec![
            KeyInput::SelectAll,
            KeyInput::Char('H'),
            KeyInput::Char('i'),
            KeyInput::Char('\n'),
            KeyInput::Char('!'),
        ];
        w.draw(&mut st).unwrap();
        assert_eq!(st.ai.draft, "Hi\n!");
        w.input.keys = vec![KeyInput::Enter];
        w.draw(&mut st).unwrap();
        assert!(st
            .cmds
            .contains(&crate::commands::Command::Ai(crate::ai::Action::Send)));
        push_char(&mut w.input, 0xd83d);
        push_char(&mut w.input, 0xde00);
        assert!(w.input.keys.contains(&KeyInput::Char('😀')));
    }
}
pub(crate) fn sample_state_for_selftest() -> AppState {
    let mut st = AppState::new(Default::default(), Default::default());
    st.settings.set_addon("ai", true);
    st.now = 100.0;
    st.ai.loaded = true;
    st.settings.ai.default_model = "sample/fast".into();
    st.ai.models=lumo_core::ai::parse_models(&serde_json::json!({"data":[{"id":"sample/fast","name":"Fast assistant","description":"A small, responsive model for everyday questions and writing.","architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]},"context_length":128000,"pricing":{"prompt":"0","completion":"0"}},{"id":"sample/art","name":"Image studio","architecture":{"output_modalities":["image"]},"pricing":{"prompt":"0.000001","completion":"0.000002"}},{"id":"sample/embedding","name":"Text embeddings","pricing":{"prompt":"0.0000001","completion":"0"}}]})).unwrap();
    st.ai.current = lumo_core::ai::Chat {
        id: 1,
        title: "Free RAM on Oracle".into(),
        model: "sample/fast".into(),
        messages: vec![
            lumo_core::ai::Message {
                role: "user".into(),
                text: "How do I free RAM on a 1 GB Oracle instance?".into(),
                ..Default::default()
            },
            lumo_core::ai::Message {
                role: "assistant".into(),
                text: "Add a 2 GB swap file, then stop unused services. Want the exact commands?"
                    .into(),
                ..Default::default()
            },
        ],
    };
    st
}

#[cfg(test)]
pub(crate) fn markdown_fixture() -> &'static str {
    r#"# 🎧 Electric Echoes

Inspired by **glossy synth pop** — a *piano-led* bridge and a bright final drop.

## Song structure

- Intro: atmospheric pads and a faint melodic synth
- Chorus: **full drop**, layered vocals and bass

---

## Production notes

| Element | Suggested settings |
| --- | --- |
| Tempo | **80–84 BPM**, steady and bright |
| Synths | Soft plucks, sparkling arpeggios and layered reverb |
| Bass | Smooth sub-bass on the downbeat |

> Keep the lead vocal centered and leave space around the synths.

### Session notes

Use `sidechain_amount` gently, then compare:

```text
Pads → gentle sidechain
Piano → short room reverb
```

Read the [session guide](https://example.org/session).
"#
}
