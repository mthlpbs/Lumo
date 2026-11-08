//! Windows Shell context menus, invoked outside the borrowed App so their
//! modal message loop can keep the island responsive. Extended verbs include
//! administrator/user actions wherever Windows exposes them for the item.
use lumo_core::search::{Entry, Target};
use std::cell::RefCell;
use windows::{
    core::{Interface, HSTRING, PCWSTR},
    Win32::{
        Foundation::*,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};
thread_local! {static HANDLER:RefCell<Option<IContextMenu>>=const{RefCell::new(None)};}
pub enum Action {
    None,
    Execute,
    Invoked(bool),
    Copy(String),
    Forget,
}
const FORGET: u32 = 0x7000;
const COPY: u32 = 0x7001;
const OPEN: u32 = 0x7002;
const BROWSER: u32 = 0x7003;
pub fn message(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if !matches!(
        msg,
        WM_DRAWITEM | WM_MEASUREITEM | WM_INITMENUPOPUP | WM_MENUCHAR
    ) {
        return None;
    }
    let handler = HANDLER.with(|cell| cell.borrow().clone())?;
    unsafe {
        if let Ok(menu) = handler.cast::<IContextMenu3>() {
            let mut result = LRESULT(0);
            menu.HandleMenuMsg2(msg, wp, lp, Some(&mut result)).ok()?;
            Some(result)
        } else if msg != WM_MENUCHAR {
            let menu = handler.cast::<IContextMenu2>().ok()?;
            menu.HandleMenuMsg(msg, wp, lp).ok()?;
            Some(LRESULT(0))
        } else {
            None
        }
    }
}
unsafe fn append(menu: HMENU, id: u32, label: &str) {
    let text = HSTRING::from(label);
    let _ = AppendMenuW(menu, MF_STRING, id as usize, &text);
}
pub fn show(owner: HWND, entry: &Entry, point: POINT, frequent: bool) -> Action {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return Action::None;
        };
        struct OwnedMenu(HMENU);
        impl Drop for OwnedMenu {
            fn drop(&mut self) {
                HANDLER.with(|c| c.borrow_mut().take());
                unsafe {
                    let _ = DestroyMenu(self.0);
                }
            }
        }
        let _owned = OwnedMenu(menu);
        let native = build_shell(owner, &entry.target, menu).ok();
        if native.is_none() {
            append(menu, OPEN, "Open");
        }
        if let Some(handler) = native.as_ref() {
            HANDLER.with(|c| *c.borrow_mut() = Some(handler.clone()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let copy = match &entry.target {
            Target::Windows(setting) => Some(("Copy settings link", setting.uri().to_string())),
            Target::Web { url, .. } => Some(("Copy search link", url.clone())),
            _ => None,
        };
        if let Some((label, _)) = &copy {
            append(menu, COPY, label);
        }
        if matches!(entry.target, Target::Web { .. }) {
            append(menu, BROWSER, "Open in default browser");
        }
        if frequent {
            append(menu, FORGET, "Remove from frequent results");
        }
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            owner,
            None,
        )
        .0 as u32;
        let _ = PostMessageW(owner, WM_NULL, WPARAM(0), LPARAM(0));
        match chosen {
            0 => Action::None,
            OPEN => Action::Execute,
            FORGET => Action::Forget,
            COPY => copy
                .map(|(_, value)| Action::Copy(value))
                .unwrap_or(Action::None),
            BROWSER => {
                if let Target::Web { url, .. } = &entry.target {
                    crate::shell::open(url);
                }
                Action::Invoked(false)
            }
            id if id < FORGET => {
                let Some(native) = native else {
                    return Action::None;
                };
                let mut verb = [0u8; 128];
                let _ = native.GetCommandString(
                    (id - 1) as usize,
                    GCS_VERBA,
                    None,
                    windows::core::PSTR(verb.as_mut_ptr()),
                    verb.len() as u32,
                );
                let verb = String::from_utf8_lossy(
                    &verb[..verb.iter().position(|b| *b == 0).unwrap_or(verb.len())],
                );
                let info = CMINVOKECOMMANDINFO {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                    hwnd: owner,
                    lpVerb: windows::core::PCSTR((id - 1) as usize as *const u8),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                };
                if native.InvokeCommand(&info).is_ok() {
                    Action::Invoked(matches!(verb.as_ref(), "open" | "runas" | "runasuser"))
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        }
    }
}
unsafe fn build_shell(
    owner: HWND,
    target: &Target,
    menu: HMENU,
) -> windows::core::Result<IContextMenu> {
    let pidl = match target {
        Target::App(app) => app.id_list().as_ptr().cast(),
        _ => return Err(E_INVALIDARG.into()),
    };
    let mut child = std::ptr::null_mut();
    let parent: IShellFolder = SHBindToParent(pidl, Some(&mut child))?;
    let context: IContextMenu = parent.GetUIObjectOf(owner, &[child as *const _], None)?;
    context.QueryContextMenu(menu, 0, 1, FORGET - 1, CMF_NORMAL | CMF_EXTENDEDVERBS)?;
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires Windows Shell"]
    fn native_application_menu_has_windows_verbs() {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
            .unwrap();
            let menu = CreatePopupMenu().unwrap();
            let path = std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None).unwrap();
            let pidl = SHGetIDListFromObject(&item).unwrap();
            let len = ILGetSize(Some(pidl)) as usize;
            let bytes = std::slice::from_raw_parts(pidl.cast::<u8>(), len).to_vec();
            windows::Win32::System::Com::CoTaskMemFree(Some(pidl.cast()));
            let app = lumo_core::search::Application::from_id_list(bytes).unwrap();
            let _context = build_shell(HWND::default(), &Target::App(app), menu).unwrap();
            assert!(GetMenuItemCount(menu) > 2);
            let labels: Vec<_> = (0..GetMenuItemCount(menu))
                .map(|position| {
                    let mut text = [0u16; 256];
                    GetMenuStringW(menu, position as u32, Some(&mut text), MF_BYPOSITION);
                    String::from_utf16_lossy(
                        &text[..text.iter().position(|&v| v == 0).unwrap_or(text.len())],
                    )
                    .replace('&', "")
                })
                .collect();
            assert!(
                labels.iter().any(|s| s == "Run as administrator"),
                "Windows must supply the administrator command: {labels:?}"
            );
            assert!(labels.iter().any(|s| s == "Properties"));
            DestroyMenu(menu).unwrap();
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}
