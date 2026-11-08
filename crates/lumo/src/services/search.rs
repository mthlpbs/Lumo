//! One bounded Applications-folder enumeration, only on launcher open.
use crate::{
    events::{AppEvent, EventSink},
    search::Event,
};
use lumo_core::search::{Application, Entry, Group, Target};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::HANDLE,
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Shell::{
            BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
            KF_FLAG_DEFAULT, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
        },
    },
};

pub fn start(request: u64, cancel: Arc<AtomicBool>, sink: EventSink) {
    let fallback = sink.clone();
    if std::thread::Builder::new()
        .name("lumo-app-search".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scan(&cancel)))
                .unwrap_or_else(|_| {
                    Err("App discovery stopped. Reopen Search to try again.".into())
                });
            sink.send(AppEvent::Search(Event { request, result }));
        })
        .is_err()
    {
        fallback.send(AppEvent::Search(Event {
            request,
            result: Err("Couldn't start app discovery. Reopen Search to try again.".into()),
        }));
    }
}

pub fn start_icons(
    request: u64,
    items: Vec<(String, Target)>,
    cancel: Arc<AtomicBool>,
    sink: EventSink,
) {
    let fallback = sink.clone();
    let failed: Vec<_> = items.iter().map(|(id, _)| (id.clone(), None)).collect();
    let panic_images = failed.clone();
    if std::thread::Builder::new()
        .name("lumo-search-icons".into())
        .spawn(move || {
            let images = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::imaging::search_icons(items, &cancel)
            }))
            .unwrap_or(panic_images);
            sink.send(AppEvent::SearchIcons(crate::search::IconEvent {
                request,
                images,
            }));
        })
        .is_err()
    {
        fallback.send(AppEvent::SearchIcons(crate::search::IconEvent {
            request,
            images: failed,
        }));
    }
}
unsafe fn take_string(value: PWSTR) -> String {
    let result = value.to_string().unwrap_or_default();
    CoTaskMemFree(Some(value.0.cast()));
    result
}
pub(crate) fn scan(cancel: &AtomicBool) -> Result<Vec<Entry>, String> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| "Windows couldn't open the app catalog.".to_owned())?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let folder: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, HANDLE::default())
                .map_err(|_| "Windows couldn't read installed applications.".to_owned())?;
        let items: IEnumShellItems = folder
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(|_| "Windows couldn't list installed applications.".to_owned())?;
        let mut out = vec![];
        let mut seen = BTreeSet::new();
        let mut cached_bytes = 0usize;
        let deadline = Instant::now() + Duration::from_secs(8);
        for _ in 0..2048 {
            if cancel.load(Ordering::Relaxed) {
                return Err("App discovery cancelled.".into());
            }
            if Instant::now() > deadline {
                return Err("App discovery took too long. Reopen Search to try again.".into());
            }
            let mut next = [None];
            let mut fetched = 0;
            items
                .Next(&mut next, Some(&mut fetched))
                .map_err(|_| "Windows couldn't finish reading applications.".to_owned())?;
            if fetched == 0 {
                break;
            }
            let Some(item) = next[0].take() else {
                break;
            };
            let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY) else {
                continue;
            };
            let title = take_string(name);
            let Ok(path) = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) else {
                continue;
            };
            let path = take_string(path);
            if title.trim().is_empty() || title.len() > 512 || !seen.insert(path.to_lowercase()) {
                continue;
            }
            let Ok(pidl) = windows::Win32::UI::Shell::SHGetIDListFromObject(&item) else {
                continue;
            };
            let length = windows::Win32::UI::Shell::ILGetSize(Some(pidl)) as usize;
            let bytes = if (4..=16384).contains(&length) {
                std::slice::from_raw_parts(pidl.cast::<u8>(), length).to_vec()
            } else {
                vec![]
            };
            CoTaskMemFree(Some(pidl.cast()));
            cached_bytes =
                cached_bytes.saturating_add(bytes.len() + path.len() + title.len() * 4 + 256);
            if cached_bytes > 2 * 1024 * 1024 {
                return Err("The installed-app catalog is too large. Turn off Applications to search Windows settings.".into());
            }
            let Some(application) = Application::from_id_list(bytes) else {
                continue;
            };
            out.push(Entry::new(
                path.clone(),
                title,
                String::new(),
                "",
                Group::App,
                Target::App(application),
            ));
        }
        out.sort_by_cached_key(|e| e.title.to_lowercase());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the Windows Applications folder"]
    fn windows_catalog_returns_launchable_desktop_and_store_entries() {
        let entries = scan(&AtomicBool::new(false)).unwrap();
        assert!(
            !entries.is_empty(),
            "Windows should expose installed applications"
        );
        assert!(entries.len() <= 2048);
        unsafe {
            use windows::Win32::UI::Shell::SHCreateItemFromIDList;
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap();
            for entry in entries.iter().take(12) {
                let Target::App(app) = &entry.target else {
                    unreachable!()
                };
                let item: IShellItem = SHCreateItemFromIDList(app.id_list().as_ptr().cast())
                    .expect("catalog IDs must resolve without executing a command");
                assert_eq!(
                    take_string(item.GetDisplayName(SIGDN_NORMALDISPLAY).unwrap()),
                    entry.title
                );
            }
            CoUninitialize();
        }
    }
}
