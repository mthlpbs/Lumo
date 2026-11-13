//! Curated native menus share Windows keyboard/accessibility behavior and
//! the island's dark surface. Shell verbs retain their native UAC semantics.
use lumo_core::search::{Entry,Target};
use crate::native_menu::{Item,Menu};
use std::cell::RefCell;
use windows::{core::{Interface,HSTRING},Win32::{Foundation::*,System::Com::CoTaskMemFree,Storage::EnhancedStorage::{PKEY_Link_TargetParsingPath,PKEY_AppUserModel_ID},UI::{Shell::*,WindowsAndMessaging::*}}};
thread_local!{static HANDLER:RefCell<Option<IContextMenu>>=const{RefCell::new(None)};}
pub enum Action{None,Execute,Invoked(bool),Copy(String),CopyIcon(bool),Forget,Service(bool),Error(String)}
pub enum BrowserAction{Forward,External,Keep,Resume,Filters,Close}
const FORGET:u32=0x7000;const COPY:u32=0x7001;const OPEN:u32=0x7002;const BROWSER:u32=0x7003;const START_SERVICE:u32=0x7004;const STOP_SERVICE:u32=0x7005;const ADMIN:u32=0x7006;const ICON_FILE:u32=0x7007;const ICON_IMAGE:u32=0x7008;
pub fn message(msg:u32,wp:WPARAM,lp:LPARAM)->Option<LRESULT>{
    if let Some(result)=crate::native_menu::message(msg,wp,lp){return Some(result);}
    if !matches!(msg,WM_DRAWITEM|WM_MEASUREITEM|WM_INITMENUPOPUP|WM_MENUCHAR){return None;}
    let handler=HANDLER.with(|c|c.borrow().clone())?;
    unsafe{if let Ok(menu)=handler.cast::<IContextMenu3>(){let mut result=LRESULT(0);menu.HandleMenuMsg2(msg,wp,lp,Some(&mut result)).ok()?;Some(result)}else{None}}
}
unsafe fn take(value:windows::core::PWSTR)->String{let text=value.to_string().unwrap_or_default();CoTaskMemFree(Some(value.0.cast()));text}
pub fn application_location(app:&lumo_core::search::Application)->Option<(String,bool)>{unsafe{
    let item:IShellItem2=SHCreateItemFromIDList(app.id_list().as_ptr().cast()).ok()?;
    if let Ok(value)=item.GetString(&PKEY_Link_TargetParsingPath){let path=take(value);if std::path::Path::new(&path).is_absolute(){return Some((path,true));}}
    if let Ok(value)=item.GetDisplayName(SIGDN_FILESYSPATH){let path=take(value);if std::path::Path::new(&path).is_absolute(){return Some((path,true));}}
    if let Ok(value)=item.GetString(&PKEY_AppUserModel_ID){let id=take(value);if !id.is_empty(){return Some((format!("shell:AppsFolder\\{id}"),false));}}
    item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).ok().map(|p|(take(p),false))
}}
unsafe fn verb_id(menu:HMENU,context:&IContextMenu,verb:&str)->Option<u32>{
    for i in 0..GetMenuItemCount(menu){let id=GetMenuItemID(menu,i);if id==u32::MAX||id==0{continue;}let mut text=[0u8;128];
        if context.GetCommandString((id-1) as usize,GCS_VERBA,None,windows::core::PSTR(text.as_mut_ptr()),text.len() as u32).is_ok(){let n=text.iter().position(|b|*b==0).unwrap_or(text.len());if String::from_utf8_lossy(&text[..n]).eq_ignore_ascii_case(verb){return Some(id-1);}}
    }None
}
fn items(entry:&Entry,frequent:bool,copy:Option<&(String,String)>,admin:bool)->Vec<Item>{
    let label=match entry.target{Target::Service(_)=>"Open Windows Services",Target::AppCommand{..}=>"Run command",Target::PowerToy{..}=>"Open tool",Target::Launch{..}=>"Open settings",_=>"Open"};
    let mut items=vec![Item::new(OPEN,label,'\u{e8a7}').primary()];
    if matches!(entry.target,Target::App(_)){items.push(Item::new(ADMIN,"Run as administrator",'\u{e83d}').disabled(!admin));}
    if let Target::Service(service)=&entry.target{if service.can_start(){items.push(Item::new(START_SERVICE,"Start service",'\u{e768}'));}if service.can_stop(){items.push(Item::new(STOP_SERVICE,"Stop service",'\u{e71a}'));}}
    if let Some((label,_))=copy{items.push(Item::separator());items.push(Item::new(COPY,label,'\u{e8c8}'));}
    if matches!(entry.target,Target::App(_)){items.push(Item::new(ICON_FILE,"Copy icon file",'\u{e8a5}'));items.push(Item::new(ICON_IMAGE,"Copy icon image",'\u{e91b}'));}
    if matches!(entry.target,Target::Web{..}){items.push(Item::new(BROWSER,"Open in default browser",'\u{e8a7}'));}
    if frequent{items.push(Item::separator());items.push(Item::new(FORGET,"Remove from frequent",'\u{e74d}'));}items
}
pub fn show(owner:HWND,entry:&Entry,point:POINT,frequent:bool)->Action{unsafe{
    let hidden=CreatePopupMenu().ok();let context=hidden.and_then(|menu|build_shell(owner,&entry.target,menu).ok());let admin=hidden.zip(context.as_ref()).and_then(|(m,c)|verb_id(m,c,"runas"));
    HANDLER.with(|cell|*cell.borrow_mut()=context.clone());
    let copy=match &entry.target{
        Target::App(app)=>application_location(app).map(|(path,file)|(if file{"Copy path"}else{"Copy app link"}.to_owned(),path)),
        Target::Windows(setting)=>Some(("Copy settings link".into(),setting.uri().into())),
        Target::Web{url,..}=>Some(("Copy link".into(),url.clone())),
        Target::Service(service)=>Some(("Copy service name".into(),service.name.clone())),_=>None,
    };
    let chosen=Menu::new(owner,items(entry,frequent,copy.as_ref(),admin.is_some())).map(|menu|menu.show(owner,point)).unwrap_or(0);
    let result=match chosen{
        OPEN=>Action::Execute,FORGET=>Action::Forget,COPY=>copy.map(|(_,value)|Action::Copy(value)).unwrap_or(Action::None),
        ICON_FILE=>Action::CopyIcon(true),ICON_IMAGE=>Action::CopyIcon(false),START_SERVICE=>Action::Service(true),STOP_SERVICE=>Action::Service(false),
        BROWSER=>{if let Target::Web{url,..}=&entry.target{crate::shell::open(url);}Action::Invoked(false)},
        ADMIN=>if let Some((context,id))=context.as_ref().zip(admin){let info=CMINVOKECOMMANDINFO{cbSize:std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,hwnd:owner,lpVerb:windows::core::PCSTR(id as usize as *const u8),nShow:SW_SHOWNORMAL.0,..Default::default()};match context.InvokeCommand(&info){Ok(())=>Action::Invoked(true),Err(_)=>Action::Error("Windows could not open this app as administrator.".into())}}else{Action::None},
        _=>Action::None,
    };
    HANDLER.with(|cell|*cell.borrow_mut()=None);if let Some(menu)=hidden{let _=DestroyMenu(menu);}result
}}
pub fn show_browser(owner:HWND,point:POINT,keep:bool,resume:bool,forward:bool,detached:bool,home:bool)->Option<BrowserAction>{
    let entries=vec![Item::new(1,"Forward",'\u{e72a}').disabled(!forward),Item::new(2,"Open in default browser",'\u{e8a7}').disabled(home),Item::separator(),Item::new(3,if keep{"Keep open: On"}else{"Keep open: Off"},'\u{e718}'),Item::new(4,if resume{"Resume page: On"}else{"Resume page: Off"},'\u{e777}'),Item::separator(),Item::new(6,"Ad blocking settings...",'\u{e713}'),Item::new(5,if detached{"Close mini browser"}else{"Close browser"},'\u{e711}')];
    match Menu::new(owner,entries).ok()?.show(owner,point){1=>Some(BrowserAction::Forward),2=>Some(BrowserAction::External),3=>Some(BrowserAction::Keep),4=>Some(BrowserAction::Resume),5=>Some(BrowserAction::Close),6=>Some(BrowserAction::Filters),_=>None}
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
    fn context_actions_match_result_and_frequency() {
        let entry=Entry::new("settings:about".into(),"About your PC".into(),String::new(),"",lumo_core::search::Group::Settings,Target::Windows(lumo_core::search::WindowsSetting::About));
        let copy=Some(("Copy settings link".into(),"ms-settings:about".into()));
        let normal=items(&entry,false,copy.as_ref(),false);
        assert!(normal.iter().any(|item|item.id==COPY));
        assert!(!normal.iter().any(|item|matches!(item.id,ADMIN|ICON_FILE|ICON_IMAGE|FORGET)));
        assert!(items(&entry,true,copy.as_ref(),false).iter().any(|item|item.id==FORGET));
    }
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
            let (location,is_file)=application_location(&app).unwrap();
            assert!(is_file && std::path::Path::new(&location).is_absolute());
            let entry=Entry::new("test-app".into(),"Test app".into(),String::new(),"",lumo_core::search::Group::App,Target::App(app.clone()));
            let copy=Some(("Copy path".into(),location));
            let actions=items(&entry,true,copy.as_ref(),true);
            for id in [OPEN,ADMIN,COPY,ICON_FILE,ICON_IMAGE,FORGET]{assert!(actions.iter().any(|item|item.id==id));}
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
