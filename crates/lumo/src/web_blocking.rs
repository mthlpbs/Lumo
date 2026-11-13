//! On-demand managed data and one shared Brave engine. No worker or timer at
//! rest. The last web view owns the last strong reference to compiled lists.
use std::{cell::RefCell,path::PathBuf,rc::{Rc,Weak}};
use lumo_core::{adblocking::Engine,addons::WEB_BLOCKING};
use crate::{state::AppState,events::{AppEvent,EventSink}};
thread_local! {static ENGINE:RefCell<Weak<Engine>>=const{RefCell::new(Weak::new())};}
pub fn invalidate(){ENGINE.with(|e|*e.borrow_mut()=Weak::new());}
fn cache_folder()->PathBuf {crate::paths::addons_dir().join(WEB_BLOCKING.id).join(WEB_BLOCKING.tag)}
fn candidates()->Vec<PathBuf>{
    let mut roots=vec![cache_folder()];
    if let Ok(exe)=std::env::current_exe(){if let Some(parent)=exe.parent(){roots.push(parent.join("protection").join(WEB_BLOCKING.tag));}}
    #[cfg(debug_assertions)] roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tools/adblock-data"));
    roots
}
fn verified(path:&std::path::Path)->bool {
    WEB_BLOCKING.files.iter().all(|file|std::fs::metadata(path.join(file.name)).is_ok_and(|m|m.len()==file.size)
        &&std::fs::read(path.join(file.name)).is_ok_and(|b|file.matches(&b)))
}
pub fn load(enabled:bool)->Result<Option<Rc<Engine>>,String>{
    if !enabled{return Ok(None);}
    ENGINE.with(|shared|{
        if let Some(engine)=shared.borrow().upgrade(){return Ok(Some(engine));}
        let root=candidates().into_iter().find(|p|verified(p)).ok_or("Ad blocking is preparing its protection data. Use Browser settings to install it or switch blocking off.")?;
        let read=|name:&str| {
            let file=WEB_BLOCKING.files.iter().find(|f|f.name==name).ok_or("Unknown protection file")?;
            let bytes=std::fs::read(root.join(name)).map_err(|e|e.to_string())?;
            if !file.matches(&bytes){return Err("Protection data failed its integrity check.".into());}
            String::from_utf8(bytes).map_err(|e|e.to_string())
        };
        let resources=read("resources.json")?;
        let engine=Rc::new(Engine::from_lists([read("easylist.txt")?,read("easyprivacy.txt")?],&resources)?);
        *shared.borrow_mut()=Rc::downgrade(&engine);
        Ok(Some(engine))
    })
}
pub fn ensure(st:&mut AppState,sink:EventSink){
    if st.ui.web.downloading||st.power.saver{return;}
    let cancel=std::sync::Arc::new(crate::services::http::Cancel::default());
    let task=cancel.clone();st.ui.web.downloading=true;st.ui.web.cancel=Some(cancel);
    st.ui.web.protection_status="Preparing ad blocking…".into();
    if let Err(e)=std::thread::Builder::new().name("lumo-adblocking-data".into()).spawn(move||{
        let result=crate::services::http::with_cancel(task.clone(),||->Result<(),String>{
            if candidates().iter().any(|p|verified(p)){return Ok(());}
            let _com=crate::services::http::MtaGuard::new();
            let client=crate::services::http::Client::new(concat!("Lumo/",env!("CARGO_PKG_VERSION"))).map_err(|e|e.to_string())?;
            let folder=cache_folder();std::fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
            for file in WEB_BLOCKING.files {
                if task.cancelled(){return Err("Preparation cancelled.".into());}
                let (status,bytes)=client.get_bytes(&WEB_BLOCKING.url(file),std::time::Duration::from_secs(45),file.size,|_|{}).map_err(|e|e.to_string())?;
                if status!=200{return Err(format!("Protection download returned HTTP {status}."));}
                if !file.matches(&bytes){return Err("Protection data failed its integrity check.".into());}
                if task.cancelled(){return Err("Preparation cancelled.".into());}
                lumo_core::store::write_bytes_atomic(&folder.join(file.name),&bytes).map_err(|e|e.to_string())?;
            }
            Ok(())
        });
        sink.send(AppEvent::AdBlockingUpdated(result));
    }){st.ui.web.downloading=false;st.ui.web.cancel=None;st.ui.web.protection_status=e.to_string();}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_is_shared_only_while_views_hold_it(){
        invalidate();
        assert!(load(false).unwrap().is_none());
        // Exercise sharing without requiring the optional protection add-on on CI.
        let first=Rc::new(Engine::from_lists(["||ads.test^".into()],"[]").unwrap());
        ENGINE.with(|shared|*shared.borrow_mut()=Rc::downgrade(&first));
        assert!(load(false).unwrap().is_none());
        let second=load(true).unwrap().unwrap();
        assert!(Rc::ptr_eq(&first,&second));let weak=Rc::downgrade(&first);
        drop(first);drop(second);assert!(weak.upgrade().is_none());
    }
}
