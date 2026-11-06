//! Observe only Ctrl transitions while the cursor is over the closed island.
//! The hook is removed on leave/expand/lock; no idle hook or key-state polling.
use std::cell::Cell;
use windows::{
    core::Result,
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};
pub const MODIFIER_CHANGED: u32 = WM_APP + 38;
thread_local! {static TARGET:Cell<isize>=const{Cell::new(0)};}
pub fn control_down() -> bool {
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 }
}
pub struct Watch(HHOOK);
unsafe extern "system" fn keyboard(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        if [
            VK_CONTROL.0 as u32,
            VK_LCONTROL.0 as u32,
            VK_RCONTROL.0 as u32,
        ]
        .contains(&key.vkCode)
        {
            TARGET.with(|t| {
                let target = t.get();
                if target != 0 {
                    let _ = PostMessageW(HWND(target as _), MODIFIER_CHANGED, WPARAM(0), LPARAM(0));
                }
            });
        }
    }
    CallNextHookEx(None, code, wp, lp)
}
impl Watch {
    pub fn start(hwnd: HWND) -> Result<Self> {
        unsafe {
            let hook =
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), GetModuleHandleW(None)?, 0)?;
            TARGET.with(|t| t.set(hwnd.0 as isize));
            let _ = PostMessageW(hwnd, MODIFIER_CHANGED, WPARAM(0), LPARAM(0));
            Ok(Self(hook))
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        TARGET.with(|t| t.set(0));
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}
