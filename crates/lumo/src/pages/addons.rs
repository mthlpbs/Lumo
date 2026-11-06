//! Optional features. Nothing runs for Script reader until it is enabled.

use crate::commands::Command;
use crate::gfx::TextStyle;
use crate::state::AppState;
use crate::ui::{wid, Ui};
use lumo_core::addons::{CLIPBOARD_HISTORY, SCRIPT_READER};
use lumo_core::geom::{palette, Color, Rect};

const HEIGHT: f32 = 64.0;
const GAP: f32 = 24.0;
const INSET: f32 = 8.0;

pub fn height(_st: &AppState) -> f32 { HEIGHT }

fn toggle(st: &mut AppState, id: &str) {
    let on = st.settings.addon_on(id);
    st.settings.set_addon(id, !on);
    st.cmd(Command::SaveSettings);
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let width = (area.w - GAP) / 2.0;
    // Two equal columns use the island's width instead of adding another
    // tall row. Each whole feature is a click target, including its blurb.
    for (i, addon) in [SCRIPT_READER, CLIPBOARD_HISTORY].iter().enumerate() {
        let cell = Rect::new(area.x + i as f32 * (width + GAP), area.y, width, HEIGHT);
        let id = if addon.id == SCRIPT_READER.id { wid("addons.script") } else { wid("addons.clipboard") };
        let response = ui.button(id, cell);
        if response.clicked { toggle(st, addon.id); }
        if response.hovered || response.pressed {
            ui.c.fill_rrect(cell, 9.0, Color::white(if response.pressed { 0.10 } else { 0.06 }));
        }
        let title = Rect::new(cell.x + INSET, cell.y + 4.0, cell.w - 2.0 * INSET - 42.0, 24.0);
        let switch = Rect::new(cell.right() - INSET - 30.0, cell.y + 7.0, 30.0, 18.0);
        ui.c.text(addon.title, title, &TextStyle::body());
        super::settings::draw_switch(ui.c, switch, st.settings.addon_on(addon.id), st.accent);
        ui.c.text(addon.blurb, Rect::new(cell.x + INSET, cell.y + 30.0, cell.w - 2.0 * INSET, 30.0), &TextStyle::footnote().color(palette::TEXT_DIM).wrap().top());
    }
    ui.c.fill_rect(Rect::new(area.center().x - 0.5, area.y + 8.0, 1.0, HEIGHT - 16.0), palette::BORDER);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumo_core::{settings::Settings, store::Store};
    #[test]
    fn script_switch_saves_both_on_and_off() {
        let mut st = AppState::new(Settings::default(), Store::default());
        assert!(!st.settings.addon_on("script"));
        toggle(&mut st, "script");
        assert!(st.settings.addon_on("script"));
        assert!(matches!(st.cmds.last(), Some(Command::SaveSettings)));
        toggle(&mut st, "script");
        assert!(!st.settings.addon_on("script"));
    }
}
