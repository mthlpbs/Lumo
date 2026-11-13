//! Native model picker and capability filters. No workers, polls, or idle animation.
use crate::{
    ai::{Action, ModelRow},
    commands::Command,
    gfx::{TextStyle, Weight},
    icons::Icon,
    state::AppState,
    ui::{wid, wid_n, CursorKind, KeyInput, Resp, Ui},
};
use lumo_core::{
    ai::{Kind, Model},
    ai_reasoning,
    geom::{palette, Color, Point, Rect},
};
const BLUE: Color = Color::rgba(0.52, 0.72, 1.0, 1.0);
const YELLOW: Color = Color::rgba(1.0, 0.78, 0.22, 1.0);
const VIOLET: Color = Color::rgba(0.75, 0.60, 1.0, 1.0);
const FREE: u8 = 1;
const IMAGES: u8 = 2;
const FILES: u8 = 4;
const GENERATE: u8 = 8;
/// Compact popover: a search header, then a grouped list sized to its content.
const MAX_W: f32 = 380.0;
const MAX_H: f32 = 372.0;
const PAD: f32 = 8.0;
const HEAD_H: f32 = 52.0;
const ROW_H: f32 = 34.0;
const GROUP_H: f32 = 28.0;
const EMPTY_H: f32 = 96.0;
const PANEL: Color = Color::rgba(0.09, 0.09, 0.10, 1.0);
fn matches_filters(m: &Model, mask: u8) -> bool {
    (mask & FREE == 0 || m.free == Some(true))
        && (mask & IMAGES == 0 || m.inputs.iter().any(|v| v == "image"))
        && (mask & FILES == 0 || m.inputs.iter().any(|v| v == "file"))
        && (mask & GENERATE == 0 || m.outputs.iter().any(|v| v == "image"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_rows_hide_only_for_hackclub_and_toggle_rebuilds_the_list() {
        let mut st = AppState::new(Default::default(), Default::default());
        let mut old = Model::manual("old");
        old.unavailable = Some("Deprecated".into());
        st.ai.models = vec![old, Model::manual("good")];
        rows(&mut st);
        assert_eq!(st.ai.model_rows.len(), 2);
        st.settings.ai.completion_url = "https://ai.hackclub.com/proxy/v1/chat/completions".into();
        rows(&mut st);
        assert_eq!(st.ai.model_rows.len(), 1);
        st.settings.ai.hide_unavailable_models = false;
        rows(&mut st);
        assert_eq!(st.ai.model_rows.len(), 2);
        st.settings.ai.hide_unavailable_models = true;
        st.ai
            .unavailable_models
            .insert((crate::ai::catalog_key(&st), "good".into()));
        st.ai.catalog_revision += 1;
        rows(&mut st);
        assert!(st.ai.model_rows.is_empty());
    }
    #[test]
    fn capability_filters_combine_with_search_and_keep_selections() {
        let mut st = AppState::new(Default::default(), Default::default());
        st.ai.models = lumo_core::ai::parse_models(&serde_json::json!({"data":[
            {"id":"p/plain","name":"Plain model"},
            {"id":"p/vision:free","name":"Vision model", "architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]}},
            {"id":"p/image","name":"Image studio", "architecture":{"input_modalities":["text","image"],"output_modalities":["image"]}}
        ]})).unwrap();
        let count = |st: &AppState| {
            st.ai
                .model_rows
                .iter()
                .filter(|row| matches!(row, ModelRow::Model(_)))
                .count()
        };
        for (mask, expected) in [
            (FREE, 1),
            (IMAGES, 2),
            (FILES, 1),
            (GENERATE, 1),
            (FREE | IMAGES | FILES, 1),
            (FREE | GENERATE, 0),
        ] {
            st.ai.model_filters = mask;
            rows(&mut st);
            assert_eq!(count(&st), expected, "mask {mask}");
        }
        st.ai.model_filters = FREE | IMAGES;
        st.ai.search = "Vision".into();
        rows(&mut st);
        assert_eq!(count(&st), 1);
        st.ai.search = "Plain".into();
        rows(&mut st);
        assert_eq!(count(&st), 0);
        st.ai.model_filters_open = true;
        st.ai.close_models();
        assert!(!st.ai.model_filters_open);
        assert_eq!(st.ai.model_filters, FREE | IMAGES);
    }
}
fn text(ui: &mut Ui, s: &str, r: Rect, size: f32, col: Color) {
    ui.c.text(s, r, &TextStyle::new(size).color(col));
}
fn surface(ui: &mut Ui, r: Rect) {
    ui.c.fill_rrect(r, 14.0, PANEL);
    ui.c.stroke_rrect(r, 14.0, Color::white(0.15), 1.0);
}
/// One heading per provider; a single provider needs none.
struct Group {
    y: f32,
    first: usize,
    count: usize,
}
/// Vertical placement of every row (and group heading) inside the scroll area.
struct Layout {
    ys: Vec<f32>,
    groups: Vec<Group>,
    total: f32,
}
fn provider_of(st: &AppState, n: usize) -> &str {
    let ModelRow::Model(i) = &st.ai.model_rows[n];
    st.ai.models[*i].provider_name()
}
fn layout(st: &AppState) -> Layout {
    let count = st.ai.model_rows.len();
    let grouped = (1..count).any(|n| provider_of(st, n) != provider_of(st, 0));
    let mut ys = Vec::with_capacity(count);
    let mut groups: Vec<Group> = vec![];
    let mut y = 0.0;
    for n in 0..count {
        if grouped && (n == 0 || provider_of(st, n) != provider_of(st, n - 1)) {
            groups.push(Group {
                y,
                first: n,
                count: 0,
            });
            y += GROUP_H;
        }
        if let Some(group) = groups.last_mut() {
            group.count += 1;
        }
        ys.push(y);
        y += ROW_H;
    }
    Layout {
        ys,
        groups,
        total: y,
    }
}
/// Height of the list area, including its bottom margin.
fn content_height(st: &AppState) -> f32 {
    if st.ai.model_rows.is_empty() {
        EMPTY_H
    } else {
        layout(st).total + 6.0
    }
}
fn tool_button(ui: &mut Ui, id: u64, r: Rect, icon: Icon, on: bool) -> Resp {
    let resp = ui.button(id, r);
    let fill = if on {
        BLUE.mul_alpha(0.18)
    } else if resp.pressed {
        Color::white(0.14)
    } else if resp.hovered {
        Color::white(0.10)
    } else {
        Color::white(0.0)
    };
    if fill.a > 0.0 {
        ui.c.fill_rrect(r, r.h / 2.0, fill);
    }
    let tint = if on {
        BLUE
    } else if resp.hovered {
        palette::TEXT
    } else {
        palette::TEXT_DIM
    };
    ui.c.icon(icon, Rect::from_center(r.center(), 16.0, 16.0), tint);
    resp
}
/// Small status pill ending at `right`; returns where the next one may end.
fn chip(ui: &mut Ui, right: f32, cy: f32, label: &str, col: Color) -> f32 {
    let style = TextStyle::new(9.5)
        .weight(Weight::Semibold)
        .center()
        .color(col);
    let w = ui.c.measure(label, &style, 200.0).0 + 12.0;
    let r = Rect::new(right - w, cy - 8.0, w, 16.0);
    ui.c.fill_rrect(r, 8.0, col.mul_alpha(0.14));
    ui.c.text(label, r, &style);
    r.x - 6.0
}
fn group_header(ui: &mut Ui, r: Rect, label: &str, count: usize, opaque: bool) {
    if opaque {
        ui.c.fill_rect(r, PANEL);
    }
    ui.c.text(
        label,
        Rect::new(r.x + 12.0, r.y + 4.0, (r.w - 64.0).max(1.0), r.h - 6.0),
        &TextStyle::new(11.0)
            .weight(Weight::Semibold)
            .color(palette::TEXT_FAINT),
    );
    ui.c.text(
        &count.to_string(),
        Rect::new(r.right() - 48.0, r.y + 4.0, 40.0, r.h - 6.0),
        &TextStyle::new(10.0).right().color(palette::TEXT_FAINT),
    );
}
fn close(ui: &mut Ui, r: Rect) -> bool {
    ui.icon_button(
        wid("ai.model-close"),
        Rect::new(r.right() - 37.0, r.y + 7.0, 30.0, 28.0),
        Icon::Close,
        palette::TEXT_DIM,
        14.0,
    )
    .clicked
}
fn selected(ui: &mut Ui, st: &mut AppState, id: String, settings: bool) {
    if st.ai.model_unavailable(&st.settings.ai, &id).is_some() {
        return;
    }
    let adjustable = st
        .ai
        .models
        .iter()
        .any(|m| m.id == id && m.reasoning.adjustable());
    let current = if settings {
        st.settings.ai.default_model.clone()
    } else {
        st.ai.model(&st.settings.ai).id
    };
    let edit_effort = adjustable && current == id;
    // Switching models always starts with its own default. Re-selecting the
    // active starred row is the deliberate action that edits its effort.
    if adjustable && !edit_effort {
        remember_effort(st, &id, None, true);
    }
    st.ai.model_filters_open = false;
    st.ai.model_cursor = None;
    ui.request_repaint();
    if settings {
        st.settings.ai.default_model = id.clone();
        st.cmd(Command::SaveSettings);
    }
    st.ai.selected = id;
    st.ai.model_menu = edit_effort;
    st.ai.reasoning_menu = edit_effort;
    st.ai.model_detail = None;
    st.ai.model_detail_scroll = 0.0;
    st.ai.hover_model = None;
    if edit_effort {
        ui.input.focused = Some(wid("ai.effort-slider"));
        if !settings {
            st.cmd(Command::FocusText);
        }
    } else if !settings {
        ui.input.focused = Some(wid("ai.prompt"));
        ui.input.caret = st.ai.draft.chars().count();
        st.cmd(Command::FocusText);
    } else {
        ui.input.focused = None;
    }
}

fn remember_effort(st: &mut AppState, id: &str, value: Option<String>, commit: bool) {
    if let Some(value) = value {
        if st.settings.ai.reasoning_efforts.len() >= 128
            && !st.settings.ai.reasoning_efforts.contains_key(id)
        {
            if let Some(first) = st.settings.ai.reasoning_efforts.keys().next().cloned() {
                st.settings.ai.reasoning_efforts.remove(&first);
            }
        }
        st.settings.ai.reasoning_efforts.insert(id.into(), value);
    } else {
        st.settings.ai.reasoning_efforts.remove(id);
    }
    if commit {
        st.cmd(Command::SaveSettings);
    }
}
fn effort(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    let m = st.ai.model(&st.settings.ai);
    if !m.reasoning.adjustable() {
        st.ai.reasoning_menu = false;
        st.ai.model_menu = true;
        ui.request_repaint();
        return;
    }
    let value = st
        .settings
        .ai
        .reasoning_efforts
        .get(&m.id)
        .filter(|v| m.reasoning.levels.contains(v))
        .or(m.reasoning.default_effort.as_ref());
    let index = value.and_then(|v| m.reasoning.levels.iter().position(|x| x == v));
    let label = value
        .map(|s| ai_reasoning::label(s))
        .unwrap_or_else(|| "Choose effort".into());
    ui.c.text(
        &label,
        Rect::new(r.x + 36.0, r.y + 9.0, r.w - 72.0, 25.0),
        &TextStyle::new(16.0)
            .weight(Weight::Semibold)
            .center()
            .color(BLUE),
    );
    let reset = ui.icon_button(
        wid("ai.effort-reset"),
        Rect::new(r.right() - 37.0, r.y + 8.0, 28.0, 28.0),
        Icon::Reset,
        palette::TEXT_DIM,
        15.0,
    );
    if reset.clicked {
        remember_effort(st, &m.id, None, true);
        ui.request_repaint();
    }
    let model_r = Rect::new(r.x + 32.0, r.y + 35.0, r.w - 64.0, 24.0);
    let change = ui.button(wid("ai.effort-model"), model_r);
    ui.c.text(
        &format!("{}  ›", m.display_name()),
        model_r,
        &TextStyle::new(11.0).center().color(if change.hovered {
            palette::TEXT
        } else {
            palette::TEXT_DIM
        }),
    );
    if change.clicked {
        st.ai.model_filters_open = false;
        ui.request_repaint();
        st.ai.reasoning_menu = false;
        st.ai.model_menu = true;
        st.ai.model_detail = None;
        st.ai.model_scroll = 0.0;
        st.ai.search.clear();
        ui.input.focused = Some(wid("ai.model-search"));
        ui.input.caret = 0;
        st.ai.reveal_model = true;
        if !settings {
            st.cmd(Command::FocusText);
        }
        return;
    }
    let track = Rect::new(r.x + 17.0, r.y + 77.0, r.w - 34.0, 24.0);
    let response = ui.button(wid("ai.effort-slider"), track.inset(0.0, -5.0));
    ui.c.fill_rrect(track, 12.0, Color::white(0.09));
    let x0 = track.x + 12.0;
    let length = track.w - 24.0;
    let count = m.reasoning.levels.len();
    let position = |i: usize| x0 + length * i as f32 / (count - 1) as f32;
    if ui.input.focused == Some(wid("ai.effort-slider")) {
        let mut next = index.unwrap_or(0);
        let mut navigate = false;
        let keys = std::mem::take(&mut ui.input.keys);
        for key in &keys {
            use crate::ui::KeyInput;
            match key {
                KeyInput::Left => {
                    next = next.saturating_sub(1);
                    navigate = true;
                }
                KeyInput::Right => {
                    next = (next + 1).min(count - 1);
                    navigate = true;
                }
                KeyInput::Home => {
                    next = 0;
                    navigate = true;
                }
                KeyInput::End => {
                    next = count - 1;
                    navigate = true;
                }
                _ => {}
            }
        }
        if navigate && index != Some(next) {
            remember_effort(st, &m.id, Some(m.reasoning.levels[next].clone()), true);
            ui.request_repaint();
        }
    }
    if let Some(i) = index {
        ui.c.fill_rrect(
            Rect::new(track.x, track.y, position(i) - track.x + 10.0, track.h),
            12.0,
            BLUE,
        );
    }
    for i in 0..count {
        ui.c.fill_circle(
            Point::new(position(i), track.center().y),
            2.0,
            Color::white(0.32),
        );
    }
    if let Some(i) = index {
        ui.c.fill_circle(
            Point::new(position(i), track.center().y),
            12.0,
            palette::TEXT,
        );
    }
    let mouse = ui
        .input
        .released
        .filter(|(id, _)| *id == wid("ai.effort-slider"))
        .map(|(_, p)| p)
        .or_else(|| {
            if response.pressed {
                ui.input.mouse
            } else {
                None
            }
        });
    if let Some(p) = mouse {
        let i = (((p.x - x0) / length).clamp(0.0, 1.0) * (count - 1) as f32).round() as usize;
        if index != Some(i) {
            remember_effort(st, &m.id, Some(m.reasoning.levels[i].clone()), false);
            ui.request_repaint();
        }
    }
    if ui
        .input
        .released
        .is_some_and(|(id, _)| id == wid("ai.effort-slider"))
    {
        st.cmd(Command::SaveSettings);
    }
    if response.hovered || reset.hovered {
        let hint = if reset.hovered {
            "Reset to the model's own setting"
        } else {
            "Higher effort can take longer and use more tokens"
        };
        text(
            ui,
            hint,
            Rect::new(r.x + 12.0, r.y + 107.0, r.w - 24.0, 22.0),
            10.0,
            palette::TEXT_DIM,
        );
    }
}
fn rows(st: &mut AppState) {
    let key = (
        st.ai.catalog_revision,
        st.ai.models.len(),
        st.ai.search.to_lowercase(),
        st.ai.model_filters
            | if st.settings.ai.hackclub() && st.settings.ai.hide_unavailable_models {
                16
            } else {
                0
            },
    );
    if st.ai.model_rows_key.as_ref() == Some(&key) {
        return;
    }
    let q = &key.2;
    let scope = crate::ai::catalog_key(st);
    let mut indices: Vec<_> = st
        .ai
        .models
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            if key.3 & 16 != 0
                && (m.unavailable.is_some()
                    || st
                        .ai
                        .unavailable_models
                        .contains(&(scope.clone(), m.id.clone())))
            {
                return false;
            }
            if !matches_filters(m, key.3) {
                return false;
            }
            if q == "free" {
                return m.free == Some(true);
            }
            q.is_empty()
                || [&m.id, &m.name, &m.provider, &m.description]
                    .iter()
                    .any(|s| s.to_lowercase().contains(q))
                || m.capabilities.iter().any(|s| s.to_lowercase().contains(q))
                || m.provider_name().to_lowercase().contains(q)
                || m.kind.name().to_lowercase().contains(q)
                || m.cost_group().to_lowercase().contains(q)
                || m.inputs
                    .iter()
                    .chain(&m.outputs)
                    .any(|s| s.to_lowercase().contains(q))
                || (q == "reasoning" && (m.reasoning.mandatory || !m.reasoning.levels.is_empty()))
        })
        .map(|(i, _)| i)
        .collect();
    indices.sort_by_cached_key(|i| {
        let m = &st.ai.models[*i];
        (
            m.provider_name().to_lowercase(),
            m.display_name().to_lowercase(),
            m.id.clone(),
        )
    });
    let mut result = vec![];
    for i in indices {
        result.push(ModelRow::Model(i));
    }
    st.ai.model_rows = result;
    st.ai.model_rows_key = Some(key);
}
/// Effort needs only its supported choices, rather than the catalog's full
/// browsing height. Share these bounds with the island's input mask.
pub fn popup_rect(st: &AppState, r: Rect) -> Rect {
    if !st.ai.reasoning_menu {
        return r;
    }
    let width = r.w.min(280.0);
    Rect::new(r.right() - width, r.y, width, r.h.min(132.0))
}

pub fn picker(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    rows(st);
    let bounds = r;
    let mut r = popup_rect(st, r);
    if !st.ai.reasoning_menu {
        if r.w > MAX_W {
            r.x = if settings {
                r.x + (r.w - MAX_W) / 2.0
            } else {
                r.right() - MAX_W
            };
            r.w = MAX_W;
        }
        if st.ai.model_detail.is_none() {
            // Size to the list rather than filling the window, anchored above
            // the composer. Freeze the height while filtering: reducing a long
            // catalog must not move the child popup (and checkbox) under the
            // pointer. `model_filter_rows` holds that frozen list height.
            let content = if st.ai.model_filters_open && st.ai.model_filter_rows > 0 {
                st.ai.model_filter_rows as f32
            } else {
                content_height(st)
            };
            let height = (HEAD_H + content).min(MAX_H).min(r.h);
            if !settings {
                r.y = r.bottom() - height;
            }
            r.h = height;
        }
    }
    let filter_panel = filter_bounds(bounds, r);
    if st.ai.model_filters_open
        && (ui.input.keys.contains(&crate::ui::KeyInput::Escape)
            || (ui.input.just_pressed
                && ui
                    .input
                    .press_pos
                    .or(ui.input.mouse)
                    .is_some_and(|p| !filter_panel.contains(p))))
    {
        st.ai.model_filters_open = false;
        ui.input.keys.retain(|k| *k != crate::ui::KeyInput::Escape);
        ui.input.clicked = None;
        ui.input.released = None;
        ui.input.just_pressed = false;
        ui.input.down = false;
        ui.input.pressed_id = None;
        ui.input.focused = Some(wid("ai.model-search"));
        ui.request_repaint();
    }
    if ui.input.just_pressed && ui.input.mouse.is_some_and(|p| !r.contains(p)) {
        st.ai.close_models();
        ui.input.focused = if settings {
            None
        } else {
            Some(wid("ai.prompt"))
        };
        ui.input.caret = st.ai.draft.chars().count();
        if !settings {
            st.cmd(Command::FocusText);
        }
        ui.input.pressed_id = None;
        ui.input.clicked = None;
        ui.input.down = false;
        ui.request_repaint();
        return;
    }
    ui.button(wid("ai.model-surface"), r);
    surface(ui, r);
    if st.ai.reasoning_menu {
        effort(ui, st, r, settings);
        return;
    }
    if st.ai.model_detail.is_some() {
        details(ui, st, r, settings);
        return;
    }
    // The search field is the header: no title bar. Esc or a click outside closes.
    let top = r.y + 9.0;
    let refresh_r = Rect::new(r.right() - PAD - 32.0, top + 1.0, 32.0, 32.0);
    let filter_r = Rect::new(refresh_r.x - 34.0, top + 1.0, 32.0, 32.0);
    let filtered = st.ai.model_filters != 0;
    let filter = tool_button(
        ui,
        wid("ai.model-filters"),
        filter_r,
        Icon::Filter,
        filtered || st.ai.model_filters_open,
    );
    if filtered {
        let badge = Rect::new(filter_r.right() - 14.0, filter_r.y - 1.0, 15.0, 15.0);
        ui.c.fill_rrect(badge, 7.5, BLUE);
        ui.c.text(
            &st.ai.model_filters.count_ones().to_string(),
            badge,
            &TextStyle::new(9.0).center().color(Color::black(1.0)),
        );
    }
    if filter.clicked {
        st.ai.model_filters_open = !st.ai.model_filters_open;
        if st.ai.model_filters_open {
            st.ai.model_filter_rows = content_height(st).ceil() as usize;
        }
        st.ai.hover_model = None;
        ui.input.focused = None;
        ui.request_repaint();
    }
    // Keep the catalog visible behind a separate modal filter popover.
    let filter_input = st.ai.model_filters_open.then(|| std::mem::take(ui.input));
    let catalog_hits = ui.hits.len();
    let search_id = wid("ai.model-search");
    // Arrow keys walk the list while the search field keeps keyboard focus.
    let mut step = 0i32;
    if ui.input.focused == Some(search_id) {
        ui.input.keys.retain(|k| match k {
            KeyInput::Up => {
                step -= 1;
                false
            }
            KeyInput::Down => {
                step += 1;
                false
            }
            _ => true,
        });
    }
    let search = Rect::new(
        r.x + PAD,
        top,
        (filter_r.x - 4.0 - (r.x + PAD)).max(60.0),
        34.0,
    );
    let search_resp = ui.text_input_capsule(
        search_id,
        search,
        &mut st.ai.search,
        "Search models or providers…",
        120,
        BLUE,
    );
    if search_resp.focused {
        st.cmd(Command::FocusText);
    }
    if search_resp.changed {
        st.ai.model_scroll = 0.0;
        st.ai.model_cursor = None;
        st.ai.hover_model = None;
    }
    if !st.ai.search.is_empty() {
        let clear = ui.icon_button(
            wid("ai.model-search-clear"),
            Rect::new(search.right() - 31.0, search.y + 3.0, 28.0, 28.0),
            Icon::Close,
            palette::TEXT_DIM,
            12.0,
        );
        if clear.clicked {
            st.ai.search.clear();
            st.ai.model_scroll = 0.0;
            st.ai.model_cursor = None;
            ui.input.focused = Some(search_id);
            ui.input.caret = 0;
            ui.input.selection_anchor = None;
            ui.input.selection_all = false;
            st.cmd(Command::FocusText);
            ui.request_repaint();
        }
    }
    if tool_button(ui, wid("ai.model-refresh"), refresh_r, Icon::Reset, false).clicked {
        st.cmd(Command::Ai(Action::FetchModels));
    }
    ui.c.fill_rect(
        Rect::new(r.x, r.y + HEAD_H - 1.0, r.w, 1.0),
        Color::white(0.06),
    );
    rows(st);
    let lay = layout(st);
    let view = Rect::new(
        r.x + 6.0,
        r.y + HEAD_H,
        r.w - 12.0,
        (r.h - HEAD_H - 6.0).max(24.0),
    );
    let max = (lay.total - view.h).max(0.0);
    let count = st.ai.model_rows.len();
    let active = if settings || st.ai.selected.is_empty() {
        st.settings.ai.default_model.clone()
    } else {
        st.ai.selected.clone()
    };
    let active_row = st
        .ai
        .model_rows
        .iter()
        .position(|row| matches!(row, ModelRow::Model(i) if st.ai.models[*i].id == active));
    if st.ai.reveal_model && count > 0 {
        st.ai.reveal_model = false;
        if let Some(index) = active_row {
            st.ai.model_scroll = (lay.ys[index] - GROUP_H - ROW_H).clamp(0.0, max);
        }
    }
    // The keyboard cursor yields to the pointer as soon as the mouse moves.
    if st.ai.model_cursor.is_some_and(|c| c >= count)
        || (st.ai.model_cursor.is_some() && st.ai.model_cursor_mouse != ui.input.mouse)
    {
        st.ai.model_cursor = None;
    }
    if step != 0 && count > 0 {
        let base = st.ai.model_cursor.or(active_row);
        let next = match base {
            Some(b) => (b as i32 + step).clamp(0, count as i32 - 1) as usize,
            None if step > 0 => 0,
            None => count - 1,
        };
        st.ai.model_cursor = Some(next);
        st.ai.model_cursor_mouse = ui.input.mouse;
        // Keep the row clear of the pinned group heading.
        let lead = if lay.groups.is_empty() { 0.0 } else { GROUP_H };
        let y = lay.ys[next];
        if y - lead < st.ai.model_scroll {
            st.ai.model_scroll = y - lead;
        } else if y + ROW_H > st.ai.model_scroll + view.h {
            st.ai.model_scroll = y + ROW_H - view.h;
        }
        ui.request_repaint();
    }
    st.ai.model_scroll = (st.ai.model_scroll - ui.wheel_in(view) * 40.0).clamp(0.0, max);
    let pointer_in_list = ui.input.mouse.is_some_and(|p| view.contains(p));
    // While searching, Enter picks the top result unless the arrows moved on.
    let top_result = (!st.ai.search.is_empty() && count > 0).then_some(0usize);
    let enter_target = st.ai.model_cursor.or(top_result);
    let cursor = st
        .ai
        .model_cursor
        .or(if pointer_in_list { None } else { top_result });
    let mut choose = None;
    let mut info = None;
    let mut preview = None;
    let fixed = ui.hits.len();
    ui.c.clip(view);
    if count == 0 {
        let message = if st.ai.fetching {
            "Fetching models…"
        } else if st.ai.models.is_empty() {
            "Fetch models or add a model ID in Settings."
        } else {
            "No matching models"
        };
        let c = view.center();
        if !st.ai.fetching {
            ui.c.icon(
                Icon::Search,
                Rect::from_center(Point::new(c.x, c.y - 18.0), 20.0, 20.0),
                palette::TEXT_FAINT,
            );
        }
        ui.c.text(
            message,
            Rect::new(view.x, c.y - 2.0, view.w, 24.0),
            &TextStyle::new(12.0).center().color(palette::TEXT_DIM),
        );
    }
    let row_w = if lay.total > view.h {
        view.w - 8.0
    } else {
        view.w
    };
    for (index, row) in st.ai.model_rows.iter().enumerate() {
        let y = view.y + lay.ys[index] - st.ai.model_scroll;
        if y + ROW_H < view.y || y > view.bottom() {
            continue;
        }
        let ModelRow::Model(i) = row;
        let m = &st.ai.models[*i];
        let unavailable = st.ai.model_unavailable(&st.settings.ai, &m.id);
        let rr = Rect::new(view.x, y, row_w, ROW_H - 2.0);
        let response = unavailable
            .is_none()
            .then(|| ui.button(wid_n("ai.model", *i as u64), rr));
        let is_active = m.id == active;
        let hovered = pointer_in_list && response.as_ref().is_some_and(|r| r.hovered);
        let focused = unavailable.is_none() && cursor == Some(index);
        if unavailable.is_none() {
            if is_active {
                ui.c.fill_rrect(rr, 9.0, BLUE.mul_alpha(0.16));
            } else if hovered || focused {
                ui.c.fill_rrect(rr, 9.0, Color::white(0.07));
            }
            if focused {
                ui.c.stroke_rrect(rr, 9.0, BLUE.mul_alpha(0.6), 1.0);
            }
        }
        // Details stay out of the way until the row is hovered or focused;
        // an unavailable row can't be chosen, so its details are always shown.
        let ir = Rect::new(rr.right() - 28.0, rr.center().y - 12.0, 24.0, 24.0);
        let irsp = ui.button(wid_n("ai.model-info", *i as u64), ir);
        let info_hover = pointer_in_list && irsp.hovered;
        if hovered || focused || info_hover || unavailable.is_some() {
            if info_hover {
                ui.c.fill_circle(ir.center(), 11.0, Color::white(0.12));
                preview = Some((*i, rr));
            }
            ui.c.stroke_circle(
                ir.center(),
                6.5,
                Color::white(if info_hover { 0.75 } else { 0.40 }),
                1.0,
            );
            ui.c.text(
                "i",
                ir,
                &TextStyle::new(10.0).center().color(if info_hover {
                    palette::TEXT
                } else {
                    palette::TEXT_DIM
                }),
            );
        }
        // Status sits at the right edge, filling leftwards; the name gets the rest.
        let cy = rr.center().y;
        let mut right = ir.x - 2.0;
        if is_active && unavailable.is_none() {
            ui.c.icon(
                Icon::Check,
                Rect::new(right - 14.0, cy - 7.0, 14.0, 14.0),
                BLUE,
            );
            right -= 20.0;
        }
        if m.reasoning.adjustable() {
            ui.c.icon(
                Icon::Star,
                Rect::new(right - 12.0, cy - 6.0, 12.0, 12.0),
                VIOLET,
            );
            right -= 18.0;
        }
        if unavailable.is_some() {
            right = chip(ui, right, cy, "Unavailable", palette::ORANGE);
        } else {
            if m.free == Some(true) {
                right = chip(ui, right, cy, "Free", YELLOW);
            }
            if m.kind != Kind::Language {
                let label = if m.kind == Kind::Image {
                    "Image"
                } else {
                    "Embedding"
                };
                right = chip(ui, right, cy, label, palette::TEXT_DIM);
            }
        }
        let name_x = rr.x + 12.0;
        let style = TextStyle::new(13.0)
            .weight(if is_active {
                Weight::Medium
            } else {
                Weight::Regular
            })
            .color(if unavailable.is_some() {
                palette::TEXT_FAINT
            } else {
                palette::TEXT
            });
        ui.c.text(
            m.display_name(),
            Rect::new(name_x, rr.y, (right - name_x - 4.0).max(24.0), rr.h),
            &style,
        );
        if irsp.clicked && !st.ai.model_filters_open {
            info = Some(m.id.clone());
        }
        if response.is_some_and(|r| r.clicked) && !st.ai.model_filters_open {
            choose = Some(m.id.clone());
        }
    }
    // Provider headings scroll with the list; the current one pins to the top
    // and is pushed out by the next.
    let scroll = st.ai.model_scroll;
    let mut pinned = None;
    for (g, group) in lay.groups.iter().enumerate() {
        let gy = group.y - scroll;
        if gy <= 0.0 {
            pinned = Some(g);
            continue;
        }
        if gy > view.h {
            break;
        }
        let label = provider_of(st, group.first);
        group_header(
            ui,
            Rect::new(view.x, view.y + gy, view.w, GROUP_H),
            label,
            group.count,
            false,
        );
    }
    if let Some(g) = pinned {
        let group = &lay.groups[g];
        let next = lay.groups.get(g + 1).map_or(f32::MAX, |n| n.y - scroll);
        let head = Rect::new(view.x, view.y + (next - GROUP_H).min(0.0), view.w, GROUP_H);
        // Rows scrolled beneath the pinned heading must not take its clicks.
        ui.interact(wid("ai.model-group"), head, CursorKind::Arrow);
        let label = provider_of(st, group.first);
        group_header(ui, head, label, group.count, true);
    }
    ui.c.unclip();
    super::ai::clip_hits(ui, fixed, view);
    scrollbar(
        ui,
        wid("ai.model-scrollbar"),
        view,
        lay.total,
        &mut st.ai.model_scroll,
    );
    st.ai.hover_model = None;
    if choose.is_none() && search_resp.submitted && !st.ai.model_filters_open {
        if let Some(ModelRow::Model(i)) = enter_target.and_then(|n| st.ai.model_rows.get(n)) {
            choose = Some(st.ai.models[*i].id.clone());
        }
    }
    if let Some(id) = choose {
        selected(ui, st, id, settings);
    }
    if let Some(id) = info {
        ui.request_repaint();
        st.ai.model_detail = Some(id);
        st.ai.model_detail_scroll = 0.0;
        st.ai.hover_model = None;
    }
    let hint = if ui.hovered(filter_r) {
        Some(("Filter models", filter_r))
    } else if ui.hovered(refresh_r) {
        Some(("Refresh model list", refresh_r))
    } else {
        None
    };
    if let Some(input) = filter_input {
        ui.hits.truncate(catalog_hits);
        *ui.input = input;
        ui.c.fill_rrect(r, 14.0, Color::black(0.32));
        filters(ui, st, filter_panel);
    } else if st.ai.model_detail.is_none() {
        if let Some((i, row)) = preview {
            let mut model = st.ai.models[i].clone();
            model.unavailable = st.ai.model_unavailable(&st.settings.ai, &model.id);
            preview_card(ui, &model, bounds, row);
        } else if let Some((label, button)) = hint {
            let w = ui.c.measure(label, &TextStyle::new(12.0), 260.0).0 + 24.0;
            super::ai::tooltip(
                ui,
                label,
                Rect::new(button.right() - w, button.bottom() + 4.0, w, 30.0),
            );
        }
    }
}
fn filter_bounds(bounds: Rect, parent: Rect) -> Rect {
    let w = (bounds.w - 16.0).min(312.0);
    let h = (bounds.h - 12.0).min(300.0).max(132.0).min(bounds.h);
    Rect::new(
        parent.right() - w - 8.0,
        (parent.y + HEAD_H - 4.0).min(bounds.bottom() - h),
        w,
        h,
    )
}
fn filters(ui: &mut Ui, st: &mut AppState, r: Rect) {
    ui.button(wid("ai.model-filter-surface"), r);
    surface(ui, r);
    let panel = r.inset(12.0, 10.0);
    text(
        ui,
        "Filter models",
        Rect::new(panel.x + 4.0, panel.y, panel.w, 24.0),
        12.0,
        palette::TEXT_DIM,
    );
    let bottom = panel.bottom() - 34.0;
    let row_h = ((bottom - panel.y - 30.0) / 4.0).min(56.0).max(16.0);
    for (i, (mask, label, hint, icon)) in [
        (
            FREE,
            "Free models",
            "No input or output charge",
            Icon::Circle,
        ),
        (
            IMAGES,
            "Accepts images",
            "Ask questions about a picture",
            Icon::Image,
        ),
        (
            FILES,
            "Accepts files",
            "Send documents with your message",
            Icon::File,
        ),
        (
            GENERATE,
            "Creates images",
            "Generate pictures from a prompt",
            Icon::Sparkles,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let row = Rect::new(
            panel.x,
            panel.y + 28.0 + i as f32 * row_h,
            panel.w,
            row_h - 4.0,
        );
        let response = ui.button(wid_n("ai.model-filter", mask as u64), row);
        let on = st.ai.model_filters & mask != 0;
        ui.c.fill_rrect(
            row,
            9.0,
            if on {
                BLUE.mul_alpha(0.15)
            } else {
                Color::white(if response.hovered { 0.09 } else { 0.035 })
            },
        );
        if on {
            ui.c.stroke_rrect(row, 9.0, BLUE.mul_alpha(0.5), 1.0);
        }
        ui.c.icon(
            icon,
            Rect::new(row.x + 12.0, row.center().y - 8.0, 16.0, 16.0),
            if on { BLUE } else { palette::TEXT_DIM },
        );
        let label_h = if row.h >= 42.0 { 24.0 } else { row.h };
        text(
            ui,
            label,
            Rect::new(
                row.x + 40.0,
                row.y + if row.h >= 42.0 { 3.0 } else { 0.0 },
                row.w - 80.0,
                label_h,
            ),
            12.0,
            if on { BLUE } else { palette::TEXT },
        );
        if row.h >= 42.0 {
            text(
                ui,
                hint,
                Rect::new(row.x + 40.0, row.y + 25.0, row.w - 80.0, 17.0),
                10.0,
                palette::TEXT_DIM,
            );
        }
        let check = Rect::new(row.right() - 30.0, row.center().y - 8.0, 16.0, 16.0);
        ui.c.stroke_rrect(check, 5.0, if on { BLUE } else { Color::white(0.35) }, 1.0);
        if on {
            ui.c.fill_rrect(check, 5.0, BLUE);
            ui.c.icon(Icon::Check, check.inset(3.0, 3.0), Color::black(1.0));
        }
        if response.clicked {
            st.ai.model_filters ^= mask;
            st.ai.model_scroll = 0.0;
            st.ai.hover_model = None;
            ui.request_repaint();
        }
    }
    if ui
        .pill_button(
            wid("ai.model-filter-clear"),
            Rect::new(panel.x, bottom, 90.0, 30.0),
            "Reset",
            None,
            Color::white(0.06),
            palette::TEXT_DIM,
        )
        .clicked
    {
        st.ai.model_filters = 0;
        st.ai.model_scroll = 0.0;
        ui.request_repaint();
    }
    rows(st);
    let count = st
        .ai
        .model_rows
        .iter()
        .filter(|row| matches!(row, ModelRow::Model(_)))
        .count();
    let label = format!("Show {count} model{}", if count == 1 { "" } else { "s" });
    if ui
        .pill_button(
            wid("ai.model-filter-done"),
            Rect::new(panel.right() - 136.0, bottom, 136.0, 30.0),
            &label,
            None,
            BLUE.mul_alpha(0.20),
            BLUE,
        )
        .clicked
    {
        st.ai.model_filters_open = false;
        ui.input.focused = Some(wid("ai.model-search"));
        ui.input.caret = st.ai.search.chars().count();
        st.cmd(Command::FocusText);
        ui.request_repaint();
    }
}
fn scrollbar(ui: &mut Ui, id: u64, r: Rect, total: f32, scroll: &mut f32) {
    if total <= r.h {
        return;
    }
    let h = (r.h * r.h / total).max(20.0).min(r.h);
    let max = total - r.h;
    let b = ui.button(id, Rect::new(r.right() - 8.0, r.y, 8.0, r.h));
    if b.pressed && r.h > h {
        if let Some(p) = ui.input.mouse {
            let value = ((p.y - r.y - h / 2.0) / (r.h - h) * max).clamp(0.0, max);
            if (*scroll - value).abs() > 0.1 {
                *scroll = value;
                ui.request_repaint();
            }
        }
    }
    ui.c.fill_rrect(
        Rect::new(r.right() - 3.0, r.y + (r.h - h) * *scroll / max, 2.0, h),
        1.0,
        Color::white(0.3),
    );
}
fn context(m: &Model) -> String {
    if m.context == 0 {
        "Not supplied".into()
    } else if m.context >= 1_000_000 {
        format!("{:.2}M tokens", m.context as f64 / 1_000_000.0)
    } else {
        format!("{:.0}K tokens", m.context as f64 / 1024.0)
    }
}
fn modalities(v: &[String]) -> String {
    if v.is_empty() {
        "Not supplied".into()
    } else {
        v.iter()
            .map(|s| match s.as_str() {
                "text" => "Text",
                "image" => "Images",
                "file" => "Files / PDF",
                "audio" => "Audio",
                "video" => "Video",
                v => v,
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
fn capabilities(m: &Model) -> String {
    let mut result = vec![];
    if m.capabilities
        .iter()
        .any(|s| s == "tools" || s == "tool_choice")
    {
        result.push("Tool use");
    }
    if m.capabilities
        .iter()
        .any(|s| s == "structured_outputs" || s == "response_format")
    {
        result.push("Structured replies");
    }
    if m.outputs.iter().any(|s| s == "image") {
        result.push("Image generation");
    }
    if m.inputs.iter().any(|s| s == "image") {
        result.push("Image understanding");
    }
    if m.reasoning.mandatory
        || !m.reasoning.levels.is_empty()
        || m.capabilities.iter().any(|s| s == "reasoning")
    {
        result.push("Reasoning");
    }
    if result.is_empty() {
        "No additional capabilities supplied".into()
    } else {
        result.join(" · ")
    }
}
fn reasoning(m: &Model) -> String {
    if m.reasoning.adjustable() {
        format!(
            "{} effort levels · {}",
            m.reasoning.levels.len(),
            m.reasoning
                .levels
                .iter()
                .map(|s| ai_reasoning::label(s))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else if m.reasoning.mandatory || m.capabilities.iter().any(|s| s == "reasoning") {
        "Model-managed reasoning · no effort slider".into()
    } else {
        "Adjustable reasoning not advertised".into()
    }
}
fn price(m: &Model) -> String {
    if m.free == Some(true) {
        return "Free · input and output".into();
    }
    let fmt = |p: Option<f64>| {
        p.map(|p| {
            let p = p * 1e6;
            if p == 0.0 {
                "$0".into()
            } else if p < 0.0001 {
                "<$0.0001".into()
            } else if p < 0.01 {
                format!("${p:.4}")
            } else {
                format!("${p:.2}")
            }
        })
        .unwrap_or_else(|| "Not supplied".into())
    };
    format!(
        "Input {} · output {} per 1M tokens",
        fmt(m.input_price),
        fmt(m.output_price)
    )
}
/// Small visual summary: advertised capabilities, never guessed benchmark or
/// account quota values. Provider identity stays in explicit details only.
fn support_tiles(ui: &mut Ui, m: &Model, r: Rect) {
    let items = [
        (Icon::Image, "Images", m.inputs.iter().any(|s| s == "image")),
        (Icon::File, "Files", m.inputs.iter().any(|s| s == "file")),
        (
            Icon::Star,
            "Reasoning",
            m.reasoning.mandatory
                || m.reasoning.adjustable()
                || m.capabilities.iter().any(|s| s == "reasoning"),
        ),
        (Icon::Sparkles, "Create images", m.kind == Kind::Image),
    ];
    let gap = 6.0;
    let w = (r.w - gap) / 2.0;
    let h = (r.h - gap) / 2.0;
    for (i, (icon, label, on)) in items.into_iter().enumerate() {
        let tile = Rect::new(
            r.x + (i % 2) as f32 * (w + gap),
            r.y + (i / 2) as f32 * (h + gap),
            w,
            h,
        );
        ui.c.fill_rrect(
            tile,
            8.0,
            if on {
                BLUE.mul_alpha(0.10)
            } else {
                Color::white(0.035)
            },
        );
        let col = if on { BLUE } else { palette::TEXT_FAINT };
        ui.c.icon(
            icon,
            Rect::new(tile.x + 9.0, tile.center().y - 7.0, 14.0, 14.0),
            col,
        );
        text(
            ui,
            label,
            Rect::new(tile.x + 30.0, tile.y, tile.w - 49.0, tile.h),
            11.0,
            if on { palette::TEXT } else { palette::TEXT_DIM },
        );
        ui.c.icon(
            if on { Icon::Check } else { Icon::Minus },
            Rect::new(tile.right() - 19.0, tile.center().y - 5.0, 10.0, 10.0),
            col,
        );
    }
}
fn preview_card(ui: &mut Ui, m: &Model, r: Rect, row: Rect) {
    let above = row.y - r.y - 6.0;
    let below = r.bottom() - row.bottom() - 6.0;
    let available = above.max(below);
    if available < 80.0 {
        return;
    }
    let h = available.min(224.0);
    let w = (r.w - 16.0).min(344.0);
    let card = Rect::new(
        r.right() - w - 8.0,
        if above > below {
            row.y - h - 5.0
        } else {
            row.bottom() + 5.0
        },
        w,
        h,
    );
    // Do not consume unrelated row hits: the transient, pointer-passive preview
    // never becomes a hover target and cannot steal the info button's click.
    ui.hits.retain(|hit| {
        hit.id == wid("ai.model-surface")
            || hit.rect.right() <= card.x
            || hit.rect.x >= card.right()
            || hit.rect.bottom() <= card.y
            || hit.rect.y >= card.bottom()
    });
    surface(ui, card);
    ui.c.clip(card.inset(10.0, 6.0));
    if h >= 170.0 {
        text(
            ui,
            m.display_name(),
            Rect::new(card.x + 12.0, card.y + 8.0, card.w - 24.0, 24.0),
            14.0,
            palette::TEXT,
        );
        text(
            ui,
            &format!(
                "{} · {}",
                m.kind.name(),
                if m.unavailable.is_some() {
                    "Unavailable"
                } else {
                    m.cost_group()
                }
            ),
            Rect::new(card.x + 12.0, card.y + 33.0, card.w - 24.0, 19.0),
            11.0,
            if m.unavailable.is_some() {
                palette::ORANGE
            } else {
                palette::TEXT_DIM
            },
        );
        support_tiles(
            ui,
            m,
            Rect::new(card.x + 12.0, card.y + 61.0, card.w - 24.0, 68.0),
        );
        text(
            ui,
            &format!("Context  {}", context(m)),
            Rect::new(card.x + 12.0, card.y + 139.0, card.w - 24.0, 20.0),
            12.0,
            palette::TEXT,
        );
        if h >= 205.0 {
            text(
                ui,
                &m.unavailable.clone().unwrap_or_else(|| price(m)),
                Rect::new(card.x + 12.0, card.y + 165.0, card.w - 24.0, 20.0),
                11.0,
                palette::TEXT_DIM,
            );
            text(
                ui,
                "✓ Advertised support   − Not advertised",
                Rect::new(card.x + 12.0, card.bottom() - 28.0, card.w - 24.0, 18.0),
                10.0,
                palette::TEXT_FAINT,
            );
        }
    } else {
        support_tiles(
            ui,
            m,
            Rect::new(
                card.x + 10.0,
                card.y + 8.0,
                card.w - 20.0,
                (h - 34.0).min(72.0),
            ),
        );
        text(
            ui,
            &format!("{} · {}", context(m), m.cost_group()),
            Rect::new(card.x + 12.0, card.bottom() - 25.0, card.w - 24.0, 19.0),
            11.0,
            palette::TEXT_DIM,
        );
    }
    ui.c.unclip();
}
fn details(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    let Some(m) = st
        .ai
        .models
        .iter()
        .find(|m| Some(&m.id) == st.ai.model_detail.as_ref())
        .cloned()
    else {
        st.ai.model_detail = None;
        ui.request_repaint();
        return;
    };
    let back = ui.button(
        wid("ai.model-detail-back"),
        Rect::new(r.x + 8.0, r.y + 7.0, 90.0, 28.0),
    );
    text(
        ui,
        "‹ Models",
        Rect::new(r.x + 14.0, r.y + 7.0, 90.0, 28.0),
        12.0,
        palette::TEXT_DIM,
    );
    if back.clicked {
        st.ai.model_detail = None;
        ui.request_repaint();
        return;
    }
    if close(ui, r) {
        st.ai.close_models();
        ui.input.focused = if settings {
            None
        } else {
            Some(wid("ai.prompt"))
        };
        ui.input.caret = st.ai.draft.chars().count();
        ui.request_repaint();
        return;
    }
    let footer = Rect::new(r.x + 12.0, r.bottom() - 41.0, r.w - 24.0, 30.0);
    let view = Rect::new(
        r.x + 12.0,
        r.y + 42.0,
        r.w - 24.0,
        (footer.y - r.y - 49.0).max(24.0),
    );
    let mut y = view.y - st.ai.model_detail_scroll;
    let x = view.x + 4.0;
    let w = view.w - 16.0;
    let start = y;
    ui.c.clip(view);
    let title = TextStyle::new(18.0).weight(Weight::Semibold).top().wrap();
    let th = ui.c.measure(m.display_name(), &title, w).1.max(25.0);
    ui.c.text(m.display_name(), Rect::new(x, y, w, th), &title);
    y += th + 6.0;
    text(
        ui,
        &format!("{} · {}", m.kind.name(), m.cost_group()),
        Rect::new(x, y, w, 21.0),
        12.0,
        palette::TEXT_DIM,
    );
    y += 33.0;
    if let Some(reason) = st.ai.model_unavailable(&st.settings.ai, &m.id) {
        let style = TextStyle::new(12.0).top().wrap().color(palette::ORANGE);
        let h = ui.c.measure(&reason, &style, w - 44.0).1.max(20.0) + 20.0;
        ui.c.fill_rrect(Rect::new(x, y, w, h), 10.0, palette::ORANGE.mul_alpha(0.10));
        ui.c.icon(
            Icon::Minus,
            Rect::new(x + 12.0, y + 12.0, 14.0, 14.0),
            palette::ORANGE,
        );
        ui.c.text(
            &reason,
            Rect::new(x + 34.0, y + 10.0, w - 44.0, h - 20.0),
            &style,
        );
        y += h + 18.0;
    }
    text(
        ui,
        "Works with",
        Rect::new(x, y, w, 22.0),
        13.0,
        palette::TEXT,
    );
    y += 29.0;
    support_tiles(ui, &m, Rect::new(x, y, w, 84.0));
    y += 92.0;
    text(
        ui,
        "✓ Advertised support   − Not advertised",
        Rect::new(x, y, w, 19.0),
        10.0,
        palette::TEXT_FAINT,
    );
    y += 33.0;
    // A pair of large, scannable values precedes the longer explanation.
    let col = (w - 10.0) / 2.0;
    for (index, label, value) in [
        (0, "Context window", context(&m)),
        (
            1,
            "Reasoning",
            if m.reasoning.adjustable() {
                "Adjustable".into()
            } else if m.reasoning.mandatory {
                "Automatic".into()
            } else {
                "Not advertised".into()
            },
        ),
    ] {
        let tile = Rect::new(x + index as f32 * (col + 10.0), y, col, 65.0);
        ui.c.fill_rrect(tile, 10.0, Color::white(0.045));
        text(
            ui,
            label,
            Rect::new(tile.x + 12.0, tile.y + 7.0, tile.w - 24.0, 19.0),
            11.0,
            palette::TEXT_DIM,
        );
        text(
            ui,
            &value,
            Rect::new(tile.x + 12.0, tile.y + 29.0, tile.w - 24.0, 26.0),
            14.0,
            palette::TEXT,
        );
    }
    y += 83.0;
    for (heading, value) in [
        ("Price per million tokens", price(&m)),
        ("Produces", modalities(&m.outputs)),
        ("Options", capabilities(&m)),
        ("Effort levels", reasoning(&m)),
        ("About", m.description.clone()),
        ("Provider", m.provider_name().into()),
        ("Model ID", m.id.clone()),
    ] {
        text(
            ui,
            heading,
            Rect::new(x, y, w, 20.0),
            11.0,
            palette::TEXT_DIM,
        );
        y += 24.0;
        let style = TextStyle::new(13.0).top().wrap().line_spacing(1.4);
        let h = ui.c.measure(&value, &style, w).1.max(19.0);
        ui.c.text(&value, Rect::new(x, y, w, h), &style);
        y += h + 23.0;
    }
    ui.c.unclip();
    let total = y - start;
    let before = st.ai.model_detail_scroll;
    st.ai.model_detail_scroll = (st.ai.model_detail_scroll - ui.wheel_in(view) * 34.0)
        .clamp(0.0, (total - view.h).max(0.0));
    if (before - st.ai.model_detail_scroll).abs() > 0.1 {
        ui.request_repaint();
    }
    scrollbar(
        ui,
        wid("ai.model-detail-scroll"),
        view,
        total,
        &mut st.ai.model_detail_scroll,
    );
    if st.ai.model_unavailable(&st.settings.ai, &m.id).is_some() {
        ui.c.fill_rrect(footer, 15.0, Color::white(0.04));
        text(
            ui,
            "Unavailable · choose another model",
            footer,
            12.0,
            palette::TEXT_FAINT,
        );
    } else if ui
        .pill_button(
            wid("ai.model-detail-select"),
            footer,
            if settings {
                "Set as default model"
            } else {
                "Use this model"
            },
            Some(Icon::Check),
            Color::white(0.10),
            palette::TEXT,
        )
        .clicked
    {
        selected(ui, st, m.id, settings);
    }
}
