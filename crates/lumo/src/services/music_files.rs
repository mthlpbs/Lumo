//! Native file/folder pickers, created only on request on a short-lived STA.

use crate::commands::FolderPick;
use crate::events::{AppEvent, EventSink, SendHwnd};
use std::path::PathBuf;
use windows::core::{w, HSTRING};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH};

pub fn choose(kind: FolderPick, owner: SendHwnd, initial: PathBuf, sink: EventSink) -> std::io::Result<()> {
    std::thread::Builder::new().name("lumo-folder-picker".into()).spawn(move || {
        let result = unsafe {
            match CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() {
                Ok(()) => {
                    let result = pick(&kind, owner, initial).map_err(|e| e.to_string());
                    CoUninitialize();
                    result
                }
                Err(e) => Err(e.to_string()),
            }
        };
        sink.send(AppEvent::FolderChosen { kind, result });
    })?;
    Ok(())
}

unsafe fn pick(kind: &FolderPick, owner: SendHwnd, initial: PathBuf) -> windows::core::Result<Option<PathBuf>> {
    let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
    let options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR | FOS_PICKFOLDERS;
    match kind {
        FolderPick::Directory => {
            dialog.SetTitle(w!("Choose your Music folder"))?;
        }
        FolderPick::LyricsDirectory => dialog.SetTitle(w!("Choose where to save lyrics"))?,
        FolderPick::PhoneInbox => dialog.SetTitle(w!("Choose where to save shared files"))?,
    }
    dialog.SetOptions(options)?;
    if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(initial.to_string_lossy().as_ref()), None) {
        let _ = dialog.SetFolder(&folder);
    }
    match dialog.Show(owner.hwnd()) {
        Err(e) if e.code().0 as u32 == 0x800704c7 => return Ok(None),
        result => result?,
    }
    let item = dialog.GetResult()?;
    let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
    let path = name.to_string();
    CoTaskMemFree(Some(name.0 as *const _));
    Ok(Some(PathBuf::from(path?)))
}
