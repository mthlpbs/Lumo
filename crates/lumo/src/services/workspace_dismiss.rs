//! Temporary outside-click dismissal for wheel-opened workspaces which Windows
//! may decline to activate. No hook while closed/pinned/resting, no worker/poll.
use std::cell::Cell;
use windows::{core::Result,Win32::{Foundation::*,System::LibraryLoader::GetModuleHandleW,UI::WindowsAndMessaging::*}};
pub const OUTSIDE_PRESS:u32=WM_APP+109;
thread_local!{static OWNER:Cell<isize>=const{Cell::new(0)};}
pub struct Watch(HHOOK);
unsafe extern "system" fn mouse(code:i32,wp:WPARAM,lp:LPARAM)->LRESULT {
    if code==HC_ACTION as i32 && matches!(wp.0 as u32,WM_LBUTTONDOWN|WM_RBUTTONDOWN|WM_MBUTTONDOWN|WM_XBUTTONDOWN) {
        OWNER.with(|owner|{let target=owner.get();if target!=0 {
            let click=&*(lp.0 as *const MSLLHOOKSTRUCT);let hit=WindowFromPoint(click.pt);
            let root=HWND(target as _);if hit==root||IsChild(root,hit).as_bool(){return;}
            let mut name=[0u16;32];let len=GetClassNameW(hit,&mut name);
            if String::from_utf16_lossy(&name[..len.max(0)as usize])=="#32768"{
                let mut menu_pid=0;let mut owner_pid=0;
                GetWindowThreadProcessId(hit,Some(&mut menu_pid));GetWindowThreadProcessId(root,Some(&mut owner_pid));
                if menu_pid==owner_pid{return;}
            }
            let _=PostMessageW(HWND(target as _),OUTSIDE_PRESS,WPARAM(0),LPARAM(hit.0 as isize));
        }});
    }
    CallNextHookEx(None,code,wp,lp)
}
impl Watch {pub fn start(owner:HWND)->Result<Self>{unsafe{
    let hook=SetWindowsHookExW(WH_MOUSE_LL,Some(mouse),GetModuleHandleW(None)?,0)?;
    OWNER.with(|v|v.set(owner.0 as isize));Ok(Self(hook))
}}}
impl Drop for Watch {fn drop(&mut self){OWNER.with(|v|v.set(0));unsafe{let _=UnhookWindowsHookEx(self.0);}}}
