//! On-demand mini browser chrome. The existing WebView host moves between
//! this window and the island; no second browser, device, timer or worker.
use crate::{
    commands::Command,
    gfx::Gfx,
    state::AppState,
    ui::{wid, Hit, Input, KeyInput, MouseButton, Ui},
};
use lumo_core::geom::{Color, Point, Rect};
use windows::{
    core::{w, Result},
    Win32::{
        Foundation::*,
        Graphics::Gdi::{MonitorFromWindow, ScreenToClient, MONITOR_DEFAULTTONEAREST},
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};
const WM_MOUSELEAVE_: u32 = 0x02a3;

pub struct BrowserWindow {
    pub hwnd: HWND,
    gfx: Gfx,
    input: Input,
    hits: Vec<Hit>,
    dirty: bool,
    shown: bool,
    activated: bool,
    #[cfg(test)]
    pub paints: usize,
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result)=crate::search_menu::message(msg,wp,lp){return result;}
    if msg == WM_NCCALCSIZE {
        return LRESULT(0);
    }
    if matches!(msg, WM_NCACTIVATE | WM_ERASEBKGND) {
        return LRESULT(1);
    }
    if let Some(Some(result)) =
        crate::app::with_app(|app| app.handle_browser_window(hwnd, msg, wp, lp))
    {
        return result;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
impl BrowserWindow {
    pub fn new(anchor: HWND, config: &lumo_core::search::Config) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("Lumo.MiniBrowser");
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
            let monitor =
                crate::window::monitor_info(MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST))
                    .unwrap_or_else(|| {
                        crate::window::pick_monitor(lumo_core::settings::MonitorChoice::Cursor)
                    });
            let available_w = (monitor.work.right - monitor.work.left - 32).max(1);
            let available_h = (monitor.work.bottom - monitor.work.top - 32).max(1);
            let scale = monitor
                .scale()
                .min(available_w as f32 / 280.0)
                .min(available_h as f32 / 200.0)
                .max(0.25);
            let width = (f32::from(config.mini_width.max(280)) * scale)
                .round()
                .min(available_w as f32) as i32;
            let height = (f32::from(config.mini_height.max(200)) * scale)
                .round()
                .min(available_h as f32) as i32;
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!("Lumo Mini Browser"),
                WS_POPUP | WS_THICKFRAME | WS_SYSMENU,
                monitor.work.right - width - 16,
                monitor.work.top + 16,
                width,
                height,
                None,
                None,
                instance,
                None,
            )?;
            let gfx = match Gfx::static_window(scale * 96.0, hwnd) {
                Ok(gfx) => gfx,
                Err(error) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(error);
                }
            };
            Ok(Self {
                hwnd,
                gfx,
                input: Input::default(),
                hits: vec![],
                dirty: true,
                shown: false,
                activated: false,
                #[cfg(test)]
                paints: 0,
            })
        }
    }
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
    pub fn apply_size(&mut self, config: &lumo_core::search::Config) {
        unsafe {
            let Some(m) =
                crate::window::monitor_info(MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST))
            else {
                return;
            };
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            let s = self.gfx.scale();
            let w = (f32::from(config.mini_width) * s)
                .round()
                .min((m.work.right - m.work.left - 16) as f32) as i32;
            let h = (f32::from(config.mini_height) * s)
                .round()
                .min((m.work.bottom - m.work.top - 16) as f32) as i32;
            if r.right - r.left != w || r.bottom - r.top != h {
                let x = r.left.clamp(m.work.left, m.work.right - w);
                let y = r.top.clamp(m.work.top, m.work.bottom - h);
                let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                self.dirty = true;
            }
        }
    }
    pub fn focus(&self) {
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(self.hwnd);
        }
    }
    pub fn focus_address(&mut self, len: usize) {
        self.input.focused = Some(wid("search.query"));
        self.input.caret = len;
        self.input.selection_all = true;
        self.input.selection_anchor = Some(0);
        self.dirty = true;
        self.focus();
    }
    pub fn blur_address(&mut self) {
        self.input.focused = None;
        self.dirty = true;
    }
    pub fn hide(&mut self) {
        if self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
            self.dirty = true;
        }
    }
    pub fn page_bounds(&self) -> (RECT, f32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut r);
        }
        let s = self.gfx.scale();
        let page = crate::search::snapped_web_rect(
            Rect::new(0.0, 0.0, r.right as f32 / s, r.bottom as f32 / s),
            s,
        );
        (
            RECT {
                left: (page.x * s).round() as i32,
                top: (page.y * s).round() as i32,
                right: (page.right() * s).round() as i32,
                bottom: (page.bottom() * s).round() as i32,
            },
            s,
        )
    }
    pub fn draw(&mut self, st: &mut AppState) -> Result<bool> {
        if !self.dirty && self.shown {
            return Ok(false);
        }
        self.dirty = false;
        #[cfg(test)]
        {
            self.paints += 1;
        }
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
        let mut c = self.gfx.begin(RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        })?;
        c.fill_rrect(area, 24.0, Color::black(1.0));
        c.stroke_rrect(area.inset(0.5, 0.5), 24.0, Color::white(0.15), 1.0);
        self.hits.clear();
        let mut ui = Ui::new(&mut c, &mut self.input, &mut self.hits, st.now);
        crate::search::draw_browser(&mut ui, st, area);
        let repaint = ui.repaint_requested();
        if let Some(text) = ui.input.clipboard_write.take() {
            st.cmd(Command::CopyText(text));
        }
        drop(ui);
        drop(c);
        self.gfx.end()?;
        self.gfx.present(bounds.left, bounds.top)?;
        self.input.end_frame();
        if !self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.shown = true;
        }
        if !self.activated {
            self.focus();
            self.activated = true;
        }
        self.dirty = repaint;
        Ok(repaint)
    }
    #[cfg(test)]
    pub fn test_resize(&mut self, scale: f32) {
        self.gfx.set_dpi(scale * 96.0);
        self.dirty = true;
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                (620.0 * scale).round() as i32,
                (420.0 * scale).round() as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
    }
    #[cfg(test)]
    pub fn capture(&mut self, st: &mut AppState) -> (i32, i32, Vec<u8>) {
        let mut r = RECT::default();
        unsafe {
            GetWindowRect(self.hwnd, &mut r).unwrap();
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        self.dirty = true;
        self.gfx.request_capture(RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        });
        self.draw(st).unwrap();
        (w, h, self.gfx.take_capture())
    }
    #[cfg(test)]
    pub fn capture_screen(&mut self, st: &mut AppState, path: &std::path::Path) {
        use windows::Win32::Graphics::Gdi::*;
        let (w, h, mask) = self.capture(st);
        crate::png::save_rgba(
            path.with_file_name("mini-browser-chrome.png"),
            w as u32,
            h as u32,
            &crate::png::pbgra_to_rgba(&mask),
        )
        .unwrap();
        unsafe {
            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
            let mut r = RECT::default();
            GetWindowRect(self.hwnd, &mut r).unwrap();
            let dc = GetDC(None);
            let mem = CreateCompatibleDC(dc);
            let mut bits = std::ptr::null_mut();
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(mem, bitmap);
            let s = self.gfx.scale();
            let close = crate::search::browser_toolbar(
                Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s),
                true,
            )
            .close
            .unwrap();
            let point = ((close.y * s) as i32..(close.bottom() * s) as i32)
                .find_map(|y| {
                    ((close.x * s) as i32..(close.right() * s) as i32)
                        .find(|&x| mask[((y * w + x) * 4) as usize] > 120)
                        .map(|x| (x, y))
                })
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                let pixel = GetPixel(dc, r.left + point.0, r.top + point.1);
                if pixel.0 & 255 > 60 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "mini browser close icon must be visible on the desktop"
                );
                let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            // Sample after DWM has presented the complete toolbar. Copying
            // immediately after UpdateLayeredWindow can read the previous frame.
            BitBlt(mem, 0, 0, w, h, dc, r.left, r.top, SRCCOPY | CAPTUREBLT).unwrap();
            let _ = GdiFlush();
            let mut pixels =
                std::slice::from_raw_parts(bits.cast::<u8>(), (w * h * 4) as usize).to_vec();
            for (i, p) in pixels.chunks_exact_mut(4).enumerate() {
                p[3] = if mask[i * 4 + 3] > 0 { 255 } else { 0 };
                if p[3] == 0 {
                    p.fill(0);
                }
            }
            crate::png::save_rgba(
                path,
                w as u32,
                h as u32,
                &crate::png::pbgra_to_rgba(&pixels),
            )
            .unwrap();
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(mem);
            ReleaseDC(None, dc);
        }
    }
    pub fn message(
        &mut self,
        st: &mut AppState,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<LRESULT> {
        unsafe {
            self.input.ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            self.input.shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            match msg {
                WM_NCHITTEST => {
                    let mut p = POINT {
                        x: lp.0 as i16 as i32,
                        y: (lp.0 >> 16) as i16 as i32,
                    };
                    let _ = ScreenToClient(self.hwnd, &mut p);
                    let mut r = RECT::default();
                    let _ = GetClientRect(self.hwnd, &mut r);
                    let s = self.gfx.scale();
                    let e = (6.0 * s).round() as i32;
                    let hit = match (p.x < e, p.x >= r.right - e, p.y < e, p.y >= r.bottom - e) {
                        (true, _, true, _) => HTTOPLEFT,
                        (_, true, true, _) => HTTOPRIGHT,
                        (true, _, _, true) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, _, _, _) => HTLEFT,
                        (_, true, _, _) => HTRIGHT,
                        (_, _, true, _) => HTTOP,
                        (_, _, _, true) => HTBOTTOM,
                        _ if p.y
                            < (72.0*s) as i32
                            && !self.hits.iter().any(|h| {
                                h.rect.contains(Point::new(p.x as f32 / s, p.y as f32 / s))
                            }) =>
                        {
                            HTCAPTION
                        }
                        _ => HTCLIENT,
                    };
                    return Some(LRESULT(hit as isize));
                }
                WM_GETMINMAXINFO => {
                    let monitor = crate::window::monitor_info(MonitorFromWindow(
                        self.hwnd,
                        MONITOR_DEFAULTTONEAREST,
                    ))?;
                    let info = &mut *(lp.0 as *mut MINMAXINFO);
                    let s = self.gfx.scale();
                    info.ptMaxTrackSize = POINT {
                        x: monitor.work.right - monitor.work.left,
                        y: monitor.work.bottom - monitor.work.top,
                    };
                    info.ptMinTrackSize = POINT {
                        x: (280.0 * s).round().min(info.ptMaxTrackSize.x as f32) as i32,
                        y: (200.0 * s).round().min(info.ptMaxTrackSize.y as f32) as i32,
                    };
                    return Some(LRESULT(0));
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
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP => {
                    let s = self.gfx.scale();
                    let p = Point::new(lp.0 as i16 as f32 / s, (lp.0 >> 16) as i16 as f32 / s);
                    self.input.mouse = Some(p);
                    if msg == WM_LBUTTONDOWN || msg == WM_LBUTTONDBLCLK {
                        self.input.double_click = msg == WM_LBUTTONDBLCLK;
                        self.input.down = true;
                        self.input.just_pressed = true;
                        self.input.press_pos = Some(p);
                        self.input.pressed_id = self
                            .hits
                            .iter()
                            .rev()
                            .find(|h| h.rect.contains(p))
                            .map(|h| h.id);
                        SetCapture(self.hwnd);
                    } else if msg == WM_LBUTTONUP {
                        let hit = self
                            .hits
                            .iter()
                            .rev()
                            .find(|h| h.rect.contains(p))
                            .map(|h| h.id);
                        if hit == self.input.pressed_id {
                            self.input.clicked = hit.map(|id| (id, MouseButton::Left));
                        }
                        self.input.released = self.input.pressed_id.map(|id| (id, p));
                        self.input.down = false;
                        self.input.pressed_id = None;
                        let _ = ReleaseCapture();
                    }
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        ..Default::default()
                    };
                    let _ = TrackMouseEvent(&mut track);
                }
                WM_MOUSELEAVE_ | WM_CAPTURECHANGED => {
                    self.input.mouse = None;
                    self.input.down = false;
                    self.input.pressed_id = None;
                }
                WM_SETCURSOR if lp.0 as u16 as u32 == HTCLIENT => {
                    let hit = self
                        .input
                        .mouse
                        .and_then(|p| self.hits.iter().rev().find(|h| h.rect.contains(p)));
                    SetCursor(
                        LoadCursorW(
                            None,
                            if hit.is_some_and(|h| h.id == wid("search.query")) {
                                IDC_IBEAM
                            } else if hit.is_some() {
                                IDC_HAND
                            } else {
                                IDC_ARROW
                            },
                        )
                        .unwrap_or_default(),
                    );
                    return Some(LRESULT(1));
                }
                WM_CHAR => crate::ai_window::push_char(&mut self.input, wp.0 as u32),
                WM_PASTE => crate::ai_window::paste_text(&mut self.input, self.hwnd),
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if let Some(action) = crate::search_web::Action::from_key(
                        wp.0 as u32,
                        GetKeyState(VK_CONTROL.0 as i32) < 0,
                        GetKeyState(VK_MENU.0 as i32) < 0,
                    ) {
                        st.cmd(Command::SearchWeb(action));
                    } else if msg == WM_SYSKEYDOWN {
                        return None;
                    } else if wp.0 == VK_ESCAPE.0 as usize {
                        if self.input.focused==Some(wid("search.query")){self.input.keys.push(KeyInput::Escape);}else{st.cmd(Command::CloseMiniBrowser);}
                    } else if GetKeyState(VK_CONTROL.0 as i32) < 0 && wp.0 == b'C' as usize {
                        self.input.keys.push(KeyInput::Copy);
                    } else if GetKeyState(VK_CONTROL.0 as i32) < 0 && wp.0 == b'X' as usize {
                        self.input.keys.push(KeyInput::Cut);
                    } else if self.input.shift
                        && [VK_LEFT.0, VK_RIGHT.0, VK_HOME.0, VK_END.0].contains(&(wp.0 as u16))
                    {
                        self.input.keys.push(match VIRTUAL_KEY(wp.0 as u16) {
                            VK_LEFT => KeyInput::SelectLeft,
                            VK_RIGHT => KeyInput::SelectRight,
                            VK_HOME => KeyInput::SelectHome,
                            _ => KeyInput::SelectEnd,
                        });
                    } else if matches!(wp.0 as u16,0x26|0x28){
                        self.input.keys.push(if wp.0 as u16==0x26{KeyInput::Up}else{KeyInput::Down});
                    } else if wp.0 == VK_RETURN.0 as usize {
                        self.input.keys.push(KeyInput::Enter);
                    } else {
                        crate::ai_window::text_key(&mut self.input, self.hwnd, wp);
                    }
                }
                WM_PAINT => {
                    let _ = windows::Win32::Graphics::Gdi::ValidateRect(self.hwnd, None);
                }
                WM_CLOSE => st.cmd(Command::CloseMiniBrowser),
                WM_KILLFOCUS=>{self.input.focused=None;st.search.address_editing=false;st.search.address_suggestions.clear();st.search.suggestions.cancel();},
                WM_EXITSIZEMOVE => {
                    let mut r = RECT::default();
                    let _ = GetWindowRect(self.hwnd, &mut r);
                    let s = self.gfx.scale();
                    st.settings.search.mini_width =
                        ((r.right - r.left) as f32 / s).round().clamp(280.0, 8192.0) as u16;
                    st.settings.search.mini_height =
                        ((r.bottom - r.top) as f32 / s).round().clamp(200.0, 8192.0) as u16;
                    st.cmd(Command::SaveSettings);
                }
                WM_SIZE | WM_MOVE => {}
                _ => return None,
            }
        }
        self.dirty = true;
        Some(LRESULT(0))
    }
}
impl Drop for BrowserWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
