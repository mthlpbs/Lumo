
pub mod expanded;
pub mod hud;
pub mod idle;
pub mod live;
pub mod lyric;
pub mod media;
pub mod motion;
pub mod music;
pub mod notice;
pub mod shape;
pub mod status;

use crate::activities::{Activity, Mode};
use crate::island::Geom;
use crate::state::AppState;
use crate::ui::Ui;
use lumo_core::geom::Rect;
use lumo_core::settings::Theme;

/// The island floats this far below the screen's top edge (DIPs).
pub const TOP_GAP: f32 = 8.0;

pub fn on(theme: Theme) -> bool {
    theme == Theme::Pill
}

/// The island's size for `m` in the Pill theme, or `None` for Classic's.
pub fn geom(m: &Mode, st: &AppState) -> Option<Geom> {
    if !on(st.settings.theme) {
        return None;
    }
    match m {
        Mode::Idle => idle::geom(st),
        Mode::Activity(Activity::Media) => media::geom(st),
        Mode::Activity(a) => live::geom(*a, st),
        Mode::Hud(t) => hud::geom(*t, st),
        Mode::Notice => notice::geom(st),
        Mode::Expanded(p) => expanded::geom(*p, st),
        // Classic's invisible strip, as for the Hidden idle look.
        Mode::Aside => None,
        Mode::Readout(_)=>None,
    }
}

/// Draw the collapsed `m` the Pill way. `false` = draw it as Classic does.
pub fn draw(ui: &mut Ui, st: &mut AppState, m: &Mode, body: Rect) -> bool {
    if !on(st.settings.theme) {
        return false;
    }
    match m {
        Mode::Idle => idle::draw(ui, st, body),
        Mode::Activity(Activity::Media) => media::draw(ui, st, body),
        Mode::Activity(a) => live::draw(ui, st, *a, body),
        Mode::Hud(t) => hud::draw(ui, st, *t, body),
        Mode::Notice => notice::draw(ui, st, body),
        Mode::Expanded(_) | Mode::Aside | Mode::Readout(_) => false,
    }
}

/// Whether the collapsed `m` needs ambient frames in the Pill theme
/// (`None` = as in Classic).
pub fn is_live(m: &Mode, st: &AppState) -> Option<bool> {
    if !on(st.settings.theme) {
        return None;
    }
    match m {
        Mode::Activity(Activity::Media) => media::is_live(st),
        Mode::Activity(a) => live::is_live(*a, st),
        _ => None,
    }
}

/// The next time the Pill content of `m` changes on its own (a one-shot
/// wake, never a poll), on top of Classic's.
pub fn next_wake(m: &Mode, st: &AppState) -> Option<f64> {
    if !on(st.settings.theme) {
        return None;
    }
    match m {
        Mode::Idle => idle::next_wake(st),
        Mode::Activity(Activity::Media) => media::next_wake(st),
        _ => None,
    }
}
