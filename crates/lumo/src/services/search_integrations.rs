//! Optional local Search sources. One bounded worker per opening/refresh;
//! no watchers, timers, browser debugging ports or work while Search is closed.
use crate::events::{AppEvent, EventSink};
use lumo_core::search::{Config, Entry, Group, Service, Target};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows::{
    core::{w, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        System::{Registry::*, Services::*, Threading::*},
        UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
};

pub struct CatalogEvent {
    pub request: u64,
    pub entries: Vec<Entry>,
    pub warning: String,
}
pub fn enabled(c: &Config) -> bool {
    c.services || c.powertoys || c.vscode_commands
}
pub fn start(request: u64, c: Config, cancel: Arc<AtomicBool>, sink: EventSink) {
    let failed = sink.clone();
    if std::thread::Builder::new()
        .name("lumo-search-sources".into())
        .spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| catalog(&c, &cancel)));
            let (entries, warning) = result.unwrap_or_else(|_| {
                (
                    vec![],
                    "Extra search sources could not load. Refresh to try again.".into(),
                )
            });
            sink.send(AppEvent::SearchIntegrations(CatalogEvent {
                request,
                entries,
                warning,
            }));
        })
        .is_err()
    {
        failed.send(AppEvent::SearchIntegrations(CatalogEvent {
            request,
            entries: vec![],
            warning: "Extra search sources could not start.".into(),
        }));
    }
}
fn cancelled(c: &AtomicBool) -> bool {
    c.load(Ordering::Relaxed)
}
fn catalog(c: &Config, cancel: &AtomicBool) -> (Vec<Entry>, String) {
    let mut out = vec![];
    let mut warnings = vec![];
    if cancelled(cancel) {
        return (out, String::new());
    }
    if c.services {
        match service_entries(cancel) {
            Ok(items) => out.extend(items),
            Err(_) => warnings.push("Windows services are unavailable."),
        }
    }
    if cancelled(cancel) {
        return (out, String::new());
    }
    if c.powertoys {
        out.extend(powertoys());
    }
    if cancelled(cancel) {
        return (out, String::new());
    }
    if c.vscode_commands {
        let windows = code_windows();
        if windows.len() >= 16 {
            warnings.push("VS Code search reached its window limit.");
        }
        vscode_entries(&windows, &mut out);
    }
    if out.len() > 2048 {
        out.truncate(2048);
        warnings.push("Search reached its local result limit.");
    }
    (out, warnings.join(" "))
}

struct Sc(SC_HANDLE);
impl Drop for Sc {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}
fn service_entries(cancel: &AtomicBool) -> windows::core::Result<Vec<Entry>> {
    unsafe {
        let manager = Sc(OpenSCManagerW(None, None, SC_MANAGER_ENUMERATE_SERVICE)?);
        // Win32 caps one enumeration buffer at 256 KiB. u64 backing keeps the
        // returned structures aligned; pointers remain within this live buffer.
        let mut storage = vec![0u64; 256 * 1024 / 8];
        let mut out = vec![];
        let mut resume = 0;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if cancelled(cancel) {
                return Ok(vec![]);
            }
            let buffer = std::slice::from_raw_parts_mut(
                storage.as_mut_ptr().cast::<u8>(),
                storage.len() * 8,
            );
            let mut needed = 0;
            let mut count = 0;
            let result = EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(buffer),
                &mut needed,
                &mut count,
                Some(&mut resume),
                None,
            );
            if count as usize > buffer.len() / std::mem::size_of::<ENUM_SERVICE_STATUS_PROCESSW>() {
                return Err(E_FAIL.into());
            }
            for s in std::slice::from_raw_parts(
                buffer.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                count as usize,
            ) {
                let name = s.lpServiceName.to_string()?;
                let title = s.lpDisplayName.to_string()?;
                let service = Service {
                    name: name.clone(),
                    state: s.ServiceStatusProcess.dwCurrentState.0,
                    accepts_stop: s.ServiceStatusProcess.dwControlsAccepted & SERVICE_ACCEPT_STOP
                        != 0,
                };
                out.push(Entry::new(
                    format!("service-{name}"),
                    title,
                    format!("{} · Windows service", service.status()),
                    &format!("{name} service services start stop"),
                    Group::Service,
                    Target::Service(service),
                ));
                if out.len() >= 1024 {
                    return Err(E_BOUNDS.into());
                }
            }
            match result {
                Ok(()) => break,
                Err(e)
                    if e.code() == ERROR_MORE_DATA.to_hresult()
                        && resume != 0
                        && Instant::now() < deadline => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
}
fn system_file(name: &str) -> Option<String> {
    unsafe {
        let mut path = [0u16; 32768];
        let n = windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut path));
        if n == 0 || n as usize >= path.len() {
            return None;
        }
        Some(
            PathBuf::from(String::from_utf16_lossy(&path[..n as usize]))
                .join(name)
                .to_string_lossy()
                .into_owned(),
        )
    }
}
fn service_error(code: u32) -> String {
    match code {
        5 => "Windows denied permission to control this service.".into(),
        1223 => "Administrator request was cancelled.".into(),
        1051 => "Other running services depend on this service. No dependents were stopped.".into(),
        1058 => {
            "This service is disabled in Windows. Change its startup type in Services first.".into()
        }
        1060 => "This service is no longer installed.".into(),
        1061 => "This service is busy. Refresh its status before trying again.".into(),
        1056 => "The service is already running.".into(),
        1062 => "The service is already stopped.".into(),
        _ => format!("Windows could not complete the service request (error {code})."),
    }
}
fn control_service(name: &str, start: bool) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 256
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '/' | '\\'))
    {
        return Err("Invalid service name.".into());
    }
    let direct = (|| -> windows::core::Result<()> {
        unsafe {
            let manager = Sc(OpenSCManagerW(None, None, SC_MANAGER_CONNECT)?);
            let handle = Sc(OpenServiceW(
                manager.0,
                &HSTRING::from(name),
                SERVICE_QUERY_STATUS | if start { SERVICE_START } else { SERVICE_STOP },
            )?);
            if start {
                StartServiceW(handle.0, None)
            } else {
                let mut status = SERVICE_STATUS::default();
                ControlService(handle.0, SERVICE_CONTROL_STOP, &mut status)
            }
        }
    })();
    if let Err(error) = direct {
        if error.code() != ERROR_ACCESS_DENIED.to_hresult() {
            return Err(service_error(error.code().0 as u32 & 0xffff));
        }
        // Elevate only Microsoft's fixed service controller for this explicit
        // action. No shell interpreter, startup-type change or dependent stop.
        let exe = HSTRING::from(
            system_file("sc.exe").ok_or("Windows service controller was not found.")?,
        );
        let args = HSTRING::from(format!(
            "{} {}",
            if start { "start" } else { "stop" },
            quote(name)
        ));
        let _mta = super::http::MtaGuard::new();
        unsafe {
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
                lpVerb: w!("runas"),
                lpFile: PCWSTR(exe.as_ptr()),
                lpParameters: PCWSTR(args.as_ptr()),
                nShow: SW_HIDE.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info).map_err(|e| service_error(e.code().0 as u32 & 0xffff))?;
            if info.hProcess.is_invalid() {
                return Err("Windows did not return the service request result.".into());
            }
            let process = Process(info.hProcess);
            if WaitForSingleObject(process.0, 30000) != WAIT_OBJECT_0 {
                return Err(
                    "The service request is still running. Refresh status in a moment.".into(),
                );
            }
            let mut code = 0;
            GetExitCodeProcess(process.0, &mut code)
                .map_err(|_| "Could not read the service request result.")?;
            if code != 0 {
                return Err(service_error(code));
            }
        }
    }
    Ok(format!(
        "{} requested. Refresh to check the latest service state.",
        if start { "Start" } else { "Stop" }
    ))
}

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn executable(pid: u32) -> Option<String> {
    unsafe {
        let process = Process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut buffer = [0u16; 32768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .ok()?;
        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}
#[derive(Clone)]
struct CodeWindow {
    hwnd: u64,
    pid: u32,
    path: String,
    title: String,
}
fn code_windows() -> Vec<CodeWindow> {
    struct Scan {
        windows: Vec<CodeWindow>,
        visited: usize,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
        let scan = &mut *(param.0 as *mut Scan);
        scan.visited += 1;
        let out = &mut scan.windows;
        if out.len() >= 16 || scan.visited > 512 {
            return FALSE;
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Some(path) = executable(pid) else {
            return TRUE;
        };
        let name = Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if !matches!(name.as_str(), "code.exe" | "code - insiders.exe") {
            return TRUE;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return TRUE;
        }
        let title = if name.starts_with("code") {
            let mut text = [0u16; 1024];
            let n = GetWindowTextW(hwnd, &mut text);
            String::from_utf16_lossy(&text[..n as usize])
        } else {
            String::new()
        };
        out.push(CodeWindow {
            hwnd: hwnd.0 as usize as u64,
            pid,
            path,
            title,
        });
        TRUE
    }
    let mut scan = Scan {
        windows: vec![],
        visited: 0,
    };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut scan as *mut _ as isize));
    }
    scan.windows
}
fn reg_app(name: &str) -> Option<PathBuf> {
    reg_path(
        &format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{name}"),
        false,
    )
}
fn command_executable(value: &str) -> Option<&str> {
    let value = value.trim();
    if let Some(tail) = value.strip_prefix('"') {
        tail.split_once('"').map(|(path, _)| path)
    } else {
        value.split_whitespace().next()
    }
}
fn reg_path(subkey: &str, command: bool) -> Option<PathBuf> {
    unsafe {
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                let mut key = HKEY::default();
                let path = HSTRING::from(subkey);
                if RegOpenKeyExW(root, &path, 0, KEY_READ | view, &mut key).is_err() {
                    continue;
                }
                let mut buffer = [0u16; 32768];
                let mut bytes = (buffer.len() * 2) as u32;
                let mut kind = REG_VALUE_TYPE::default();
                let result = RegQueryValueExW(
                    key,
                    None,
                    None,
                    Some(&mut kind),
                    Some(buffer.as_mut_ptr().cast()),
                    Some(&mut bytes),
                );
                let _ = RegCloseKey(key);
                if result.is_ok() && kind == REG_SZ && bytes as usize <= buffer.len() * 2 {
                    let text = String::from_utf16_lossy(&buffer[..bytes as usize / 2]);
                    let text = text.trim_end_matches('\0');
                    let Some(value) = (if command {
                        command_executable(text)
                    } else {
                        Some(text.trim_matches('"'))
                    }) else {
                        continue;
                    };
                    let path = PathBuf::from(value);
                    let local = matches!(path.components().next(),Some(std::path::Component::Prefix(prefix))if matches!(prefix.kind(),std::path::Prefix::Disk(_)|std::path::Prefix::VerbatimDisk(_)));
                    if local && path.is_absolute() && path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }
}
fn item(
    id: &str,
    title: &str,
    subtitle: &str,
    keywords: &str,
    group: Group,
    target: Target,
) -> Entry {
    Entry::new(
        id.into(),
        title.into(),
        subtitle.into(),
        keywords,
        group,
        target,
    )
}
fn launch(path: &Path, arguments: String) -> Target {
    Target::Launch {
        path: path.to_string_lossy().into_owned(),
        arguments,
    }
}
pub fn quote(value: &str) -> String {
    // Windows argv quoting, including embedded quotes and trailing backslashes.
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n('\\', slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

// Activation events and Settings arguments are the same public interfaces used
// by PowerToys Run (Microsoft/PowerToys common/interop/shared_constants.h and
// common/Common.UI/SettingsDeepLink.cs). OpenEvent never creates a new listener.
const TOYS:&[(&str,&str,&str,&str)]=&[
    ("ColorPicker","Color Picker","pick colour color eyedropper","Local\\ShowColorPickerEvent-8c46be2a-3e05-4186-b56b-4ae986ef2525"),
    ("FancyZones","FancyZones editor","window layout zones","Local\\FancyZones-ToggleEditorEvent-1e174338-06a3-472b-874d-073b21c62f14"),
    ("Hosts","Hosts File Editor","hosts dns","Local\\Hosts-ShowHostsEvent-5a0c0aae-5ff5-40f5-95c2-20e37ed671f0"),
    ("MeasureTool","Screen Ruler","measure pixels screen","Local\\MeasureToolEvent-3d46745f-09b3-4671-a577-236be7abd199"),
    ("PowerOcr","Text Extractor","ocr capture text","Local\\PowerOCREvent-dc864e06-e1af-4ecc-9078-f98bee745e3a"),
    ("ShortcutGuide","Shortcut Guide","keyboard shortcuts","Local\\ShortcutGuide-TriggerEvent-d4275ad3-2531-4d19-9252-c0becbd9b496"),
    ("RegistryPreview","Registry Preview","registry reg","Local\\RegistryPreviewEvent-4C559468-F75A-4E7F-BC4F-9C9688316687"),
    ("EnvironmentVariables","Environment Variables editor","environment path variables","Local\\PowerToysEnvironmentVariables-ShowEnvironmentVariablesEvent-1021f616-e951-4d64-b231-a8f972159978"),
    ("Workspaces","Workspaces editor","workspace layouts","Local\\Workspaces-LaunchEditorEvent-a55ff427-cf62-4994-a2cd-9f72139296bf"),
];
fn powertoys() -> Vec<Entry> {
    let mut candidates = vec![];
    if let Some(path) =
        reg_path("Software\\Classes\\powertoys\\shell\\open\\command", true).filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("PowerToys.exe"))
        })
    {
        candidates.push(path);
    }
    if let Some(path) = reg_app("PowerToys.exe") {
        candidates.push(path);
    }
    for (env, relative) in [
        ("ProgramFiles", "PowerToys/PowerToys.exe"),
        ("LOCALAPPDATA", "PowerToys/PowerToys.exe"),
        ("LOCALAPPDATA", "Programs/PowerToys/PowerToys.exe"),
    ] {
        if let Some(root) = std::env::var_os(env) {
            candidates.push(PathBuf::from(root).join(relative));
        }
    }
    let Some(exe) = candidates.into_iter().find(|p| p.is_file()) else {
        return vec![];
    };
    let mut out = vec![item(
        "toy-settings",
        "PowerToys settings",
        "PowerToys",
        "tools utilities",
        Group::PowerToys,
        launch(&exe, "--open-settings=Dashboard".into()),
    )];
    for &(key, title, keywords, event) in TOYS {
        // Settings entries remain available even if the utility is disabled.
        out.push(item(
            &format!("toy-settings-{key}"),
            &format!("{title} settings"),
            "PowerToys",
            &format!("powertoys {keywords}"),
            Group::PowerToys,
            launch(&exe, format!("--open-settings={key}")),
        ));
        if unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(event)) }
            .map(|h| {
                drop(Process(h));
            })
            .is_ok()
        {
            out.push(item(
                &format!("toy-{key}"),
                title,
                "PowerToys · Open tool",
                &format!("powertoys {keywords}"),
                Group::PowerToys,
                Target::PowerToy {
                    event: event.into(),
                },
            ));
        }
    }
    for (key, title) in [
        ("AlwaysOnTop", "Always On Top"),
        ("Awake", "Awake"),
        ("KBM", "Keyboard Manager"),
        ("ImageResizer", "Image Resizer"),
        ("PowerRename", "PowerRename"),
        ("MouseUtils", "Mouse utilities"),
        ("AdvancedPaste", "Advanced Paste"),
        ("Peek", "Peek"),
        ("PowerLauncher", "PowerToys Run"),
        ("CmdPal", "Command Palette"),
    ] {
        out.push(item(
            &format!("toy-settings-{key}"),
            &format!("{title} settings"),
            "PowerToys",
            "powertoys tool settings",
            Group::PowerToys,
            launch(&exe, format!("--open-settings={key}")),
        ));
    }
    out
}

const CODE_COMMANDS: &[(&str, &str, &[u16])] = &[
    ("Command Palette", "commands actions", &[0x11, 0x10, 0x50]),
    ("Settings", "preferences options", &[0x11, 0xBC]),
    ("Find files", "quick open", &[0x11, 0x50]),
    (
        "Find in files",
        "search project workspace",
        &[0x11, 0x10, 0x46],
    ),
    ("Go to symbol", "outline symbol", &[0x11, 0x10, 0x4F]),
    ("Show terminal", "terminal console", &[0x11, 0xC0]),
    ("New terminal", "terminal console", &[0x11, 0x10, 0xC0]),
    ("Extensions", "plugins marketplace", &[0x11, 0x10, 0x58]),
    ("Source control", "git changes", &[0x11, 0x10, 0x47]),
    ("Explorer", "files sidebar", &[0x11, 0x10, 0x45]),
    ("Run and debug", "debugger", &[0x11, 0x10, 0x44]),
    ("Problems", "errors warnings", &[0x11, 0x10, 0x4D]),
    ("New file", "untitled editor", &[0x11, 0x4E]),
];
fn vscode_entries(windows: &[CodeWindow], out: &mut Vec<Entry>) {
    for window in windows.iter().filter(|w| {
        Path::new(&w.path)
            .file_name()
            .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("code"))
    }) {
        for (title, words, keys) in CODE_COMMANDS {
            out.push(item(
                &format!("code-{}-{}-{title}", window.pid, window.hwnd),
                &format!("VS Code: {title}"),
                &window.title,
                &format!("vscode code {words}"),
                Group::VSCode,
                Target::AppCommand {
                    window: window.hwnd,
                    pid: window.pid,
                    keys: keys.to_vec(),
                },
            ));
        }
    }
}
fn valid_window(window: u64, pid: u32) -> Option<HWND> {
    unsafe {
        let h = HWND(window as usize as *mut _);
        let mut current = 0;
        GetWindowThreadProcessId(h, Some(&mut current));
        (IsWindow(h).as_bool() && current == pid).then_some(h)
    }
}
pub fn target_available(target: &Target) -> bool {
    match target {
        Target::AppCommand { window, pid, .. } => valid_window(*window, *pid).is_some(),
        _ => true,
    }
}
fn code_command(window: u64, pid: u32, keys: &[u16]) -> Result<(), String> {
    unsafe {
        let h =
            valid_window(window, pid).ok_or("This VS Code window has closed. Refresh Search.")?;
        let path = executable(pid).ok_or("VS Code could not be identified.")?;
        let name = Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if !matches!(name.as_str(), "code.exe" | "code - insiders.exe")
            || !CODE_COMMANDS.iter().any(|(_, _, k)| *k == keys)
        {
            return Err("This VS Code action is unavailable.".into());
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
        if GetForegroundWindow() != h {
            return Err("Windows could not focus VS Code. Focus it and try again.".into());
        }
        let mut input = vec![];
        for (key, up) in keys
            .iter()
            .map(|k| (*k, false))
            .chain(keys.iter().rev().map(|k| (*k, true)))
        {
            input.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(key),
                        dwFlags: if up {
                            KEYEVENTF_KEYUP
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        },
                        ..Default::default()
                    },
                },
            });
        }
        if SendInput(&input, std::mem::size_of::<INPUT>() as i32) != input.len() as u32 {
            return Err("Windows blocked the VS Code shortcut.".into());
        }
        Ok(())
    }
}
fn launch_target(path: &str, arguments: &str) -> Result<(), String> {
    if path.chars().chain(arguments.chars()).any(char::is_control) {
        return Err("This action contains invalid characters.".into());
    }
    if Path::new(path).is_absolute() && !Path::new(path).exists() {
        return Err(
            "This app is no longer installed. Refresh Search or edit the saved action.".into(),
        );
    }
    let _mta = super::http::MtaGuard::new();
    let path = HSTRING::from(path);
    let args = HSTRING::from(arguments);
    let result = unsafe { ShellExecuteW(None, w!("open"), &path, &args, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        Err("Windows could not open this action.".into())
    } else {
        Ok(())
    }
}
pub fn run_action(target: Target, service_action: Option<bool>, sink: EventSink) {
    let refresh = matches!(target, Target::Service(_));
    let failed = sink.clone();
    if std::thread::Builder::new()
        .name("lumo-search-action".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match target {
                Target::Service(s) => {
                    if let Some(start) = service_action {
                        control_service(&s.name, start)
                    } else {
                        launch_target(
                            &system_file("services.msc")
                                .ok_or("Windows Services was not found.".to_owned())?,
                            "",
                        )
                        .map(|_| "Opened Windows Services.".into())
                    }
                }
                Target::Launch { path, arguments } => {
                    launch_target(&path, &arguments).map(|_| "Opened PowerToys settings.".into())
                }
                Target::PowerToy { event } => {
                    if !TOYS.iter().any(|t| t.3 == event) {
                        return Err("This PowerToys tool is unavailable.".into());
                    }
                    unsafe {
                        let h = Process(
                            OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(event)).map_err(
                                |_| "Start PowerToys and enable this tool, then refresh Search.",
                            )?,
                        );
                        SetEvent(h.0).map_err(|_| "PowerToys could not open this tool.")?;
                    }
                    Ok("Opened PowerToys tool.".into())
                }
                Target::AppCommand { window, pid, keys } => {
                    code_command(window, pid, &keys).map(|_| "Opened VS Code command.".into())
                }
                _ => Err("This search action is unavailable.".into()),
            }))
            .unwrap_or_else(|_| Err("The search action could not finish.".into()));
            sink.send(AppEvent::SearchAction { result, refresh });
        })
        .is_err()
    {
        failed.send(AppEvent::SearchAction {
            result: Err("The search action could not start.".into()),
            refresh: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoting_preserves_windows_arguments() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("C:\\path\\"), "\"C:\\path\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
    #[test]
    #[ignore = "requires the Windows service manager; read-only"]
    fn native_services_are_real_and_drivers_are_excluded() {
        let entries = service_entries(&AtomicBool::new(false)).unwrap();
        assert!(!entries.is_empty());
        assert!(entries
            .iter()
            .any(|e| matches!(&e.target,Target::Service(s)if s.name=="EventLog")));
        assert!(entries.len() <= 1024);
        assert!(entries
            .iter()
            .all(|e| matches!(&e.target,Target::Service(s)if (1..=7).contains(&s.state))));
    }
    #[test]
    fn vscode_catalog_never_creates_launch_targets() {
        let mut out = vec![];
        vscode_entries(&[], &mut out);
        assert!(out.is_empty());
        vscode_entries(
            &[CodeWindow {
                hwnd: 42,
                pid: 7,
                path: "C:/Code/Code.exe".into(),
                title: "Project".into(),
            }],
            &mut out,
        );
        assert_eq!(out.len(), CODE_COMMANDS.len());
        assert!(out.iter().all(|e| e.group == Group::VSCode
            && matches!(
                e.target,
                Target::AppCommand {
                    window: 42,
                    pid: 7,
                    ..
                }
            )));
        assert!(
            code_command(0, 0, &[0x11, 0xBC]).is_err(),
            "a closed app must not launch through a command"
        );
    }
    #[test]
    #[ignore = "reads running VS Code window metadata only"]
    fn vscode_discovery_only_uses_open_code_windows() {
        let windows = code_windows();
        let mut out = vec![];
        vscode_entries(&windows, &mut out);
        assert_eq!(out.len(), windows.len() * CODE_COMMANDS.len());
        assert!(out
            .iter()
            .all(|e| matches!(e.target, Target::AppCommand { .. })));
        eprintln!(
            "Read-only VS Code check: {} open windows, {} commands",
            windows.len(),
            out.len()
        );
    }
}
