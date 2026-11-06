//! Today: the local date and month calendar.

use crate::gfx::{TextStyle, Weight};
use crate::state::AppState;
use crate::ui::Ui;
use lumo_core::datetime::{civil_from_days, days_from_civil, month_grid, weekday_from_days, LocalDateTime};
use lumo_core::geom::{palette, Color, Rect};

const HEIGHT: f32 = 164.0;

const DATE_W: f32 = 104.0;
const DATE_GAP: f32 = 40.0;
const CELL_W: f32 = 26.0;
const CELL_H: f32 = 20.0;
const INITIALS_H: f32 = 18.0;

const WEEKDAY_LONG: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTH_LONG: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
/// Monday-first initials.
const INITIALS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

pub fn height(_st: &AppState) -> f32 {
    HEIGHT
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    // Fixed layout height: the island's spring may still be opening.
    let area = Rect::new(area.x, area.y, area.w, HEIGHT);
    let cal_w = CELL_W * 7.0;
    let x = area.x + (area.w - DATE_W - DATE_GAP - cal_w) / 2.0;
    let cal_top = area.y + ((area.h - calendar_block_h(&st.local)) / 2.0).max(0.0);
    draw_date(ui, st, Rect::new(x, area.y, DATE_W, area.h), cal_top);
    draw_calendar(ui, st, Rect::new(x + DATE_W + DATE_GAP, area.y, cal_w, area.h), cal_top);
}

// ------------------------------------------------------------------ date

/// `top` = the calendar block's top edge. The weekday line uses the same
/// 16-high band and 11 DIP caps as the initials row, so both share a baseline.
fn draw_date(ui: &mut Ui, st: &AppState, r: Rect, top: f32) {
    let t = st.local;
    if !valid_date(&t) {
        return;
    }
    let wd = t.weekday() as usize % 7;
    let y = top;
    ui.c.text(&WEEKDAY_LONG[wd].to_uppercase(), Rect::new(r.x, y, r.w, INITIALS_H - 2.0), &TextStyle::caption().weight(Weight::Bold).color(st.accent));
    ui.c.text(&t.day.to_string(), Rect::new(r.x - 2.0, y + 14.0, r.w + 2.0, 58.0), &TextStyle::hero().weight(Weight::Semibold));
    ui.c.text(
        &format!("{} {}", MONTH_LONG[(t.month - 1) as usize], t.year),
        Rect::new(r.x, y + 74.0, r.w, 18.0),
        &TextStyle::body().color(palette::TEXT_DIM),
    );
    ui.c.text(&format!("Week {}", iso_week(&t)), Rect::new(r.x, y + 92.0, r.w, 16.0), &TextStyle::footnote().color(palette::TEXT_FAINT));
}

// ------------------------------------------------------------------ calendar

/// Rows with at least one day in them (4–6).
fn week_rows(t: &LocalDateTime) -> usize {
    month_grid(t.year, t.month).iter().filter(|w| w.iter().any(|d| d.is_some())).count().max(1)
}

/// Height of the initials and the weeks that contain days.
fn calendar_block_h(t: &LocalDateTime) -> f32 {
    let rows = if valid_date(t) { week_rows(t) } else { 6 };
    INITIALS_H + rows as f32 * CELL_H
}

/// `top` = the block's top edge (shared with the date block).
fn draw_calendar(ui: &mut Ui, st: &AppState, r: Rect, top: f32) {
    let t = st.local;
    if !valid_date(&t) {
        return;
    }
    let grid = month_grid(t.year, t.month);
    let rows = week_rows(&t);
    let mut y = top;

    // Every initial in one tone; weekend days are told apart by the dimmer numbers.
    let initials_st = TextStyle::caption().weight(Weight::Semibold).color(palette::TEXT_FAINT).center();
    for (col, s) in INITIALS.iter().enumerate() {
        ui.c.text(s, Rect::new(r.x + col as f32 * CELL_W, y, CELL_W, INITIALS_H - 2.0), &initials_st);
    }
    y += INITIALS_H;

    let day_st = TextStyle::footnote().tabular().center();
    let today_ink = ink_on(st.accent);
    for week in grid.iter().take(rows) {
        for (col, day) in week.iter().enumerate() {
            let Some(d) = *day else { continue };
            let cell = Rect::new(r.x + col as f32 * CELL_W, y, CELL_W, CELL_H);
            let label = d.to_string();
            if d == t.day {
                ui.c.fill_circle(cell.center(), 9.5, st.accent);
                ui.c.text(&label, cell, &day_st.weight(Weight::Bold).color(today_ink));
            } else {
                let c = if col >= 5 { palette::TEXT_DIM } else { Color::white(0.9) };
                ui.c.text(&label, cell, &day_st.color(c));
            }
        }
        y += CELL_H;
    }
}

/// ISO-8601 week number (weeks start Monday; week 1 holds the first Thursday).
fn iso_week(t: &LocalDateTime) -> i64 {
    let z = days_from_civil(t.year, t.month, t.day);
    let mon0 = (weekday_from_days(z) as i64 + 6) % 7; // 0 = Monday
    let thursday = z - mon0 + 3;
    let (ty, _, _) = civil_from_days(thursday);
    (thursday - days_from_civil(ty, 1, 1)) / 7 + 1
}

/// The app fills `local` from GetLocalTime; guard against the zeroed default.
fn valid_date(t: &LocalDateTime) -> bool {
    (1..=12).contains(&t.month) && t.day >= 1
}

/// Text colour that stays legible on an accent fill.
fn ink_on(fill: Color) -> Color {
    if fill.luminance() > 0.6 {
        palette::ON_ACCENT_DARK
    } else {
        palette::ON_ACCENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> LocalDateTime {
        LocalDateTime { year, month, day, hour, minute, second: 0 }
    }

    #[test]
    fn iso_weeks() {
        assert_eq!(iso_week(&dt(2026, 10, 24, 0, 0)), 43);
        // 1 Jan 2027 is a Friday: still week 53 of 2026.
        assert_eq!(iso_week(&dt(2027, 1, 1, 0, 0)), 53);
        assert_eq!(iso_week(&dt(2027, 1, 4, 0, 0)), 1);
        // 29 Dec 2025 (Monday) already belongs to week 1 of 2026.
        assert_eq!(iso_week(&dt(2025, 12, 29, 0, 0)), 1);
    }

}
