//! Internal blank document: no renderer, content, network or background work.
use crate::state::AppState;
pub const URL: &str = "lumo://home";
pub fn is_home(st: &AppState) -> bool {
    st.search.web_url.as_deref() == Some(URL)
}

/// A quiet starting hint, without a homepage or another search field.
pub fn draw(ui: &mut crate::ui::Ui, page: lumo_core::geom::Rect, editing: bool) {
    if editing { return; }
    use crate::{gfx::TextStyle, icons::Icon};
    use lumo_core::geom::{palette, Color, Rect};
    let center = page.center();
    let top = center.y - 50.0;
    ui.c.icon(Icon::Globe, Rect::new(center.x - 15.0, top, 30.0, 30.0), Color::white(0.30));
    ui.c.text("Start browsing", Rect::new(page.x + 16.0, top + 44.0, page.w - 32.0, 24.0),
        &TextStyle::body().center().color(palette::TEXT_DIM));
    ui.c.text("Search or enter a web address above", Rect::new(page.x + 16.0, top + 72.0, page.w - 32.0, 24.0),
        &TextStyle::footnote().center().color(palette::TEXT_FAINT));
}
