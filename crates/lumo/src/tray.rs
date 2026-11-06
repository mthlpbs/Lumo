//! Notification-area icon, tray menu, and the island's Exit-only context menu.

use crate::events::WM_APP_TRAY;

use lumo_core::settings::{IdleStyle, Theme};
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    pub fn add(hwnd: HWND) -> Result<Tray> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            // Ask for the small-icon size so Windows picks lumo.ico's own 16-24 px
            // frames; LoadIconW gives the 32 px one and shrinks it (blurry).
            let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
            let icon = LoadImageW(hinst, PCWSTR(1 as _), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR)
                .map(|h| HICON(h.0))
                .or_else(|_| LoadIconW(hinst, PCWSTR(1 as _)))
                .or_else(|_| LoadIconW(None, IDI_APPLICATION))?;
            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: WM_APP_TRAY,
                hIcon: icon,
                ..Default::default()
            };
            let tip: Vec<u16> = format!("Lumo {}", crate::version::VERSION).encode_utf16().collect();
            nid.szTip[..tip.len()].copy_from_slice(&tip);
            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                return Err(windows::core::Error::from_win32());
            }
        }
        Ok(Tray { hwnd })
    }

    pub fn remove(self) {
        unsafe {
            let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    ToggleIdleStyle(IdleStyle),
    SetTheme(Theme),
    ToggleClipboard,
    ToggleSystemHistory,
    ToggleLyrics,
    TogglePronunciation,
    /// true: hide in full-screen apps; false: stay on top of them.
    SetHideFullscreen(bool),
    /// true: tuck the resting island away over maximized apps; false: show it.
    SetHideMaximized(bool),
    /// Show over games (frame rate, GPU and CPU on the island over a game).
    ToggleGameStats,
    /// Show over games for a few seconds at a time instead of all along.
    ToggleGamePeek,
    ToggleSounds,
    TogglePinned,
    ToggleAutostart,
    OpenSettings,
    OpenSettingsWindow,
    ReloadSettings,
    OpenDataFolder,
    /// "About Lumo 0.2.3 (…)": opens this version's release notes.
    About,
    /// "Lyrics by LRCLIB": opens lrclib.net.
    LyricsCredit,
    AppleMusicToken,
    AppleMusicHelp,
    AppleMusicForget,
    RecordScreen,
    /// "Update to Lumo 0.2.5…" (`crate::updates`).
    Update,
    Quit,
}

pub enum MenuItem {
    Item { label: String, cmd: MenuCmd, checked: bool },
    Sub { label: String, items: Vec<MenuItem> },
    Separator,
}

fn item(label: &str, cmd: MenuCmd, checked: bool) -> MenuItem {
    MenuItem::Item { label: label.to_string(), cmd, checked }
}

/// The notification-area menu deliberately exposes only these three actions.
pub fn tray_menu_items(autostart: bool) -> Vec<MenuItem> {
    vec![
        item("Start with Windows", MenuCmd::ToggleAutostart, autostart),
        item("Settings", MenuCmd::OpenSettingsWindow, false),
        item("Exit", MenuCmd::Quit, false),
    ]
}

fn build(menu: HMENU, items: &[MenuItem], cmds: &mut Vec<MenuCmd>) {
    for it in items {
        unsafe {
            match it {
                MenuItem::Item { label, cmd, checked } => {
                    cmds.push(*cmd);
                    let id = cmds.len();
                    let w: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    let flags = MF_STRING | if *checked { MF_CHECKED } else { MF_UNCHECKED };
                    let _ = AppendMenuW(menu, flags, id, PCWSTR(w.as_ptr()));
                }
                MenuItem::Sub { label, items } => {
                    if let Ok(sub) = CreatePopupMenu() {
                        build(sub, items, cmds);
                        let w: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(w.as_ptr()));
                    }
                }
                MenuItem::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
            }
        }
    }
}

/// Show the menu at `pt` (screen pixels); returns the chosen command.
pub fn show_menu(owner: HWND, pt: POINT, items: &[MenuItem]) -> Option<MenuCmd> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let mut cmds = Vec::new();
        build(menu, items, &mut cmds);
        // Required so the menu closes when clicking elsewhere; afterwards the
        // previous app gets its focus back.
        let prev = GetForegroundWindow();
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, owner, None);
        let _ = PostMessageW(owner, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let idx = chosen.0 as usize;
        let cmd = if idx == 0 { None } else { cmds.get(idx - 1).copied() };
        let launches = matches!(
            cmd,
            Some(MenuCmd::OpenSettings | MenuCmd::OpenSettingsWindow | MenuCmd::OpenDataFolder | MenuCmd::About | MenuCmd::LyricsCredit | MenuCmd::AppleMusicHelp)
        );
        if !launches && !prev.is_invalid() && prev != owner && IsWindow(prev).as_bool() && GetForegroundWindow() == owner {
            let _ = SetForegroundWindow(prev);
        }
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_has_exactly_startup_settings_and_exit() {
        for enabled in [false, true] {
            let items = tray_menu_items(enabled);
            let rows: Vec<_> = items.iter().map(|item| match item {
                MenuItem::Item { label, cmd, checked } => (label.as_str(), *cmd, *checked),
                _ => panic!("tray menu must not contain extra groups or separators"),
            }).collect();
            assert_eq!(rows, [
                ("Start with Windows", MenuCmd::ToggleAutostart, enabled),
                ("Settings", MenuCmd::OpenSettingsWindow, false),
                ("Exit", MenuCmd::Quit, false),
            ]);
        }
    }

}
