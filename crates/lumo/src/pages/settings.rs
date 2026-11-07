//! Settings page, behind the gear on the right of the header.
//!
//! Theme and idle look pickers, switches, and full-screen behavior.
//! A change saves at once and applies the way the tray's does
//! (`Command::SaveSettings`). Update controls live only in the standalone
//! Settings window's About section.

use crate::commands::Command;
use crate::gfx::{TextStyle, Weight};
use crate::pill::status;
use crate::state::AppState;
use crate::ui::{wid, wid_n, PillStyle, Ui, PILL_SMALL};
use lumo_core::geom::{palette, Color, Point, Rect};
use lumo_core::settings::{IdleStyle, Theme};

/// Pickers, switches and choices, followed by the update row when available.
const MUSIC_FOLDER_Y: f32 = SWITCHES_Y + ROWS as f32 * ROW_H;
const MUSIC_FOLDER_H: f32 = 72.0;
const LABELS_Y: f32 = 0.0;
const PICKERS_Y: f32 = 18.0;
const SWITCHES_Y: f32 = 52.0;
const ROW_H: f32 = 26.0;
const COL_GAP: f32 = 24.0;
/// A row's hover wash reaches this far past its text and switch.
const ROW_BLEED: f32 = 8.0;
/// Pickers: the selection's inset in the track, and each choice's padding.
/// Side by side, the theme and idle look pickers may close their choices up
/// to SEG_PAD_MIN to fit their columns (the Pill theme's five idle looks at
/// the default width).
const SEG_INSET: f32 = 2.0;
const SEG_PAD: f32 = 11.0;
const SEG_PAD_MIN: f32 = 7.5;
/// Where they still don't fit (a narrow island), each takes a row, its
/// label on the left, in the room the labels-above layout uses, so the page
/// keeps its height.
const STACK_Y: [f32; 2] = [0.0, 28.0];
const STACK_GAP: f32 = 16.0;
/// The full-screen choice's picker, inside its row.
const ROW_PICKER_H: f32 = 24.0;
const SWITCH_W: f32 = 30.0;
const SWITCH_H: f32 = 18.0;
const KNOB_INSET: f32 = 2.0;

const THEMES: [(Theme, &str); 2] = [(Theme::Classic, "Classic"), (Theme::Pill, "Pill")];

fn idle_name(s: IdleStyle, eyes: &'static str) -> &'static str {
    match s {
        IdleStyle::Clock => "Clock",
        IdleStyle::Eyes => eyes,
        IdleStyle::Minimal => "Minimal",
        IdleStyle::Hidden => "Hidden",
        IdleStyle::Status => "Status",
    }
}

/// The switches, as the tray menu names them (new versions: Settings only).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Switch {
    Lyrics,
    Pronunciation,
    Sounds,
    Autostart,
}

/// What a row of the two columns holds: a switch, or the full-screen choice
/// (the tray menu's "In full-screen apps" submenu).
#[derive(Clone, Copy)]
enum Row {
    Switch(Switch, &'static str),
    Fullscreen,
    Maximized,
}

const LEFT: &[Row] = &[
    Row::Switch(Switch::Lyrics, "Lyrics on closed island"),
    Row::Switch(Switch::Pronunciation, "Lyrics pronunciation"),
    Row::Switch(Switch::Sounds, "Sounds"),
];
const RIGHT: &[Row] = &[
    Row::Fullscreen,
    Row::Switch(Switch::Autostart, "Start with Windows"),
    Row::Maximized,
];
const COLUMNS: [&[Row]; 2] = [LEFT, RIGHT];
/// Rows in the taller column.
const ROWS: usize = if LEFT.len() > RIGHT.len() { LEFT.len() } else { RIGHT.len() };

/// The full-screen choices as the tray menu names them, and the
/// `hide_in_fullscreen` each one sets.
const FULLSCREEN: [(bool, &str); 2] = [(true, "Hide"), (false, "Stay on top")];

pub fn height(st: &AppState) -> f32 {
    let body = MUSIC_FOLDER_Y + music_folder_height(st);
    body
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let col_w = ((area.w - COL_GAP) / 2.0).max(0.0);
    let cols = [area.x, area.x + col_w + COL_GAP];
    let label_st = TextStyle::caption().weight(Weight::Semibold).color(palette::TEXT_FAINT);

    // Pickers.
    let theme_names: Vec<&str> = THEMES.iter().map(|t| t.1).collect();
    let theme_sel = THEMES.iter().position(|t| t.0 == st.settings.theme).unwrap_or(0);
    let looks = status::looks(st.settings.theme);
    let eyes = "Eyes";
    let look_names: Vec<&str> = looks.iter().map(|s| idle_name(*s, eyes)).collect();
    let look_sel = looks.iter().position(|s| *s == status::shown(st.settings.theme, st.settings.idle_style)).unwrap_or(0);
    // Side by side under their labels when both fit their columns, each
    // closed up only as far as it needs; else a row each. Whether they fit is
    // side exactly where they always did (at a narrow island picking Pill,
    // with its fifth look, can stack them).
    let theme_pad = pad_to_fit(ui, &theme_names, col_w);
    let side_by_side = theme_pad >= SEG_PAD_MIN && pad_to_fit(ui, &look_names, col_w) >= SEG_PAD_MIN;
    let (theme_pick, look_pick) = if side_by_side {
        ui.c.text("THEME", Rect::new(cols[0], area.y + LABELS_Y, col_w, 14.0), &label_st);
        ui.c.text("IDLE LOOK", Rect::new(cols[1], area.y + LABELS_Y, col_w, 14.0), &label_st);
        let y = area.y + PICKERS_Y;
        let look_pad = pad_to_fit(ui, &look_names, col_w);
        (
            picker_padded(ui, "settings.theme", cols[0], y, PILL_SMALL, &theme_names, theme_sel, theme_pad),
            picker_padded(ui, "settings.idle", cols[1], y, PILL_SMALL, &look_names, look_sel, look_pad),
        )
    } else {
        // A row each, the labels in a column of their own and both pickers
        // starting after it.
        let labels = ["THEME", "IDLE LOOK"];
        let label_w = labels.iter().map(|l| ui.c.measure(l, &label_st, 300.0).0.ceil()).fold(0.0f32, f32::max);
        let x = area.x + label_w + STACK_GAP;
        let pick = |ui: &mut Ui, i: usize, id: &str, names: &[&str], sel: usize| {
            let row = Rect::new(area.x, area.y + STACK_Y[i], area.w, ROW_H);
            ui.c.text(labels[i], Rect::new(row.x, row.y, label_w + 2.0, row.h), &label_st);
            let pad = pad_to_fit(ui, names, row.right() - x).max(0.0);
            picker_padded(ui, id, x, row.center().y - ROW_PICKER_H / 2.0, ROW_PICKER_H, names, sel, pad)
        };
        (pick(ui, 0, "settings.theme", &theme_names, theme_sel), pick(ui, 1, "settings.idle", &look_names, look_sel))
    };
    if let Some(i) = theme_pick {
        st.settings.theme = THEMES[i].0;
        st.cmd(Command::SaveSettings);
    }
    if let Some(i) = look_pick {
        st.settings.idle_style = looks[i];
        st.cmd(Command::SaveSettings);
    }

    // Switches and the two choices, in two columns.
    let accent = st.accent;
    let card = super::clipboard::pill_card(st, area);
    for (c, column) in COLUMNS.iter().enumerate() {
        for (r, &kind) in column.iter().enumerate() {
            let row = Rect::new(cols[c], area.y + SWITCHES_Y + r as f32 * ROW_H, col_w, ROW_H);
            match kind {
                Row::Switch(sw, label) => {
                    let on = switch_on(st, sw);
                    let id = wid_n("settings.switch", (c * ROWS + r) as u64);
                    if switch_row_on(ui, id, row, label, on, accent, card) {
                        flip(st, sw, on);
                    }
                }
                Row::Fullscreen => {
                    let sel = FULLSCREEN.iter().position(|f| f.0 == st.settings.hide_in_fullscreen).unwrap_or(0);
                    if let Some(i) = choice_row(ui, row, "In full-screen apps", "settings.fullscreen", &FULLSCREEN.map(|f| f.1), sel) {
                        st.settings.hide_in_fullscreen = FULLSCREEN[i].0;
                        st.cmd(Command::SaveSettings);
                    }
                }
                Row::Maximized => {
                    let sel = FULLSCREEN.iter().position(|f| f.0 == st.settings.hide_over_maximized).unwrap_or(0);
                    if let Some(i) = choice_row(ui, row, "In maximized apps", "settings.maximized", &FULLSCREEN.map(|f| f.1), sel) {
                        st.settings.hide_over_maximized = FULLSCREEN[i].0;
                        st.cmd(Command::SaveSettings);
                    }
                }

            }
        }
    }
    draw_music_folder(ui, st, Rect::new(area.x, area.y + MUSIC_FOLDER_Y, area.w, music_folder_height(st)), card);
}

fn music_folder_height(st: &AppState) -> f32 {
    if st.settings.local_lyrics { MUSIC_FOLDER_H } else { 44.0 }
}

/// Local lyrics are opt-in. Choosing a folder saves it without copying or
/// modifying anything in the person's music library.
fn draw_music_folder(ui: &mut Ui, st: &mut AppState, r: Rect, card: Option<(Rect, f32)>) {
    let on = st.settings.local_lyrics;
    let col_w = ((r.w - COL_GAP) / 2.0).max(0.0);
    let mut x = r.x;
    let buttons_y = r.y + ROW_H;
    if on {
        let choosing = st.ui.choosing_music_dir;
        let label = if choosing { "Choosing…" } else { "Change folder" };
        let w = ui.pill_width(label, false, true);
        let b = Rect::new(x, buttons_y, w, ROW_PICKER_H);
        if ui.pill(wid("settings.music-folder"), b, label, None, PillStyle::Utility).clicked && !choosing {
            st.cmd(Command::ChooseMusicDirectory);
        }
        x = b.right() + 8.0;
        if !st.settings.music_directory.is_empty() {
            let w = ui.pill_width("Reset", false, true);
            let b = Rect::new(x, buttons_y, w, ROW_PICKER_H);
            if ui.pill(wid("settings.music-folder.reset"), b, "Reset", None, PillStyle::Utility).clicked {
                st.settings.music_directory.clear();
                st.cmd(Command::SaveSettings);
            }
        }
    }
    let row = Rect::new(r.x, r.y, col_w, ROW_H);
    if switch_row_on(ui, wid("settings.local-lyrics"), row, "Local lyrics for music players", Some(on), st.accent, card) {
        st.settings.local_lyrics = !on;
        st.cmd(Command::SaveSettings);
    }
    let line = if on {
        crate::paths::music_dir(&st.settings).to_string_lossy().into_owned()
    } else {
        "Sidecar and embedded lyrics first; LRCLIB for missing lyrics and streaming.".to_string()
    };
    let path_y = if on { buttons_y + ROW_PICKER_H + 4.0 } else { buttons_y };
    ui.c.text(&line, Rect::new(r.x, path_y, r.w, 17.0), &TextStyle::footnote().color(palette::TEXT_DIM));
}

/// A choice's label with its picker where the other rows have their
/// switch. Returns the choice clicked, if any.
fn choice_row(ui: &mut Ui, r: Rect, label: &str, id: &str, names: &[&str], sel: usize) -> Option<usize> {
    let label_w = ui.c.measure(label, &TextStyle::body(), r.w).0;
    let pad = pad_to_fit(ui, names, (r.w - label_w - 10.0).max(0.0)).max(2.0);
    let w = 2.0 * SEG_INSET + names.iter().map(|n| choice_width(ui, n, pad)).sum::<f32>();
    let x = r.right() - w;
    ui.c.text(label, Rect::new(r.x, r.y, (x - 10.0 - r.x).max(0.0), r.h), &TextStyle::body());
    picker_padded(ui, id, x, r.center().y - ROW_PICKER_H / 2.0, ROW_PICKER_H, names, sel, pad)
}

/// Where a switch reads from. Start with Windows is read off the UI thread
/// when the page opens; until then it shows off, dimmed.
fn switch_on(st: &AppState, sw: Switch) -> Option<bool> {
    let s = &st.settings;
    match sw {
        Switch::Lyrics => Some(s.island_lyrics),
        Switch::Pronunciation => Some(s.lyrics_pronunciation),
        Switch::Sounds => Some(s.sounds),
        Switch::Autostart => st.ui.autostart,
    }
}

fn flip(st: &mut AppState, sw: Switch, on: Option<bool>) {
    let s = &mut st.settings;
    match sw {
        Switch::Lyrics => s.island_lyrics = !s.island_lyrics,
        Switch::Pronunciation => s.lyrics_pronunciation = !s.lyrics_pronunciation,
        Switch::Sounds => s.sounds = !s.sounds,
        Switch::Autostart => {
            if let Some(on) = on {
                st.cmd(Command::SetAutostart(!on));
            }
            return;
        }
    }
    st.cmd(Command::SaveSettings);
}

// ---------------------------------------------------------------- widgets

/// The widest padding (up to SEG_PAD) that fits a picker of `names` into `w`.
fn pad_to_fit(ui: &mut Ui, names: &[&str], w: f32) -> f32 {
    let n = names.len().max(1) as f32;
    let text: f32 = names.iter().map(|s| ui.c.measure(s, &picker_text(), 300.0).0).sum();
    // Each choice's width is rounded up: up to a DIP each.
    ((w - 2.0 * SEG_INSET - text - n) / (2.0 * n)).min(SEG_PAD)
}

/// Choices side by side in a track `h` tall, the chosen one lifted (the
/// Focus page's picker, a size smaller). Returns the choice clicked, if any.
pub(crate) fn picker(ui: &mut Ui, id: &str, x: f32, y: f32, h: f32, names: &[&str], sel: usize) -> Option<usize> {
    picker_padded(ui, id, x, y, h, names, sel, SEG_PAD)
}

/// [`picker`] with `pad` around each choice's name.
#[allow(clippy::too_many_arguments)]
fn picker_padded(ui: &mut Ui, id: &str, x: f32, y: f32, h: f32, names: &[&str], sel: usize, pad: f32) -> Option<usize> {
    let st = picker_text();
    let inner_h = h - 2.0 * SEG_INSET;
    let mut cx = x + SEG_INSET;
    let cells: Vec<Rect> = names
        .iter()
        .map(|n| {
            let w = choice_width(ui, n, pad);
            let cell = Rect::new(cx, y + SEG_INSET, w, inner_h);
            cx += w;
            cell
        })
        .collect();
    ui.c.fill_rrect(Rect::new(x, y, cx + SEG_INSET - x, h), h / 2.0, palette::SURFACE);
    let mut picked = None;
    for (i, (name, cell)) in names.iter().zip(&cells).enumerate() {
        let color = if i == sel {
            ui.c.fill_rrect(*cell, inner_h / 2.0, Color::white(0.16));
            palette::TEXT
        } else {
            let resp = ui.button(wid_n(id, i as u64), Rect::new(cell.x, y, cell.w, h));
            if resp.clicked {
                picked = Some(i);
            }
            if resp.pressed || resp.hovered {
                ui.c.fill_rrect(*cell, inner_h / 2.0, Color::white(if resp.pressed { 0.10 } else { 0.06 }));
                palette::TEXT
            } else {
                palette::TEXT_DIM
            }
        };
        ui.c.text(name, *cell, &st.color(color).center());
    }
    picked
}

fn picker_text() -> TextStyle {
    TextStyle::footnote().weight(Weight::Semibold)
}

/// One choice in a picker: its name and the padding around it.
fn choice_width(ui: &mut Ui, name: &str, pad: f32) -> f32 {
    (2.0 * pad + ui.c.measure(name, &picker_text(), 300.0).0).ceil()
}

/// A whole picker: its choices and the track's inset.
pub(crate) fn picker_width(ui: &mut Ui, names: &[&str]) -> f32 {
    2.0 * SEG_INSET + names.iter().map(|n| choice_width(ui, n, SEG_PAD)).sum::<f32>()
}

/// A label with a switch at the row's right end; the whole row toggles.
/// `None` = not known yet: drawn off and dimmed, and it doesn't click.
/// Returns true when clicked.
pub(crate) fn switch_row(ui: &mut Ui, id: u64, r: Rect, label: &str, on: Option<bool>, accent: Color) -> bool {
    switch_row_on(ui, id, r, label, on, accent, None)
}

/// [`switch_row`] on the open card (`card` from `clipboard::pill_card`): in
/// Pill, a bottom row's hover wash follows the card's rounder corners.
fn switch_row_on(ui: &mut Ui, id: u64, r: Rect, label: &str, on: Option<bool>, accent: Color, card: Option<(Rect, f32)>) -> bool {
    let hit = Rect::new(r.x - ROW_BLEED, r.y, r.w + 2.0 * ROW_BLEED, r.h);
    let resp = if on.is_some() { ui.button(id, hit) } else { Default::default() };
    if resp.hovered || resp.pressed {
        super::clipboard::fill_box(ui, card, hit.inset(0.0, 1.0), 8.0, Color::white(if resp.pressed { 0.10 } else { 0.06 }));
    }
    let sw = Rect::new(r.right() - SWITCH_W, r.center().y - SWITCH_H / 2.0, SWITCH_W, SWITCH_H);
    ui.c.text(label, Rect::new(r.x, r.y, (sw.x - 10.0 - r.x).max(0.0), r.h), &TextStyle::body());
    let alpha = if on.is_some() { 1.0 } else { 0.5 };
    ui.c.with_alpha(alpha, |c| draw_switch(c, sw, on == Some(true), accent));
    resp.clicked
}

/// A switch's track and knob.
pub(crate) fn draw_switch(c: &mut crate::gfx::Canvas, r: Rect, on: bool, accent: Color) {
    c.fill_rrect(r, r.h / 2.0, if on { accent } else { palette::TRACK });
    let d = r.h - 2.0 * KNOB_INSET;
    let x = if on { r.right() - KNOB_INSET - d / 2.0 } else { r.x + KNOB_INSET + d / 2.0 };
    c.fill_circle(Point::new(x, r.center().y), d / 2.0, palette::ON_ACCENT);
}

#[cfg(test)]
mod update_navigation_tests {
    use super::*;
    use lumo_core::{settings::Settings, store::Store};
    #[test]
    fn update_notifications_route_to_about_without_changing_the_island_page() {
        let mut st = AppState::new(Settings::default(), Store::default());
        let page = st.page;
        st.open_update();
        assert_eq!(st.page, page);
        assert!(st.cmds.contains(&Command::OpenUpdateSettings));
        assert!(!st.update.sheet);
    }
}
