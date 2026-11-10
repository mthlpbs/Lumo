
use crate::activities::{HudTag, Mode};
use crate::app::App;
use crate::clock;
use crate::gfx::{Image, ImageData};
use crate::icons::Icon;
use crate::pages::{self, PageKind};
use crate::paths;
use crate::png;
use crate::state::{
    AppState, FpsAccess, GameState, Hud, HudKind, LyricsState, LyricsStatus, MediaSession, Notice, NoticeAction, 
    PhoneAsk, PairedPhone, PhoneDrop, PhoneTransfer, PhoneView, PowerState, VolumeState,
};
use crate::ui::wid;
use lumo_core::addons::{Status as AddonStatus, CLIPBOARD_HISTORY, SCRIPT_READER};
use lumo_core::geom::{palette, Color, Point, Rect};
use lumo_core::lrc::{parse_lrc, Lyrics};
use lumo_core::phonedrop::QrMatrix;
use lumo_core::pomodoro::{Phase as PomoPhase, RunState};
use lumo_core::power::{PowerReading, PsuReading, Rail};
use lumo_core::settings::{IdleStyle, Theme};
use lumo_core::shelf::ShelfItem;
use lumo_core::frames::FrameRate;
use lumo_core::sysmon::{
    AppUse, BatteryReading, CoreKind, CoreLoad, CpuReading, DiskActivity, Drive, GpuReading, MemoryReading, Module, Monitor, NetReading, Reading,
    Series, HISTORY_LEN,
};
use lumo_core::timeline::Timeline;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, IsWindowVisible, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

// ---------------------------------------------------------------- timing

/// A scene must stay settled for this many consecutive steps before capture.
const STABLE_STEPS: u32 = 2;
/// Minimum steps per scene (lets the content cross-fade start and finish).
const MIN_STEPS: u32 = 3;
/// Seconds a scene may take to settle before it counts as a failure.
const SETTLE_TIMEOUT: f64 = 3.0;
/// The same on a software rasterizer (CI runners have no GPU). A fresh
/// virtual machine can take longer to bring a scene to rest: the first open
/// page with Japanese lyrics took 3.1-4.2 s there. Still short enough to
/// catch an island that never settles.
const SOFT_GPU_SETTLE_TIMEOUT: f64 = 8.0;
/// Seconds to wait for a simulated tab click to switch the page.
const CLICK_TIMEOUT: f64 = 1.5;
/// Stop visiting scenes after this many seconds. The CI and Store test steps
/// wait 120 s for the whole run; this leaves room for PNG writers and optional
/// animation recordings. The Wine runner allows 180 s.
const SCENE_DEADLINE: f64 = 65.0;
/// Allow PNG writers to finish after the last scene.
const WRITE_DEADLINE: f64 = SCENE_DEADLINE + 3.5;
/// One frame (layout + Direct2D render + present + readback) must fit in a 60 Hz frame.
/// On a software rasterizer (CI runners) frame times are reported, not
/// counted: they land on whole screen refreshes (about 16 or 31 ms) on a CPU
/// shared with other machines, which says little about a real PC.
const FRAME_BUDGET_MS: f64 = 16.0;
/// Frames re-rendered when the first sample misses the budget (filters scheduler noise).
const FRAME_SAMPLES: usize = 3;
// Eye-blink timing of the idle "eyes" style (activities.rs). Only used to
// avoid capturing a mid-blink screenshot; harmless if it drifts.
const BLINK_PERIOD: f64 = 5.5;
const BLINK_GUARD: (f64, f64) = (0.3, 0.3);

// ---------------------------------------------------------------- fake data (all original)

const TITLE: &str = "Paper Boats";
const ARTIST: &str = "The Lanterns";
const ALBUM: &str = "Harbor Lights";
const TRACK_LEN_S: f64 = 201.0;
const TRACK_POS_S: f64 = 64.0;
/// Pomodoro time left in the running focus session (18:24).
const TIMER_LEFT_S: f64 = 1104.5;
const ART_PX: u32 = 240;
/// The battery in the low-battery scene (Windows warns at 20 %).
const LOW_BATTERY: u8 = 19;

/// Original placeholder lyrics written for the self-test (not from any real song).
const LYRICS_LRC: &str = "\
[ti:Paper Boats]
[ar:The Lanterns]
[00:38.00]Folded up the morning into something small
[00:43.50]Set it on the water by the harbor wall
[00:49.00]Ink along the edges of a name I used to know
[00:54.50]Watched it find the current, watched it go
[01:00.00]Paper boats, carry what I couldn't say
[01:05.50]Over the grey and out across the bay
[01:11.00]If the tide is gentle they will find the light
[01:16.50]Lanterns on the pier keep burning through the night
[01:22.00]Every little sail a letter never sent
[01:27.50]Every little ripple something someone meant
[01:33.00]So I fold another and I let it drift away
[01:38.50]Paper boats, I'll meet you there someday
";

/// The same song in Japanese, Korean and Chinese (written for the test, same
/// timings), for the pronunciation lines (LUMO_SELFTEST_LYRICS=ja|ko|zh).
const LYRICS_JA: &str = "\
[00:38.00]朝をたたんで小さな形にした
[00:43.50]港の壁のそばで水に浮かべた
[00:49.00]端ににじんだインク、昔の名前
[00:54.50]流れを見つけて、遠くへ行った
[01:00.00]紙の舟よ、言えなかった言葉を運んで
[01:05.50]灰色の海をこえて湾の向こうへ
[01:11.00]潮がやさしければ光を見つける
[01:16.50]桟橋のランタンは夜通し灯る
[01:22.00]小さな帆はどれも出せない手紙
[01:27.50]小さな波はどれも誰かの想い
[01:33.00]だから また ひとつ おって ながすよ
[01:38.50]紙の舟よ、いつかそこで会おう OK
";

const LYRICS_KO: &str = "\
[00:38.00]아침을 접어서 작게 만들었어
[00:43.50]항구 벽 옆에서 물 위에 띄웠어
[00:49.00]가장자리에 번진 잉크, 옛 이름
[00:54.50]물길을 찾아서 멀리 떠나갔어
[01:00.00]종이배야, 못 한 말을 실어 가 줘
[01:05.50]회색 바다를 건너 만 너머로
[01:11.00]물결이 다정하면 빛을 찾겠지
[01:16.50]부두의 등불은 밤새 타올라
[01:22.00]작은 돛마다 부치지 못한 편지
[01:27.50]작은 물결마다 누군가의 마음
[01:33.00]그래서 또 하나 접어 흘려보내
[01:38.50]종이배야, 언젠가 거기서 만나 Oh
";

const LYRICS_ZH: &str = "\
[00:38.00]把清晨折成小小的形状
[00:43.50]放在港口的墙边水面上
[00:49.00]边缘晕开的墨水，旧日的名字
[00:54.50]看它找到水流，看它远去
[01:00.00]纸船啊，带走我说不出的话
[01:05.50]越过灰色的海，驶向海湾那边
[01:11.00]如果潮水温柔，它们会找到光
[01:16.50]码头的灯笼彻夜燃烧
[01:22.00]每一片小帆都是没寄出的信
[01:27.50]每一道涟漪都是谁的心意
[01:33.00]所以我再折一只，让它漂走
[01:38.50]纸船啊，总有一天我们在那里相见
";

const NOTES_TXT: &str = "Weekend trip\n\n- ferry leaves at 9:40\n- pack the blue jacket\n- ask about the lighthouse tour\n";

const CLIPS: [&str; 6] = [
    "Lunch: noodles from the corner place, then a walk by the river",
    "Tracking number PX-2048-7731",
    "cargo build -p lumo --release",
    "#5AC8FA",
    "https://example.com/projects/lumo/notes",
    "Meet at the north entrance at 6:15",
];

const GIB: u64 = 1024 * 1024 * 1024;

/// The phone-drop link in the screenshots. Made up: no server answers it.
const PHONE_URL: &str = "HTTP://192.168.1.20:53530/K7Q2M9XWZ4R8T6YB3NPV5HJC2D/";

/// The pairing link in the screenshots: the contract's test secret, so no
/// real pair can use it.
const PAIR_URL: &str = "https://drop.oyenza.com/p#k=AAECAwQFBgcICQoLDA0ODw";

/// Paired phones as the pairing scenes show them (made up).
fn fake_paired(scene: Scene, unix: i64) -> Vec<PairedPhone> {
    match scene {
        Scene::ShelfPaired | Scene::ShelfPairedPhones | Scene::PairedNotice => vec![
            PairedPhone { room: "geDM0jEhk4eiOz_li-FtORW2YlPBbebr7NsHjLkHI_4".into(), name: "Pixel 10".into(), seen: unix - 5 * 60 },
            PairedPhone { room: "ZmFrZS1yb29tLWZvci10aGUtc2VsZi10ZXN0LWlwaG9u".into(), name: "iPhone".into(), seen: unix - 2 * 86_400 - 3600 },
        ],
        _ => Vec::new(),
    }
}

/// Phone drop as each phone scene shows it (None = closed).
fn fake_phone(scene: Scene, now: f64) -> Option<PhoneDrop> {
    let ready = PhoneDrop { url: PHONE_URL.into(), qr: QrMatrix::encode(PHONE_URL), ..Default::default() };
    let files = vec![("IMG_2041.HEIC".to_string(), 2_480_112u64), ("IMG_2042.HEIC".to_string(), 2_391_870), ("Harbor walk.mov".to_string(), 18_402_331)];
    let total = files.iter().map(|f| f.1).sum();
    match scene {
        Scene::ShelfPhone => Some(ready),
        Scene::ShelfPhoneAsk => Some(PhoneDrop {
            visitor: Some("iPhone".into()),
            ask: Some(PhoneAsk { id: 7, from: "iPhone".into(), files, total }),
            ..ready
        }),
        Scene::ShelfPhoneReceive => Some(PhoneDrop {
            visitor: Some("iPhone".into()),
            transfer: Some(PhoneTransfer { id: 7, from: "iPhone".into(), file: 2, files: 3, done: 9_100_000, total }),
            ..ready
        }),
        Scene::ShelfPairing => Some(PhoneDrop { view: PhoneView::Pairing { url: PAIR_URL.into(), qr: QrMatrix::encode(PAIR_URL), until: now + 580.0 }, ..ready }),
        Scene::ShelfPaired => Some(PhoneDrop {
            outcome: Some(("Pixel 10 is paired".into(), true, now + 60.0)),
            outcome_line: Some("It can now send from any network, even with this panel closed.".into()),
            ..ready
        }),
        Scene::ShelfPairedPhones => Some(PhoneDrop { view: PhoneView::Paired, ..ready }),
        _ => None,
    }
}

/// Paths of the sample apps (keys for their icons only; never opened) and their tile colours.
const SAMPLE_ICONS: [(&str, u32); 8] = [
    ("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", 0x1A9FD9),
    ("C:\\Users\\Sample\\AppData\\Roaming\\Spotify\\Spotify.exe", 0x1DB954),
    ("C:\\Windows\\explorer.exe", 0xF2C14E),
    ("C:\\Users\\Sample\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe", 0x2F80ED),
    ("C:\\Windows\\System32\\dwm.exe", 0x00A4EF),
    ("C:\\Users\\Sample\\AppData\\Local\\Discord\\app-1.0.9200\\Discord.exe", 0x5865F2),
    ("C:\\ProgramData\\Microsoft\\Windows Defender\\Platform\\4.18.25010.11-0\\MsMpEng.exe", 0x0078D4),
    ("C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE", 0x2B579A),
];

/// A plain rounded tile standing in for an app icon, laid out like the
/// shell's icon pictures (the tile fills the middle 72 %).
fn sample_icon(rgb: u32) -> Image {
    const PX: u32 = 48;
    let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    let inset = PX as f32 * 0.14;
    let (lo, hi, radius) = (inset, PX as f32 - inset, PX as f32 * 0.16);
    let mut px = vec![0u8; (PX * PX * 4) as usize];
    for y in 0..PX {
        for x in 0..PX {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Distance outside the rounded square, smoothed over one pixel.
            let dx = (lo + radius - fx).max(fx - (hi - radius)).max(0.0);
            let dy = (lo + radius - fy).max(fy - (hi - radius)).max(0.0);
            let a = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let i = ((y * PX + x) * 4) as usize;
            let k = |c: u8| (f32::from(c) * a).round() as u8;
            px[i..i + 4].copy_from_slice(&[k(b), k(g), k(r), (a * 255.0).round() as u8]);
        }
    }
    ImageData::new(PX, PX, px)
}

fn sample_app(i: usize, name: &str, cpu: f32, memory: u64, io: f64) -> AppUse {
    let path = SAMPLE_ICONS[i].0;
    let image = path.rsplit('\\').next().unwrap_or(path).to_lowercase();
    AppUse { image, name: name.into(), path: Some(path.into()), pid: 1000 + i as u64, cpu, memory, io, processes: 1 }
}

/// The kernel's own entry: no file, so the page shows Windows' mark.
fn sample_system(cpu: f32, memory: u64, io: f64) -> AppUse {
    AppUse { image: "system".into(), name: "System".into(), path: None, pid: 4, cpu, memory, io, processes: 1 }
}

/// The System page's sample minute (all made up): a laptop with a hybrid
/// CPU, or a desktop without a battery (with its power: from the power
/// supply, or estimated); nothing yet when the page has just opened.
fn fake_system(scene: Scene) -> Monitor {
    let mut m = Monitor::default();
    if scene == Scene::SystemWaiting {
        return m;
    }
    const MIB: f64 = 1024.0 * 1024.0;
    for i in 0..HISTORY_LEN {
        let t = i as f32;
        let last = i + 1 == HISTORY_LEN;
        let spike = if (38..45).contains(&i) { 0.22 * ((t - 38.0) / 6.0 * std::f32::consts::PI).sin() } else { 0.0 };
        let cpu = if last { 0.23 } else { (0.19 + 0.05 * (t / 4.3).sin() + 0.03 * (t / 1.7 + 1.1).sin() + spike).clamp(0.03, 0.95) };
        let gpu = if last { 0.31 } else { (0.24 + 0.08 * (t / 5.1 + 0.4).sin() + 0.04 * (t / 2.3).sin() + if (18..30).contains(&i) { 0.3 } else { 0.0 }).clamp(0.02, 0.98) };
        let burst = |at: usize, len: usize, peak: f64| if (at..at + len).contains(&i) { peak * (((i - at) as f64 + 0.5) / len as f64 * std::f64::consts::PI).sin() } else { 0.0 };
        let read = if last { 1.2 * MIB } else { 0.15 * MIB + burst(12, 6, 6.5 * MIB) + burst(41, 4, 2.8 * MIB) };
        let write = if last { 0.33 * MIB } else { 0.05 * MIB + burst(20, 9, 1.9 * MIB) };
        let down = if last { 1.2 * MIB } else { 0.08 * MIB + burst(5, 14, 3.4 * MIB) + burst(46, 10, 1.6 * MIB) };
        let up = if last { 0.082 * MIB } else { 0.02 * MIB + burst(25, 8, 0.6 * MIB) };
        let sample = Sample { cpu, gpu, mem: 0.6 + 0.004 * (t / 6.0).sin(), read, write, down, up, t };
        let desktop = matches!(scene, Scene::SystemMemory | Scene::SystemPower | Scene::SystemPowerEstimate);
        let mut r = sample_reading(!desktop, &sample);
        if matches!(scene, Scene::SystemPower | Scene::SystemPowerEstimate) {
            r.power = sample_power(&r, scene == Scene::SystemPower, t);
        }
        m.push(r);
    }
    m
}

/// The desktop's power that second: what the power supply reports (its
/// output a little above the parts' sum, as the board, memory and fans add
/// theirs), or the estimate.
fn sample_power(r: &Reading, psu: bool, t: f32) -> Option<PowerReading> {
    let cpu_w = r.cpu.as_ref().and_then(|c| c.power_w);
    let gpu_w = r.gpu.as_ref().and_then(|g| g.power_w);
    if !psu {
        return PowerReading::estimate(cpu_w, gpu_w, true);
    }
    let out_w = cpu_w? + gpu_w? + 46.0 + 2.0 * (t / 2.9).sin();
    let rail = |nominal: f32, volts: f32, amps: f32| Rail { nominal, volts, amps };
    let reading = PsuReading {
        name: "MSI MEG Ai1600T".into(),
        out_w,
        in_w: Some(out_w / 0.93),
        efficiency: Some(0.93),
        mains_v: Some(229.6),
        temp_c: Some(38.5),
        fan_rpm: Some(0),
        rails: vec![rail(12.0, 12.06, (out_w - 14.0) / 12.06), rail(5.0, 5.02, 1.9), rail(3.3, 3.31, 1.4)],
    };
    Some(PowerReading::from_psu(reading, cpu_w, gpu_w))
}

/// Show over games, for its scenes: a game at about 144 fps (one hitch in
/// the minute), the GPU near full and warm. Before the one-time OK the
/// frame rate is unknown and no game has been counted.
fn fake_game(scene: Scene, system: &mut Monitor) -> GameState {
    if !scene.game() {
        return GameState::default();
    }
    let allowed = matches!(scene, Scene::GameStats | Scene::SystemGames);
    let in_front = scene != Scene::SystemGamesAllow;
    if in_front {
        if let Some(r) = system.latest.as_mut() {
            if let Some(c) = r.cpu.as_mut() {
                c.load = 0.38;
            }
            if let Some(g) = r.gpu.as_mut() {
                g.util = 0.97;
                g.engine_3d = 0.97;
                g.temp_c = Some(71.0);
                g.mem_used = GIB * 63 / 10;
            }
        }
    }
    let mut history = Series::default();
    if allowed {
        for i in 0..HISTORY_LEN {
            let t = i as f32;
            let dip = if (33..36).contains(&i) { 38.0 } else { 0.0 };
            history.push(Some(141.0 + 3.0 * (t / 3.7).sin() + 1.5 * (t / 1.3 + 0.6).sin() - dip));
        }
    }
    GameState {
        active: in_front,
        app: in_front.then(|| "Harbor Racers".to_string()),
        rate: (allowed && in_front).then_some(FrameRate { fps: 144.2, frame_ms: 6.93, worst_ms: 9.4 }),
        history,
        access: if allowed { FpsAccess::Ready } else { FpsAccess::NeedsOk },
    }
}

/// One second of the sample minute.
struct Sample {
    cpu: f32,
    gpu: f32,
    mem: f32,
    read: f64,
    write: f64,
    down: f64,
    up: f64,
    /// Seconds into the minute.
    t: f32,
}

fn sample_reading(laptop: bool, s: &Sample) -> Reading {
    let total = GIB * 16;
    const MIB: u64 = 1024 * 1024;
    // Six performance cores with two threads each, then eight efficiency cores.
    let cores = (0..20)
        .map(|k| {
            let p = k < 12;
            let wave = 0.5 + 0.5 * (s.t / 3.0 + k as f32 * 1.7).sin();
            let load = if p { s.cpu * (0.6 + 1.1 * wave) } else { s.cpu * (0.3 + 0.5 * wave) };
            CoreLoad { load: load.clamp(0.02, 1.0), kind: if p { CoreKind::Performance } else { CoreKind::Efficiency } }
        })
        .collect();
    Reading {
        cpu: Some(CpuReading {
            load: s.cpu,
            user: s.cpu * 0.62,
            kernel: s.cpu * 0.38,
            interrupt: s.cpu * 0.03,
            cores,
            logical: 20,
            base_mhz: Some(2_600),
            mhz: Some(3_400),
            mhz_p: Some(3_900),
            mhz_e: Some(2_800),
            temp_c: laptop.then_some(54.0),
            // Package power follows the load (a laptop's chip is a smaller one).
            power_w: Some(if laptop { 18.5 } else { 34.0 + 165.0 * s.cpu }),
            uptime_s: 3 * 86_400 + 4 * 3600 + 12 * 60,
        }),
        gpu: Some(GpuReading {
            name: "NVIDIA GeForce RTX 4060 Laptop GPU".into(),
            util: s.gpu,
            engine_3d: s.gpu,
            copy: 0.04,
            video_decode: 0.12,
            video_encode: 0.0,
            mem_used: GIB * 21 / 10,
            mem_total: GIB * 8,
            integrated: false,
            temp_c: Some(61.0),
            fan_rpm: Some(2_150),
            mhz: Some(1_890),
            power_w: Some(12.0 + 95.0 * s.gpu),
        }),
        memory: Some(MemoryReading {
            total,
            used: (total as f64 * f64::from(s.mem)) as u64,
            committed: GIB * 121 / 10,
            commit_limit: GIB * 184 / 10,
            cached: GIB * 42 / 10,
            wired: GIB * 9 / 10,
            compressed: GIB * 6 / 10,
            free: GIB * 22 / 10,
            page_used: GIB * 3 / 10,
            page_total: GIB * 24 / 10,
        }),
        drives: vec![
            Drive { root: "C:".into(), label: "Windows".into(), total: GIB * 476, free: GIB * 212, system: true },
            Drive { root: "D:".into(), label: "Data".into(), total: GIB * 931, free: GIB * 640, system: false },
        ],
        disk: Some(DiskActivity { read_bps: s.read, write_bps: s.write, active: Some(0.14) }),
        network: Some(NetReading {
            online: true,
            adapter: "Wi-Fi".into(),
            ipv4: Some("192.168.1.42".into()),
            mac: "3C:22:FB:0A:11:9E".into(),
            link_bps: 866_700_000,
            down_bps: s.down,
            up_bps: s.up,
            received: GIB * 34 / 10,
            sent: 410 * MIB,
        }),
        battery: laptop.then(|| BatteryReading {
            percent: 76,
            secs_left: Some(2 * 3600 + 14 * 60),
            design_mwh: Some(57_000),
            full_mwh: Some(51_870),
            cycles: Some(193),
            rate_mw: Some(-8_200),
            ..BatteryReading::default()
        }),
        power: None,
        top_cpu: vec![
            sample_app(0, "Microsoft Edge", 0.091, GIB * 21 / 10, 1.2e6),
            sample_app(1, "Spotify", 0.042, 412 * MIB, 4.1e5),
            sample_system(0.023, 0, 8.2e5),
            sample_app(4, "Desktop Window Manager", 0.018, 88 * MIB, 0.0),
            sample_app(2, "Windows Explorer", 0.013, 96 * MIB, 4.0e4),
            sample_app(5, "Discord", 0.009, 520 * MIB, 1.2e4),
            sample_app(3, "Visual Studio Code", 0.008, 940 * MIB, 9.6e4),
            sample_app(6, "Antimalware Service Executable", 0.004, 280 * MIB, 3.1e6),
        ],
        top_memory: vec![
            sample_app(0, "Microsoft Edge", 0.091, GIB * 21 / 10, 1.2e6),
            sample_app(3, "Visual Studio Code", 0.008, 940 * MIB, 9.6e4),
            sample_app(5, "Discord", 0.009, 520 * MIB, 1.2e4),
            sample_app(1, "Spotify", 0.042, 412 * MIB, 4.1e5),
            sample_app(6, "Antimalware Service Executable", 0.004, 280 * MIB, 3.1e6),
            sample_app(7, "Microsoft Word", 0.002, 210 * MIB, 3.0e3),
            sample_app(2, "Windows Explorer", 0.013, 96 * MIB, 4.0e4),
            sample_app(4, "Desktop Window Manager", 0.018, 88 * MIB, 0.0),
        ],
        top_io: vec![
            sample_app(6, "Antimalware Service Executable", 0.004, 280 * MIB, 3.1e6),
            sample_app(0, "Microsoft Edge", 0.091, GIB * 21 / 10, 1.2e6),
            sample_system(0.023, 0, 8.2e5),
            sample_app(1, "Spotify", 0.042, 412 * MIB, 4.1e5),
            sample_app(3, "Visual Studio Code", 0.008, 940 * MIB, 9.6e4),
            sample_app(2, "Windows Explorer", 0.013, 96 * MIB, 4.0e4),
            sample_app(5, "Discord", 0.009, 520 * MIB, 1.2e4),
            sample_app(7, "Microsoft Word", 0.002, 210 * MIB, 3.0e3),
        ],
    }
}

/// Judge one reading from the real sources: the core readings must be
/// there; the rest is reported (a build server has no battery or GPU counters).
fn system_readings(p: crate::services::sysmon::Probe) -> (bool, String) {
    use lumo_core::sysmon::fmt_bytes;
    let r = &p.reading;
    let cpu = r.cpu.as_ref().filter(|c| (0.0..=1.0).contains(&c.load) && c.logical > 0);
    let mem = r.memory.as_ref().filter(|m| m.total > 0 && m.used <= m.total);
    let ok = cpu.is_some() && mem.is_some() && !r.drives.is_empty() && !r.top_memory.is_empty();
    let apps: Vec<String> = r.top_memory.iter().take(3).map(|a| format!("{} ({}{})", a.name, fmt_bytes(a.memory), if a.path.is_some() { ", path" } else { "" })).collect();
    let cores = cpu.map_or("missing".into(), |c| match c.hybrid() {
        Some((p, e)) => format!("{} ({p} P + {e} E)", c.cores.len()),
        None if c.cores.is_empty() => "total only".into(),
        None => c.cores.len().to_string(),
    });
    let ms = |d: std::time::Duration| format!("{:.1} ms", d.as_secs_f64() * 1000.0);
    let detail = format!(
        "cpu {} · cores {} · temperature {} · package power {} · gpu {} · memory {} · drives {} · disk activity {} · network {} · battery {} · top memory {} · light reading {} · first full {} · full {} ({}) · light reading CPU {}",
        cpu.map_or("missing".into(), |c| format!("{:.0}% of {} threads{}", c.load * 100.0, c.logical, c.mhz.map_or(String::new(), |m| format!(" at {m} MHz")))),
        cores,
        cpu.and_then(|c| c.temp_c).map_or("not reported".into(), |t| format!("{t:.0} °C")),
        cpu.and_then(|c| c.power_w).map_or("not reported".into(), |w| format!("{w:.1} W")),
        r.gpu.as_ref().map_or("none".into(), |g| format!("{} {:.0}% {} of {}", g.name, g.util * 100.0, fmt_bytes(g.mem_used), fmt_bytes(g.mem_total))),
        mem.map_or("missing".into(), |m| format!("{} of {} (wired {}, compressed {}, cached {}, free {})", fmt_bytes(m.used), fmt_bytes(m.total), fmt_bytes(m.wired), fmt_bytes(m.compressed), fmt_bytes(m.cached), fmt_bytes(m.free))),
        if r.drives.is_empty() { "none".into() } else { r.drives.iter().map(|d| d.root.clone()).collect::<Vec<_>>().join(" ") },
        if r.disk.is_some() { "yes" } else { "no" },
        r.network.as_ref().map_or("unavailable".into(), |n| if n.online { format!("{} {} {}", n.adapter, n.ipv4.as_deref().unwrap_or(""), lumo_core::sysmon::fmt_link(n.link_bps)) } else { "offline".into() }),
        r.battery.as_ref().map_or("none".into(), |b| format!("{}%", b.percent)),
        if apps.is_empty() { "none".into() } else { apps.join(", ") },
        ms(p.light),
        ms(p.first_full),
        ms(p.full),
        p.parts.iter().map(|(name, d)| format!("{name} {}", ms(*d))).collect::<Vec<_>>().join(", "),
        match (p.light_cycles, cpu.and_then(|c| c.mhz)) {
            (Some(cycles), Some(mhz)) => format!("{cycles} cycles = {:.2} ms at {mhz} MHz", cycles as f64 / (f64::from(mhz) * 1000.0)),
            (Some(cycles), None) => format!("{cycles} cycles"),
            _ => "not measured".into(),
        },
    );
    (ok, detail)
}



const SAMPLE_PLACES: &str = r#"{"places":[
    {"id":"jakarta","label":"Kota Jakarta","kind":"kota","prov":"DKI Jakarta","lat":-6.1833,"lng":106.8333,"tz":"Asia/Jakarta"},
    {"id":"banda-aceh","label":"Kota Banda Aceh","kind":"kota","prov":"Aceh","lat":5.55,"lng":95.3167,"tz":"Asia/Jakarta"},
    {"id":"bandar-lampung","label":"Kota Bandar Lampung","kind":"kota","prov":"Lampung","lat":-5.4167,"lng":105.2667,"tz":"Asia/Jakarta"},
    {"id":"kab-bandung","label":"Kab. Bandung","kind":"kab","prov":"Jawa Barat","lat":-7.0167,"lng":107.5333,"tz":"Asia/Jakarta","h0":-2},
    {"id":"bandung","label":"Kota Bandung","kind":"kota","prov":"Jawa Barat","lat":-6.9,"lng":107.6167,"tz":"Asia/Jakarta","h0":-2},
    {"id":"kab-bandung-barat","label":"Kab. Bandung Barat","kind":"kab","prov":"Jawa Barat","lat":-6.85,"lng":107.5167,"tz":"Asia/Jakarta","h0":-2},
    {"id":"makkah","label":"Makkah","kind":"abroad","prov":"Arab Saudi","lat":21.4225,"lng":39.8261,"tz":"Asia/Riyadh","method":"ummalqura"}
]}"#;

fn fake_power(plugged: bool) -> PowerState {
    PowerState { has_battery: true, percent: 76, charging: plugged, on_ac: plugged, saver: false }
}

// ---------------------------------------------------------------- scenes

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    AddonsScriptOn,
    IdleClock,
    IdleEyes,
    Media,
    Timer,
    MediaTimer,
    VolumeHud,
    ChargingHud,
    BluetoothHud,
    Page(PageKind),
    /// The shelf while files are dragged over the island.
    ShelfDrop,
    /// An empty shelf: the drop zone and the Phone drop tile.
    ShelfEmpty,
    /// Phone drop open: the QR code waits for a phone.
    ShelfPhone,
    /// An iPhone asks to send three files.
    ShelfPhoneAsk,
    /// Files coming in from the iPhone.
    ShelfPhoneReceive,
    /// Click the first header tab on the Clipboard page; expect the Music page.
    TabClick,
    /// Click the gear in the header on Music; expect Settings.
    GearClick,
    /// System page on a desktop (no battery): Memory and its apps.
    SystemMemory,
    /// System page: the drives and disk activity.
    SystemDrives,
    /// System page: the network.
    SystemNetwork,
    /// System page: the laptop's battery.
    SystemBattery,
    /// System page just opened: no reading yet.
    SystemWaiting,
    /// Settings with Stay on top chosen for full-screen apps.
    SettingsStayOnTop,
    /// Phone drop showing a pairing QR code (paired phones).
    ShelfPairing,
    /// A phone just finished pairing.
    ShelfPaired,
    /// The paired phones, each with Remove.
    ShelfPairedPhones,
    /// Files from a paired phone landed while the island was closed.
    PairedNotice,
    /// System page: the graphics card.
    SystemGpu,
    GameStats,
    /// ...before the one-time OK: the frame rate waits, the rest shows.
    GameStatsAllow,
    /// System page, Games: a game in front, its frame rate's minute.
    SystemGames,
    /// System page, Games, before the one-time OK.
    SystemGamesAllow,
    IdleStatus,
    /// Music playing while a synced lyric line is sung (the Pill theme shows
    /// the line on the closed island).
    MediaLyric,
    /// Same track, a long line followed by a short one: its width must shrink.
    MediaLyricLong,
    MediaLyricShort,
    /// Music retains its lyrics with the closed-island LRC switch off.
    MusicLrcOff,
    /// The battery ran low: the pop-up, not charging.
    LowBatteryHud,
    /// The Script page partway through a reading (paused, so the shot holds still).
    ScriptReading,
    /// The same reading with Lumo Recorder installed: Record shows.
    ScriptRecorder,
    /// System page on a desktop, Power: its power supply reports over USB.
    SystemPower,
    /// ...and a desktop whose power supply doesn't: the estimate.
    SystemPowerEstimate,
    /// A maximized window under the island, Step aside on (no longer offered): the
    /// clock waits out of the way, an invisible strip at the top edge.
    AsideIdle,
    /// ...and while a song sings its lines.
    AsideLyric,
    /// ...a new song's title still drops down for its few seconds.
    AsidePeek,
    /// ...and so does the volume pop-up.
    AsideHud,
    /// Settings with Step aside off (no row for it since 0.2.6).
    SettingsAsideStay,
    /// A new version is out: the island tells once, with Update.
    UpdateNotice,
    /// The first start after an update: "Updated to", with What's new.
    UpdatedNotice,
    /// Settings while a new version is offered: Update on its last row.
    SettingsUpdate,
    /// The Update sheet (the notice or the menu's "Update to" line opens it).
    UpdateSheet,
    /// The Update sheet once Windows has the Setup: Install now, or on next start.
    UpdateReady,
}

/// A later release than this build, as the update file describes one.
fn fake_release() -> lumo_core::update::Release {
    let mut parts: Vec<u64> = crate::version::VERSION.split('.').filter_map(|p| p.parse().ok()).collect();
    if let Some(last) = parts.last_mut() {
        *last += 1;
    }
    let version = parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
    let repo = "https://github.com/mthlpbs/lumo/releases";
    lumo_core::update::Release {
        date: "2026-10-26".into(),
        headline: "A smoother island, and fixes".into(),
        notes: format!("{repo}/tag/v{version}"),
        windows: Some(lumo_core::update::Download { url: format!("{repo}/download/v{version}/Lumo-Setup-{version}.exe"), size: 2_500_000, sha256: "0".repeat(64) }),
        version,
    }
}

/// Running under Wine: its ntdll exports `wine_get_version`.
fn under_wine() -> bool {
    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    unsafe { GetModuleHandleW(w!("ntdll.dll")).is_ok_and(|h| GetProcAddress(h, s!("wine_get_version")).is_some()) }
}

/// What the Script page reads in its self-test scenes.
const SAMPLE_SCRIPT: &str = "Hi everyone, this one takes two minutes. Today I'll show you the Shelf, and how to send a file to your phone.\n\n\
Drop any file on the island and it waits on the Shelf. Press the phone button, scan the code, and the file is on your phone. \
No cable, no account, nothing to install on the phone.\n\nThat's it. Thanks for watching.";

const SCENES: [Scene; 56] = [
    Scene::IdleClock,
    Scene::IdleEyes,
    Scene::Media,
    Scene::Timer,
    Scene::MediaTimer,
    Scene::VolumeHud,
    Scene::ChargingHud,
    Scene::BluetoothHud,
    Scene::Page(PageKind::Media),
    Scene::Page(PageKind::Focus),
    Scene::Page(PageKind::Shelf),
    Scene::ShelfDrop,
    Scene::ShelfPhone,
    Scene::ShelfPhoneAsk,
    Scene::ShelfPhoneReceive,
    // Last of the shelf scenes: hiding the items resets their fade-in.
    Scene::ShelfEmpty,
    Scene::Page(PageKind::Tasks),
    Scene::Page(PageKind::Clipboard),
    Scene::TabClick,
    Scene::GearClick,
    Scene::Page(PageKind::System),
    Scene::SystemMemory,
    Scene::SystemDrives,
    Scene::SystemNetwork,
    Scene::SystemBattery,
    Scene::SystemWaiting,
    Scene::SettingsStayOnTop,
    // Appended so the numbers of the screenshots above stay the same. They
    // show an empty shelf: items shown again after ShelfEmpty would fade in.
    Scene::ShelfPairing,
    Scene::ShelfPaired,
    Scene::ShelfPairedPhones,
    Scene::PairedNotice,
    Scene::SystemGpu,
    Scene::Page(PageKind::Addons),
    Scene::AddonsScriptOn,
    Scene::GameStats,
    Scene::GameStatsAllow,
    Scene::SystemGames,
    Scene::SystemGamesAllow,
    // Appended so the numbers above stay. Every theme runs them; the Pill
    // theme draws the first two its own way.
    Scene::IdleStatus,
    Scene::MediaLyric,
    Scene::LowBatteryHud,
    // The Script page: empty, then partway through a reading.
    Scene::Page(PageKind::Script),
    Scene::ScriptReading,
    // ...and with the recorder installed (Record by Paste).
    Scene::ScriptRecorder,
    // The whole PC's power: read from the power supply, then estimated.
    Scene::SystemPower,
    Scene::SystemPowerEstimate,
    // A maximized window under the island: what rests steps aside, what
    // happens still shows. Then the Settings page with Stay picked.
    Scene::AsideIdle,
    Scene::AsideLyric,
    Scene::AsidePeek,
    Scene::AsideHud,
    Scene::SettingsAsideStay,
    // New versions: the island's notice, "Updated to", Settings offering
    // one, and the Update sheet before and after the Setup is in.
    Scene::SettingsUpdate,
    Scene::MediaLyricLong,
    Scene::MediaLyricShort,
    Scene::MusicLrcOff,
    Scene::Page(PageKind::Ai),
];

impl Scene {
    fn name(self) -> String {
        match self {
            Scene::AddonsScriptOn => "page-addons-script-on".into(),
            Scene::IdleClock => "idle-clock".into(),
            Scene::IdleEyes => "idle-eyes".into(),
            Scene::Media => "media-activity".into(),
            Scene::Timer => "timer-activity".into(),
            Scene::MediaTimer => "media-timer".into(),
            Scene::VolumeHud => "volume-hud".into(),
            Scene::ChargingHud => "charging-hud".into(),
            Scene::BluetoothHud => "bluetooth-hud".into(),
            Scene::Page(p) => format!("page-{}", p.key()),
            Scene::ShelfDrop => "page-shelf-drop".into(),
            Scene::ShelfEmpty => "page-shelf-empty".into(),
            Scene::ShelfPhone => "page-shelf-phone".into(),
            Scene::ShelfPhoneAsk => "page-shelf-phone-ask".into(),
            Scene::ShelfPhoneReceive => "page-shelf-phone-receive".into(),
            Scene::TabClick => "tab-click-media".into(),
            // Reached by clicking the gear.
            Scene::GearClick => "page-settings".into(),
            Scene::SystemMemory => "page-system-memory".into(),
            Scene::SystemDrives => "page-system-drives".into(),
            Scene::SystemNetwork => "page-system-network".into(),
            Scene::SystemBattery => "page-system-battery".into(),
            Scene::SystemWaiting => "page-system-waiting".into(),
            Scene::SettingsStayOnTop => "page-settings-stay-on-top".into(),
            Scene::ShelfPairing => "page-shelf-pairing".into(),
            Scene::ShelfPaired => "page-shelf-paired".into(),
            Scene::ShelfPairedPhones => "page-shelf-paired-phones".into(),
            Scene::PairedNotice => "paired-received-notice".into(),
            Scene::SystemGpu => "page-system-gpu".into(),
            Scene::GameStats => "game-stats".into(),
            Scene::GameStatsAllow => "game-stats-allow".into(),
            Scene::SystemGames => "page-system-games".into(),
            Scene::SystemGamesAllow => "page-system-games-allow".into(),
            Scene::IdleStatus => "idle-status".into(),
            Scene::MediaLyric => "media-lyric".into(),
            Scene::MediaLyricLong => "media-lyric-long".into(),
            Scene::MediaLyricShort => "media-lyric-short".into(),
            Scene::MusicLrcOff => "music-lrc-off".into(),
            Scene::LowBatteryHud => "low-battery-hud".into(),
            Scene::ScriptReading => "page-script-reading".into(),
            Scene::ScriptRecorder => "page-script-recorder".into(),
            Scene::SystemPower => "page-system-power".into(),
            Scene::SystemPowerEstimate => "page-system-power-estimate".into(),
            Scene::AsideIdle => "aside-idle".into(),
            Scene::AsideLyric => "aside-media-lyric".into(),
            Scene::AsidePeek => "aside-new-song".into(),
            Scene::AsideHud => "aside-volume-hud".into(),
            Scene::SettingsAsideStay => "page-settings-maximized-stay".into(),
            Scene::UpdateNotice => "update-notice".into(),
            Scene::UpdatedNotice => "updated-notice".into(),
            Scene::SettingsUpdate => "page-settings-update".into(),
            Scene::UpdateSheet => "page-update-sheet".into(),
            Scene::UpdateReady => "page-update-ready".into(),
        }
    }

    fn page(self) -> Option<PageKind> {
        match self {
            Scene::AddonsScriptOn => Some(PageKind::Addons),
            Scene::Page(p) => Some(p),
            Scene::ShelfDrop | Scene::ShelfEmpty | Scene::ShelfPhone | Scene::ShelfPhoneAsk | Scene::ShelfPhoneReceive => Some(PageKind::Shelf),
            Scene::TabClick => Some(PageKind::ALL[0]),
            Scene::MusicLrcOff => Some(PageKind::Media),
            Scene::GearClick
            | Scene::SettingsStayOnTop
            | Scene::SettingsAsideStay
            | Scene::SettingsUpdate
            | Scene::UpdateSheet
            | Scene::UpdateReady => Some(PageKind::Settings),
            Scene::SystemMemory
            | Scene::SystemDrives
            | Scene::SystemNetwork
            | Scene::SystemBattery
            | Scene::SystemWaiting
            | Scene::SystemGpu
            | Scene::SystemGames
            | Scene::SystemGamesAllow
            | Scene::SystemPower
            | Scene::SystemPowerEstimate => Some(PageKind::System),
            Scene::ShelfPairing | Scene::ShelfPaired | Scene::ShelfPairedPhones => Some(PageKind::Shelf),
            Scene::ScriptReading | Scene::ScriptRecorder => Some(PageKind::Script),
            _ => None,
        }
    }

    fn expanded(self) -> bool {
        self.page().is_some()
    }

    fn settings_section(self) -> Option<crate::settings_window::Section> {
        use crate::settings_window::Section;
        match self {
            Self::GearClick => Some(Section::General),
            Self::SettingsStayOnTop | Self::SettingsAsideStay => Some(Section::Display),
            Self::SettingsUpdate | Self::UpdateSheet | Self::UpdateReady => Some(Section::About),
            _ => None,
        }
    }

    /// Show over games is on (and a game is in front, or was).
    fn game(self) -> bool {
        matches!(self, Scene::GameStats | Scene::GameStatsAllow | Scene::SystemGames | Scene::SystemGamesAllow)
    }

    fn media_playing(self) -> bool {
        matches!(self, Scene::Media | Scene::MediaTimer | Scene::MediaLyric | Scene::MediaLyricLong | Scene::MediaLyricShort | Scene::AsideLyric | Scene::AsidePeek) || self.expanded()
    }

    /// A maximized window is under the island.
    fn window_under(self) -> bool {
        matches!(self, Scene::AsideIdle | Scene::AsideLyric | Scene::AsidePeek | Scene::AsideHud)
    }

    /// The island rests stepped aside: nothing to see, a strip to point at.
    fn rests_aside(self) -> bool {
        matches!(self, Scene::AsideIdle | Scene::AsideLyric | Scene::AsidePeek)
    }

    fn timer_running(self) -> bool {
        matches!(self, Scene::Timer | Scene::MediaTimer) || self.expanded()
    }
}

/// GET the phone page from a running Phone drop server: the address must be
/// reachable and the page must come back whole. The detail never includes
/// the link's secret.
pub fn fetch_phone_page(url: &str) -> (bool, String) {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let rest = url.split_once("//").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
    let Ok(addr) = host.parse::<SocketAddr>() else {
        return (false, format!("link has no address: {host}"));
    };
    let fetch = || -> std::io::Result<String> {
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        write!(s, "GET /{path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: lumo-selftest\r\n\r\n")?;
        let mut out = String::new();
        s.read_to_string(&mut out)?;
        Ok(out)
    };
    match fetch() {
        Ok(resp) if resp.starts_with("HTTP/1.1 200") && resp.contains("<title>Lumo") && resp.trim_end().ends_with("</html>") => {
            (true, format!("served the page at {addr} ({} bytes)", resp.len()))
        }
        Ok(resp) => (false, format!("unexpected answer from {addr}: {}", resp.lines().next().unwrap_or(""))),
        Err(e) => (false, format!("couldn't fetch the page at {addr}: {e}")),
    }
}

// ---------------------------------------------------------------- report

#[derive(Clone, Debug, Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
}

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Check {
    Check { name: name.into(), ok, detail: detail.into() }
}

#[derive(Debug, Serialize)]
struct StateReport {
    index: usize,
    name: String,
    expanded: bool,
    page: Option<&'static str>,
    image: String,
    alpha_image: String,
    ok: bool,
    settle_ms: u64,
    /// Best of the sampled frame times (full frame: layout, render, present, readback).
    frame_ms: f64,
    frame_samples_ms: Vec<f64>,
    canvas_px: [i32; 2],
    window_pos_px: [i32; 2],
    /// Island body in canvas DIPs: x, y, w, h.
    body_dip: [f32; 4],
    measured_width_px: i32,
    checks: Vec<Check>,
}

#[derive(Debug, Serialize)]
struct Report {
    app: &'static str,
    version: &'static str,
    ok: bool,
    exit_code: i32,
    total_ms: u64,
    scale: f32,
    expanded_width_dip: f32,
    /// "gpu", "gpu-warp" or "software".
    renderer: String,
    frame_budget_ms: f64,
    states_passed: usize,
    states_total: usize,
    failed_checks: Vec<String>,
    /// Frames over the budget on a software GPU (CI runners), where frame
    /// time is reported and doesn't fail the run: "scene: detail".
    slow_frames: Vec<String>,
    /// Checks that are not tied to a single screenshot (fixtures, input, files).
    checks: Vec<Check>,
    states: Vec<StateReport>,
}

// ---------------------------------------------------------------- driver

enum Phase {
    /// Record one transition frame by frame (LUMO_SELFTEST_ANIM=1).
    Anim { seq: usize },
    /// Record the media pill playing, then paused (LUMO_SELFTEST_LOOP=1).
    Loop,
    /// Film the lyrics moving to the next line (LUMO_SELFTEST_SCROLL=<Hz>).
    Scroll,
    Settle { i: usize, since: f64, steps: u32, stable: u32 },
    Click { i: usize },
    ClickWait { i: usize, since: f64, from: PageKind, want: PageKind },
    Finish { since: f64 },
    Done(i32),
}

pub struct SelfTest {
    dir: PathBuf,
    t0: Instant,
    phase: Phase,
    art: Option<Image>,
    art_color: Option<Color>,
    lyrics: LyricsState,
    track_key: String,
    /// Where the media-lyric scene holds the song: inside a sung line.
    lyric_pos: f64,
    states: Vec<StateReport>,
    checks: Vec<Check>,
    writes: Vec<(String, JoinHandle<Vec<String>>)>,
    scale: f32,
    expanded_width: f32,
    /// LUMO_SELFTEST_DPI: render at this DPI instead of the monitor's.
    dpi: Option<u32>,
    /// LUMO_SELFTEST_ANIM=1: after the scenes, record transitions frame by frame.
    anim: bool,
    /// LUMO_SELFTEST_LOOP=1: after the scenes, record the media pill on a held
    /// clock, frame by frame.
    media_loop: bool,
    /// LUMO_SELFTEST_SCROLL=<Hz>: after the scenes, film the lyrics moving to
    /// the next line on a held clock, one frame per refresh at that rate.
    scroll_hz: Option<f64>,
    dpi_applied: bool,
    /// LUMO_SELFTEST_THEME=classic|pill: run every scene in that theme.
    theme: Theme,
    local_lyrics: bool,
    /// The frame budget the scenes were held to (for the report).
    budget_ms: f64,
    /// Frames over that budget on a software GPU, reported and not counted.
    slow_frames: Vec<String>,
    /// LUMO_SELFTEST_LYRICS=ja|ko|zh: the sample song in that language.
    lyrics_lang: Option<String>,
    renderer: &'static str,
    /// The seeded shelf and its thumbnails (the empty-shelf scene hides
    /// them, and the page forgets thumbnails of items that left).
    shelf_items: Vec<ShelfItem>,
    shelf_thumbs: Vec<(String, Option<Image>)>,
    /// The real add-on download (LUMO_SELFTEST_ADDONS=download), running beside the scenes.
    addon_fetch: Option<std::sync::mpsc::Receiver<(bool, String)>>,
    /// Show over games' frame count, run on Lumo's own frames: Windows' frame
    /// log must deliver them (LUMO_SELFTEST_FRAMES=required on Windows CI,
    /// which runs as admin).
    frames: Option<crate::services::frames::Frames>,
}

/// Transitions recorded in animation mode: (name, from, to).
const ANIMS: [(&str, Scene, Scene); 5] = [
    ("expand", Scene::Media, Scene::Page(PageKind::Media)),
    ("page", Scene::Page(PageKind::Media), Scene::Page(PageKind::Focus)),
    ("collapse", Scene::Page(PageKind::Media), Scene::Media),
    ("hud", Scene::Media, Scene::VolumeHud),
    ("notice", Scene::IdleClock, Scene::PairedNotice),
];
/// Frames recorded per transition (~16 ms apart).
const ANIM_FRAMES: usize = 40;
/// The media loop recording: seconds playing, seconds paused, frames a
/// second, and the clock step it starts on.
const LOOP_PLAY_S: f64 = 4.8;
const LOOP_PAUSE_S: f64 = 2.4;
const LOOP_FPS: f64 = 30.0;
const LOOP_ALIGN_S: f64 = 4.8;
const LOOP_PREROLL_S: f64 = 2.4;
/// The lyrics scroll film (LUMO_SELFTEST_SCROLL): where the track starts
/// (the next line starts 0.5 s in), unsaved lead-in, and the filmed length.
const SCROLL_FROM_S: f64 = 65.0;
const SCROLL_PREROLL_S: f64 = 1.0;
const SCROLL_FILM_S: f64 = 1.3;

impl SelfTest {
    pub fn new(dir: String) -> Self {
        let track_key = MediaSession::make_track_key(TITLE, ARTIST, ALBUM);
        Self {
            dir: PathBuf::from(dir),
            t0: Instant::now(),
            phase: Phase::Settle { i: 0, since: 0.0, steps: 0, stable: 0 },
            art: None,
            art_color: None,
            lyrics: LyricsState { track_key: track_key.clone(), ..Default::default() },
            track_key,
            lyric_pos: TRACK_POS_S,
            states: Vec::new(),
            checks: Vec::new(),
            writes: Vec::new(),
            scale: 1.0,
            expanded_width: 0.0,
            dpi: std::env::var("LUMO_SELFTEST_DPI").ok().and_then(|v| v.trim().parse().ok()),
            anim: std::env::var("LUMO_SELFTEST_ANIM").is_ok_and(|v| v.trim() == "1"),
            media_loop: std::env::var("LUMO_SELFTEST_LOOP").is_ok_and(|v| v.trim() == "1"),
            scroll_hz: std::env::var("LUMO_SELFTEST_SCROLL").ok().and_then(|v| v.trim().parse().ok()).filter(|hz: &f64| (20.0..=500.0).contains(hz)),
            dpi_applied: false,
            theme: match std::env::var("LUMO_SELFTEST_THEME").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
                Ok("pill") => Theme::Pill,
                _ => Theme::Classic,
            },
            local_lyrics: std::env::var("LUMO_SELFTEST_LOCAL_LYRICS").is_ok_and(|v| v.trim() == "1"),
            budget_ms: FRAME_BUDGET_MS,
            slow_frames: Vec::new(),
            lyrics_lang: std::env::var("LUMO_SELFTEST_LYRICS").ok().map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty()),
            renderer: "software",
            shelf_items: Vec::new(),
            shelf_thumbs: Vec::new(),
            addon_fetch: None,
            frames: None,
        }
    }

    /// Seed the app with fake data. Called once before the first frame.
    pub fn begin(&mut self, st: &mut AppState) {
        // The real readers behind the System page (its scenes show sample
        // readings); before the scene clock starts, as it takes a moment.
        let (ok, detail) = system_readings(crate::services::sysmon::probe());
        self.checks.push(check("system-readings", ok, detail));
        // Show over games: count Lumo's own frames through the whole run.
        let required = std::env::var("LUMO_SELFTEST_FRAMES").is_ok_and(|v| v.trim().eq_ignore_ascii_case("required"));
        match crate::services::frames::Frames::start_selftest(std::process::id()) {
            Ok(f) => self.frames = Some(f),
            Err(e) if required => self.checks.push(check("frame-count", false, format!("the frame count didn't start: {e:?}"))),
            Err(e) => crate::pinfo!("selftest: frame count not started ({e:?})"),
        }
        self.t0 = Instant::now();
        crate::pinfo!("selftest: output folder {}", self.dir.display());
        match fs::create_dir_all(&self.dir) {
            Ok(()) => self.clear_old_outputs(),
            Err(e) => self.checks.push(check("output-folder", false, format!("{}: {e}", self.dir.display()))),
        }

        // Quiet, deterministic settings (defaults otherwise: the home folder is fresh).
        st.settings.sounds = false;
        st.settings.show_media_activity = true;
        st.settings.lyrics = true;
        st.settings.hud_volume = true;
        st.settings.hud_power = true;
        st.settings.hud_bluetooth = true;
        st.accent = palette::BLUE;
        st.pinned = false;
        st.volume = VolumeState { level: 0.64, muted: false, known: true };
        st.power = fake_power(false);

        // Album art + lyrics.
        let art = ImageData::new(ART_PX, ART_PX, make_art(ART_PX));
        self.art_color = Some(art.accent_color());
        self.art = Some(art);
        // LUMO_SELFTEST_LRC=<file>: a real song for a one-off check on CI
        // (fetched there at run time; song lyrics never go in the repo).
        let file = std::env::var("LUMO_SELFTEST_LRC").ok().and_then(|p| fs::read_to_string(p.trim()).ok());
        let lrc = match (file.as_deref(), self.lyrics_lang.as_deref()) {
            (Some(f), _) => f,
            (None, Some("ja")) => LYRICS_JA,
            (None, Some("ko")) => LYRICS_KO,
            (None, Some("zh")) => LYRICS_ZH,
            _ => LYRICS_LRC,
        };
        let parsed = parse_lrc(lrc);
        if file.is_none() {
            let lyric_ok = parsed.synced && parsed.lines.len() == 12 && parsed.line_index_at(TRACK_POS_S) == Some(4);
            self.checks.push(check(
                "lyrics-fixture",
                lyric_ok,
                format!("{} lines, synced {}, line at {TRACK_POS_S}s = {:?}", parsed.lines.len(), parsed.synced, parsed.line_index_at(TRACK_POS_S)),
            ));
        }
        if let Some(pos) = lyric_scene_pos(&parsed) {
            self.lyric_pos = pos;
        }
        self.lyrics = LyricsState { track_key: self.track_key.clone(), status: LyricsStatus::Found, ruby: crate::services::romanize::pronounce(&parsed).ruby, lyrics: Some(parsed), source: "Sample".into(), ..LyricsState::default() };

        seed_tasks(st);
        seed_clipboard(st);
        let today = st.day_key();
        st.store.focus_sessions.insert(today, 2);
        // A week of history for the Focus page's chart and streak.
        for (d, n) in [(-1, 6), (-2, 8), (-3, 4), (-4, 9), (-6, 3)] {
            st.store.focus_sessions.insert(st.local.add_days(d).day_key(), n);
        }
        self.seed_shelf(st);

        // Read-only: the Store build must reach the startup task its manifest declares.
        let (ok, detail) = match crate::autostart::describe() {
            Ok(d) => (true, d),
            Err(d) => (false, d),
        };
        self.checks.push(check("start-with-windows", ok, detail));

        // LUMO_SELFTEST_ADDONS=download (Windows CI): fetch every add-on's files
        // from their releases and check each against the size and SHA-256 built in.
        if std::env::var("LUMO_SELFTEST_ADDONS").is_ok_and(|v| v.trim() == "download") {
            let (tx, rx) = std::sync::mpsc::channel();
            let dir = paths::addons_dir().join("selftest-download");
            let _ = std::thread::Builder::new().name("selftest-addon".into()).spawn(move || {
                let (mut ok, mut said) = (true, Vec::new());
                for a in lumo_core::addons::ALL {
                    let (fine, detail) = crate::services::addons::fetch_for_selftest(a, &dir);
                    ok &= fine;
                    said.push(format!("{}: {detail}", a.id));
                }
                let _ = tx.send((ok, said.join("; ")));
            });
            self.addon_fetch = Some(rx);
        }
    }

    /// Called every ~50 ms; returns Some(exit code) when finished.
    pub fn step(&mut self, app: &mut App) -> Option<i32> {
        if !self.dpi_applied {
            let (ok, detail) = app.selftest_phone_drop();
            self.checks.push(check("phone-drop-server", ok, detail));
            self.dpi_applied = true;
            if let Some(dpi) = self.dpi {
                crate::pinfo!("selftest: rendering at {dpi} DPI ({:.0} %)", dpi as f32 / 96.0 * 100.0);
                app.selftest_set_dpi(dpi);
            }
        }
        self.scale = app.scale();
        self.expanded_width = app.st.settings.expanded_width;
        self.renderer = app.renderer();
        let now = clock::mono();
        let elapsed = self.t0.elapsed().as_secs_f64();
        if elapsed > SCENE_DEADLINE && !matches!(self.phase, Phase::Finish { .. } | Phase::Done(_) | Phase::Anim { .. } | Phase::Loop | Phase::Scroll) {
            let at = match &self.phase {
                Phase::Settle { i, .. } | Phase::Click { i } | Phase::ClickWait { i, .. } => SCENES[*i].name(),
                _ => String::new(),
            };
            crate::pwarn!("selftest: deadline reached while at '{at}'");
            self.checks.push(check("deadline", false, format!("stopped after {elapsed:.1} s at '{at}'")));
            self.phase = Phase::Finish { since: now };
        }

        match self.phase {
            Phase::Anim { seq } => {
                self.record_anim(app, seq);
                self.phase = if seq + 1 < ANIMS.len() {
                    Phase::Anim { seq: seq + 1 }
                } else if self.media_loop {
                    Phase::Loop
                } else if self.scroll_hz.is_some() {
                    Phase::Scroll
                } else {
                    Phase::Finish { since: now }
                };
            }
            Phase::Loop => {
                self.record_loop(app);
                self.phase = if self.scroll_hz.is_some() { Phase::Scroll } else { Phase::Finish { since: clock::mono() } };
            }
            Phase::Scroll => {
                if let Some(hz) = self.scroll_hz {
                    self.record_scroll(app, hz);
                }
                self.phase = Phase::Finish { since: clock::mono() };
            }
            Phase::Settle { i, since, steps, stable } => {
                let scene = SCENES[i];
                let since = if steps == 0 { now } else { since };
                self.apply(app, scene);
                let settled = app.selftest_settled();
                let stable = if settled { stable + 1 } else { 0 };
                let steps = steps + 1;
                let waited = now - since;
                let timed_out = waited > self.settle_limit(app);
                let ready = steps >= MIN_STEPS && stable >= STABLE_STEPS && !(scene == Scene::IdleEyes && mid_blink());
                if ready || timed_out {
                    self.capture(app, i, !timed_out || settled, waited);
                    self.phase = self.next_phase(i + 1, now);
                } else {
                    self.phase = Phase::Settle { i, since, steps, stable };
                }
            }
            Phase::Click { i } => {
                let from = app.st.page;
                // The first tab, or the gear (found by its widget id: its place
                // depends on how wide the clock and date are).
                let (what, at, want) = match SCENES[i] {
                    Scene::GearClick => ("tray Settings", None, PageKind::Settings),
                    _ => ("the first section tab", app.selftest_hit_center(crate::ui::wid_n("tab",0)), PageKind::ALL[0]),
                };
                match at {
                    Some(p) => {
                        crate::pinfo!("selftest: clicking {what} at ({:.0}, {:.0}) DIP on page '{}'", p.x, p.y, from.key());
                        app.selftest_click(p);
                    }
                    None if SCENES[i]==Scene::GearClick => app.st.cmd(crate::commands::Command::OpenSettings),
                    None => crate::pwarn!("selftest: {what} isn't on page '{}'", from.key()),
                }
                self.phase = Phase::ClickWait { i, since: now, from, want };
            }
            Phase::ClickWait { i, since, from, want } => {
                let page = app.st.page;
                let gear = SCENES[i] == Scene::GearClick;
                if page != from || (gear && app.selftest_settings_open()) || now - since > CLICK_TIMEOUT {
                    let ok = if gear { page == from && app.selftest_settings_open() } else { from != want && page == want };
                    let name = if gear { "gear-click" } else { "tab-click" };
                    self.checks.push(check(
                        name,
                        ok,
                        if gear { format!("standalone Settings open: {}, island stays on '{}'", app.selftest_settings_open(), from.key()) }
                        else { format!("page '{}' -> '{}' after {:.0} ms (expected '{}')", from.key(), page.key(), (now - since) * 1000.0, want.key()) },
                    ));
                    if gear { app.selftest_close_settings(); }
                    if !ok {
                        crate::pwarn!("selftest: {name} did not switch to '{}' (page is '{}')", want.key(), page.key());
                    }
                    self.phase = Phase::Settle { i, since: now, steps: 0, stable: 0 };
                }
            }
            Phase::Finish { since } => {
                let all_done = self.writes.iter().all(|(_, h)| h.is_finished());
                let deadline = if self.anim || self.media_loop || self.scroll_hz.is_some() { WRITE_DEADLINE + 40.0 } else { WRITE_DEADLINE };
                if all_done || elapsed > deadline {
                    let code = self.finish(all_done, now - since);
                    self.phase = Phase::Done(code);
                    return Some(code);
                }
            }
            Phase::Done(code) => return Some(code),
        }
        None
    }

    fn next_phase(&self, i: usize, now: f64) -> Phase {
        match SCENES.get(i) {
            Some(Scene::TabClick | Scene::GearClick) => Phase::Click { i },
            Some(_) => Phase::Settle { i, since: now, steps: 0, stable: 0 },
            None if self.anim => Phase::Anim { seq: 0 },
            None if self.media_loop => Phase::Loop,
            None if self.scroll_hz.is_some() => Phase::Scroll,
            None => Phase::Finish { since: now },
        }
    }

    /// Settle the `from` state, switch to `to`, and save every frame of the
    /// transition as anim-<name>-NN.png. Blocks the UI thread for ~1-3 s,
    /// which is fine in self-test mode.
    fn record_anim(&mut self, app: &mut App, seq: usize) {
        let (name, from, to) = ANIMS[seq];
        let t = Instant::now();
        loop {
            self.apply(app, from);
            let _ = app.selftest_capture();
            if (app.selftest_settled() && t.elapsed().as_millis() > 200) || t.elapsed().as_secs_f64() > 2.5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        self.apply(app, to);
        let mut settle_frame = None;
        // Where each frame sat on screen, to check that the island's centre holds still.
        let mut placed = Vec::with_capacity(ANIM_FRAMES);
        for k in 0..ANIM_FRAMES {
            let (w, h, px, at) = app.selftest_capture();
            if settle_frame.is_none() && app.selftest_settled() {
                settle_frame = Some(k);
            }
            placed.push(format!("{{\"frame\": {k}, \"x\": {}, \"y\": {}, \"w\": {w}, \"h\": {h}}}", at.0, at.1));
            let scale = self.scale;
            self.queue_write(&format!("anim-{name}-{k:02}"), w, h, px, scale);
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        let json = format!("{{\"name\": \"{name}\", \"scale\": {}, \"frames\": [\n  {}\n]}}\n", self.scale, placed.join(",\n  "));
        if let Err(e) = fs::write(self.dir.join(format!("anim-{name}.json")), json) {
            crate::pwarn!("selftest: writing anim-{name}.json failed: {e}");
        }
        crate::pinfo!("selftest: anim '{name}' recorded, settled at frame {settle_frame:?}");
    }

    /// The media pill on a held clock: LOOP_PLAY_S playing, then LOOP_PAUSE_S
    /// paused, LOOP_FPS frames a second, saved as loop-media-NNN.png. The
    /// clock starts on a whole LOOP_ALIGN_S, so every build films the same
    /// phase of the music, after LOOP_PREROLL_S of unsaved frames (whatever
    /// builds up while music plays is already there on the first frame).
    fn record_loop(&mut self, app: &mut App) {
        let t = Instant::now();
        loop {
            self.apply(app, Scene::Media);
            let _ = app.selftest_capture();
            if (app.selftest_settled() && t.elapsed().as_millis() > 200) || t.elapsed().as_secs_f64() > 2.5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        let start = ((clock::mono() + LOOP_PREROLL_S + 1.0) / LOOP_ALIGN_S).ceil() * LOOP_ALIGN_S;
        let paused_at = start + LOOP_PLAY_S;
        let frames = ((LOOP_PLAY_S + LOOP_PAUSE_S) * LOOP_FPS).round() as usize;
        let preroll = (LOOP_PREROLL_S * LOOP_FPS).round() as usize;
        let mut placed = Vec::with_capacity(frames);
        for i in 0..preroll + frames {
            let now = start + (i as f64 - preroll as f64) / LOOP_FPS;
            clock::hold(Some(now));
            self.apply(app, Scene::Media);
            let playing = now < paused_at;
            if playing {
                app.st.media_paused_at = None;
            } else {
                if let Some(m) = app.st.media.as_mut() {
                    m.timeline = Timeline::new(TRACK_POS_S, TRACK_LEN_S, now, false);
                }
                app.st.media_paused_at = Some(paused_at);
            }
            let (w, h, px, at) = app.selftest_capture();
            let Some(k) = i.checked_sub(preroll) else { continue };
            placed.push(format!("{{\"frame\": {k}, \"t\": {:.4}, \"playing\": {playing}, \"x\": {}, \"y\": {}, \"w\": {w}, \"h\": {h}}}", now - start, at.0, at.1));
            let scale = self.scale;
            self.queue_write(&format!("loop-media-{k:03}"), w, h, px, scale);
        }
        clock::hold(None);
        let json = format!("{{\"name\": \"media\", \"fps\": {LOOP_FPS}, \"scale\": {}, \"frames\": [\n  {}\n]}}\n", self.scale, placed.join(",\n  "));
        if let Err(e) = fs::write(self.dir.join("loop-media.json"), json) {
            crate::pwarn!("selftest: writing loop-media.json failed: {e}");
        }
        crate::pinfo!("selftest: media loop recorded, {frames} frames");
    }

    /// The Music page on a held clock while the lyrics move on to the next
    /// line ("Over the grey...", at 01:05.50), one frame per refresh at `hz`,
    /// after SCROLL_PREROLL_S of unsaved frames. Saves the lyrics column (grey)
    /// as scroll-lyrics-NNN.png and scroll-lyrics.json; qa tooling measures
    /// from them how evenly the lines move.
    fn record_scroll(&mut self, app: &mut App, hz: f64) {
        let t = Instant::now();
        loop {
            self.apply(app, Scene::Page(PageKind::Media));
            let _ = app.selftest_capture();
            if (app.selftest_settled() && t.elapsed().as_millis() > 200) || t.elapsed().as_secs_f64() > 2.5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        let start = clock::mono() + SCROLL_PREROLL_S + 1.0;
        let pre = (SCROLL_PREROLL_S * hz).round() as usize;
        let frames = (SCROLL_FILM_S * hz).round() as usize;
        let mut placed = Vec::with_capacity(frames);
        for i in 0..pre + frames {
            let now = start + (i as f64 - pre as f64) / hz;
            clock::hold(Some(now));
            self.apply(app, Scene::Page(PageKind::Media));
            if let Some(m) = app.st.media.as_mut() {
                // Sampled at `now`: a timeline sampled later doesn't run backwards.
                m.timeline = Timeline::new(SCROLL_FROM_S + (now - start), TRACK_LEN_S, now, true);
            }
            let (w, h, px, _) = app.selftest_capture();
            let Some(k) = i.checked_sub(pre) else { continue };
            // The right half of the page body: the lyrics column.
            let s = self.scale;
            let body = app.island_body_in_capture();
            let x0 = ((body.x + body.w * 0.5) * s).round().clamp(0.0, w as f32) as usize;
            let x1 = (body.right() * s).round().clamp(0.0, w as f32) as usize;
            let y0 = ((body.y + pages::HEADER_H) * s).round().clamp(0.0, h as f32) as usize;
            let y1 = ((body.bottom() - pages::PAD) * s).round().clamp(0.0, h as f32) as usize;
            if x1 <= x0 || y1 <= y0 || px.len() < (w * h * 4) as usize {
                continue;
            }
            let mut gray = Vec::with_capacity((x1 - x0) * (y1 - y0));
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &px[(y * w as usize + x) * 4..][..4];
                    gray.push(p[0].max(p[1]).max(p[2]));
                }
            }
            placed.push(format!("{{\"frame\": {k}, \"t\": {:.5}, \"w\": {}, \"h\": {}}}", now - start, x1 - x0, y1 - y0));
            let path = self.dir.join(format!("scroll-lyrics-{k:03}.png"));
            let (gw, gh) = ((x1 - x0) as u32, (y1 - y0) as u32);
            if let Err(e) = png::save_gray(&path, gw, gh, &gray) {
                crate::pwarn!("selftest: {e}");
            }
        }
        clock::hold(None);
        let json = format!("{{\"name\": \"lyrics\", \"hz\": {hz}, \"scale\": {}, \"frames\": [\n  {}\n]}}\n", self.scale, placed.join(",\n  "));
        if let Err(e) = fs::write(self.dir.join("scroll-lyrics.json"), json) {
            crate::pwarn!("selftest: writing scroll-lyrics.json failed: {e}");
        }
        crate::pinfo!("selftest: lyrics scroll recorded, {frames} frames at {hz} Hz");
    }

    // ------------------------------------------------------------ scene state

    /// Put the app into `scene`. Idempotent: called on every step and right
    /// before each capture so clocks (track position, countdown) are pinned.
    fn apply(&self, app: &mut App, scene: Scene) {
        let now = clock::mono();
        if matches!(scene,Scene::Page(PageKind::Ai)){let sample=crate::ai_window::sample_state_for_selftest();app.st.ai=sample.ai;app.st.settings.ai=sample.settings.ai;app.st.settings.set_addon("ai",true);}
        let st = &mut app.st;
        st.settings.idle_style = match scene {
            Scene::IdleEyes => IdleStyle::Eyes,
            Scene::IdleStatus => IdleStyle::Status,
            _ => IdleStyle::Clock,
        };
        st.settings.local_lyrics = self.local_lyrics;
        st.settings.island_lyrics = scene != Scene::MusicLrcOff;
        st.foreground_app = Some(crate::state::ForegroundApp {
            executable: "c:\\apps\\music.exe".into(), name: "Media Player".into(), maximized_here: false,
        });
        if self.local_lyrics { st.settings.music_directory = "C:\\Music".into(); }
        if st.settings.theme != self.theme {
            st.settings.theme = self.theme;
            st.accent = palette::BLUE;
        }
        let st = &mut app.st;
        st.power = fake_power(scene == Scene::ChargingHud);
        if scene == Scene::LowBatteryHud {
            st.power.percent = LOW_BATTERY;
        }
        st.volume = VolumeState { level: 0.64, muted: false, known: true };
        st.pinned = false;
        st.drag_hover = scene == Scene::ShelfDrop;
        // Start with Windows as Settings shows it once read (it's read off the UI thread).
        st.ui.autostart = Some(true);
        // Hide in full-screen apps (the default), except in the Stay on top scene.
        st.settings.hide_in_fullscreen = scene != Scene::SettingsStayOnTop;
        // Step aside is off in the app since 0.2.6; the self-test keeps drawing what's left of it.
        st.settings.step_aside = scene != Scene::SettingsAsideStay;
        // A later version offered, only where Settings shows it.
        st.update = match scene {
            Scene::SettingsUpdate | Scene::UpdateSheet | Scene::UpdateReady => crate::state::UpdateView {
                offer: Some(fake_release()),
                sheet: scene != Scene::SettingsUpdate,
                ready: scene == Scene::UpdateReady,
                next_start: false,
                ..crate::state::UpdateView::default()
            },
            _ => crate::state::UpdateView::default(),
        };
        // A new song's title shows for its few seconds (held here).
        st.ui.peek_until = if scene == Scene::AsidePeek { now + 30.0 } else { 0.0 };
        st.phone = fake_phone(scene, now);
        st.paired = fake_paired(scene, st.unix);
        st.pair_server = (!st.paired.is_empty() || scene == Scene::ShelfPairing).then_some(true);
        let empty_shelf = matches!(scene, Scene::ShelfEmpty | Scene::ShelfPairing | Scene::ShelfPaired | Scene::ShelfPairedPhones);
        st.store.shelf.items = if empty_shelf { Vec::new() } else { self.shelf_items.clone() };
        if !empty_shelf {
            for (path, thumb) in &self.shelf_thumbs {
                st.shelf_thumbs.entry(path.clone()).or_insert_with(|| thumb.clone());
            }
        }

        if scene.media_playing() {
            let pos = match scene {
                Scene::MediaLyric | Scene::AsideLyric => self.lyric_pos,
                Scene::MediaLyricLong => 12.0,
                Scene::MediaLyricShort => 22.0,
                _ => TRACK_POS_S,
            };
            let timeline = Timeline::new(pos, TRACK_LEN_S, now, true);
            match st.media.as_mut() {
                Some(m) if m.track_key == self.track_key => m.timeline = timeline,
                _ => st.media = Some(self.media_session(timeline)),
            }
            if st.lyrics.track_key != self.track_key || st.lyrics.status != LyricsStatus::Found {
                st.lyrics = self.lyrics.clone();
            }
            if matches!(scene, Scene::MediaLyricLong | Scene::MediaLyricShort) {
                st.lyrics = LyricsState {
                    track_key: self.track_key.clone(), status: LyricsStatus::Found,
                    lyrics: Some(lumo_core::lrc::parse_lrc("[00:10]A very long lyric line stretches across the island from one end to the other\n[00:20]Go home\n[00:40]\n")),
                    source: "Sample".into(), ..LyricsState::default()
                };
            } else if scene == Scene::MusicLrcOff {
                st.lyrics = self.lyrics.clone();
            }
        } else {
            st.media = None;
            st.lyrics = LyricsState::default();
        }

        st.pomodoro.phase = PomoPhase::Focus;
        st.pomodoro.cycle_done = 1;
        st.pomodoro.run = if scene.timer_running() { RunState::Running { ends_at: now + TIMER_LEFT_S } } else { RunState::Idle };

        let until = now + 30.0;
        st.hud = match scene {
            Scene::VolumeHud | Scene::AsideHud => Some(Hud { kind: HudKind::Volume { level: 0.64, muted: false }, until }),
            Scene::ChargingHud => Some(Hud { kind: HudKind::Charging { percent: 76, plugged: true }, until }),
            Scene::BluetoothHud => {
                Some(Hud { kind: HudKind::Bluetooth { name: "Sound Buds".into(), connected: true, battery: Some(80), audio: true }, until })
            }
            Scene::LowBatteryHud => Some(Hud { kind: HudKind::LowBattery { percent: LOW_BATTERY }, until }),
            _ => None,
        };
        st.notice = match scene {
            Scene::UpdateNotice => Some(crate::updates::new_version_notice(&fake_release(), st.accent, until)),
            Scene::UpdatedNotice => Some(crate::updates::updated_notice(Some(fake_release().headline), until)),
            Scene::PairedNotice => Some(Notice {
                icon: Icon::Phone,
                color: palette::GREEN,
                title: "3 files from Pixel 10 are on the Shelf".into(),
                title_dim: String::new(),
                subtitle: "Open the Shelf to see them".into(),
                until,
                action: Some(NoticeAction::OpenPage(PageKind::Shelf)),
                inline: None,
            }),
            _ => None,
        };
        // Script reader: on for the Script page's scenes, off (no tab) elsewhere.
        let script = scene.page() == Some(PageKind::Script) || scene == Scene::AddonsScriptOn;
        st.settings.set_addon(SCRIPT_READER.id, script);
        st.addons.insert(SCRIPT_READER.id, if script { AddonStatus::Ready } else { AddonStatus::Off });
        // Clipboard history (an add-on, off by default): on for its own page only.
        let clip = scene.page() == Some(PageKind::Clipboard);
        st.settings.clipboard_history = clip;
        st.addons.insert(CLIPBOARD_HISTORY.id, if clip { AddonStatus::Ready } else { AddonStatus::Off });
        // The Script page: empty, or partway through the sample script.
        st.script = lumo_core::script::Reader::default();
        if matches!(scene, Scene::ScriptReading | Scene::ScriptRecorder) {
            st.store.script.text = SAMPLE_SCRIPT.into();
            st.script.word = 22;
            st.script.active = true;
        } else {
            st.store.script.text.clear();
        }
        st.recorder = scene == Scene::ScriptRecorder;
        st.system = fake_system(scene);
        st.settings.game_stats = scene.game();
        st.game = fake_game(scene, &mut st.system);
        st.ui.system_module = match scene {
            Scene::SystemMemory => Module::Memory,
            Scene::SystemDrives => Module::Drives,
            Scene::SystemNetwork => Module::Network,
            Scene::SystemBattery => Module::Battery,
            Scene::SystemGpu => Module::Gpu,
            Scene::SystemGames | Scene::SystemGamesAllow => Module::Games,
            Scene::SystemPower | Scene::SystemPowerEstimate => Module::Power,
            _ => Module::Cpu,
        };
        for (path, rgb) in SAMPLE_ICONS {
            st.app_icons.entry(path.to_string()).or_insert_with(|| Some(sample_icon(rgb)));
        }
        if let Some(p) = scene.page() {
            // Settings now lives in its own native window, never in the island.
            st.page = if p == PageKind::Settings { PageKind::Media } else { p };
        }
        if scene.settings_section().is_none() { app.selftest_close_settings(); }
        app.selftest_window_under(scene.window_under());
        app.selftest_hover(None);
        app.selftest_set_expanded(scene.expanded());
    }

    fn media_session(&self, timeline: Timeline) -> MediaSession {
        MediaSession {
            app_id: "Spotify.exe".into(),
            app_name: "Spotify".into(),
            music_source: Some(lumo_core::nowplaying::MusicSource::Streaming),
            title: TITLE.into(),
            artist: ARTIST.into(),
            album: ALBUM.into(),
            art: self.art.clone(),
            art_color: self.art_color,
            timeline,
            can_prev: true,
            can_next: true,
            can_play_pause: true,
            can_seek: true,
            track_key: self.track_key.clone(),
        }
    }

    fn seed_shelf(&mut self, st: &mut AppState) {
        let home = paths::data_dir();
        let base = std::path::absolute(&home).unwrap_or(home).join("shelf-demo");
        let txt = base.join("Trip notes.txt");
        let img = base.join("Harbor sketch.png");
        let folder = base.join("Project assets");
        let mut errors = Vec::new();
        if let Err(e) = fs::create_dir_all(&folder) {
            errors.push(format!("{}: {e}", folder.display()));
        }
        if let Err(e) = fs::write(folder.join("readme.txt"), "Drafts and exports for the harbor poster.\n") {
            errors.push(format!("readme.txt: {e}"));
        }
        if let Err(e) = fs::write(&txt, NOTES_TXT) {
            errors.push(format!("{}: {e}", txt.display()));
        }
        if let Some(art) = &self.art {
            if let Err(e) = png::save_pbgra(&img, art.width, art.height, &art.pixels) {
                errors.push(e);
            }
        }
        let made = [&folder, &img, &txt].iter().filter(|p| p.exists()).count();
        self.checks.push(check("shelf-files", errors.is_empty() && made == 3, if errors.is_empty() { format!("{made} items in {}", base.display()) } else { errors.join("; ") }));

        // Oldest first: add_paths puts each new path at the front.
        let unix = st.unix;
        for (k, p) in [&folder, &img, &txt].into_iter().enumerate() {
            let s = p.to_string_lossy().into_owned();
            st.store.shelf.add_paths(&[s.as_str()], unix - 3600 + k as i64 * 600);
            // Pre-seeded thumbnails keep the screenshots identical on every machine
            // (the app skips LoadThumb for paths it already has an entry for).
            let thumb = if p == &img { self.art.clone() } else { None };
            self.shelf_thumbs.push((s.clone(), thumb.clone()));
            st.shelf_thumbs.insert(s, thumb);
        }
        self.shelf_items = st.store.shelf.items.clone();
    }

    // ------------------------------------------------------------ capture + checks

    /// Frame budget shared by the supported solid themes.
    fn frame_budget(&self, _app: &App) -> (f64, usize) { (FRAME_BUDGET_MS, FRAME_SAMPLES) }

    /// Seconds a scene may take to settle (longer on a software GPU).
    fn settle_limit(&self, app: &App) -> f64 {
        if app.selftest_software_gpu() {
            SOFT_GPU_SETTLE_TIMEOUT
        } else {
            SETTLE_TIMEOUT
        }
    }

    fn capture(&mut self, app: &mut App, i: usize, settled: bool, waited: f64) {
        let scene = SCENES[i];
        let name = format!("{:02}-{}", i + 1, scene.name());
        if let Some(section) = scene.settings_section() {
            let started = Instant::now();
            match app.selftest_capture_settings(section) {
                Ok((w, h, pixels, at, scale)) => {
                    let elapsed = round2(started.elapsed().as_secs_f64() * 1000.0);
                    let valid = w > 0 && h > 0 && pixels.len() == w as usize * h as usize * 4;
                    let painted = pixels.chunks_exact(4).filter(|p| p[3] > 0).count();
                    let checks = vec![
                        check("native-settings-window", app.selftest_settings_open() && valid, format!("current Settings window: {w}x{h}")),
                        check("settings-painted", valid && painted > pixels.len() / 8, format!("{painted} visible pixels")),
                    ];
                    let ok = checks.iter().all(|c| c.ok);
                    self.queue_write(&name, w, h, pixels, scale);
                    self.states.push(StateReport {
                        index: i + 1, name: scene.name(), expanded: false, page: Some("settings"),
                        image: format!("{name}.png"), alpha_image: format!("{name}-alpha.png"), ok,
                        settle_ms: (waited * 1000.0).round().max(0.0) as u64,
                        frame_ms: elapsed, frame_samples_ms: vec![elapsed], canvas_px: [w, h],
                        window_pos_px: [at.0, at.1], body_dip: [0.0, 0.0, w as f32 / scale, h as f32 / scale],
                        measured_width_px: w, checks,
                    });
                    crate::pinfo!("selftest: [{:02}/{}] {}: {} (native Settings, {w}x{h} px)", i + 1, SCENES.len(), scene.name(), if ok { "ok" } else { "FAILED" });
                }
                Err(error) => self.checks.push(check("native-settings-window", false, format!("{}: {error}", scene.name()))),
            }
            app.selftest_close_settings();
            return;
        }
        let (budget, tries) = self.frame_budget(app);
        self.budget_ms = budget;
        let mut samples = Vec::with_capacity(tries);
        let mut shot = (0, 0, Vec::new(), (0, 0));
        for _ in 0..tries {
            self.apply(app, scene);
            let t = Instant::now();
            shot = app.selftest_capture();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            samples.push(round2(ms));
            if ms < budget {
                break;
            }
        }
        let (w, h, px, present) = shot;
        let scale = app.scale();
        let body = app.island_body_in_capture();
        let frame = Frame { w, h, px: &px };
        let best = samples.iter().copied().fold(f64::INFINITY, f64::min);

        let mut checks = vec![check("settled", settled, format!("{:.0} ms (limit {:.0} ms)", waited * 1000.0, self.settle_limit(app) * 1000.0))];
        let mut measured = 0;
        if w <= 0 || h <= 0 || px.len() != (w as usize) * (h as usize) * 4 {
            checks.push(check("pixels", false, format!("{w}x{h} frame with {} bytes", px.len())));
        } else if scene.rests_aside() {
            // Nothing drawn over the window under the island: the strip it
            // rests as is there only for the pointer (alpha 1/255 at most).
            let seen = px.chunks_exact(4).filter(|p| p[3] > 1).count();
            checks.push(check("aside-invisible", seen == 0, format!("{seen} pixels drawn with alpha above 1/255")));
            let (mode, aside) = app.selftest_mode();
            // Keep the compact width while retracting, rather than stretching
            // sideways into the Hidden idle look's fixed-width hit strip.
            let rest_w = crate::activities::geom(&crate::activities::resting_mode(&app.st), &app.st).w;
            checks.push(check(
                "aside-strip",
                mode == Mode::Aside && aside && (body.w - rest_w).abs() < 0.5 && body.h <= 8.5,
                format!("{mode:?} (stepped aside: {aside}), body {:.1}x{:.1} DIP, want compact width {rest_w:.1} and height at most 8", body.w, body.h),
            ));
        } else {
            let cx = ((body.x + body.w / 2.0) * scale).floor() as i32;
            // 3 DIPs inside the body's top edge: the window's top rows where
            // the island hangs from the edge, below the gap where it floats.
            let pill = crate::pill::on(self.theme);
            let edge = if pill { body.y } else { 0.0 };
            let top = ((edge + 3.0) * scale).round() as i32;
            let [b, g, r, a] = frame.at(cx, top);
            let ground = rgb8(lumo_core::look::of(self.theme).ground);
            let off = [r, g, b].iter().zip(ground).map(|(&p, q)| p.abs_diff(q)).max().unwrap_or(0);
            let inside = if pill { ", 3 DIPs inside the floating body" } else { "" };
            checks.push(check(
                "body-opaque-ground",
                a >= 254 && off <= 6,
                format!("pixel ({cx}, {top}) = rgba({r}, {g}, {b}, {a}), ground rgb{ground:?}{inside}"),
            ));


            let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)].map(|(x, y)| frame.at(x, y)[3]);
            checks.push(check("corners-transparent", corners.iter().all(|&a| a <= 1), format!("corner alpha {corners:?}")));

            let mid = ((body.y + body.h / 2.0) * scale).floor() as i32;
            measured = frame.opaque_run(cx, mid);
            let want = body.w * scale;
            checks.push(check(
                "body-width",
                (measured as f32 - want).abs() <= 2.0,
                format!("{measured} px opaque at row {mid}, geometry {want:.1} px"),
            ));
            if scene.expanded() {
                let ew = app.st.settings.expanded_width * scale;
                checks.push(check(
                    "expanded-width",
                    (measured as f32 - ew).abs() <= 2.0,
                    format!("{measured} px vs settings.expanded_width {:.0} x {scale:.2} = {ew:.1} px", app.st.settings.expanded_width),
                ));
            }

            let look = lumo_core::look::of(self.theme);
            let area = body;
            let lit = frame.content_pixels(area, scale, rgb8(look.ground));
            checks.push(check("content-drawn", lit >= 20, format!("{lit} content pixels inside the island")));
            if pill {
                pill_checks(&frame, body, scale, &mut checks);
            }
        }
        // Frame time counts on a real GPU only (see FRAME_BUDGET_MS).
        let soft_gpu = app.selftest_software_gpu();
        let timing = format!("best {best:.2} ms of {samples:?} (budget {budget} ms)");
        let slow = soft_gpu && best >= budget;
        if !soft_gpu {
            checks.push(check("frame-time", best < budget, timing.clone()));
        }
        checks.push(window_check(app.hwnd(), app.selftest_surface(), present, w, h));
        if let Some(want) = scene.page() {
            let page = app.st.page;
            checks.push(check("page", page == want, format!("showing '{}', expected '{}'", page.key(), want.key())));
        }
        if scene == Scene::MediaLyric {
            checks.push(lyric_line_check(&app.st));
        }
        if scene == Scene::MediaLyricShort {
            let long = self.states.iter().find(|s| s.name == "media-lyric-long").map(|s| s.body_dip[2]).unwrap_or(0.0);
            checks.push(check("lyric-width-shrinks", long > body.w + 100.0,
                format!("same track: long line {long:.1} DIP, short line {:.1} DIP", body.w)));
        }
        if scene == Scene::MusicLrcOff {
            checks.push(check("music-lyrics-with-lrc-off", !app.st.settings.island_lyrics
                && app.st.settings.lyrics && app.st.lyrics.status == LyricsStatus::Found,
                "closed lyrics off, expanded Music lyrics available"));
        }
        if scene == Scene::AsideHud {
            // Stepped aside, and still what just happened shows over the window.
            let (mode, aside) = app.selftest_mode();
            let want = if scene == Scene::AsideHud { Mode::Hud(HudTag::Volume) } else { Mode::Activity(crate::activities::Activity::Media) };
            checks.push(check("aside-still-shows", aside && mode == want, format!("{mode:?} (stepped aside: {aside}), want {want:?}")));
        }
        if scene.page() == Some(PageKind::Script) {
            // The script stays out of screen shares and recordings: ask Windows.
            let build = crate::window::windows_build();
            let can = build.is_some_and(|b| b >= crate::window::EXCLUDE_FROM_CAPTURE_BUILD);
            let hidden = crate::window::capture_excluded(app.hwnd());
            let said = if hidden { "left out of capture" } else { "in capture" };
            // Wine says Windows 10 but can't leave a window out of capture.
            let wine = under_wine();
            let note = if wine { " (Wine, which can't leave a window out)" } else { "" };
            checks.push(check("script-hidden-from-capture", hidden == can || (wine && !hidden), format!("display affinity: {said}, Windows build {build:?}{note}")));
        }

        let ok = checks.iter().all(|c| c.ok);
        crate::pinfo!(
            "selftest: [{:02}/{}] {}: {} (settle {:.0} ms, frame {best:.2} ms, {w}x{h} px)",
            i + 1,
            SCENES.len(),
            scene.name(),
            if ok { "ok" } else { "FAILED" },
            waited * 1000.0
        );
        for c in checks.iter().filter(|c| !c.ok) {
            crate::pwarn!("selftest:   {} failed: {}", c.name, c.detail);
        }
        if slow {
            crate::pwarn!("selftest:   slow frame on a software GPU, not counted: {timing}");
            self.slow_frames.push(format!("{}: {timing}", scene.name()));
        }

        if w > 0 && h > 0 && px.len() == (w as usize) * (h as usize) * 4 {
            self.queue_write(&name, w, h, px, scale);
        }
        self.states.push(StateReport {
            index: i + 1,
            name: scene.name(),
            expanded: scene.expanded(),
            page: scene.page().map(|p| p.key()),
            image: format!("{name}.png"),
            alpha_image: format!("{name}-alpha.png"),
            ok,
            settle_ms: (waited * 1000.0).round().max(0.0) as u64,
            frame_ms: round2(best),
            frame_samples_ms: samples,
            canvas_px: [w, h],
            window_pos_px: [present.0, present.1],
            body_dip: [body.x, body.y, body.w, body.h],
            measured_width_px: measured,
            checks,
        });
    }

    /// Composite + alpha PNGs, encoded off the UI thread.
    fn queue_write(&mut self, name: &str, w: i32, h: i32, px: Vec<u8>, scale: f32) {
        let dir = self.dir.clone();
        let base = name.to_string();
        let job = move || {
            let mut errors = Vec::new();
            let (cw, ch, comp) = composite(w as usize, h as usize, &px, scale);
            if let Err(e) = png::save_rgba(dir.join(format!("{base}.png")), cw, ch, &comp) {
                errors.push(e);
            }
            let alpha: Vec<u8> = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            if let Err(e) = png::save_gray(dir.join(format!("{base}-alpha.png")), w as u32, h as u32, &alpha) {
                errors.push(e);
            }
            errors
        };
        match std::thread::Builder::new().name("lumo-selftest-png".into()).spawn(job) {
            Ok(handle) => self.writes.push((name.to_string(), handle)),
            Err(e) => self.checks.push(check("png-writer", false, format!("{name}: could not start a writer thread: {e}"))),
        }
    }

    fn finish(&mut self, writers_done: bool, waited: f64) -> i32 {
        let mut write_errors = Vec::new();
        let mut written = 0;
        for (name, handle) in std::mem::take(&mut self.writes) {
            if !handle.is_finished() {
                write_errors.push(format!("{name}: still encoding"));
                continue; // detached; the process exits soon
            }
            match handle.join() {
                Ok(errs) if errs.is_empty() => written += 1,
                Ok(errs) => write_errors.extend(errs),
                Err(_) => write_errors.push(format!("{name}: writer thread panicked")),
            }
        }
        let detail = if write_errors.is_empty() {
            format!("{written} screenshot pairs written in {:.0} ms after the last capture", waited * 1000.0)
        } else {
            write_errors.join("; ")
        };
        self.checks.push(check("png-files", writers_done && write_errors.is_empty(), detail));
        // The GPU renderer presents every frame through DXGI, so the count
        // must have seen them; the software renderer presents none.
        if let Some(f) = self.frames.take() {
            let n = f.total();
            drop(f);
            if self.renderer != "software" {
                self.checks.push(check("frame-count", n > 0, format!("{n} of Lumo's own frames counted from Windows' frame log")));
            } else {
                crate::pinfo!("selftest: frame count saw {n} frames (the software renderer presents none)");
            }
        }
        if let Some(rx) = self.addon_fetch.take() {
            let (ok, detail) = rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap_or((false, "still downloading 60 s after the scenes".into()));
            self.checks.push(check("addon-download", ok, detail));
        }
        if self.states.len() < SCENES.len() {
            self.checks.push(check("states-visited", false, format!("{} of {} states captured", self.states.len(), SCENES.len())));
        }

        let mut failed: Vec<String> = self.checks.iter().filter(|c| !c.ok).map(|c| c.name.clone()).collect();
        for s in &self.states {
            failed.extend(s.checks.iter().filter(|c| !c.ok).map(|c| format!("{}: {}", s.name, c.name)));
        }
        let ok = failed.is_empty();
        let mut code = if ok { 0 } else { 1 };
        let total_ms = self.t0.elapsed().as_millis() as u64;
        let report = Report {
            app: "Lumo",
            version: env!("CARGO_PKG_VERSION"),
            ok,
            exit_code: code,
            total_ms,
            scale: self.scale,
            expanded_width_dip: self.expanded_width,
            renderer: self.renderer.to_string(),
            frame_budget_ms: self.budget_ms,
            states_passed: self.states.iter().filter(|s| s.ok).count(),
            states_total: SCENES.len(),
            failed_checks: failed.clone(),
            slow_frames: std::mem::take(&mut self.slow_frames),
            checks: std::mem::take(&mut self.checks),
            states: std::mem::take(&mut self.states),
        };
        let path = self.dir.join("report.json");
        match serde_json::to_vec_pretty(&report).map_err(|e| e.to_string()).and_then(|j| fs::write(&path, j).map_err(|e| e.to_string())) {
            Ok(()) => crate::pinfo!("selftest: report written to {}", path.display()),
            Err(e) => {
                crate::pwarn!("selftest: could not write {}: {e}", path.display());
                code = 1;
            }
        }
        if !report.slow_frames.is_empty() {
            crate::pinfo!("selftest: {} slow frame(s) on a software GPU, reported and not counted", report.slow_frames.len());
        }
        if ok {
            crate::pinfo!("selftest: all {} states passed in {total_ms} ms", report.states_total);
        } else {
            crate::pwarn!("selftest: {} check(s) failed in {total_ms} ms: {}", failed.len(), failed.join(", "));
        }
        code
    }

    /// Remove screenshots and the report from an earlier run (only our own file names).
    fn clear_old_outputs(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let b = name.as_bytes();
            let ours = name == "report.json"
                || (name.ends_with(".png") && b.len() > 3 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b'-');
            if ours && e.path().is_file() {
                if let Err(err) = fs::remove_file(e.path()) {
                    crate::pwarn!("selftest: could not remove old {name}: {err}");
                }
            }
        }
    }
}

// ---------------------------------------------------------------- fixtures

fn seed_tasks(st: &mut AppState) {
    let unix = st.unix;
    let day_start = st.day_start_unix();
    let tasks = &mut st.store.tasks;
    let done = tasks.add("Water the balcony plants", unix - 5 * 3600);
    tasks.add("Sketch the onboarding screens", unix - 3 * 3600);
    tasks.add("Book the ferry for Saturday", unix - 2 * 3600);
    if let Some(id) = done {
        tasks.toggle(id, (unix - 1800).max(day_start + 60));
    }
}

fn seed_clipboard(st: &mut AppState) {
    let unix = st.unix;
    // Oldest first, so the last one ends up at the top.
    for (k, text) in CLIPS.iter().enumerate() {
        st.clip.push(text, unix - (CLIPS.len() - k) as i64 * 420);
    }
    // Pin the colour code (index counted from the newest).
    let pin = st.clip.iter().position(|c| c.text == "#5AC8FA");
    if let Some(i) = pin {
        st.clip.toggle_pin(i);
    }
}

/// Where the media-lyric scene holds the song: 1.5 s into the first sung line
/// after the one the other music scenes hold at (so the two show different
/// lines), else into any sung line. `None` without synced lines.
fn lyric_scene_pos(l: &Lyrics) -> Option<f64> {
    if !l.synced {
        return None;
    }
    // A line with words that lasts a second or more, inside the track.
    let held = |k: usize| {
        let start = l.lines[k].time_s;
        let len = l.lines.get(k + 1).map_or(4.0, |n| n.time_s - start);
        let pos = start + (len / 2.0).min(1.5);
        (len >= 1.0 && pos < TRACK_LEN_S && !l.lines[k].text.trim().is_empty()).then_some(pos)
    };
    let after = l.line_index_at(TRACK_POS_S).map_or(0, |i| i + 1);
    (after..l.lines.len()).chain(0..after).find_map(held)
}

/// Procedural album art: a harbor sunset with a paper boat (premultiplied BGRA, opaque).
fn make_art(size: u32) -> Vec<u8> {
    const SS: usize = 3; // 3x3 supersampling for smooth edges
    let n = size.max(1) as usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0.0f32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / n as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / n as f32;
                    for (a, c) in acc.iter_mut().zip(art_sample(u, v)) {
                        *a += c;
                    }
                }
            }
            let o = (y * n + x) * 4;
            let to8 = |c: f32| ((c / (SS * SS) as f32).clamp(0.0, 1.0) * 255.0).round() as u8;
            px[o] = to8(acc[2]);
            px[o + 1] = to8(acc[1]);
            px[o + 2] = to8(acc[0]);
            px[o + 3] = 255;
        }
    }
    px
}

fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let edge = |p0: (f32, f32), p1: (f32, f32)| (p1.0 - p0.0) * (p.1 - p0.1) - (p1.1 - p0.1) * (p.0 - p0.0);
    let (d1, d2, d3) = (edge(a, b), edge(b, c), edge(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn art_sample(u: f32, v: f32) -> [f32; 3] {
    const HORIZON: f32 = 0.64;
    let (sun_x, sun_y, sun_r) = (0.66, HORIZON - 0.03, 0.15);
    let mut c = if v < HORIZON {
        mix(rgb(0xFFB37A), rgb(0xE0607E), v / HORIZON)
    } else {
        mix(rgb(0x2F6F8F), rgb(0x13233F), (v - HORIZON) / (1.0 - HORIZON))
    };
    let d = ((u - sun_x).powi(2) + (v - sun_y).powi(2)).sqrt();
    if v < HORIZON && d < sun_r {
        c = mix(c, rgb(0xFFE3A3), 0.92);
    }
    // Glitter: short streaks on the water under the sun, narrowing with depth.
    if v > HORIZON + 0.015 && v < HORIZON + 0.24 {
        let depth = (v - HORIZON) / 0.24;
        let band = ((v - HORIZON) * 42.0).fract();
        if band < 0.4 && (u - sun_x).abs() < 0.12 * (1.0 - depth) {
            c = mix(c, rgb(0xFFD08A), 0.6 * (1.0 - depth));
        }
    }
    // Paper boat: a hull trapezoid and a folded sail.
    let paper = rgb(0xF7F3EA);
    let hull = v > 0.70 && v < 0.78 && {
        let t = (v - 0.70) / 0.08;
        u > 0.17 + 0.06 * t && u < 0.51 - 0.06 * t
    };
    let sail = in_triangle((u, v), (0.34, 0.46), (0.45, 0.70), (0.24, 0.70));
    if hull {
        c = mix(paper, rgb(0xD8D0C2), ((v - 0.70) / 0.08) * 0.5);
    }
    if sail {
        c = if u > 0.34 { mix(paper, rgb(0xD9D2C4), 0.55) } else { paper };
    }
    c
}

// ---------------------------------------------------------------- pixels

/// The rows above the Pill body: the largest alpha, the largest on the top
/// row (the screen's edge), how many pixels have colour (anything but a
/// black shadow or a hit pixel) and how many are the invisible hit pixels
/// (alpha 1).
struct GapScan {
    most: u8,
    edge: u8,
    coloured: usize,
    hit: usize,
}

/// Premultiplied BGRA frame with bounds-checked reads.
struct Frame<'a> {
    w: i32,
    h: i32,
    px: &'a [u8],
}

impl Frame<'_> {
    /// `[b, g, r, a]`, or transparent outside the frame.
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return [0; 4];
        }
        let o = (y as usize * self.w as usize + x as usize) * 4;
        match self.px.get(o..o + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0; 4],
        }
    }

    /// Width of the run of mostly-opaque pixels through (`x`, `y`).
    fn opaque_run(&self, x: i32, y: i32) -> i32 {
        if self.at(x, y)[3] < 128 {
            return 0;
        }
        let (mut l, mut r) = (x, x);
        while l > 0 && self.at(l - 1, y)[3] >= 128 {
            l -= 1;
        }
        while r + 1 < self.w && self.at(r + 1, y)[3] >= 128 {
            r += 1;
        }
        r - l + 1
    }

    /// Opaque, bright pixels inside the island body (text, icons, art).
    /// Opaque pixels inside the island that stand clearly apart from its
    /// ground colour (text, icons, art).
    fn content_pixels(&self, body: Rect, scale: f32, ground: [u8; 3]) -> usize {
        let x0 = (body.x * scale).floor().max(0.0) as i32;
        let x1 = ((body.x + body.w) * scale).ceil() as i32;
        let y1 = ((body.y + body.h) * scale).ceil() as i32;
        let mut n = 0;
        for y in (body.y * scale).floor().max(0.0) as i32..y1.min(self.h) {
            for x in x0..x1.min(self.w) {
                let [b, g, r, a] = self.at(x, y);
                let off = [r, g, b].iter().zip(ground).map(|(&p, q)| p.abs_diff(q)).max().unwrap_or(0);
                if a >= 200 && off >= 140 {
                    n += 1;
                }
            }
        }
        n
    }
}

impl Frame<'_> {

    /// What rows `0..rows` hold (see [`GapScan`]).
    fn gap_scan(&self, rows: i32) -> GapScan {
        let mut g = GapScan { most: 0, edge: 0, coloured: 0, hit: 0 };
        for y in 0..rows.min(self.h) {
            for x in 0..self.w {
                let [b, gr, r, a] = self.at(x, y);
                g.most = g.most.max(a);
                if y == 0 {
                    g.edge = g.edge.max(a);
                }
                // Premultiplied: black at any alpha stays (0, 0, 0); 2 allows rounding.
                g.coloured += usize::from(r.max(gr).max(b) > 2);
                g.hit += usize::from(a == 1);
            }
        }
        g
    }

    /// The largest alpha along each edge: top, bottom, left, right.
    fn edge_alpha(&self) -> [u8; 4] {
        let row = |y: i32| (0..self.w).map(|x| self.at(x, y)[3]).max().unwrap_or(0);
        let col = |x: i32| (0..self.h).map(|y| self.at(x, y)[3]).max().unwrap_or(0);
        [row(0), row(self.h - 1), col(0), col(self.w - 1)]
    }

    /// The smallest pixel rectangle (x0, y0, x1, y1, the far edges exclusive)
    /// holding every pixel with at least `min` alpha.
    fn drawn_bounds(&self, min: u8) -> Option<(i32, i32, i32, i32)> {
        let mut b: Option<(i32, i32, i32, i32)> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.at(x, y)[3] >= min {
                    b = Some(b.map_or((x, y, x + 1, y + 1), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1))));
                }
            }
        }
        b
    }

}

/// The island window must be a visible, top-most, non-activating layered
/// tool window, laid out where the app placed it, with the frame inside it.
fn window_check(hwnd: HWND, surface: (i32, i32, i32, i32), present: (i32, i32), w: i32, h: i32) -> Check {
    let need = WS_EX_LAYERED.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0;
    let (visible, ex, rect) = unsafe {
        let visible = IsWindowVisible(hwnd).as_bool();
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let mut rc = RECT::default();
        let rect = GetWindowRect(hwnd, &mut rc).ok().map(|_| rc);
        (visible, ex, rect)
    };
    let styles_ok = ex & need == need;
    let (sx, sy, sw, sh) = surface;
    let rect_ok = rect.is_some_and(|r| {
        r.left == sx && r.top == sy && r.right - r.left == sw && r.bottom - r.top == sh
            && present.0 >= r.left && present.1 >= r.top && present.0 + w <= r.right && present.1 + h <= r.bottom
    });
    let rect_s = rect.map(|r| format!("({}, {}) {}x{}", r.left, r.top, r.right - r.left, r.bottom - r.top)).unwrap_or_else(|| "unavailable".into());
    check(
        "window",
        visible && styles_ok && rect_ok,
        format!("visible {visible}, ex-style {ex:#x} (needs {need:#x}), rect {rect_s} (laid out ({sx}, {sy}) {sw}x{sh}), frame at ({}, {}) {w}x{h}", present.0, present.1),
    )
}

/// Pill: what the gap above the floating body may hold besides the invisible
/// hit pixels. The shadow's soft upper edge reaches into it (spec 6.2: about
/// 21/255 just above the body, 3/255 at the screen's edge); it is black, at
/// most the first alpha anywhere in the gap and the second on the screen's
/// edge, so the island still reads as floating. Both at 1 = nothing but the
/// hit pixels.
const PILL_GAP_SHADOW_MAX: u8 = 64;
const PILL_GAP_EDGE_MAX: u8 = 8;

/// The Pill theme's own checks. The island floats, so the gap above the body
/// shows the screen: nothing in it but the invisible hit pixels (alpha 1)
/// that take the mouse and the shadow's soft upper edge, and the body's top
/// edge where its geometry says. Its shadow fades out inside the room the
/// window keeps beside and below the body: a shadow that reaches the window's
/// side or bottom edge is cut off there, a hard line on a light wallpaper.
/// (The top edge is the screen's, and the gap check reads it.)
fn pill_checks(frame: &Frame, body: Rect, scale: f32, checks: &mut Vec<Check>) {
    // Rows wholly above the body.
    let gap = (body.y * scale).floor() as i32;
    let g = frame.gap_scan(gap);
    let cx = ((body.x + body.w / 2.0) * scale).floor() as i32;
    let first = (0..frame.h).find(|&y| frame.at(cx, y)[3] >= 128);
    let want = body.y * scale;
    let starts = first.is_some_and(|y| (y as f32 - want).abs() <= 1.0);
    checks.push(check(
        "pill-gap",
        gap > 0 && g.most <= PILL_GAP_SHADOW_MAX && g.edge <= PILL_GAP_EDGE_MAX && g.coloured == 0 && starts,
        format!(
            "rows 0-{} above the body: max alpha {} (want <= {PILL_GAP_SHADOW_MAX}), {} on the screen's edge (want <= {PILL_GAP_EDGE_MAX}), \
             {} coloured pixels (want 0: shadow and hit pixels are black), {} hit pixels at alpha 1; the body starts at row {} (geometry {want:.1})",
            gap - 1,
            g.most,
            g.edge,
            g.coloured,
            g.hit,
            first.map_or("-".to_string(), |y| y.to_string())
        ),
    ));
    let [_, bottom, left, right] = frame.edge_alpha();
    // How far past the body anything but the hit pixels is drawn, against the room there.
    let (w, h) = (frame.w as f32 / scale, frame.h as f32 / scale);
    let reach = frame.drawn_bounds(2).map_or("nothing drawn".to_string(), |(x0, _, x1, y1)| {
        format!(
            "drawn up to {:.1} DIP left, {:.1} right and {:.1} below the body (room {:.1}, {:.1}, {:.1})",
            body.x - x0 as f32 / scale,
            x1 as f32 / scale - body.right(),
            y1 as f32 / scale - body.bottom(),
            body.x,
            w - body.right(),
            h - body.bottom()
        )
    });
    checks.push(check(
        "pill-shadow-inside",
        [left, right, bottom].iter().all(|&a| a <= 1),
        format!("edge alpha left {left}, right {right}, bottom {bottom} (want <= 1); {reach}"),
    ));
}

/// The media-lyric scene's premise: music plays and the song is inside a
/// synced line that has words.
fn lyric_line_check(st: &AppState) -> Check {
    let Some(m) = st.media.as_ref().filter(|m| m.is_playing()) else {
        return check("lyric-line", false, "no music playing");
    };
    let pos = m.timeline.position_at(st.now);
    let lyrics = st.lyrics.lyrics.as_ref().filter(|_| st.lyrics.track_key == m.track_key);
    match lyrics.and_then(|l| l.line_index_at(pos).map(|k| (k, l.lines[k].text.trim()))) {
        Some((k, text)) if !text.is_empty() => {
            // The reading the pill sings under it, when the song has them.
            let reading = st.lyrics.ruby.get(k).filter(|_| st.settings.lyrics_pronunciation).and_then(|p| lumo_core::romanize::reading(p));
            let under = reading.map(|r| format!(", read \"{r}\"")).unwrap_or_default();
            check("lyric-line", true, format!("line {} \"{text}\"{under} sung at {pos:.1} s", k + 1))
        }
        Some((k, _)) => check("lyric-line", false, format!("line {} at {pos:.1} s has no words", k + 1)),
        None => check("lyric-line", false, format!("no synced line at {pos:.1} s")),
    }
}

/// Centre of the first header tab. Mirrors `pages::draw_header`: tabs start at
/// the header's left edge and are 30 DIPs wide while inactive.
fn first_tab_point(body: Rect) -> Point {
    let header = crate::pages::header_rect(body);
    Point::new(header.x + 15.0, header.center().y)
}

fn mid_blink() -> bool {
    let phase = clock::mono() % BLINK_PERIOD;
    phase < BLINK_GUARD.0 || phase > BLINK_PERIOD - BLINK_GUARD.1
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Composite the island over a neutral desktop gradient.
fn composite(w: usize, h: usize, pbgra: &[u8], scale: f32) -> (u32, u32, Vec<u8>) {
    let pad_x = (48.0 * scale).round().max(0.0) as usize;
    let pad_b = (40.0 * scale).round().max(0.0) as usize;
    let (ow, oh) = (w + 2 * pad_x, h + pad_b);
    let (tl, tr, bl, br) = (rgb(0x5D6B84), rgb(0x837C98), rgb(0x8E8C96), rgb(0xB9A596));
    let fx = 1.0 / (ow.max(2) - 1) as f32;
    let fy = 1.0 / (oh.max(2) - 1) as f32;
    let mut out = vec![0u8; ow * oh * 4];
    for y in 0..oh {
        let v = y as f32 * fy;
        let (left, right) = (mix(tl, bl, v), mix(tr, br, v));
        for x in 0..ow {
            let bg = mix(left, right, x as f32 * fx);
            let mut c = [bg[0] * 255.0, bg[1] * 255.0, bg[2] * 255.0];
            if y < h && x >= pad_x && x < pad_x + w {
                let o = (y * w + (x - pad_x)) * 4;
                if let Some(p) = pbgra.get(o..o + 4) {
                    let inv = 1.0 - p[3] as f32 / 255.0;
                    c = [p[2] as f32 + c[0] * inv, p[1] as f32 + c[1] * inv, p[0] as f32 + c[2] * inv];
                }
            }
            let o = (y * ow + x) * 4;
            out[o] = c[0].round().clamp(0.0, 255.0) as u8;
            out[o + 1] = c[1].round().clamp(0.0, 255.0) as u8;
            out[o + 2] = c[2].round().clamp(0.0, 255.0) as u8;
            out[o + 3] = 255;
        }
    }
    (ow as u32, oh as u32, out)
}

/// A colour as 8-bit (r, g, b).
fn rgb8(c: Color) -> [u8; 3] {
    [c.r, c.g, c.b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}
