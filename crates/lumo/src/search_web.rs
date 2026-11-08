//! Optional web results inside the island. Uses the installed Evergreen
//! WebView2 runtime, with Microsoft's small loader loaded only on first use.
//! No browser engine is shipped, and Close releases the controller/process.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::OnceLock,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler, ContentLoadingEventHandler,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DOMContentLoadedEventHandler, FocusChangedEventHandler, HistoryChangedEventHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler, SourceChangedEventHandler,
};
use windows::{
    core::{s, Interface, HRESULT, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn},
        System::{LibraryLoader::*, WinRT::EventRegistrationToken},
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

pub const CHANGED: u32 = WM_APP + 92;
pub const CLOSE: u32 = WM_APP + 93;
pub const SWIPE: u32 = WM_APP + 94;
pub const ACTION: u32 = WM_APP + 95;
pub const PAGE_FOCUS: u32 = WM_APP + 96;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Back,
    Forward,
    Reload,
    Stop,
    Address,
}
impl Action {
    pub fn from_key(key: u32, ctrl: bool, alt: bool) -> Option<Self> {
        match key {
            0x4c if ctrl => Some(Self::Address),
            0x52 if ctrl => Some(Self::Reload),
            0x74 => Some(Self::Reload),
            0x25 if alt => Some(Self::Back),
            0x27 if alt => Some(Self::Forward),
            _ => None,
        }
    }
    pub fn from_message(value: usize) -> Option<Self> {
        [
            Self::Back,
            Self::Forward,
            Self::Reload,
            Self::Stop,
            Self::Address,
        ]
        .get(value)
        .copied()
    }
}
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Navigation {
    pub url: String,
    pub loading: bool,
    /// Milestones from real navigation events; no idle timer or fake percentage.
    pub progress: f32,
    pub back: bool,
    pub forward: bool,
}
type Create = unsafe extern "system" fn(
    PCWSTR,
    PCWSTR,
    *mut core::ffi::c_void,
    *mut core::ffi::c_void,
) -> HRESULT;
fn loader() -> Result<Create, String> {
    // Pin the tiny loader after first use: outstanding asynchronous COM
    // callbacks may still return through it after a Search dismissal.
    static MODULE: OnceLock<Result<usize, String>> = OnceLock::new();
    let module=MODULE.get_or_init(||unsafe{
        let mut path=std::env::current_exe().map_err(|e|e.to_string())?.with_file_name("WebView2Loader.dll");
        #[cfg(debug_assertions)] if !path.exists(){path=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/webview2/WebView2Loader.dll");}
        LoadLibraryExW(&HSTRING::from(path.to_string_lossy().as_ref()),None,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32).map(|h|h.0 as usize).map_err(|_|"Web search needs WebView2Loader.dll beside lumo.exe. Extract the full portable ZIP or use Setup.".into())
    });
    unsafe {
        let h = HMODULE((*module.as_ref().map_err(Clone::clone)?) as _);
        let proc = GetProcAddress(h, s!("CreateCoreWebView2EnvironmentWithOptions"))
            .ok_or("Microsoft's WebView2 loader could not start.")?;
        Ok(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            Create,
        >(proc))
    }
}
struct State {
    owner: HWND,
    host: HWND,
    closed: bool,
    controller: Option<ICoreWebView2Controller>,
    web: Option<ICoreWebView2>,
    error: String,
    bounds: Option<RECT>,
    visible: bool,
    navigation: Navigation,
    navigation_id: u64,
    stopped: bool,
    paused: bool,
    pause_generation: u64,
    discard: bool,
}
pub struct Browser {
    state: Rc<RefCell<State>>,
    hook: Option<SwipeHook>,
}
fn notify(owner: HWND) {
    unsafe {
        let _ = PostMessageW(owner, CHANGED, WPARAM(0), LPARAM(0));
    }
}
fn fail(state: &Rc<RefCell<State>>, message: &str) {
    let mut st = state.borrow_mut();
    if !st.closed {
        st.error = message.into();
        st.navigation.loading = false;
        st.navigation.progress = 0.0;
        notify(st.owner);
    }
}
fn take_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe {
        let out = value.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(value.0.cast()));
        out
    }
}
fn web_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}
/// Safari-style compact location while the address field isn't being edited.
/// Show the real authority, including HTTP/ports; never an apparent username.
pub fn display_address(value: &str) -> String {
    let Some((scheme, rest)) = value.split_once("://") else {
        return value.into();
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .rsplit('@')
        .next()
        .unwrap_or(rest);
    if scheme == "https" {
        authority.into()
    } else {
        format!("{scheme}://{authority}")
    }
}
impl Browser {
    pub fn discarded(&self) -> bool {
        self.state.borrow().discard
    }
    pub fn resume(&self) -> bool {
        let web = {
            let mut st = self.state.borrow_mut();
            if st.discard || st.closed {
                return false;
            }
            st.paused = false;
            st.pause_generation = st.pause_generation.wrapping_add(1);
            st.web.clone()
        };
        web.and_then(|w| w.cast::<ICoreWebView2_3>().ok())
            .is_some_and(|web| unsafe { web.Resume().is_ok() })
    }
    /// No hidden page polling. A suspended retained page keeps DOM/history;
    /// if the runtime refuses suspension, the owner releases it and keeps URL.
    pub fn pause(&mut self) -> bool {
        let (web, bounds, generation) = {
            let mut st = self.state.borrow_mut();
            if st.paused {
                return !st.discard;
            }
            st.paused = true;
            st.pause_generation = st.pause_generation.wrapping_add(1);
            (st.web.clone(), st.bounds, st.pause_generation)
        };
        let Some(web) = web.and_then(|w| w.cast::<ICoreWebView2_3>().ok()) else {
            return false;
        };
        if let Some(bounds) = bounds {
            self.place(bounds, false);
        }
        let weak = Rc::downgrade(&self.state);
        let callback =
            webview2_com::TrySuspendCompletedHandler::create(Box::new(move |result, success| {
                if let Some(state) = weak.upgrade() {
                    let resume = {
                        let mut st = state.borrow_mut();
                        if st.closed {
                            return Ok(());
                        }
                        if st.paused && st.pause_generation == generation {
                            if result.is_err() || !success {
                                st.discard = true;
                                notify(st.owner);
                            }
                            None
                        } else if !st.paused && success {
                            st.web.clone()
                        } else {
                            None
                        }
                    };
                    if let Some(web) = resume.and_then(|w| w.cast::<ICoreWebView2_3>().ok()) {
                        unsafe {
                            let _ = web.Resume();
                        }
                    }
                }
                Ok(())
            }));
        unsafe { web.TrySuspend(&callback).is_ok() }
    }
    pub fn navigation(&self) -> Navigation {
        self.state.borrow().navigation.clone()
    }
    pub fn action(&self, action: Action) {
        if action == Action::Stop {
            let mut st = self.state.borrow_mut();
            st.stopped = true;
            st.navigation.loading = false;
            st.navigation.progress = 0.0;
            notify(st.owner);
        }
        let web = self.state.borrow().web.clone();
        if action == Action::Reload && web.is_none() {
            let mut st = self.state.borrow_mut();
            st.stopped = false;
            st.navigation.loading = true;
            st.navigation.progress = 0.08;
            notify(st.owner);
        }
        if let Some(web) = web {
            unsafe {
                match action {
                    Action::Back => {
                        let _ = web.GoBack();
                    }
                    Action::Forward => {
                        let _ = web.GoForward();
                    }
                    Action::Reload => {
                        let mut source = PWSTR::null();
                        let _ = web.Source(&mut source);
                        let source = take_string(source);
                        let target = self.state.borrow().navigation.url.clone();
                        if !web_url(&source) && web_url(&target) {
                            let _ = web.Navigate(&HSTRING::from(target));
                        } else {
                            let _ = web.Reload();
                        }
                    }
                    Action::Stop => {
                        {
                            let mut st = self.state.borrow_mut();
                            st.stopped = true;
                            st.navigation.loading = false;
                            st.navigation.progress = 0.0;
                        }
                        let _ = web.Stop();
                        notify(self.state.borrow().owner);
                    }
                    Action::Address => {}
                }
            }
        }
    }
    pub fn navigate(&self, url: &str) {
        if web_url(url) {
            {
                let mut st = self.state.borrow_mut();
                st.navigation.url = url.into();
                st.navigation.loading = true;
                st.navigation.progress = 0.08;
                st.stopped = false;
                st.error.clear();
                notify(st.owner);
            }
            let web = self.state.borrow().web.clone();
            if let Some(web) = web {
                unsafe {
                    let _ = web.Navigate(&HSTRING::from(url));
                }
            }
        }
    }
    pub fn focus(&self) {
        let controller = self.state.borrow().controller.clone();
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT);
            }
        }
    }
    pub fn new(owner: HWND, url: &str) -> Result<Self, String> {
        if !web_url(url) && !(cfg!(test) && url == "about:blank") {
            return Err("This web address is not supported.".into());
        }
        let create = loader()?;
        // A rounded child host clips both native page pixels and input. It is
        // created only for explicit web browsing and destroyed with the view.
        let host = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                PCWSTR::null(),
                WS_CHILD | WS_CLIPCHILDREN,
                0,
                0,
                1,
                1,
                owner,
                None,
                GetModuleHandleW(None).map_err(|e| e.to_string())?,
                None,
            )
            .map_err(|e| e.to_string())?
        };
        let state = Rc::new(RefCell::new(State {
            owner,
            host,
            closed: false,
            controller: None,
            web: None,
            error: String::new(),
            bounds: None,
            visible: false,
            navigation: Navigation {
                url: url.into(),
                loading: true,
                progress: 0.08,
                ..Default::default()
            },
            navigation_id: 0,
            stopped: false,
            paused: false,
            pause_generation: 0,
            discard: false,
        }));
        let ready = state.clone();
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                if ready.borrow().closed {
                    return Ok(());
                }
                let Some(environment) = environment.filter(|_| result.is_ok()) else {
                    fail(&ready,"Microsoft Edge WebView2 Runtime is unavailable. Install the Evergreen Runtime from Microsoft, then reopen Search.");
                    return Ok(());
                };
                let complete = ready.clone();
                let callback = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        let Some(controller) = controller.filter(|_| result.is_ok()) else {
                            fail(
                                &complete,
                                "Web results could not open. Close Search and try again.",
                            );
                            return Ok(());
                        };
                        if complete.borrow().closed {
                            unsafe {
                                let _ = controller.Close();
                            }
                            return Ok(());
                        }
                        unsafe {
                            let _ = controller.SetIsVisible(false);
                            if let Ok(c) = controller.cast::<ICoreWebView2Controller2>() {
                                let _ = c.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                                    A: 255,
                                    R: 28,
                                    G: 28,
                                    B: 30,
                                });
                            }
                            let web = match controller.CoreWebView2() {
                                Ok(web) => web,
                                Err(_) => {
                                    let _ = controller.Close();
                                    fail(&complete, "Web results could not open.");
                                    return Ok(());
                                }
                            };
                            if let Ok(settings) = web.Settings() {
                                let _ = settings.SetAreDevToolsEnabled(false);
                                let _ = settings.SetIsStatusBarEnabled(false);
                                let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
                            }
                            let mut token = EventRegistrationToken::default();
                            let _ = controller.add_GotFocus(
                                &FocusChangedEventHandler::create(Box::new(move |_, _| {
                                    let _ = PostMessageW(owner, PAGE_FOCUS, WPARAM(0), LPARAM(0));
                                    Ok(())
                                })),
                                &mut token,
                            );
                            let _ = controller.add_AcceleratorKeyPressed(
                                &AcceleratorKeyPressedEventHandler::create(Box::new(
                                    move |_, args| {
                                        if let Some(args) = args {
                                            let mut key = 0;
                                            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                                            {
                                                args.VirtualKey(&mut key)?;
                                                args.KeyEventKind(&mut kind)?;
                                                if key == VK_ESCAPE.0 as u32
                                                    && kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                                                {
                                                    args.SetHandled(true)?;
                                                    let _ = PostMessageW(
                                                        owner,
                                                        CLOSE,
                                                        WPARAM(0),
                                                        LPARAM(0),
                                                    );
                                                } else if matches!(kind,COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN|COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN)
                                                {
                                                    if let Some(action) = Action::from_key(
                                                        key,
                                                        GetKeyState(VK_CONTROL.0 as i32) < 0,
                                                        GetKeyState(VK_MENU.0 as i32) < 0,
                                                    ) {
                                                        args.SetHandled(true)?;
                                                        let _ = PostMessageW(
                                                            owner,
                                                            ACTION,
                                                            WPARAM(action as usize),
                                                            LPARAM(0),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        Ok(())
                                    },
                                )),
                                &mut token,
                            );
                            let weak = Rc::downgrade(&complete);
                            let _ = web.add_NavigationStarting(
                                &NavigationStartingEventHandler::create(Box::new(
                                    move |_, args| {
                                        if let Some(args) = args {
                                            let mut uri = PWSTR::null();
                                            {
                                                args.Uri(&mut uri)?;
                                                let uri = take_string(uri);
                                                if !web_url(&uri)
                                                    && !(cfg!(test)
                                                        && (uri == "about:blank"
                                                            || uri.starts_with("data:text/html;")))
                                                {
                                                    args.SetCancel(true)?;
                                                } else if let Some(state) = weak.upgrade() {
                                                    let mut st = state.borrow_mut();
                                                    if st.closed {
                                                        return Ok(());
                                                    }
                                                    args.NavigationId(&mut st.navigation_id)?;
                                                    st.stopped = false;
                                                    st.error.clear();
                                                    st.navigation.url = uri;
                                                    st.navigation.loading = true;
                                                    st.navigation.progress = 0.12;
                                                    notify(st.owner);
                                                }
                                            }
                                        }
                                        Ok(())
                                    },
                                )),
                                &mut token,
                            );
                            let weak = Rc::downgrade(&complete);
                            let _ = web.add_SourceChanged(
                                &SourceChangedEventHandler::create(Box::new(move |web, _| {
                                    if let (Some(state), Some(web)) = (weak.upgrade(), web) {
                                        let mut uri = PWSTR::null();
                                        web.Source(&mut uri)?;
                                        let mut st = state.borrow_mut();
                                        if !st.closed {
                                            st.navigation.url = take_string(uri);
                                            if st.navigation.loading {
                                                st.navigation.progress = 0.4;
                                            }
                                            notify(st.owner);
                                        } else {
                                            let _ = take_string(uri);
                                        }
                                    }
                                    Ok(())
                                })),
                                &mut token,
                            );
                            let weak = Rc::downgrade(&complete);
                            let _ = web.add_HistoryChanged(
                                &HistoryChangedEventHandler::create(Box::new(move |web, _| {
                                    if let (Some(state), Some(web)) = (weak.upgrade(), web) {
                                        let mut back = BOOL(0);
                                        let mut forward = BOOL(0);
                                        web.CanGoBack(&mut back)?;
                                        web.CanGoForward(&mut forward)?;
                                        let mut st = state.borrow_mut();
                                        if !st.closed {
                                            st.navigation.back = back.as_bool();
                                            st.navigation.forward = forward.as_bool();
                                            notify(st.owner);
                                        }
                                    }
                                    Ok(())
                                })),
                                &mut token,
                            );
                            let weak = Rc::downgrade(&complete);
                            let _ = web.add_ContentLoading(
                                &ContentLoadingEventHandler::create(Box::new(move |_, _| {
                                    if let Some(state) = weak.upgrade() {
                                        let mut st = state.borrow_mut();
                                        if !st.closed && st.navigation.loading {
                                            st.navigation.progress = 0.7;
                                            notify(st.owner);
                                        }
                                    }
                                    Ok(())
                                })),
                                &mut token,
                            );
                            if let Ok(web2) = web.cast::<ICoreWebView2_2>() {
                                let weak = Rc::downgrade(&complete);
                                let _ = web2.add_DOMContentLoaded(
                                    &DOMContentLoadedEventHandler::create(Box::new(move |_, _| {
                                        if let Some(state) = weak.upgrade() {
                                            let mut st = state.borrow_mut();
                                            if !st.closed && st.navigation.loading {
                                                st.navigation.progress = 0.88;
                                                notify(st.owner);
                                            }
                                        }
                                        Ok(())
                                    })),
                                    &mut token,
                                );
                            }
                            let weak = Rc::downgrade(&complete);
                            let _=web.add_NavigationCompleted(&NavigationCompletedEventHandler::create(Box::new(move |_,args|{
                                if let (Some(state),Some(args))=(weak.upgrade(),args){let mut st=state.borrow_mut();let mut id=0;args.NavigationId(&mut id)?;if st.closed||id!=st.navigation_id{return Ok(());}
                                    st.navigation.loading=false;st.navigation.progress=0.0;let mut success=BOOL(0);args.IsSuccess(&mut success)?;
                                    if !success.as_bool()&&!st.stopped {let mut status=COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;args.WebErrorStatus(&mut status)?;if status!=COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED{st.error="This page couldn’t load. Check your connection and try again.".into();}}
                                    notify(st.owner);
                                }Ok(())
                            })),&mut token);
                            let _ = web.add_NewWindowRequested(
                                &NewWindowRequestedEventHandler::create(Box::new(|web, args| {
                                    if let Some(args) = args {
                                        let mut uri = PWSTR::null();
                                        {
                                            args.Uri(&mut uri)?;
                                            args.SetHandled(true)?;
                                            let uri = take_string(uri);
                                            if web_url(&uri) {
                                                if let Some(web) = web {
                                                    web.Navigate(&HSTRING::from(uri))?;
                                                }
                                            }
                                        }
                                    }
                                    Ok(())
                                })),
                                &mut token,
                            );
                            let _ = web.add_PermissionRequested(
                                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                                    if let Some(args) = args {
                                        {
                                            args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                                        }
                                    }
                                    Ok(())
                                })),
                                &mut token,
                            );
                            let (url, stopped) = {
                                let st = complete.borrow();
                                (st.navigation.url.clone(), st.stopped)
                            };
                            if !stopped && web.Navigate(&HSTRING::from(url)).is_err() {
                                let _ = controller.Close();
                                fail(&complete, "The web search page could not open.");
                                return Ok(());
                            }
                            {
                                let mut st = complete.borrow_mut();
                                st.controller = Some(controller);
                                st.web = Some(web);
                            }
                            notify(owner);
                        }
                        Ok(())
                    },
                ));
                if unsafe { environment.CreateCoreWebView2Controller(host, &callback) }.is_err() {
                    fail(
                        &ready,
                        "Web results could not open. Close Search and try again.",
                    );
                }
                Ok(())
            },
        ));
        let profile = crate::paths::cache_dir().join("SearchWeb");
        let _ = std::fs::create_dir_all(&profile);
        let result = unsafe {
            create(
                PCWSTR::null(),
                PCWSTR(HSTRING::from(profile.to_string_lossy().as_ref()).as_ptr()),
                std::ptr::null_mut(),
                handler.as_raw(),
            )
        };
        if result.is_err() {
            state.borrow_mut().closed = true;
            unsafe {
                let _ = DestroyWindow(host);
            }
            return Err("Microsoft Edge WebView2 Runtime is unavailable. Install the Evergreen Runtime from Microsoft and reopen Search.".into());
        }
        Ok(Self { state, hook: None })
    }
    pub fn status(&self) -> (bool, String) {
        let st = self.state.borrow();
        (st.web.is_some(), st.error.clone())
    }
    pub fn place(&mut self, bounds: RECT, visible: bool) {
        // COM/Win32 calls can dispatch completion callbacks synchronously.
        // Never hold the shared state borrow while entering those APIs.
        let (controller, host, owner, moved, visibility) = {
            let mut st = self.state.borrow_mut();
            let Some(controller) = st.controller.clone() else {
                return;
            };
            let moved = st.bounds != Some(bounds);
            let visibility = st.visible != visible;
            st.bounds = Some(bounds);
            st.visible = visible;
            (controller, st.host, st.owner, moved, visibility)
        };
        unsafe {
            if moved {
                let width = (bounds.right - bounds.left).max(1);
                let height = (bounds.bottom - bounds.top).max(1);
                let _ = SetWindowPos(
                    host,
                    None,
                    bounds.left,
                    bounds.top,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(owner) as f32 / 96.0;
                let diameter = (36.0 * dpi).round() as i32;
                let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter);
                if !region.is_invalid() && SetWindowRgn(host, region, true) == 0 {
                    let _ = DeleteObject(region);
                }
                let _ = controller.SetBounds(RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                });
            }
            if visibility {
                let _ = controller.SetIsVisible(visible);
                let _ = ShowWindow(host, if visible { SW_SHOWNA } else { SW_HIDE });
            }
        }
        if visible && self.hook.is_none() {
            self.hook = SwipeHook::new(owner).ok();
        } else if !visible {
            self.hook = None;
        }
    }
    #[cfg(test)]
    pub fn test_web(&self) -> Option<ICoreWebView2> {
        self.state.borrow().web.clone()
    }
    #[cfg(test)]
    pub fn test_host(&self) -> HWND {
        self.state.borrow().host
    }
    #[cfg(test)]
    pub fn test_swipe(&self, at: POINT) -> LRESULT {
        let event = MSLLHOOKSTRUCT {
            pt: at,
            mouseData: 120u32 << 16,
            ..Default::default()
        };
        unsafe {
            mouse_hook(
                0,
                WPARAM(WM_MOUSEHWHEEL as usize),
                LPARAM((&event as *const MSLLHOOKSTRUCT) as isize),
            )
        }
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        self.hook = None;
        let (host, controller, web) = {
            let mut st = self.state.borrow_mut();
            st.closed = true;
            (st.host, st.controller.take(), st.web.take())
        };
        // Closing the native host can complete a pending controller creation.
        // Its callback observes closed=true, without borrowing reentrantly.
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.Close();
            }
        }
        drop(web);
        unsafe {
            let _ = DestroyWindow(host);
        }
    }
}

// Browser input belongs to WebView2 child windows. Forward only horizontal
// wheel packets over our island, so the same native gesture closes Search.
// This hook exists only while web results are visible; it never polls.
thread_local! {static OWNER:Cell<HWND>=const{Cell::new(HWND(std::ptr::null_mut()))};}
struct SwipeHook(HHOOK);
unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && wp.0 == WM_MOUSEHWHEEL as usize {
        let event = &*(lp.0 as *const MSLLHOOKSTRUCT);
        let owner = OWNER.with(Cell::get);
        let target = WindowFromPoint(event.pt);
        if !owner.is_invalid() && (target == owner || IsChild(owner, target).as_bool()) {
            let delta = (event.mouseData >> 16) as u16;
            let _ = PostMessageW(owner, SWIPE, WPARAM((delta as usize) << 16), LPARAM(0));
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}
impl SwipeHook {
    fn new(owner: HWND) -> windows::core::Result<Self> {
        unsafe {
            let hook =
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), GetModuleHandleW(None)?, 0)?;
            OWNER.with(|cell| cell.set(owner));
            Ok(Self(hook))
        }
    }
}
impl Drop for SwipeHook {
    fn drop(&mut self) {
        OWNER.with(|cell| cell.set(HWND::default()));
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn pump_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(Instant::now() < deadline, "WebView2 callback timed out");
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "requires the installed Evergreen WebView2 Runtime"]
    fn installed_runtime_renders_offline_and_closes_without_bundled_engine() {
        unsafe {
            let home = std::env::current_dir()
                .unwrap()
                .join(".tools/search-v2-web-test");
            std::fs::create_dir_all(&home).unwrap();
            std::env::set_var("LUMO_HOME", home);
            windows::Win32::System::Ole::OleInitialize(None).unwrap();
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("Lumo WebView2 fixture"),
                WS_POPUP,
                0,
                0,
                640,
                480,
                None,
                None,
                GetModuleHandleW(None).unwrap(),
                None,
            )
            .unwrap();
            let mut browser = Browser::new(owner, "about:blank").unwrap();
            pump_until(|| browser.status().0 || !browser.status().1.is_empty());
            assert!(browser.status().1.is_empty(), "{}", browser.status().1);
            browser.place(
                RECT {
                    left: 20,
                    top: 72,
                    right: 620,
                    bottom: 456,
                },
                false,
            );
            let web = browser.test_web().unwrap();
            web.NavigateToString(windows::core::w!(
                "<!doctype html><title>Lumo runtime test</title><p id='ready'>WebView2 ready</p>"
            ))
            .unwrap();
            let result = Rc::new(RefCell::new(None));
            let complete = result.clone();
            // NavigateToString completes asynchronously. A navigation-completed
            // callback proves the installed runtime rendered our offline fixture.
            let mut token = EventRegistrationToken::default();
            let callback = webview2_com::NavigationCompletedEventHandler::create(Box::new(
                move |web, _args| {
                    if let Some(web) = web {
                        let complete = complete.clone();
                        unsafe {
                            web.ExecuteScript(
                                windows::core::w!(
                                    "document.getElementById('ready')?.textContent || ''"
                                ),
                                &webview2_com::ExecuteScriptCompletedHandler::create(Box::new(
                                    move |code, value| {
                                        code?;
                                        if value == "\"WebView2 ready\"" {
                                            *complete.borrow_mut() = Some(value);
                                        }
                                        Ok(())
                                    },
                                )),
                            )?;
                        }
                    }
                    Ok(())
                },
            ));
            web.add_NavigationCompleted(&callback, &mut token).unwrap();
            pump_until(|| result.borrow().is_some());
            assert_eq!(result.borrow().as_deref(), Some("\"WebView2 ready\""));
            // Exercise actual navigation/history/loading on loopback; no live
            // provider requests or user browsing history are used by this test.
            use std::{
                io::{Read, Write},
                sync::{
                    atomic::{AtomicBool, AtomicUsize, Ordering},
                    Arc,
                },
            };
            let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = server.local_addr().unwrap();
            server.set_nonblocking(true).unwrap();
            struct Fixture {
                stop: Arc<AtomicBool>,
                thread: Option<std::thread::JoinHandle<()>>,
            }
            impl Drop for Fixture {
                fn drop(&mut self) {
                    self.stop.store(true, Ordering::Relaxed);
                    if let Some(thread) = self.thread.take() {
                        let _ = thread.join();
                    }
                }
            }
            let stop = Arc::new(AtomicBool::new(false));
            let ended = stop.clone();
            let requests = Arc::new(AtomicUsize::new(0));
            let seen = requests.clone();
            let _fixture = Fixture {
                stop,
                thread: Some(std::thread::spawn(move || {
                    while !ended.load(Ordering::Relaxed) {
                        if let Ok((mut stream, _)) = server.accept() {
                            let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                            let mut buf = [0; 2048];
                            let _ = stream.read(&mut buf);
                            seen.fetch_add(1, Ordering::Relaxed);
                            std::thread::sleep(Duration::from_millis(160));
                            let body="<!doctype html><title>Lumo navigation fixture</title><p>Offline browser navigation</p>";
                            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                            let _ = stream.write_all(response.as_bytes());
                        } else {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                })),
            };
            let one = format!("http://{address}/one");
            let two = format!("http://{address}/two");
            browser.navigate(&one);
            pump_until(|| browser.navigation().loading);
            assert!(browser.navigation().progress > 0.0);
            pump_until(|| !browser.navigation().loading && browser.navigation().url == one);
            assert!(browser.status().1.is_empty());
            browser.navigate(&two);
            pump_until(|| !browser.navigation().loading && browser.navigation().url == two);
            assert!(browser.navigation().back);
            browser.action(Action::Back);
            pump_until(|| !browser.navigation().loading && browser.navigation().url == one);
            assert!(browser.navigation().forward);
            browser.action(Action::Forward);
            pump_until(|| !browser.navigation().loading && browser.navigation().url == two);
            let before = requests.load(Ordering::Relaxed);
            browser.action(Action::Reload);
            pump_until(|| browser.navigation().loading);
            pump_until(|| !browser.navigation().loading);
            assert!(requests.load(Ordering::Relaxed) > before);
            browser.navigate(&format!("http://{address}/slow"));
            pump_until(|| browser.navigation().loading);
            browser.action(Action::Stop);
            pump_until(|| !browser.navigation().loading);
            assert!(browser.status().1.is_empty());
            // A failed connection produces a host retry state and recovers on
            // the next valid navigation instead of leaving stale loading chrome.
            let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let unavailable = unused.local_addr().unwrap();
            drop(unused);
            browser.navigate(&format!("http://{unavailable}/unavailable"));
            pump_until(|| !browser.status().1.is_empty());
            assert!(!browser.navigation().loading);
            browser.navigate(&one);
            pump_until(|| !browser.navigation().loading && browser.navigation().url == one);
            assert!(browser.status().1.is_empty());
            let state = browser.state.clone();
            drop(web);
            drop(browser);
            assert!(
                state.borrow().closed
                    && state.borrow().controller.is_none()
                    && state.borrow().web.is_none()
            );
            DestroyWindow(owner).unwrap();
            windows::Win32::System::Ole::OleUninitialize();
        }
    }
}
