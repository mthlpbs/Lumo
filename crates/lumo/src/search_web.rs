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
    PermissionRequestedEventHandler, SourceChangedEventHandler, DocumentTitleChangedEventHandler,
};
use windows::{
    core::{s, Interface, HRESULT, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        Graphics::Gdi::{CreateRoundRectRgn, CreateRectRgn, CombineRgn, RGN_AND, RGN_DIFF, DeleteObject, SetWindowRgn},
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
    pub title:String,
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
    parent: HWND,
    host: HWND,
    closed: bool,
    controller: Option<ICoreWebView2Controller>,
    web: Option<ICoreWebView2>,
    error: String,
    bounds: Option<RECT>,
    viewport:(i32,i32),
    cover_top:i32,
    overlays:Vec<RECT>,
    clip_radius:f32,
    site_events:Vec<crate::ai_sites::Event>,
    region_dirty:bool,
    filters:Option<Rc<lumo_core::adblocking::Engine>>,
    frames:Vec<ICoreWebView2Frame2>,
    blocking_error:String,
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
fn resource_kind(context:COREWEBVIEW2_WEB_RESOURCE_CONTEXT,url:&str,page:&str)->u32 {
    use lumo_core::adblocking::*;
    match context {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT=>if url==page{DOCUMENT}else{SUBDOCUMENT},
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT=>SCRIPT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE=>IMAGE,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET=>STYLESHEET,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT=>FONT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA=>MEDIA,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH=>XHR,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING=>PING,
        _=>OTHER,
    }
}
fn resource_filter(web:&ICoreWebView2,enabled:bool) {
    unsafe {
        // Include frames and service workers on current Evergreen runtimes.
        if let Ok(web)=web.cast::<ICoreWebView2_22>() {
            if enabled {let _=web.AddWebResourceRequestedFilterWithRequestSourceKinds(windows::core::w!("*"),COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL);}
            else {let _=web.RemoveWebResourceRequestedFilterWithRequestSourceKinds(windows::core::w!("*"),COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL);}
        } else if enabled {let _=web.AddWebResourceRequestedFilter(windows::core::w!("*"),COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);}
        else {let _=web.RemoveWebResourceRequestedFilter(windows::core::w!("*"),COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);}
    }
}
fn apply_cosmetics(state:&Rc<RefCell<State>>,web:&ICoreWebView2) {
    let (enabled,frames)={let st=state.borrow();if st.closed{return;}(st.filters.is_some(),st.frames.clone())};
    let script=if enabled{"window.__lumoBlock?.start()"}else{"window.__lumoBlock?.stop()"};
    unsafe {
        let handler=webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|_,_|Ok(())));
        let _=web.ExecuteScript(&HSTRING::from(script),&handler);
        for frame in frames {let _=frame.ExecuteScript(&HSTRING::from(script),&handler);}
    }
}
fn blocker_message(state:&Rc<RefCell<State>>,source:&str,json:&str)->Option<(String,String)> {
    if json.len()>65536{return None;}
    let value:serde_json::Value=serde_json::from_str(json).ok()?;
    if value["channel"]!="lumo.adblocking"{return None;}
    let session=value["session"].as_u64()?;
    let engine={let st=state.borrow();if st.closed{return None;}st.filters.clone()};
    let Some(engine)=engine else{return Some((serde_json::json!({"channel":"lumo.adblocking","enabled":false,"session":session,"url":source}).to_string(),String::new()));};
    let tokens=|key:&str|->Option<Vec<String>>{
        let items=value[key].as_array()?;if items.len()>128{return None;}
        items.iter().map(|v|{let s=v.as_str()?;(s.len()<=640).then(||s.to_string())}).collect()
    };
    let initial=value["initial"].as_bool()?;
    let data=engine.cosmetics(source,&tokens("classes")?,&tokens("ids")?,initial);
    let response=serde_json::json!({"channel":"lumo.adblocking","enabled":true,"session":session,"url":source,"initial":initial,"generic":data.generic,"selectors":data.selectors,"actions":data.actions}).to_string();
    let script=if data.scriptlets.is_empty(){String::new()}else{
        format!("if(location.href==={}&&window.__lumoBlock?.enabled&&!window.__lumoBlock.scriptsApplied){{window.__lumoBlock.scriptsApplied=true;{}}}",serde_json::to_string(source).ok()?,data.scriptlets)
    };
    Some((response,script))
}
fn register_frame(state:&Rc<RefCell<State>>,frame:&ICoreWebView2Frame)->windows::core::Result<()> {
    let Ok(frame2)=frame.cast::<ICoreWebView2Frame2>()else{return Ok(());};
    let key=frame2.as_raw()as usize;
    state.borrow_mut().frames.push(frame2.clone());
    let weak=Rc::downgrade(state);let mut token=EventRegistrationToken::default();
    unsafe {
        frame.add_Destroyed(&webview2_com::FrameDestroyedEventHandler::create(Box::new(move|_,_|{
            if let Some(state)=weak.upgrade(){state.borrow_mut().frames.retain(|f|f.as_raw()as usize!=key);}Ok(())
        })),&mut token)?;
        let weak=Rc::downgrade(state);
        frame2.add_WebMessageReceived(&webview2_com::FrameWebMessageReceivedEventHandler::create(Box::new(move|frame,args|{
            if let (Some(state),Some(frame),Some(args))=(weak.upgrade(),frame,args){
                let mut source=PWSTR::null();let mut json=PWSTR::null();args.Source(&mut source)?;args.WebMessageAsJson(&mut json)?;
                let(source,json)=(take_string(source),take_string(json));
                if let Some((response,script))=blocker_message(&state,&source,&json){
                    let frame=frame.cast::<ICoreWebView2Frame2>()?;frame.PostWebMessageAsJson(&HSTRING::from(response))?;
                    if !script.is_empty(){frame.ExecuteScript(&HSTRING::from(script),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|_,_|Ok(()))))?;}
                }
            }Ok(())
        })),&mut token)?;
        let weak=Rc::downgrade(state);
        frame2.add_DOMContentLoaded(&webview2_com::FrameDOMContentLoadedEventHandler::create(Box::new(move|frame,_|{
            if let (Some(state),Some(frame))=(weak.upgrade(),frame){
                let enabled=state.borrow().filters.is_some();
                frame.cast::<ICoreWebView2Frame2>()?.ExecuteScript(&HSTRING::from(if enabled{"window.__lumoBlock?.start()"}else{"window.__lumoBlock?.stop()"}),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|_,_|Ok(()))))?;
            }Ok(())
        })),&mut token)?;
    }
    Ok(())
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
        Self::new_filtered(owner,url,&Default::default())
    }
    pub fn new_ai(owner:HWND,url:&str,site:crate::ai_sites::Site)->Result<Self,String>{
        Self::new_ai_filtered(owner,url,site,&Default::default())
    }
    pub fn new_filtered(owner:HWND,url:&str,config:&lumo_core::web::Config)->Result<Self,String>{
        Self::new_with_site(owner,url,None,config)
    }
    pub fn new_ai_filtered(owner:HWND,url:&str,site:crate::ai_sites::Site,config:&lumo_core::web::Config)->Result<Self,String>{
        Self::new_with_site(owner,url,Some(site),config)
    }
    fn new_with_site(owner: HWND, url: &str, site:Option<crate::ai_sites::Site>, config:&lumo_core::web::Config) -> Result<Self, String> {
        if !web_url(url) && !(cfg!(test) && url == "about:blank") {
            return Err("This web address is not supported.".into());
        }
        let blocking=crate::web_blocking::load(config.adblocking)?;
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
            parent: owner,
            host,
            closed: false,
            controller: None,
            web: None,
            error: String::new(),
            bounds: None,
            viewport:(1,1),
            cover_top:0,
            overlays:Vec::new(),
            clip_radius:18.0,
            site_events:Vec::new(),
            region_dirty:true,
            filters:blocking,
            frames:Vec::new(),
            blocking_error:String::new(),
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
                    fail(&ready,"Microsoft Edge WebView2 Runtime is unavailable. Install the Evergreen Runtime from Microsoft, then reopen this page.");
                    return Ok(());
                };
                let complete = ready.clone();
                let filter_environment=environment.clone();
                let callback = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        let Some(controller) = controller.filter(|_| result.is_ok()) else {
                            fail(
                                &complete,
                                "Web results could not open. Close this page and try again.",
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
                            // Start at real size, including profiles used by
                            // older builds that fitted pages with zoom.
                            let _ = controller.SetZoomFactor(1.0);
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
                                if let Ok(settings)=settings.cast::<ICoreWebView2Settings6>() {let _=settings.SetIsSwipeNavigationEnabled(false);}
                                let _ = settings.SetIsStatusBarEnabled(false);
                                let _ = settings.SetAreDefaultScriptDialogsEnabled(site.is_some());
                            }
                            let mut token = EventRegistrationToken::default();
                            let swipe_state=Rc::downgrade(&complete);
                            web.add_WebMessageReceived(&webview2_com::WebMessageReceivedEventHandler::create(Box::new(move|_,args|{
                                if let (Some(state),Some(args))=(swipe_state.upgrade(),args){
                                    let mut json=PWSTR::null();args.WebMessageAsJson(&mut json)?;let json=take_string(json);
                                    if json.len()>256{return Ok(());}
                                    if let Ok(value)=serde_json::from_str::<serde_json::Value>(&json){
                                        if value["channel"]=="lumo.workspace-swipe" {
                                            if let Some(delta)=value["delta"].as_f64().filter(|d|d.is_finite()&&*d!=0.0){
                                                let st=state.borrow();if !st.closed&&st.visible {
                                                    let delta=delta.round().clamp(-32767.0,32767.0) as i16;
                                                    let _=PostMessageW(st.owner,SWIPE,WPARAM((delta as u16 as usize)<<16),LPARAM(st.parent.0 as isize));
                                                }
                                            }
                                        }
                                    }
                                }Ok(())
                            })),&mut token)?;
                            web.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(include_str!("web_gestures.js")),&webview2_com::AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_,_|Ok(()))))?;
                            let blocking_state=Rc::downgrade(&complete);
                            web.add_WebMessageReceived(&webview2_com::WebMessageReceivedEventHandler::create(Box::new(move|web,args|{
                                if let (Some(state),Some(web),Some(args))=(blocking_state.upgrade(),web,args){
                                    let mut source=PWSTR::null();let mut json=PWSTR::null();args.Source(&mut source)?;args.WebMessageAsJson(&mut json)?;
                                    let(source,json)=(take_string(source),take_string(json));
                                    if let Some((response,script))=blocker_message(&state,&source,&json){
                                        web.PostWebMessageAsJson(&HSTRING::from(response))?;
                                        if !script.is_empty(){web.ExecuteScript(&HSTRING::from(script),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|_,_|Ok(()))))?;}
                                    }
                                }Ok(())
                            })),&mut token)?;
                            let script=include_str!("adblocking_bridge.js").replace("__LUMO_BLOCKING_ENABLED__",if complete.borrow().filters.is_some(){"true"}else{"false"});
                            web.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(script),&webview2_com::AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_,_|Ok(()))))?;
                            if let Ok(web4)=web.cast::<ICoreWebView2_4>() {
                                let weak=Rc::downgrade(&complete);
                                web4.add_FrameCreated(&webview2_com::FrameCreatedEventHandler::create(Box::new(move|_,args|{
                                    if let (Some(state),Some(args))=(weak.upgrade(),args){register_frame(&state,&args.Frame()?)?;}Ok(())
                                })),&mut token)?;
                            }
                            let filter_state=Rc::downgrade(&complete);
                            let env=filter_environment.clone();
                            web.add_WebResourceRequested(&webview2_com::WebResourceRequestedEventHandler::create(Box::new(move|_,args|{
                                if let (Some(state),Some(args))=(filter_state.upgrade(),args){
                                    let (engine,page,pending)={let st=state.borrow();if st.closed{return Ok(());} (st.filters.clone(),st.navigation.url.clone(),!st.blocking_error.is_empty())};
                                    if engine.is_none() && pending {
                                        let response=env.CreateWebResourceResponse(None,503,windows::core::w!("Protection data unavailable"),windows::core::w!("Content-Length: 0\r\nCache-Control: no-store"))?;
                                        args.SetResponse(&response)?;return Ok(());
                                    }
                                    if let Some(engine)=engine {
                                        let request=args.Request()?;let mut uri=PWSTR::null();request.Uri(&mut uri)?;let uri=take_string(uri);
                                        let mut context=COREWEBVIEW2_WEB_RESOURCE_CONTEXT_OTHER;args.ResourceContext(&mut context)?;
                                        let mut referer=PWSTR::null();let mut method=PWSTR::null();
                                        request.Method(&mut method)?;let method=take_string(method);
                                        let headers=request.Headers()?;
                                        let _=headers.GetHeader(windows::core::w!("Referer"),&mut referer);let referer=take_string(referer);
                                        let source=if web_url(&referer){referer.as_str()}else{page.as_str()};
                                        let decision=engine.check(&uri,source,resource_kind(context,&uri,&page),&method);
                                        if decision.blocked {
                                            let response=if let Some((mime,body))=decision.redirect {
                                                let stream=windows::Win32::UI::Shell::SHCreateMemStream(Some(&body));
                                                env.CreateWebResourceResponse(stream.as_ref(),200,windows::core::w!("OK"),&HSTRING::from(format!("Content-Type: {mime}\r\nCache-Control: no-store")))?
                                            }else{env.CreateWebResourceResponse(None,403,windows::core::w!("Blocked by Lumo ad blocking"),windows::core::w!("Content-Length: 0\r\nCache-Control: no-store"))?};
                                            args.SetResponse(&response)?;
                                        }else if let Some(url)=decision.rewritten_url.filter(|s|web_url(s)){request.SetUri(&HSTRING::from(url))?;}

                                    }
                                }Ok(())
                            })),&mut token)?;
                            if complete.borrow().filters.is_some(){resource_filter(&web,true);}

                            let _ = controller.add_GotFocus(
                                &FocusChangedEventHandler::create(Box::new(move |_, _| {
                                    let _ = PostMessageW(owner, PAGE_FOCUS, WPARAM(site.is_some() as usize), LPARAM(0));
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
                                                    && site.is_none()
                                                {
                                                    args.SetHandled(true)?;
                                                    let _ = PostMessageW(
                                                        owner,
                                                        CLOSE,
                                                        WPARAM(site.is_some() as usize),
                                                        LPARAM(0),
                                                    );
                                                } else if matches!(kind,COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN|COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN)
                                                {
                                                    if let Some(action) = Action::from_key(
                                                        key,
                                                        GetKeyState(VK_CONTROL.0 as i32) < 0,
                                                        GetKeyState(VK_MENU.0 as i32) < 0,
                                                    ) {
                                                        if site.is_some() && action==Action::Address{return Ok(());}
                                                        args.SetHandled(true)?;
                                                        let _ = PostMessageW(
                                                            owner,
                                                            ACTION,
                                                            WPARAM(action as usize),
                                                            LPARAM(site.is_some() as isize),
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
                                            let refresh = !st.navigation.loading;
                                            drop(st);
                                            if refresh { apply_cosmetics(&state, &web); }
                                        } else {
                                            let _ = take_string(uri);
                                        }
                                    }
                                    Ok(())
                                })),
                                &mut token,
                            );
                            let weak = Rc::downgrade(&complete);
                            let title_state=Rc::downgrade(&complete);
                            let _=web.add_DocumentTitleChanged(&DocumentTitleChangedEventHandler::create(Box::new(move|web,_|{
                                if let (Some(state),Some(web))=(title_state.upgrade(),web){
                                    let mut title=PWSTR::null();web.DocumentTitle(&mut title)?;let title=take_string(title);
                                    let mut st=state.borrow_mut();if !st.closed{st.navigation.title=title.chars().take(512).collect();notify(st.owner);}
                                }Ok(())
                            })),&mut token);
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
                                    &DOMContentLoadedEventHandler::create(Box::new(move |web, _| {
                                        if let Some(state) = weak.upgrade() {
                                            if let Some(web)=web {apply_cosmetics(&state,&web);}
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
                            if let Some(site)=site{
                                let weak=Rc::downgrade(&complete);
                                let messages=web.add_WebMessageReceived(&webview2_com::WebMessageReceivedEventHandler::create(Box::new(move|_,args|{
                                    if let (Some(state),Some(args))=(weak.upgrade(),args){
                                        let mut source=PWSTR::null();let mut json=PWSTR::null();
                                        args.Source(&mut source)?;args.WebMessageAsJson(&mut json)?;
                                        let(source,json)=(take_string(source),take_string(json));
                                        if let Some(event)=crate::ai_sites::event(site,&source,&json){
                                            let mut st=state.borrow_mut();
                                            if !st.closed&&st.site_events.len()<16{st.site_events.push(event);notify(st.owner);}
                                        }
                                    }Ok(())
                                })),&mut token);
                                if let Err(error)=messages{crate::pwarn!("AI site notifications unavailable: {error}");}
                                let script=web.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(site.script()),&webview2_com::AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|result,_|{if let Err(error)=result{crate::pwarn!("AI site reply observer unavailable: {error}");}Ok(())})));
                                if let Err(error)=script{crate::pwarn!("AI site reply observer unavailable: {error}");}
                            }
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
                                &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                                    if let Some(args) = args {
                                        {
                                            args.SetState(if site.is_some(){COREWEBVIEW2_PERMISSION_STATE_DEFAULT}else{COREWEBVIEW2_PERMISSION_STATE_DENY})?;
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
                        "Web results could not open. Close this page and try again.",
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
            return Err("Microsoft Edge WebView2 Runtime is unavailable. Install the Evergreen Runtime from Microsoft and reopen this page.".into());
        }
        Ok(Self { state, hook: None })
    }
    pub fn set_blocking(&self,enabled:bool){self.replace_blocker(crate::web_blocking::load(enabled));}
    fn replace_blocker(&self,engine:Result<Option<Rc<lumo_core::adblocking::Engine>>,String>){
        let (web,was,enabled)={let mut st=self.state.borrow_mut();let was=st.filters.is_some() || !st.blocking_error.is_empty();
            match engine{Ok(engine)=>{st.filters=engine;st.blocking_error.clear();},Err(e)=>{st.blocking_error=e;st.filters=None;}}
            (st.web.clone(),was,st.filters.is_some() || !st.blocking_error.is_empty())};
        if let Some(web)=web{if was!=enabled{resource_filter(&web,enabled);}apply_cosmetics(&self.state,&web);}
    }
    pub fn status(&self) -> (bool, String) {
        let st = self.state.borrow();
        (st.web.is_some(), if st.blocking_error.is_empty(){st.error.clone()}else{st.blocking_error.clone()})
    }
    pub fn reparent(&mut self, parent: HWND) -> Result<(), String> {
        let (host, controller) = { let st=self.state.borrow(); (st.host,st.controller.clone()) };
        self.hook=None;
        {let mut st=self.state.borrow_mut();st.bounds=None;st.visible=false;}
        unsafe {
            let _=ShowWindow(host,SW_HIDE);
            if let Some(controller)=controller.as_ref(){let _=controller.SetIsVisible(false);}
            SetParent(host,parent).map_err(|e|e.to_string())?;
            if let Some(controller)=controller { let _=controller.NotifyParentWindowPositionChanged(); }
        }
        let mut st=self.state.borrow_mut(); st.parent=parent; st.bounds=None; st.visible=false;
        Ok(())
    }
    pub fn parent_moved(&self) {
        let controller=self.state.borrow().controller.clone();
        if let Some(controller)=controller {unsafe{let _=controller.NotifyParentWindowPositionChanged();}}
    }
    pub fn place(&mut self, bounds: RECT, visible: bool) {
        let parent=self.state.borrow().parent;
        let scale=unsafe{windows::Win32::UI::HiDpi::GetDpiForWindow(parent)} as f32/96.0;
        self.place_scaled(bounds,visible,scale);
    }
    pub fn hide(&mut self){
        // A hidden in-flight chat keeps its last viewport. Resizing it to the
        // closed island can change provider markup mid-response.
        let bounds={let st=self.state.borrow();st.bounds.unwrap_or(RECT{left:0,top:0,right:st.viewport.0,bottom:st.viewport.1})};
        self.place(bounds,false);
    }
    pub fn place_scaled(&mut self, bounds: RECT, visible: bool, scale: f32) {
        // COM/Win32 calls can dispatch completion callbacks synchronously.
        // Never hold the shared state borrow while entering those APIs.
        let (controller, host, owner, parent, moved, visibility,region_dirty,cover_top,overlays,clip_radius) = {
            let mut st = self.state.borrow_mut();
            let Some(controller) = st.controller.clone() else {
                return;
            };
            let moved = st.bounds != Some(bounds);
            let visibility = st.visible != visible;
            st.bounds = Some(bounds);
            st.viewport=((bounds.right-bounds.left).max(1),(bounds.bottom-bounds.top).max(1));
            st.visible = visible;
            let dirty=st.region_dirty;st.region_dirty=false;
            (controller, st.host, st.owner, st.parent, moved, visibility,dirty,st.cover_top,st.overlays.clone(),st.clip_radius)
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
            }
            if moved || region_dirty {
                let width=(bounds.right-bounds.left).max(1);let height=(bounds.bottom-bounds.top).max(1);
                let diameter = (2.0 * clip_radius * scale).round() as i32;
                let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter);
                if cover_top>0 && !region.is_invalid(){
                    let clip=CreateRectRgn(0,cover_top.min(height),width+1,height+1);
                    if !clip.is_invalid(){let _=CombineRgn(region,region,clip,RGN_AND);let _=DeleteObject(clip);}
                }
                // Cover only the native overlay controls, without reducing the
                // website's layout viewport or injecting any visual elements.
                for overlay in &overlays{
                    let cut=CreateRoundRectRgn(overlay.left,overlay.top,overlay.right+1,overlay.bottom+1,(28.0*scale)as i32,(28.0*scale)as i32);
                    if !cut.is_invalid(){let _=CombineRgn(region,region,cut,RGN_DIFF);let _=DeleteObject(cut);}
                }
                if !region.is_invalid() && SetWindowRgn(host, region, true) == 0 {
                    let _ = DeleteObject(region);
                }
            }
            if moved {
                let _ = controller.SetBounds(RECT {
                    left: 0,
                    top: 0,
                    right: (bounds.right-bounds.left).max(1),
                    bottom: (bounds.bottom-bounds.top).max(1),
                });
            }
            if visibility {
                let _ = controller.SetIsVisible(visible);
                let _ = ShowWindow(host, if visible { SW_SHOWNA } else { SW_HIDE });
            }
        }
        if visible && self.hook.is_none() {
            self.hook = SwipeHook::new(owner,parent).ok();
        } else if !visible {
            self.hook = None;
        }
    }
    pub fn cover_top(&self,pixels:i32){let mut st=self.state.borrow_mut();let pixels=pixels.max(0);if st.cover_top!=pixels{st.cover_top=pixels;st.region_dirty=true;}}
    pub fn overlays(&self,rects:Vec<RECT>){let mut st=self.state.borrow_mut();if st.overlays!=rects{st.overlays=rects;st.region_dirty=true;}}
    pub fn clip_radius(&self,radius:f32){let mut st=self.state.borrow_mut();if st.clip_radius!=radius{st.clip_radius=radius;st.region_dirty=true;}}
    pub fn parent(&self)->HWND{self.state.borrow().parent}
    pub fn take_site_events(&self)->Vec<crate::ai_sites::Event>{std::mem::take(&mut self.state.borrow_mut().site_events)}
    #[cfg(test)]
    pub fn test_web(&self) -> Option<ICoreWebView2> {
        self.state.borrow().web.clone()
    }
    #[cfg(test)]
    pub fn test_host(&self) -> HWND {
        self.state.borrow().host
    }
    #[cfg(test)]
    pub fn test_viewport(&self)->(i32,i32){self.state.borrow().viewport}
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
thread_local! {
    static OWNER:Cell<HWND>=const{Cell::new(HWND(std::ptr::null_mut()))};
    // Search and AI can briefly overlap during a same-owner handoff. Dropping
    // one hook must not clear the other visible page's gesture destination.
    static HOOK_USERS:Cell<u32>=const{Cell::new(0)};
    static HOOK_TARGETS:RefCell<Vec<(isize,HWND)>>=const{RefCell::new(Vec::new())};
}
struct SwipeHook(HHOOK);
unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && wp.0 == WM_MOUSEHWHEEL as usize {
        let event = &*(lp.0 as *const MSLLHOOKSTRUCT);
        let owner = OWNER.with(Cell::get);
        let target = WindowFromPoint(event.pt);
        let parent=HOOK_TARGETS.with(|targets|targets.borrow().iter().find(|(_,parent)|target==*parent||IsChild(*parent,target).as_bool()).map(|(_,parent)|*parent));
        if let Some(parent)=parent.filter(|_|!owner.is_invalid()) {
            let delta = (event.mouseData >> 16) as u16;
            let _ = PostMessageW(owner, SWIPE, WPARAM((delta as usize) << 16), LPARAM(parent.0 as isize));
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}
impl SwipeHook {
    fn new(owner: HWND,parent:HWND) -> windows::core::Result<Self> {
        unsafe {
            let hook =
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), GetModuleHandleW(None)?, 0)?;
            OWNER.with(|cell| cell.set(owner));
            HOOK_USERS.with(|users|users.set(users.get()+1));
            HOOK_TARGETS.with(|targets|targets.borrow_mut().push((hook.0 as isize,parent)));
            Ok(Self(hook))
        }
    }
}
impl Drop for SwipeHook {
    fn drop(&mut self) {
        HOOK_TARGETS.with(|targets|targets.borrow_mut().retain(|(id,_)|*id!=self.0.0 as isize));
        HOOK_USERS.with(|users|{let remaining=users.get().saturating_sub(1);users.set(remaining);if remaining==0{OWNER.with(|cell|cell.set(HWND::default()));}});
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    struct LocalServer {
        address:std::net::SocketAddr,
        stop:std::sync::Arc<std::sync::atomic::AtomicBool>,
        worker:Option<std::thread::JoinHandle<()>>,
    }
    impl LocalServer {
        fn new()->Self {
            use std::io::{Read,Write};
            let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
            let stop=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));let signal=stop.clone();
            let worker=std::thread::spawn(move||{
                for stream in listener.incoming(){
                    if signal.load(std::sync::atomic::Ordering::Relaxed){break;}
                    let Ok(mut stream)=stream else{break;};let _=stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut bytes=[0u8;4096];let n=stream.read(&mut bytes).unwrap_or(0);let request=String::from_utf8_lossy(&bytes[..n]);
                    let (body,mime)=if request.starts_with("GET /blocked.js ") {("window.unblockedScript=true;","application/javascript")}
                        else if request.starts_with("GET /frame.html "){("<!doctype html><div class='advert'>Ad</div><div class='card'>Sponsored content</div>","text/html")}
                        else{("<!doctype html><div class='advert'>Ad</div><p>Content</p>","text/html")};
                    let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                }
            });
            Self{address,stop,worker:Some(worker)}
        }
        fn url(&self,path:&str)->String{format!("http://{}{path}",self.address)}
    }
    impl Drop for LocalServer {
        fn drop(&mut self){self.stop.store(true,std::sync::atomic::Ordering::Relaxed);let _=std::net::TcpStream::connect(self.address);if let Some(worker)=self.worker.take(){let _=worker.join();}}
    }
    fn evaluate(web:&ICoreWebView2,script:&str)->String {
        let result=Rc::new(RefCell::new(None));let done=result.clone();
        let started=unsafe {web.ExecuteScript(&HSTRING::from(script),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|status,text|{
            *done.borrow_mut()=Some(if status.is_ok(){text.to_string()}else{String::new()});Ok(())
        })))};
        if started.is_err(){return String::new();}
        pump_until(||result.borrow().is_some());
        let value=result.borrow_mut().take().unwrap();value
    }
    #[test] #[ignore="requires a Windows desktop and WebView2; all content is offline"]
    fn shared_filters_block_requests_hide_elements_and_disable_live() {
        unsafe {
            let home=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/web-filter-test-home");
            std::fs::create_dir_all(&home).unwrap();std::env::set_var("LUMO_HOME",home);
            windows::Win32::System::Ole::OleInitialize(None).unwrap();
            let owner=CreateWindowExW(WINDOW_EX_STYLE(0),windows::core::w!("STATIC"),windows::core::w!("Lumo filters fixture"),WS_POPUP|WS_VISIBLE,0,0,640,480,None,None,GetModuleHandleW(None).unwrap(),None).unwrap();
            let config=lumo_core::web::Config::default();
            let server=LocalServer::new();
            let fixture=Rc::new(lumo_core::adblocking::Engine::from_lists(["/blocked.js$script\n/redirect.js$script,redirect=replacement.js\n##.advert\n##.late-ad\n127.0.0.1##.card:has-text(Sponsored)\n127.0.0.1##+js(fixture, true)".into()],r#"[{"name":"fixture.js","kind":"template","content":"d2luZG93Ll9fZml4dHVyZT17ezF9fTs="},{"name":"replacement.js","kind":{"mime":"application/javascript"},"content":"d2luZG93Ll9fcmVkaXJlY3RlZD10cnVlOw=="}]"#).unwrap());
            for site in [None,Some(crate::ai_sites::Site::ChatGpt)] {
                let mut browser=Browser::new_with_site(owner,"about:blank",site,&config).unwrap();
                pump_until(||browser.status().0||!browser.status().1.is_empty());assert!(browser.status().0,"{}",browser.status().1);
                browser.place_scaled(RECT{left:0,top:0,right:640,bottom:480},true,1.0);
                let web=browser.test_web().unwrap();
                browser.replace_blocker(Ok(Some(fixture.clone())));
                pump_until(||!browser.navigation().loading);
                assert!(browser.status().1.is_empty(),"{}",browser.status().1);
                let mut swipe=BOOL(1);web.Settings().unwrap().cast::<ICoreWebView2Settings6>().unwrap().IsSwipeNavigationEnabled(&mut swipe).unwrap();
                assert!(!swipe.as_bool(),"history swipe must be disabled in both views");
                // The loopback server supplies every unblocked request;
                // no DNS or third-party request is performed.
                let env=Rc::new(Cell::new(false));let observed=env.clone();
                let mut token=EventRegistrationToken::default();
                web.add_WebResourceRequested(&webview2_com::WebResourceRequestedEventHandler::create(Box::new(move|_,args|{
                    if let Some(args)=args {
                        let mut response=args.Response().ok();
                        if let Some(response)=response.take(){let mut status=0;response.StatusCode(&mut status)?;if status==403{observed.set(true);}}
                    }Ok(())
                })),&mut token).unwrap();
                browser.navigate(&server.url("/"));
                pump_until(||!browser.navigation().loading);
                pump_until(||evaluate(&web,"!!document.querySelector('.advert')")=="true");
                pump_until(||evaluate(&web,"getComputedStyle(document.querySelector('.advert')).display")=="\"none\"");
                assert_eq!(evaluate(&web,"document.documentElement.style.overscrollBehaviorX"),"\"none\"");
                evaluate(&web,"const s=document.createElement('script');s.src='/blocked.js';s.onerror=()=>window.blockedError=true;document.head.append(s)");
                pump_until(||env.get());
                pump_until(||evaluate(&web,"window.blockedError===true")=="true");
                pump_until(||evaluate(&web,"window.__fixture===true")=="true");
                evaluate(&web,"const r=document.createElement('script');r.src='/redirect.js';document.head.append(r)");
                pump_until(||evaluate(&web,"window.__redirected===true")=="true");
                evaluate(&web,"const late=document.createElement('div');late.className='late-ad';document.body.append(late);");
                pump_until(||evaluate(&web,"getComputedStyle(document.querySelector('.late-ad')).display")=="\"none\"");
                evaluate(&web,"const frame=document.createElement('iframe');frame.src='/frame.html';document.body.append(frame);");
                pump_until(||!browser.state.borrow().frames.is_empty());
                let frame=browser.state.borrow().frames[0].clone();
                let frame_eval=|script:&str|{
                    let out=Rc::new(RefCell::new(None));let done=out.clone();
                    frame.ExecuteScript(&HSTRING::from(script),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|_,text|{*done.borrow_mut()=Some(text.to_string());Ok(())}))).unwrap();
                    pump_until(||out.borrow().is_some());let text=out.borrow_mut().take().unwrap();text
                };
                pump_until(||frame_eval("!!document.querySelector('.card')")=="true");
                pump_until(||frame_eval("getComputedStyle(document.querySelector('.advert')).display")=="\"none\"");
                pump_until(||frame_eval("getComputedStyle(document.querySelector('.card')).display")=="\"none\"");
                browser.replace_blocker(Ok(None));
                pump_until(||evaluate(&web,"getComputedStyle(document.querySelector('.advert')).display")=="\"block\"");
                browser.replace_blocker(Ok(Some(fixture.clone())));
                pump_until(||evaluate(&web,"getComputedStyle(document.querySelector('.advert')).display")=="\"none\"");
                browser.replace_blocker(Err("Protection data unavailable".into()));
                evaluate(&web,"fetch('/pending').then(r=>window.__pendingStatus=r.status)");
                pump_until(||evaluate(&web,"window.__pendingStatus")=="503");
                browser.replace_blocker(Ok(None));
                evaluate(&web,"fetch('/').then(r=>window.__enabledStatus=r.status)");
                pump_until(||evaluate(&web,"window.__enabledStatus")=="200");
                // A horizontal packet over a reparented mini host still reaches
                // the island owner and is swallowed before WebView2 sees it.
                let mini=CreateWindowExW(WINDOW_EX_STYLE(0),windows::core::w!("STATIC"),windows::core::w!("Mini fixture"),WS_POPUP|WS_VISIBLE,100,100,640,480,None,None,GetModuleHandleW(None).unwrap(),None).unwrap();
                browser.reparent(mini).unwrap();browser.place_scaled(RECT{left:0,top:0,right:640,bottom:480},true,1.0);
                assert_eq!(browser.test_swipe(POINT{x:120,y:120}),LRESULT(1));
                let mut msg=MSG::default();assert!(PeekMessageW(&mut msg,owner,SWIPE,SWIPE,PM_REMOVE).as_bool());
                assert_eq!((msg.wParam.0>>16)as u16 as i16,120);
                drop(browser);DestroyWindow(mini).unwrap();
            }
            DestroyWindow(owner).unwrap();windows::Win32::System::Ole::OleUninitialize();
        }
    }
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
    #[ignore="requires a Windows desktop and WebView2; all content is offline"]
    fn ai_sites_bridge_observes_replies_and_full_page_overlay_without_polling(){
        unsafe{
            use crate::ai_sites::{Site,Event};
            use windows::Win32::Graphics::Gdi::{GetWindowRgn,PtInRegion};
            let home=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/ai-sites-web-qa-home");
            std::fs::create_dir_all(&home).unwrap();std::env::set_var("LUMO_HOME",home);
            windows::Win32::System::Ole::OleInitialize(None).unwrap();
            let owner=CreateWindowExW(WINDOW_EX_STYLE(0),windows::core::w!("STATIC"),windows::core::w!("Lumo AI site fixture"),WS_POPUP,0,0,640,480,None,None,GetModuleHandleW(None).unwrap(),None).unwrap();
            let eval=|web:&ICoreWebView2,script:&str|{
                let done=Rc::new(RefCell::new(None));let signal=done.clone();
                web.ExecuteScript(&HSTRING::from(script),&webview2_com::ExecuteScriptCompletedHandler::create(Box::new(move|result,value|{result?;*signal.borrow_mut()=Some(value);Ok(())}))).unwrap();
                pump_until(||done.borrow().is_some());
                let result=done.borrow_mut().take().unwrap();result
            };
            // Search and AI use the same existing profile. The only cookie
            // involved is a fictional value inside this isolated QA home.
            let mut search=Browser::new(owner,"about:blank").unwrap();pump_until(||search.status().0);
            search.place_scaled(RECT{left:0,top:0,right:640,bottom:480},true,1.0);
            let cookies=search.test_web().unwrap().cast::<ICoreWebView2_2>().unwrap().CookieManager().unwrap();
            let cookie=cookies.CreateCookie(windows::core::w!("lumo_ai_site_qa"),windows::core::w!("shared-fixture"),windows::core::w!("chatgpt.com"),windows::core::w!("/")).unwrap();
            cookies.AddOrUpdateCookie(&cookie).unwrap();
            for site in Site::ALL{
                let mut browser=Browser::new_ai(owner,"about:blank",site).unwrap();
                pump_until(||browser.status().0||!browser.status().1.is_empty());assert!(browser.status().0,"{}",browser.status().1);
                browser.overlays(vec![RECT{left:440,top:8,right:632,bottom:46}]);
                browser.place_scaled(RECT{left:0,top:0,right:640,bottom:480},true,1.0);
                let region=CreateRectRgn(0,0,0,0);GetWindowRgn(browser.test_host(),region);
                assert!(!PtInRegion(region,500,20).as_bool());assert!(PtInRegion(region,300,20).as_bool());let _=DeleteObject(region);
                let web=browser.test_web().unwrap();
                if site==Site::ChatGpt{
                    let found=Rc::new(RefCell::new(None));let signal=found.clone();
                    web.cast::<ICoreWebView2_2>().unwrap().CookieManager().unwrap().GetCookies(windows::core::w!("https://chatgpt.com/"),&webview2_com::GetCookiesCompletedHandler::create(Box::new(move|result,list|{
                        result?;let mut matched=false;
                        if let Some(list)=list{let mut count=0;list.Count(&mut count)?;for index in 0..count{let cookie=list.GetValueAtIndex(index)?;let mut name=PWSTR::null();cookie.Name(&mut name)?;if take_string(name)=="lumo_ai_site_qa"{let mut value=PWSTR::null();cookie.Value(&mut value)?;matched=take_string(value)=="shared-fixture";}}}
                        *signal.borrow_mut()=Some(matched);Ok(())
                    }))).unwrap();
                    pump_until(||found.borrow().is_some());assert_eq!(*found.borrow(),Some(true),"AI and Search share cookies");
                }
                // The same fixture carries each provider's recognized response
                // shape; no account, prompt, or third-party navigation is used.
                web.NavigateToString(windows::core::w!("<!doctype html><textarea>Local fixture prompt</textarea><button data-testid='send-button'>Send</button><div id='reply' data-message-author-role='assistant' data-message-role='assistant' data-testid='assistant-message' class='font-claude-response prose'>Previous answer</div><model-response><message-content id='gemini-reply'>Previous answer</message-content></model-response>")).unwrap();
                pump_until(||eval(&web,"document.getElementById('reply') !== null")=="true");
                assert_eq!(eval(&web,"window.__lumoAiSite === true"),"true","the observer is installed automatically on document creation");
                eval(&web,"document.querySelector('[data-testid=send-button]').click(); const stop=document.createElement('button');stop.id='stop';stop.setAttribute('data-testid','stop-button');stop.textContent='Stop';document.body.append(stop);");
                pump_until(||browser.state.borrow().site_events.contains(&Event::Working));
                assert!(!browser.take_site_events().iter().any(|e|matches!(e,Event::Ready(_))));
                eval(&web,"document.getElementById('reply').textContent='Actual local fixture answer'; document.getElementById('gemini-reply').textContent='Actual local fixture answer';document.getElementById('stop').remove();");
                pump_until(||browser.state.borrow().site_events.iter().any(|e|matches!(e,Event::Ready(_))));
                assert!(browser.take_site_events().contains(&Event::Ready("Actual local fixture answer".into())));
                if site==Site::Muse {
                    browser.hide();
                    eval(&web,"document.querySelector('textarea').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));const answer=document.createElement('div');answer.setAttribute('data-role','assistant');answer.textContent='Muse completed local answer';document.body.append(answer)");
                    pump_until(||browser.state.borrow().site_events.iter().any(|e|matches!(e,Event::Ready(text)if text=="Muse completed local answer")));
                    assert!(browser.take_site_events().iter().any(|e|matches!(e,Event::Ready(_))));
                }
                eval(&web,"document.getElementById('reply').textContent='Unrelated later history';document.getElementById('gemini-reply').textContent='Unrelated later history';");
                assert!(browser.take_site_events().is_empty(),"idle history updates must not notify");
                let retained=web.clone();browser.reparent(owner).unwrap();assert_eq!(browser.test_web().unwrap(),retained);
            }
            assert_eq!(OWNER.with(Cell::get),owner,"closing AI must not disable a retained Search hook");
            cookies.DeleteCookie(&cookie).unwrap();drop(search);assert!(OWNER.with(Cell::get).is_invalid());
            DestroyWindow(owner).unwrap();windows::Win32::System::Ole::OleUninitialize();
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
