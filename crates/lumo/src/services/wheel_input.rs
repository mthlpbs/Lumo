//! Identify wheel sources only while the pointer is over Lumo. No hooks,
//! raw-input subscriptions, polling or worker. Unknown input keeps legacy UX.
use std::sync::OnceLock;
use windows::{core::{s,w},Win32::{Foundation::BOOL,System::LibraryLoader::{GetModuleHandleW,GetProcAddress},UI::Input::{GetCurrentInputMessageSource,INPUT_MESSAGE_SOURCE,IMDT_MOUSE,IMDT_TOUCHPAD}}};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Source { Mouse, Touchpad, Unknown }
type Register=unsafe extern "system" fn(BOOL)->BOOL;
pub struct Watch { register:Option<Register> }
impl Watch {
    pub fn start()->Self {
        static REGISTER:OnceLock<Option<Register>>=OnceLock::new();
        let function=*REGISTER.get_or_init(||unsafe{
            let module=GetModuleHandleW(w!("user32.dll")).ok()?;
            let address=GetProcAddress(module,s!("RegisterTouchpadCapableThread"))?;
            Some(std::mem::transmute::<unsafe extern "system" fn()->isize,Register>(address))
        });
        let register=function.filter(|f|unsafe{f(BOOL(1)).as_bool()});
        Self{register}
    }
    pub fn source(&self)->Source {
        let mut source=INPUT_MESSAGE_SOURCE::default();
        if unsafe{GetCurrentInputMessageSource(&mut source)}.is_err(){return Source::Unknown;}
        classify(source.deviceType.0,self.register.is_some())
    }
}
fn classify(kind:i32,touchpad_aware:bool)->Source {
    if kind==IMDT_TOUCHPAD.0 {Source::Touchpad}
    else if kind==IMDT_MOUSE.0 && touchpad_aware {Source::Mouse}
    else {Source::Unknown}
}
impl Drop for Watch {fn drop(&mut self){if let Some(f)=self.register{unsafe{let _=f(BOOL(0));}}}}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn unverified_input_never_becomes_a_mouse_shortcut(){
        assert_eq!(classify(IMDT_TOUCHPAD.0,true),Source::Touchpad);
        assert_eq!(classify(IMDT_TOUCHPAD.0,false),Source::Touchpad);
        assert_eq!(classify(IMDT_MOUSE.0,true),Source::Mouse);
        assert_eq!(classify(IMDT_MOUSE.0,false),Source::Unknown);
        assert_eq!(classify(0,true),Source::Unknown);
        assert_eq!(classify(4,true),Source::Unknown);
    }
}
