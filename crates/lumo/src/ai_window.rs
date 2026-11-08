//! Full chat is a native, resizable window, constructed only when opened.
use crate::{
    gfx::{Gfx, TextStyle},
    state::AppState,
    ui::{wid, Hit, Input, KeyInput, MouseButton},
};
use lumo_core::geom::{palette, Color, Point, Rect};
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
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(Some(r)) = crate::app::with_app(|app| app.handle_ai_window(hwnd, msg, wp, lp)) {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
impl AiWindow {
    pub fn new() -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("Lumo.AiChat");
            let wc = WNDCLASSEXW {
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
            let w =
                (600.0 * scale).min((monitor.work.right - monitor.work.left - 24) as f32) as i32;
            let h =
                (680.0 * scale).min((monitor.work.bottom - monitor.work.top - 24) as f32) as i32;
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_APPWINDOW,
                class,
                w!("Ask AI — Lumo"),
                WS_POPUP | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX,
                monitor.work.left + (monitor.work.right - monitor.work.left - w) / 2,
                monitor.work.bottom - h - 12,
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
            })
        }
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
                monitor.work.left + (monitor.work.right - monitor.work.left - width) / 2,
                monitor.work.bottom - height - gap,
                width,
                height,
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.invalidate();
    }
    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, false);
        }
    }
    pub fn focus_prompt(&mut self, len: usize) {
        self.input.focused = Some(wid("ai.prompt"));
        self.input.caret = len;
        self.invalidate();
    }
    pub fn prompt_focused(&self) -> bool {
        self.input.focused == Some(wid("ai.prompt"))
    }
    pub fn draw(&mut self, st: &mut AppState) -> Result<()> {
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
        ui.c.fill_rrect(
            area.inset(1.0, 1.0),
            18.0,
            Color::rgba(0.025, 0.025, 0.03, 1.0),
        );
        ui.c.stroke_rrect(area.inset(1.0, 1.0), 18.0, Color::white(0.15), 1.0);
        ui.c.icon(
            crate::icons::Icon::Sparkles,
            Rect::new(24.0, 18.0, 22.0, 22.0),
            palette::BLUE,
        );
        ui.c.text(
            "Ask AI",
            Rect::new(58.0, 10.0, area.w - 130.0, 40.0),
            &TextStyle::headline(),
        );
        if ui
            .icon_button(
                wid("ai.window-close"),
                Rect::new(area.w - 48.0, 12.0, 32.0, 32.0),
                crate::icons::Icon::Close,
                palette::TEXT_DIM,
                16.0,
            )
            .clicked
        {
            unsafe {
                let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        crate::pages::ai::draw_chat(
            &mut ui,
            st,
            Rect::new(24.0, 65.0, area.w - 48.0, area.h - 89.0),
            true,
        );
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
        drop(ui);
        unsafe {
            let _ = KillTimer(self.hwnd, 73);
            if moving || live {
                // Pending reply dots need only 30 fps; finite transitions
                // retain their usual cadence. No timer while idle.
                SetTimer(self.hwnd, 73, if moving { 16 } else { 33 }, None);
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
    fn capture(&mut self, st: &mut AppState, path: &std::path::Path) {
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
                        x: (560.0 * self.gfx.scale()) as i32,
                        y: (400.0 * self.gfx.scale()) as i32,
                    };
                    return Some(LRESULT(0));
                }
                WM_NCCALCSIZE => return Some(LRESULT(0)),
                WM_NCHITTEST => {
                    let mut p = POINT {
                        x: lp.0 as i16 as i32,
                        y: (lp.0 >> 16) as i16 as i32,
                    };
                    let _ = ScreenToClient(self.hwnd, &mut p);
                    let mut r = RECT::default();
                    let _ = GetClientRect(self.hwnd, &mut r);
                    let edge = 7;
                    let left = p.x < edge;
                    let right = p.x >= r.right - edge;
                    let top = p.y < edge;
                    let bottom = p.y >= r.bottom - edge;
                    let hit = match (left, right, top, bottom) {
                        (true, _, true, _) => HTTOPLEFT,
                        (_, true, true, _) => HTTOPRIGHT,
                        (true, _, _, true) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, _, _, _) => HTLEFT,
                        (_, true, _, _) => HTRIGHT,
                        (_, _, true, _) => HTTOP,
                        (_, _, _, true) => HTBOTTOM,
                        _ if p.y < 52 * self.gfx.scale() as i32
                            && p.x < r.right - 58 * self.gfx.scale() as i32 =>
                        {
                            HTCAPTION
                        }
                        _ => HTCLIENT,
                    };
                    return Some(LRESULT(hit as isize));
                }
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    let scale = self.gfx.scale();
                    let p = Point::new(
                        lp.0 as i16 as f32 / scale,
                        (lp.0 >> 16) as i16 as f32 / scale,
                    );
                    self.input.mouse = Some(p);
                    if msg == WM_LBUTTONDOWN {
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
                        if id == self.input.pressed_id {
                            self.input.clicked = id.map(|id| (id, MouseButton::Left));
                        }
                        if self.input.down {
                            self.input.released = self.input.pressed_id.map(|id| (id, p));
                        }
                        self.input.down = false;
                        self.input.pressed_id = None;
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
                WM_SIZE => {}
                _ => return None,
            }
        }
        self.invalidate();
        Some(LRESULT(0))
    }
}
impl Drop for AiWindow {
    fn drop(&mut self) {
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
        if ctrl && wp.0 == b'V' as usize {
            paste_text(input, hwnd);
            return;
        }
        let key = match VIRTUAL_KEY(wp.0 as u16) {
            VK_BACK => Some(KeyInput::Backspace),
            VK_DELETE => Some(KeyInput::Delete),
            VK_LEFT => Some(KeyInput::Left),
            VK_RIGHT => Some(KeyInput::Right),
            VK_HOME => Some(KeyInput::Home),
            VK_END => Some(KeyInput::End),
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
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn starred_model_uses_default_then_reselection_opens_original_slider() {
        use crate::{commands::Command, ui::wid_n};
        let mut st = sample_state_for_selftest();
        st.settings.ai.provider = lumo_core::ai::Provider::Compatible;
        st.settings.ai.completion_url = "https://example.org/v1/chat/completions".into();
        st.ai.models[0].reasoning = lumo_core::ai_reasoning::Reasoning::parse(&serde_json::json!({
            "reasoning": {"supported_efforts": ["high", "low", "max"], "default_effort": "low"}
        })).unwrap();
        let model_id = st.ai.models[0].id.clone();
        st.ai.selected = st.ai.models[1].id.clone();
        st.settings.ai.reasoning_efforts.insert(model_id.clone(), "max".into());
        let mut window = AiWindow::new().unwrap();
        window.input.clicked = Some((wid("ai.models"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.input.clicked = Some((wid_n("ai.model", 0), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert_eq!(st.ai.selected, model_id);
        assert!(!st.ai.model_menu && !st.ai.reasoning_menu, "switching to a supported model selects it directly");
        assert!(!st.settings.ai.reasoning_efforts.contains_key(&model_id), "a new selection uses the model default even after an earlier override");
        let (_, body) = lumo_core::ai::request(&st.settings.ai, &st.ai.model(&st.settings.ai), &st.ai.current.messages).unwrap();
        assert!(body.get("reasoning_effort").is_none(), "default means no forced effort in the request");
        window.input.clicked = Some((wid("ai.models"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.input.clicked = Some((wid_n("ai.model", 0), MouseButton::Left));
        window.draw(&mut st).unwrap();
        window.draw(&mut st).unwrap();
        assert!(st.ai.model_menu && st.ai.reasoning_menu, "reselecting the active starred model opens its slider");
        let slider = window.hits.iter().find(|h| h.id == wid("ai.effort-slider")).unwrap().rect;
        let popup = window.hits.iter().find(|h| h.id == wid("ai.model-surface")).unwrap().rect;
        assert!(popup.w <= 280.0 && popup.h <= 132.0, "restore the earlier compact popup");
        let point = |p: Point, scale: f32| LPARAM((((p.y * scale).round() as u16 as usize) << 16 | (p.x * scale).round() as u16 as usize) as isize);
        let start = Point::new(slider.x + 12.0, slider.center().y);
        let end = Point::new(slider.right() - 12.0, slider.center().y);
        st.cmds.clear();
        window.message(WM_LBUTTONDOWN, WPARAM(0), point(start, window.gfx.scale()));
        window.draw(&mut st).unwrap();
        window.message(WM_MOUSEMOVE, WPARAM(0), point(end, window.gfx.scale()));
        window.draw(&mut st).unwrap();
        assert_eq!(st.settings.ai.reasoning_efforts.get(&model_id).map(String::as_str), Some("max"));
        assert!(!st.cmds.contains(&Command::SaveSettings), "dragging does not write settings every frame");
        window.message(WM_LBUTTONUP, WPARAM(0), point(end, window.gfx.scale()));
        window.draw(&mut st).unwrap();
        assert_eq!(st.cmds.iter().filter(|c| **c == Command::SaveSettings).count(), 1);
        let (_, body) = lumo_core::ai::request(&st.settings.ai, &st.ai.model(&st.settings.ai), &st.ai.current.messages).unwrap();
        assert_eq!(body["reasoning_effort"], "max");
        if let Some(root) = std::env::var_os("LUMO_AI_QA") {
            let root = std::path::PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            window.capture(&mut st, &root.join("effort-slider.png"));
        }
        window.input.keys.push(KeyInput::Home);
        window.draw(&mut st).unwrap();
        assert_eq!(st.settings.ai.reasoning_efforts.get(&model_id).map(String::as_str), Some("low"));
        window.input.clicked = Some((wid("ai.effort-reset"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(!st.settings.ai.reasoning_efforts.contains_key(&model_id));
        window.input.clicked = Some((wid("ai.effort-model"), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(st.ai.model_menu && !st.ai.reasoning_menu);
        window.input.clicked = Some((wid_n("ai.model", 1), MouseButton::Left));
        window.draw(&mut st).unwrap();
        assert!(!st.ai.model_menu && !st.ai.reasoning_menu, "plain models still select directly");
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
        assert!(w.hits.iter().find(|h| h.id == wid("ai.model-surface")).unwrap().rect.w >= 450.0);
        let providers = st
            .ai
            .model_rows
            .iter()
            .filter_map(|r| {
                if let ModelRow::Provider(p) = r {
                    Some(p.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(providers, ["Google", "LiquidAI", "OpenAI"]);
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
            .filter_map(|row| {
                if let ModelRow::Model(i) = row {
                    Some(st.ai.models[*i].id.as_str())
                } else {
                    None
                }
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
        assert!((0..4).all(|i| w
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
            ((bounds.left + bounds.right) - (monitor.work.left + monitor.work.right)).abs() <= 2,
            "popup is horizontally centered"
        );
        assert!(
            bounds.bottom <= monitor.work.bottom && monitor.work.bottom - bounds.bottom <= 24,
            "popup sits above the taskbar"
        );
        w.draw(&mut st).unwrap();
        assert!(w.hits.iter().any(|h| h.id == wid("ai.prompt")));
        assert!(w.hits.iter().any(|h| h.id == wid("ai.send")));
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
            copy.y >= 100.0 && copy.y < 200.0,
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
        let progress = w
            .hits
            .iter()
            .find(|h| h.id == wid("ai.generation"))
            .unwrap()
            .rect;
        w.input.mouse = Some(progress.center());
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("generation-working.png"),
            );
        }
        w.input.clicked = Some((wid("ai.generation"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st
            .cmds
            .contains(&crate::commands::Command::Ai(crate::ai::Action::Stop)));
        st.cmds.clear();
        st.ai.busy = false;
        st.ai.generation = crate::ai::Generation::Ready;
        st.ai.generation_finished = 100.0;
        if let Some(dir) = std::env::var_os("LUMO_AI_QA") {
            w.capture(
                &mut st,
                &std::path::PathBuf::from(dir).join("generation-ready.png"),
            );
        }
        w.input.clicked = Some((wid("ai.generation"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(
            st.cmds.is_empty(),
            "opening a ready reply must not send another request"
        );
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
        w.input.clicked = Some((wid("ai.generation"), MouseButton::Left));
        w.draw(&mut st).unwrap();
        assert!(st
            .cmds
            .contains(&crate::commands::Command::Ai(crate::ai::Action::Select(
                other.id
            ))));
        assert!(!st
            .cmds
            .contains(&crate::commands::Command::Ai(crate::ai::Action::Stop)));
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
