//! Styled Win32 HMENU: native focus, dismissal, keyboard and accessibility.
//! Owner drawing changes appearance only. No idle window, renderer or timer.
use std::cell::Cell;
use windows::{core::{w,PWSTR},Win32::{Foundation::*,Graphics::{Gdi::*,Dwm::*},System::Threading::GetCurrentThreadId,UI::{Accessibility::{MSAAMENUINFO,MSAA_MENU_SIG},Controls::*,HiDpi::GetDpiForWindow,WindowsAndMessaging::*}}};

pub struct Item{pub id:u32,pub label:String,pub glyph:char,pub enabled:bool,pub separator:bool,pub primary:bool}
impl Item{
    pub fn new(id:u32,label:&str,glyph:char)->Self{Self{id,label:label.into(),glyph,enabled:true,separator:false,primary:false}}
    pub fn primary(mut self)->Self{self.primary=true;self}
    pub fn disabled(mut self,disabled:bool)->Self{self.enabled=!disabled;self}
    pub fn separator()->Self{Self{id:0,label:String::new(),glyph:' ',enabled:false,separator:true,primary:false}}
}
#[repr(C)]
struct Row{accessible:MSAAMENUINFO,text:Vec<u16>,glyph:char,enabled:bool,separator:bool,primary:bool}
pub struct Menu{handle:HMENU,rows:Vec<Box<Row>>,font:HFONT,strong_font:HFONT,glyph_font:HFONT,background:HBRUSH,scale:f32,width:f32,styled:Cell<(isize,i32,i32)>}
thread_local!{static ACTIVE:Cell<*const Menu>=const{Cell::new(std::ptr::null())};}
fn rgb(r:u32,g:u32,b:u32)->COLORREF{COLORREF(r|(g<<8)|(b<<16))}
unsafe fn font(face:&str,size:i32)->HFONT{
    let mut f=LOGFONTW{lfHeight:-size,lfWeight:400,lfQuality:ANTIALIASED_QUALITY,..Default::default()};
    for (dst,src) in f.lfFaceName.iter_mut().zip(face.encode_utf16()){*dst=src;}CreateFontIndirectW(&f)
}
impl Menu{
    pub fn new(owner:HWND,items:Vec<Item>)->windows::core::Result<Self>{unsafe{
        let handle=CreatePopupMenu()?;let scale=(GetDpiForWindow(owner) as f32/96.0).max(1.0);
        let mut result=Self{handle,rows:vec![],font:font("Segoe UI Variable Text",(13.0*scale).round() as i32),strong_font:font("Segoe UI Semibold",(13.0*scale).round() as i32),glyph_font:font("Segoe MDL2 Assets",(16.0*scale).round() as i32),background:CreateSolidBrush(rgb(22,22,24)),scale,width:260.0,styled:Cell::new((0,0,0))};
        SetMenuInfo(handle,&MENUINFO{cbSize:std::mem::size_of::<MENUINFO>() as u32,fMask:MIM_BACKGROUND|MIM_STYLE,dwStyle:MNS_NOCHECK,hbrBack:result.background,..Default::default()})?;
        for (i,item) in items.into_iter().enumerate(){
            let mut text:Vec<u16>=item.label.encode_utf16().collect();let len=text.len();text.push(0);
            let mut row=Box::new(Row{accessible:MSAAMENUINFO{dwMSAASignature:MSAA_MENU_SIG as u32,cchWText:len as u32,pszWText:PWSTR(text.as_mut_ptr())},text,glyph:item.glyph,enabled:item.enabled,separator:item.separator,primary:item.primary});
            let info=MENUITEMINFOW{cbSize:std::mem::size_of::<MENUITEMINFOW>() as u32,fMask:MIIM_FTYPE|MIIM_ID|MIIM_DATA|MIIM_STATE|MIIM_STRING,fType:MFT_OWNERDRAW,wID:if item.separator{0x7800+i as u32}else{item.id},fState:if item.enabled{MFS_ENABLED}else{MFS_DISABLED},dwItemData:(&mut *row as *mut Row) as usize,dwTypeData:PWSTR(row.text.as_mut_ptr()),cch:len as u32,..Default::default()};
            InsertMenuItemW(handle,i as u32,true,&info)?;result.width=result.width.max((item.label.chars().count() as f32*6.8+62.0).min(320.0));result.rows.push(row);
        }Ok(result)
    }}
    pub fn show(&self,owner:HWND,at:POINT)->u32{unsafe{
        struct Reset;impl Drop for Reset{fn drop(&mut self){ACTIVE.with(|p|p.set(std::ptr::null()));}}
        ACTIVE.with(|p|p.set(self));let _reset=Reset;
        let hook=SetWindowsHookExW(WH_CALLWNDPROC,Some(menu_hook),None,GetCurrentThreadId()).ok();
        let _=SetForegroundWindow(owner);
        let chosen=TrackPopupMenu(self.handle,TPM_RETURNCMD|TPM_RIGHTBUTTON|TPM_NONOTIFY,at.x,at.y,0,owner,None).0 as u32;
        if let Some(h)=hook{let _=UnhookWindowsHookEx(h);}let _=PostMessageW(owner,WM_NULL,WPARAM(0),LPARAM(0));chosen
    }}
    #[cfg(test)]pub fn handle(&self)->HMENU{self.handle}
    unsafe fn style(&self,hwnd:HWND){
        let mut name=[0u16;32];let n=GetClassNameW(hwnd,&mut name);if String::from_utf16_lossy(&name[..n as usize])!="#32768"{return;}
        let mut r=RECT::default();if GetWindowRect(hwnd,&mut r).is_err(){return;}let key=(hwnd.0 as isize,r.right-r.left,r.bottom-r.top);if self.styled.get()==key{return;}self.styled.set(key);
        let dark=BOOL(1);let corner=DWMWCP_ROUND;let border=rgb(55,55,59).0;
        let _=DwmSetWindowAttribute(hwnd,DWMWA_USE_IMMERSIVE_DARK_MODE,(&dark as *const BOOL).cast(),std::mem::size_of_val(&dark) as u32);
        let _=DwmSetWindowAttribute(hwnd,DWMWA_WINDOW_CORNER_PREFERENCE,(&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),std::mem::size_of_val(&corner) as u32);
        let _=DwmSetWindowAttribute(hwnd,DWMWA_BORDER_COLOR,(&border as *const u32).cast(),4);
        let radius=(24.0*self.scale).round() as i32;let region=CreateRoundRectRgn(0,0,key.1+1,key.2+1,radius,radius);
        if !region.is_invalid() && SetWindowRgn(hwnd,region,true)==0{let _=DeleteObject(region);}
    }
    unsafe fn draw(&self,item:&DRAWITEMSTRUCT){
        let Some(row)=self.rows.iter().find(|r|(&***r as *const Row) as usize==item.itemData)else{return;};
        let dc=item.hDC;let r=item.rcItem;let s=self.scale;let dip=|x:f32|(x*s).round() as i32;
        FillRect(dc,&r,self.background);
        if row.separator{let pen=CreatePen(PS_SOLID,1,rgb(45,45,48));let old=SelectObject(dc,pen);let _=MoveToEx(dc,r.left+dip(14.0),(r.top+r.bottom)/2,None);let _=LineTo(dc,r.right-dip(14.0),(r.top+r.bottom)/2);SelectObject(dc,old);let _=DeleteObject(pen);return;}
        if item.itemState.0&ODS_SELECTED.0!=0 && row.enabled{let brush=CreateSolidBrush(rgb(43,43,47));let old=SelectObject(dc,brush);let pen=SelectObject(dc,GetStockObject(NULL_PEN));let _=RoundRect(dc,r.left+dip(5.0),r.top+dip(2.0),r.right-dip(5.0),r.bottom-dip(2.0),dip(12.0),dip(12.0));SelectObject(dc,pen);SelectObject(dc,old);let _=DeleteObject(brush);}
        SetBkMode(dc,TRANSPARENT);SetTextColor(dc,if row.enabled{rgb(238,238,241)}else{rgb(125,125,133)});
        if row.primary&&row.enabled{SetTextColor(dc,rgb(148,194,255));}
        let old=SelectObject(dc,self.glyph_font);let mut glyph=vec![row.glyph as u16];let mut icon=RECT{left:r.left+dip(13.0),right:r.left+dip(33.0),top:r.top,bottom:r.bottom};DrawTextW(dc,&mut glyph,&mut icon,DT_CENTER|DT_VCENTER|DT_SINGLELINE|DT_NOPREFIX);
        SetTextColor(dc,if row.enabled{rgb(238,238,241)}else{rgb(125,125,133)});SelectObject(dc,if row.primary{self.strong_font}else{self.font});let mut text=row.text[..row.text.len()-1].to_vec();let mut text_rect=RECT{left:r.left+dip(44.0),right:r.right-dip(16.0),..r};DrawTextW(dc,&mut text,&mut text_rect,DT_LEFT|DT_VCENTER|DT_SINGLELINE|DT_END_ELLIPSIS|DT_NOPREFIX);SelectObject(dc,old);
    }
}
impl Drop for Menu{fn drop(&mut self){unsafe{let _=DestroyMenu(self.handle);let _=DeleteObject(self.font);let _=DeleteObject(self.strong_font);let _=DeleteObject(self.glyph_font);let _=DeleteObject(self.background);}}}
unsafe extern "system" fn menu_hook(code:i32,wp:WPARAM,lp:LPARAM)->LRESULT{
    if code>=0{let message=&*(lp.0 as *const CWPSTRUCT);if message.message==WM_WINDOWPOSCHANGED{ACTIVE.with(|p|{if let Some(menu)=p.get().as_ref(){menu.style(message.hwnd);}});}}
    CallNextHookEx(None,code,wp,lp)
}
pub fn message(msg:u32,wp:WPARAM,lp:LPARAM)->Option<LRESULT>{ACTIVE.with(|p|unsafe{
    let menu=p.get().as_ref()?;
    match msg{
        WM_MEASUREITEM if wp.0==0=>{let m=&mut *(lp.0 as *mut MEASUREITEMSTRUCT);let row=menu.rows.iter().find(|r|(&***r as *const Row) as usize==m.itemData)?;m.itemWidth=(menu.width*menu.scale).round() as u32;m.itemHeight=((if row.separator{8.0}else{34.0})*menu.scale).round() as u32;Some(LRESULT(1))},
        WM_DRAWITEM if wp.0==0=>{menu.draw(&*(lp.0 as *const DRAWITEMSTRUCT));Some(LRESULT(1))},
        WM_MENUCHAR=>{let key=char::from_u32((wp.0&0xffff) as u32)?.to_lowercase().next()?;let mut found=None;for (i,row) in menu.rows.iter().enumerate(){if row.enabled&&String::from_utf16_lossy(&row.text).chars().next().is_some_and(|c|c.to_lowercase().next()==Some(key)){found=Some(i);break;}}found.map(|i|LRESULT(((MNC_EXECUTE<<16)|i as u32) as isize))},
        _=>None,
    }
})}

#[cfg(test)]mod tests{use super::*;
    #[test]#[ignore="requires a native desktop; no menu is shown"]fn menu_rows_expose_accessible_names_and_disabled_state(){
        let menu=Menu::new(HWND::default(),vec![Item::new(1,"Open",'\u{e8a7}').primary(),Item::separator(),Item::new(2,"Run as administrator",'\u{e83d}').disabled(true)]).unwrap();
        assert_eq!(unsafe{GetMenuItemCount(menu.handle())},3);assert_eq!(menu.rows[0].accessible.cchWText,4);assert!(!menu.rows[2].enabled);
    }
    #[test]#[ignore="shows and captures an isolated native menu; no command is invoked"]
    fn native_menu_renders_and_dismisses_without_invoking(){unsafe{
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        const DONE:u32=WM_APP+88;
        unsafe extern "system" fn proc(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->LRESULT{
            if let Some(result)=message(msg,wp,lp){return result;}if msg==DONE{let _=EndMenu();return LRESULT(0);}DefWindowProcW(hwnd,msg,wp,lp)
        }
        let instance=GetModuleHandleW(None).unwrap();let class=w!("Lumo.NativeMenuFixture");let _=RegisterClassW(&WNDCLASSW{lpfnWndProc:Some(proc),hInstance:instance.into(),lpszClassName:class,hbrBackground:HBRUSH(GetStockObject(BLACK_BRUSH).0),..Default::default()});
        let old=GetForegroundWindow();let owner=CreateWindowExW(WS_EX_TOOLWINDOW,class,w!("Lumo menu preview"),WS_POPUP,140,160,420,420,None,None,instance,None).unwrap();let _=ShowWindow(owner,SW_SHOWNOACTIVATE);
        let owner_bits=owner.0 as usize;let pid=windows::Win32::System::Threading::GetCurrentProcessId();
        let capture=std::thread::spawn(move||{
            struct End(usize);impl Drop for End{fn drop(&mut self){unsafe{let _=PostMessageW(HWND(self.0 as *mut _),DONE,WPARAM(0),LPARAM(0));}}}let _end=End(owner_bits);
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
            loop{let mut after:Option<HWND>=None;let mut found=None;loop{let Ok(hwnd)=FindWindowExW(None,after.as_ref(),w!("#32768"),None)else{break;};let mut process=0;GetWindowThreadProcessId(hwnd,Some(&mut process));if process==pid && IsWindowVisible(hwnd).as_bool(){found=Some(hwnd);break;}after=Some(hwnd);}
                if let Some(hwnd)=found{
                    std::thread::sleep(std::time::Duration::from_millis(350));let mut r=RECT::default();GetWindowRect(hwnd,&mut r).unwrap();
                    if let Some(root)=std::env::var_os("LUMO_SEARCH_QA"){
                        let width=r.right-r.left;let height=r.bottom-r.top;let dc=GetDC(None);let mem=CreateCompatibleDC(dc);let mut bits=std::ptr::null_mut();let info=BITMAPINFO{bmiHeader:BITMAPINFOHEADER{biSize:std::mem::size_of::<BITMAPINFOHEADER>() as u32,biWidth:width,biHeight:-height,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()},..Default::default()};
                        let bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&mut bits,None,0).unwrap();let old=SelectObject(mem,bitmap);BitBlt(mem,0,0,width,height,dc,r.left,r.top,SRCCOPY|CAPTUREBLT).unwrap();let _=GdiFlush();let mut pixels=std::slice::from_raw_parts(bits.cast::<u8>(),(width*height*4) as usize).to_vec();for p in pixels.chunks_exact_mut(4){p[3]=255;}
                        let root=std::path::PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();crate::png::save_rgba(root.join("native-context-menu.png"),width as u32,height as u32,&crate::png::pbgra_to_rgba(&pixels)).unwrap();SelectObject(mem,old);let _=DeleteObject(bitmap);let _=DeleteDC(mem);ReleaseDC(None,dc);
                    }return;
                }
                assert!(std::time::Instant::now()<deadline,"native menu did not become visible");std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        let menu=Menu::new(owner,vec![Item::new(1,"Open",'\u{e8a7}').primary(),Item::new(2,"Run as administrator",'\u{e83d}'),Item::separator(),Item::new(3,"Copy path",'\u{e8c8}'),Item::new(4,"Copy icon file",'\u{e8a5}'),Item::new(5,"Copy icon image",'\u{e91b}'),Item::separator(),Item::new(6,"Remove from frequent",'\u{e74d}')]).unwrap();
        assert_eq!(menu.show(owner,POINT{x:160,y:180}),0);capture.join().unwrap();DestroyWindow(owner).unwrap();if !old.is_invalid(){let _=SetForegroundWindow(old);}
    }}
}
