//! Input-selected compact readouts. Readers are opened only for a visible
//! Performance/Network view and reuse the System service's rest guards.
use crate::{gfx::TextStyle, icons::Icon, island::Geom, state::AppState, ui::Ui};
use lumo_core::{
    geom::{Color, Rect},
    settings::{ClockColor, ClockFont, ClosedView},
    sysmon::{fmt_pct, fmt_rate},
};

pub fn clock_style(st: &AppState, size: f32) -> TextStyle {
    let mut style = TextStyle::new(size)
        .weight(crate::gfx::Weight::Semibold)
        .tabular();
    style.family = Some(match st.settings.clock_font {
        ClockFont::System => "Segoe UI",
        ClockFont::Rounded => "Trebuchet MS",
        ClockFont::Mono => "Consolas",
        ClockFont::Serif => "Georgia",
    });
    style.color = match st.settings.clock_color {
        ClockColor::White => Color::white(1.0),
        ClockColor::WindowsAccent => st.accent,
        ClockColor::Blue => Color::hex(0x8EC5FF),
        ClockColor::Mint => Color::hex(0x8DE0C1),
        ClockColor::Rose => Color::hex(0xFFB2CD),
        ClockColor::Gold => Color::hex(0xF6D68A),
    };
    style
}
/// Clock baseline; compact readout values add one DIP for legibility.
pub fn clock_size(st: &AppState) -> f32 {
    if crate::pill::on(st.settings.theme)
        && st.settings.idle_style != lumo_core::settings::IdleStyle::Status
    {
        15.0
    } else {
        13.0
    }
}
pub fn geom(view: ClosedView) -> Geom {
    Geom::new(
        if view == ClosedView::Performance {
            280.0
        } else {
            264.0
        },
        if view == ClosedView::Performance {
            36.0
        } else {
            32.0
        },
        if view == ClosedView::Performance {
            17.0
        } else {
            15.0
        },
    )
}
const BLUE: Color = Color::hex(0x8EC5FF);
const MINT: Color = Color::hex(0x8DE0C1);
const LILAC: Color = Color::hex(0xC8B4FF);
pub fn draw(ui: &mut Ui, st: &AppState, view: ClosedView, body: Rect) {
    use crate::gfx::Weight;
    let reading = st.system.latest.as_ref();
    let value = TextStyle::new(clock_size(st) + 1.0)
        .weight(Weight::Semibold)
        .tabular();
    if view == ClosedView::Performance {
        let items = [
            (
                Icon::Cpu,
                reading.and_then(|r| r.cpu.as_ref()).map(|c| c.load),
                BLUE,
            ),
            (
                Icon::Gpu,
                reading.and_then(|r| r.gpu.as_ref()).map(|g| g.util),
                LILAC,
            ),
            (
                Icon::Memory,
                reading
                    .and_then(|r| r.memory.as_ref())
                    .map(|m| m.used_frac()),
                MINT,
            ),
        ];
        let w = (body.w - 28.0 - 24.0) / 3.0;
        for (i, (icon, load, ink)) in items.into_iter().enumerate() {
            let load = load.filter(|f| f.is_finite());
            let x = body.x + 14.0 + i as f32 * (w + 12.0);
            ui.c.icon(
                icon,
                Rect::new(x, body.y + 7.0, 16.0, 16.0),
                ink.mul_alpha(if load.is_some() { 0.85 } else { 0.35 }),
            );
            ui.c.text(
                &load.map_or_else(|| "—".into(), fmt_pct),
                Rect::new(x + 23.0, body.y + 3.0, w - 23.0, 24.0),
                &value.right().color(if load.is_some() {
                    Color::white(0.96)
                } else {
                    Color::white(0.35)
                }),
            );
            let bar = Rect::new(x, body.y + 29.0, w, 2.0);
            ui.c.fill_rrect(bar, 1.0, Color::white(0.12));
            if let Some(load) = load.filter(|f| f.is_finite() && *f > 0.0) {
                ui.c.fill_rrect(
                    Rect::new(bar.x, bar.y, (bar.w * load.clamp(0.0, 1.0)).max(3.0), bar.h),
                    1.0,
                    ink,
                );
            }
        }
    } else if view == ClosedView::Network {
        let net = reading
            .and_then(|r| r.network.as_ref())
            .filter(|n| n.online);
        let w = (body.w - 48.0) / 2.0;
        for (i, (icon, rate, ink)) in [
            (Icon::Download, net.map(|n| n.down_bps), MINT),
            (Icon::Upload, net.map(|n| n.up_bps), BLUE),
        ]
        .into_iter()
        .enumerate()
        {
            let rate = rate.filter(|r| r.is_finite());
            let x = body.x + 14.0 + i as f32 * (w + 20.0);
            ui.c.icon(
                icon,
                Rect::new(x, body.center().y - 8.0, 16.0, 16.0),
                ink.mul_alpha(if rate.is_some() { 0.85 } else { 0.35 }),
            );
            ui.c.text(
                &rate.map_or_else(|| "—".into(), fmt_rate),
                Rect::new(x + 23.0, body.y, w - 23.0, body.h),
                &value.color(if rate.is_some() {
                    Color::white(0.96)
                } else {
                    Color::white(0.35)
                }),
            );
        }
        ui.c.fill_rect(
            Rect::new(body.center().x, body.y + 8.0, 1.0, body.h - 16.0),
            Color::white(0.12),
        );
    }
}
