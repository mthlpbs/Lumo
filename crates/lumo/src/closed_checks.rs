//! Native checks of the visible compact readouts and their actual sampler.
use super::*;
use lumo_core::settings::{ClockColor, ClockFont, ClosedView};
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}
fn fixture() -> (App, HWND, HWND) {
    window::register_classes(proc, proc).unwrap();
    let hwnd = window::create_island_window().unwrap();
    let helper = window::create_helper_window().unwrap();
    let (tx, rx) = mpsc::channel();
    let sink = EventSink::new(tx, SendHwnd::new(hwnd));
    let mut app = App::new(
        hwnd,
        helper,
        sink,
        rx,
        Settings::default(),
        Store::default(),
        Some("unused".into()),
        false,
        false,
    )
    .unwrap();
    app.settings_writable = false;
    app.st.settings.hide_over_maximized = false;
    app.st.media = None;
    app.st.hud = None;
    app.st.notice = None;
    app.expanded = false;
    (app, hwnd, helper)
}
fn finish(mut app: App, hwnd: HWND, helper: HWND) {
    app.sysmon = None;
    app.clock.set_active(false);
    app.gfx.set_shown(false);
    clock::hold(None);
    unsafe {
        DestroyWindow(hwnd).unwrap();
        DestroyWindow(helper).unwrap();
    }
}
fn example() -> lumo_core::sysmon::Reading {
    use lumo_core::sysmon::*;
    Reading {
        cpu: Some(CpuReading {
            load: 0.37,
            ..Default::default()
        }),
        gpu: Some(GpuReading {
            util: 0.64,
            temp_c: Some(61.0),
            ..Default::default()
        }),
        memory: Some(MemoryReading {
            used: 8 * 1024 * 1024 * 1024,
            total: 16 * 1024 * 1024 * 1024,
            ..Default::default()
        }),
        network: Some(NetReading {
            online: true,
            down_bps: 1024.0 * 1024.0,
            up_bps: 128.0 * 1024.0,
            ..Default::default()
        }),
        ..Default::default()
    }
}
#[test]
#[ignore = "requires a Windows desktop; offline compact layouts and clock styles"]
fn layouts_render_in_both_themes_and_scroll_has_three_views() {
    let (mut app, hwnd, helper) = fixture();
    app.st.power.saver = true;
    clock::hold(Some(100.0));
    let mut d = vec![];
    let output = std::env::var_os("LUMO_COMPACT_QA").map(std::path::PathBuf::from);
    if let Some(p) = &output {
        std::fs::create_dir_all(p).unwrap();
    }
    for theme in [Theme::Classic, Theme::Pill] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            app.st.settings.theme = theme;
            app.gfx.set_dpi(scale * 96.0);
            app.island
                .set_floating(pill::on(theme).then_some(pill::TOP_GAP));
            for view in [ClosedView::Performance, ClosedView::Network] {
                app.st.closed_view = view;
                app.mode = Mode::Readout(view);
                app.layout_surface();
                app.island.snap(app.geom_for(&app.mode));
                app.st.system.latest = Some(example());
                // Renderer creation needs one settling paint before it parks.
                app.selftest_capture();
                let (w, h, p, _) = app.selftest_capture();
                assert!(
                    !app.clock.pacing().0,
                    "static metrics must not run ambient frames"
                );
                assert!(
                    app.sysmon.is_none() && app.frames.is_none(),
                    "offline/saver render must not open metric or FPS readers"
                );
                if scale == 1.25 {
                    if let Some(dir) = &output {
                        crate::png::save_rgba(
                            dir.join(format!("compact-{view:?}-{theme:?}.png")),
                            w as u32,
                            h as u32,
                            &crate::png::pbgra_to_rgba(&p),
                        )
                        .unwrap();
                    }
                }
            }
        }
        for font in [
            ClockFont::System,
            ClockFont::Rounded,
            ClockFont::Mono,
            ClockFont::Serif,
        ] {
            for color in [
                ClockColor::White,
                ClockColor::WindowsAccent,
                ClockColor::Blue,
                ClockColor::Mint,
                ClockColor::Rose,
                ClockColor::Gold,
            ] {
                for h24 in [false, true] {
                    app.st.settings.clock_font = font;
                    app.st.settings.clock_color = color;
                    app.st.settings.use_24h = h24;
                    app.st.closed_view = ClosedView::Time;
                    app.mode = Mode::Idle;
                    app.layout_surface();
                    app.island.snap(app.geom_for(&app.mode));
                    app.selftest_capture();
                }
            }
        }
        app.st.closed_view = ClosedView::Time;
        app.mode = Mode::Idle;
        app.layout_surface();
        app.island.snap(app.geom_for(&app.mode));
        app.wheel_pager = Default::default();
        for (i, expected) in [
            ClosedView::Performance,
            ClosedView::Network,
            ClosedView::Time,
        ]
        .into_iter()
        .enumerate()
        {
            clock::hold(Some(200.0 + i as f64));
            let p = app.island.body_rect_at(app.cx).center();
            let s = app.gfx.scale();
            let x = app.surface.x + (p.x * s) as i32;
            let y = app.surface.y + (p.y * s) as i32;
            app.handle_island(
                WM_MOUSEWHEEL,
                WPARAM(((-120i16) as u16 as usize) << 16),
                LPARAM((((y as u16 as usize) << 16) | (x as u16 as usize)) as isize),
                &mut d,
            );
            assert_eq!(app.st.closed_view, expected);
        }
    }
    finish(app, hwnd, helper);
}
#[test]
#[ignore = "requires Windows counters; verifies real compact updates and no sampling on Time"]
fn real_metrics_update_and_release_the_sampler_when_hidden() {
    use std::time::{Duration, Instant};
    let (mut app, hwnd, helper) = fixture();
    let mut d = vec![];
    // Keep all general services off, but allow this test's on-demand reader.
    app.selftest = None;
    app.st.power.saver = false;
    app.st.settings.system_history = true;
    app.st.closed_view = ClosedView::Performance;
    app.mode = Mode::Readout(ClosedView::Performance);
    app.layout_surface();
    app.island.snap(app.geom_for(&app.mode));
    app.frame(&mut d, None);
    assert!(
        app.sysmon_compact && !app.sysmon_on,
        "compact visibility must survive the complete frame without enabling full page sampling"
    );
    let wait = |app: &mut App, d: &mut Vec<Deferred>| {
        let end = Instant::now() + Duration::from_secs(10);
        let mut received = 0;
        while received < 2 {
            assert!(Instant::now() < end, "two live readings must arrive; received={received}, compact={}, full={}, mode={:?}, target={:?}, worker={:?}",app.sysmon_compact,app.sysmon_on,app.mode,app.target_mode(),app.sysmon.as_ref().map(|s|s.test_state()));
            while let Ok(ev) = app.rx.try_recv() {
                if matches!(&ev, AppEvent::System(_)) {
                    received += 1;
                }
                app.on_event(ev);
            }
            app.frame(d, None);
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    wait(&mut app, &mut d);
    assert!(app.st.system.latest.as_ref().unwrap().memory.is_some());
    let live = app.sysmon.as_ref().unwrap().test_state();
    assert!(
        live.0 >= 2 && live.1 && !live.2,
        "compact readings must use light sampling"
    );
    assert!(
        app.frames.is_none(),
        "compact metrics must never create an FPS session"
    );
    app.st.closed_view = ClosedView::Time;
    app.frame(&mut d, None);
    let asleep = app.sysmon.as_ref().unwrap().test_state();
    assert!(!app.sysmon_compact && !asleep.1);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        app.sysmon.as_ref().unwrap().test_state().0,
        asleep.0,
        "Clock must cause zero additional samples even with the legacy history flag on"
    );
    let mut old = example();
    old.cpu.as_mut().unwrap().load = 0.99;
    app.on_event(AppEvent::System(Box::new({
        let mut m = lumo_core::sysmon::Monitor::default();
        m.push(old);
        m
    })));
    assert!(
        app.st.system.latest.is_none(),
        "queued readings after closing are discarded"
    );
    app.st.closed_view = ClosedView::Network;
    app.mode = Mode::Readout(ClosedView::Network);
    app.island.snap(app.geom_for(&app.mode));
    app.frame(&mut d, None);
    wait(&mut app, &mut d);
    assert!(app.st.system.latest.as_ref().unwrap().network.is_some());
    for (locked, off, saver) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        app.locked = locked;
        app.display_off = off;
        app.st.power.saver = saver;
        app.frame(&mut d, None);
        assert!(
            !app.sysmon.as_ref().unwrap().test_state().1,
            "rest must drop all sampler devices"
        );
        app.locked = false;
        app.display_off = false;
        app.st.power.saver = false;
        app.frame(&mut d, None);
    }
    app.expanded = true;
    app.st.page = PageKind::Media;
    app.mode = Mode::Expanded(PageKind::Media);
    app.frame(&mut d, None);
    assert!(
        !app.sysmon.as_ref().unwrap().test_state().1,
        "opening another page sleeps compact metrics"
    );
    finish(app, hwnd, helper);
}
