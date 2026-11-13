//! Chat-site preferences and the managed ad-blocking switch.
use crate::{
    commands::Command,
    gfx::TextStyle,
    state::AppState,
    ui::{wid, PillStyle, Ui},
};
use lumo_core::{
    geom::{palette, Rect},
};

#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub protection_status: String,
    pub downloading: bool,
    pub cancel: Option<std::sync::Arc<crate::services::http::Cancel>>,
}
fn label(ui: &mut Ui, text: &str, r: Rect) {
    ui.c.text(text, r, &TextStyle::body());
}
fn button(ui: &mut Ui, id: &str, r: Rect, text: &str) -> bool {
    ui.pill(wid(id), r, text, None, PillStyle::Utility).clicked
}
pub fn adblocking(ui:&mut Ui,st:&mut AppState,area:Rect)->f32 {
    let r=Rect::new(area.x,area.y,area.w,52.0);let enabled=st.settings.web.adblocking;
    label(ui,"Block ads and trackers",Rect::new(r.x,r.y,r.w-58.0,26.0));
    ui.c.text("Brave adblock-rust for Browser and AI websites",Rect::new(r.x,r.y+26.0,r.w-58.0,24.0),&TextStyle::footnote().color(palette::TEXT_DIM));
    crate::pages::settings::draw_switch(ui.c,Rect::new(r.right()-44.0,r.y+14.0,36.0,20.0),enabled,st.accent);
    if ui.button(wid("web.adblocking.enabled"),r).clicked {st.settings.web.adblocking=!enabled;st.cmd(Command::SaveSettings);}
    let mut y=r.bottom()+14.0;
    if enabled {
        if button(ui,"web.adblocking.prepare",Rect::new(r.x,y,188.0,32.0),if st.ui.web.downloading{"Cancel preparation"}else{"Install protection data"}) {
            if let Some(c)=&st.ui.web.cancel{c.cancel();}else{st.cmd(Command::PrepareAdBlocking);}
        }
        y+=42.0;
    }
    let status=if st.ui.web.protection_status.is_empty(){"Protection data loads only when blocking is enabled. Reload open websites after changing this setting."}else{&st.ui.web.protection_status};
    ui.c.text(status,Rect::new(r.x,y,r.w,54.0),&TextStyle::footnote().wrap().top().color(palette::TEXT_DIM));
    y+72.0
}
