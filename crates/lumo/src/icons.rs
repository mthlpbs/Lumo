//! Vector icons drawn on a 24×24 grid as SVG path data (original artwork).
//!
//! Stroked parts use a 2-unit round-capped stroke; filled parts are solid.
//! Geometry is parsed once and cached by [`crate::gfx::Gfx`].

pub const STROKE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Search,
    Globe,
    ArrowRight,
    ArrowLeft,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Refresh,
    ExternalLink,
    ResumePage,
    NewChat,
    TemporaryChat,
    TemporaryChatChecked,
    Markdown,
    ExpandDown,
    CollapseUp,
    Sparkles,
    Send,
    Play,
    Pause,
    Next,
    Prev,
    Music,
    Lyrics,
    PopOut,
    PopIn,
    MiniPlayer,
    ExitMiniPlayer,
    Backward,
    Forward,
    Timer,
    Coffee,
    Tray,
    ListChecks,
    Check,
    CheckCircle,
    Circle,
    Plus,
    Minus,
    Close,
    Clipboard,
    Copy,
    Clock,
    Cpu,
    Gpu,
    Memory,
    Disk,
    Battery,
    Bolt,
    Bluetooth,
    Headphones,
    Volume,
    VolumeLow,
    VolumeMute,
    Pin,
    PinFill,
    /// Remember visibility over the app underneath the island.
    KeepVisible,
    Folder,
    File,
    Image,
    Star,
    Trash,
    Undo,
    Reset,
    Sliders,
    Filter,
    VsCode,
    Eyedropper,
    Open,
    Phone,
    /// The header's Settings button.
    Gear,
    /// The System tab: a dial with its needle.
    Gauge,
    /// An app whose own icon isn't known.
    App,
    /// The Windows mark, for Windows' own processes in the System page's lists.
    Windows,
    /// The Script tab: lines of text with a cue arrow at the line to say.
    Script,
    Moon,
    Stop,
    /// A new version: an arrow down into a tray.
    Download,
    Upload,
    /// Record (Lumo Recorder): a dot that fits inside `Circle`.
    Record,
}

pub struct Part {
    pub d: &'static str,
    pub fill: bool,
}

const fn s(d: &'static str) -> Part {
    Part { d, fill: false }
}

const fn f(d: &'static str) -> Part {
    Part { d, fill: true }
}

const CIRCLE_9: &str = "M3 12a9 9 0 1 0 18 0a9 9 0 1 0-18 0";
const SPEAKER: &str = "M4 9.5h3l4.5-4v13L7 14.5H4z";
const TEMPORARY_CHAT: &str = "M8 3.8a8.5 8.5 0 0 1 6-.5M17 4.6a8.5 8.5 0 0 1 3 4.9M20.3 12.8a8.5 8.5 0 0 1-3.5 5.3M13.5 20a8.5 8.5 0 0 1-5.7-.6L3 21l1.2-4M3.6 13.5a8.5 8.5 0 0 1 1.4-6";

/// Builds a `&'static [Part]` (const-promoted so it can be returned).
macro_rules! p {
    ($($part:expr),* $(,)?) => {{
        const PARTS: &[Part] = &[$($part),*];
        PARTS
    }};
}

pub fn parts(icon: Icon) -> &'static [Part] {
    use Icon::*;
    match icon {
        ChevronDown => p![s("M6 9l6 6 6-6")],
        Star => p![f("M12 2.5l2.9 5.9 6.5.9-4.7 4.6 1.1 6.5-5.8-3.1-5.8 3.1 1.1-6.5L2.6 9.3l6.5-.9z")],
        Image => p![s("M3 4h18v16H3zM3 16l5-5 5 5 3-3 5 5"),s("M15 8h.01")],
        NewChat => p![s("M12 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6"),s("M10 14l1-4 7-7 3 3-7 7-4 1zM16 5l3 3")],
        TemporaryChat => p![s(TEMPORARY_CHAT)],
        TemporaryChatChecked => p![s(TEMPORARY_CHAT),s("M8.5 11.5l2.5 2.5 5-5")],
        Markdown => p![s("M2 6h20v12H2zM5 15V9l3 3 3-3v6M17 9v6M14.5 12.5L17 15l2.5-2.5")],
        ExpandDown => p![s("M6 5l6 6 6-6M6 13l6 6 6-6")],
        CollapseUp => p![s("M6 11l6-6 6 6M6 19l6-6 6 6")],
        Sparkles => p![s("M12 2l2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5z"),s("M20 2v4M18 4h4")],
        Send => p![s("M12 20V4M5 11l7-7 7 7")],
        Play => p![f("M8 5.6v12.8a1.1 1.1 0 0 0 1.7.93l10-6.4a1.1 1.1 0 0 0 0-1.86l-10-6.4A1.1 1.1 0 0 0 8 5.6z")],
        Pause => p![
            f("M7.2 5h1.6a1.2 1.2 0 0 1 1.2 1.2v11.6a1.2 1.2 0 0 1-1.2 1.2H7.2A1.2 1.2 0 0 1 6 17.8V6.2A1.2 1.2 0 0 1 7.2 5z"),
            f("M15.2 5h1.6a1.2 1.2 0 0 1 1.2 1.2v11.6a1.2 1.2 0 0 1-1.2 1.2h-1.6a1.2 1.2 0 0 1-1.2-1.2V6.2A1.2 1.2 0 0 1 15.2 5z"),
        ],
        Next => p![
            f("M4.5 6.3v11.4a.9.9 0 0 0 1.4.74l8.3-5.7a.9.9 0 0 0 0-1.48L5.9 5.56a.9.9 0 0 0-1.4.74z"),
            f("M17 5.5h1.6a.9.9 0 0 1 .9.9v11.2a.9.9 0 0 1-.9.9H17a.9.9 0 0 1-.9-.9V6.4a.9.9 0 0 1 .9-.9z"),
        ],
        Prev => p![
            f("M19.5 6.3v11.4a.9.9 0 0 1-1.4.74l-8.3-5.7a.9.9 0 0 1 0-1.48l8.3-5.7a.9.9 0 0 1 1.4.74z"),
            f("M5.4 5.5H7a.9.9 0 0 1 .9.9v11.2a.9.9 0 0 1-.9.9H5.4a.9.9 0 0 1-.9-.9V6.4a.9.9 0 0 1 .9-.9z"),
        ],
        Music => p![s("M9 18V5.5l11-2V16"), s("M3 18a3 3 0 1 0 6 0a3 3 0 1 0-6 0"), s("M14 16a3 3 0 1 0 6 0a3 3 0 1 0-6 0")],
        Lyrics => p![
            s("M5.5 4h13A2.5 2.5 0 0 1 21 6.5v9a2.5 2.5 0 0 1-2.5 2.5H10l-5 3v-3.1A2.5 2.5 0 0 1 3 15.5v-9A2.5 2.5 0 0 1 5.5 4z"),
            f("M7 8h3v3H8.7c0 1-.5 1.8-1.7 2.4v-1.3c.5-.3.8-.7.8-1.1H7zM13 8h3v3h-1.3c0 1-.5 1.8-1.7 2.4v-1.3c.5-.3.8-.7.8-1.1H13z"),
        ],
        PopOut => p![s("M14 4h6v6M20 4l-7 7M10 20H4v-6M4 20l7-7")],
        PopIn => p![s("M20 4l-6 6M14 4v6h6M4 20l6-6M4 14h6v6")],
        MiniPlayer => p![s("M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM6 7l4 4M10 7v4H6"), f("M12 12h6v5h-6z")],
        ExitMiniPlayer => p![s("M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM10 11L6 7M6 11V7h4"), f("M12 12h6v5h-6z")],
        Backward => p![
            f("M11.5 5.5v13l-10-6.5z"), f("M22 5.5v13L12 12z"),
        ],
        Forward => p![
            f("M2 5.5v13L12 12z"), f("M12.5 5.5v13l10-6.5z"),
        ],
        Timer => p![s("M4 13a8 8 0 1 0 16 0a8 8 0 1 0-16 0"), s("M12 9v4l2.5 2.5"), s("M10 2.5h4"), s("M19 5.5l1.5 1.5")],
        Coffee => p![
            s("M5 9h11v5a5 5 0 0 1-5 5h-1a5 5 0 0 1-5-5z"),
            s("M16 10.5h1.5a2.5 2.5 0 0 1 0 5H16"),
            s("M8.5 3.5V6"),
            s("M12 3.5V6"),
        ],
        Tray => p![
            s("M4 13l2.4-7.2a1.6 1.6 0 0 1 1.5-1.1h8.2a1.6 1.6 0 0 1 1.5 1.1L20 13"),
            s("M4 13v5a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5"),
            s("M4 13h4.5l1.5 2.5h4l1.5-2.5H20"),
        ],
        ListChecks => p![
            s("M11 6h9"),
            s("M11 12h9"),
            s("M11 18h9"),
            s("M3.5 6l1.6 1.6L8 4.7"),
            s("M3.5 12l1.6 1.6L8 10.7"),
            s("M4 18h3"),
        ],
        Check => p![s("M5 12.5l4.5 4.5L19 7.5")],
        CheckCircle => p![s(CIRCLE_9), s("M8 12.3l2.8 2.8L16.3 9.5")],
        Circle => p![s(CIRCLE_9)],
        Plus => p![s("M12 5v14"), s("M5 12h14")],
        Minus => p![s("M5 12h14")],
        Close => p![s("M6.5 6.5l11 11"), s("M17.5 6.5l-11 11")],
        Clipboard => p![
            s("M9 3.5h6a1 1 0 0 1 1 1V6a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V4.5a1 1 0 0 1 1-1z"),
            s("M16 5h1.5A1.5 1.5 0 0 1 19 6.5v13a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-13A1.5 1.5 0 0 1 6.5 5H8"),
        ],
        Copy => p![
            s("M9.5 8.5h9A1.5 1.5 0 0 1 20 10v9.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 8 19.5V10a1.5 1.5 0 0 1 1.5-1.5z"),
            s("M16 8.5v-3A1.5 1.5 0 0 0 14.5 4h-9A1.5 1.5 0 0 0 4 5.5V15a1.5 1.5 0 0 0 1.5 1.5H8"),
        ],
        Globe => p![s("M21 12a9 9 0 1 1-18 0a9 9 0 1 1 18 0"),s("M3 12h18"),s("M12 3c5 5 5 13 0 18c-5-5-5-13 0-18")],
        ArrowRight => p![s("M5 12h14M13 6l6 6-6 6")],
        ArrowLeft => p![s("M19 12H5M11 6l-6 6 6 6")],
        ChevronLeft => p![s("M15 5l-7 7 7 7")],
        ChevronRight => p![s("M9 5l7 7-7 7")],
        Refresh => p![s("M20 10a8 8 0 1 0-2 8M20 4v6h-6")],
        ExternalLink => p![s("M14 4h6v6M20 4L10 14M10 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4")],
        ResumePage => p![s("M14 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h7M14 3v6h6L14 3zM20 13v4h-4M20 17a5 5 0 1 0-1 5")],
        Search => p![s("M17 10.5a6.5 6.5 0 1 1-13 0a6.5 6.5 0 1 1 13 0"),s("M15.5 15.5L21 21")],
        Clock => p![s(CIRCLE_9), s("M12 7.5V12l3 2")],
        Gpu => p![s("M3 6h16v11H3zM19 9h2v5h-2M6 17v3M10 17v3M14 17v3"),s("M6 11.5a3 3 0 1 0 6 0a3 3 0 1 0-6 0M15 9v5")],
        Upload => p![s("M12 14.5V4"),s("M7.5 8.5L12 4l4.5 4.5"),s("M4.5 16v2.5A1.5 1.5 0 0 0 6 20h12a1.5 1.5 0 0 0 1.5-1.5V16")],
        Cpu => p![
            s("M7.5 6h9A1.5 1.5 0 0 1 18 7.5v9a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 6 16.5v-9A1.5 1.5 0 0 1 7.5 6z"),
            s("M10 10h4v4h-4z"),
            s("M9.5 3v3M14.5 3v3M9.5 18v3M14.5 18v3M3 9.5h3M3 14.5h3M18 9.5h3M18 14.5h3"),
        ],
        Memory => p![
            s("M4 7.5h16a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1z"),
            s("M7 11v2M10.5 11v2M14 11v2M17.5 11v2"),
            s("M6 16.5v2M18 16.5v2"),
        ],
        Disk => p![
            s("M4 14l2.5-8A1.5 1.5 0 0 1 8 5h8a1.5 1.5 0 0 1 1.5 1l2.5 8"),
            s("M4 14h16v4a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18z"),
            s("M16.5 16.8h.01"),
        ],
        Battery => p![s("M4.5 7h12A1.5 1.5 0 0 1 18 8.5v7a1.5 1.5 0 0 1-1.5 1.5h-12A1.5 1.5 0 0 1 3 15.5v-7A1.5 1.5 0 0 1 4.5 7z"), s("M21 10.5v3")],
        Bolt => p![f("M13.4 2.5L5 13.6h6.2l-1.3 7.9L19 10.4h-6.3z")],
        Bluetooth => p![s("M7 7.5l10 9L12 21V3l5 4.5-10 9")],
        Headphones => p![
            s("M4 18v-5a8 8 0 0 1 16 0v5"),
            s("M4 15.5A1.5 1.5 0 0 1 5.5 14H7a1 1 0 0 1 1 1v4.5a1 1 0 0 1-1 1H5.5A1.5 1.5 0 0 1 4 19z"),
            s("M20 15.5a1.5 1.5 0 0 0-1.5-1.5H17a1 1 0 0 0-1 1v4.5a1 1 0 0 0 1 1h1.5A1.5 1.5 0 0 0 20 19z"),
        ],
        Volume => p![s(SPEAKER), s("M15 9a4 4 0 0 1 0 6"), s("M17.8 6.5a7.5 7.5 0 0 1 0 11")],
        VolumeLow => p![s(SPEAKER), s("M15 9a4 4 0 0 1 0 6")],
        VolumeMute => p![s(SPEAKER), s("M16 9.5l5 5"), s("M21 9.5l-5 5")],
        Pin => p![s("M9 4h6l-1 5.5 3 3V14H7v-1.5l3-3z"), s("M12 14v6")],
        PinFill => p![f("M9 4h6l-1 5.5 3 3V14H7v-1.5l3-3z"), s("M12 14v6")],
        KeepVisible => p![s("M20 11V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h6M4 8h16M14 17l3 3 5-6")],
        Folder => p![s("M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z")],
        File => p![s("M7.5 3H14l5 5v11.5a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 6 19.5v-15A1.5 1.5 0 0 1 7.5 3z"), s("M14 3v5h5")],
        Trash => p![
            s("M4 7h16"),
            s("M9.5 7V5a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1v2"),
            s("M6.5 7l.9 12a1.5 1.5 0 0 0 1.5 1.4h6.2a1.5 1.5 0 0 0 1.5-1.4L17.5 7"),
        ],
        Undo => p![s("M9 14L4.5 9.5 9 5"), s("M4.5 9.5H14a5.5 5.5 0 0 1 0 11h-3")],
        Reset => p![s("M4 12a8 8 0 1 0 2.4-5.7"), s("M4 4v4.5h4.5")],
        VsCode => p![s("M16 3L22 6V18L16 21V3ZM16 5L5 15L2 12L5 9L16 19")],
        Eyedropper => p![s("M14 4L20 10M12 6L18 12M15 5L18 2L22 6L19 9M14 8L4 18L3 22L7 21L17 11")],
        Filter => p![s("M3 5H21L14 13V19L10 21V13Z")],
        Sliders => p![
            s("M4 6h10M18 6h2M16 4v4"),
            s("M4 12h4M12 12h8M10 10v4"),
            s("M4 18h12M20 18h.01M18 16v4"),
        ],
        Open => p![s("M14 4h6v6"), s("M20 4l-9 9"), s("M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10")],
        Phone => p![s("M9 2.5h6A2.5 2.5 0 0 1 17.5 5v14a2.5 2.5 0 0 1-2.5 2.5H9A2.5 2.5 0 0 1 6.5 19V5A2.5 2.5 0 0 1 9 2.5z"), s("M11 18.5h2")],
        Gauge => p![s("M4.6 18.5A8.5 8.5 0 1 1 19.4 18.5"), s("M12 14.5l3.8-4.3"), f("M10.6 14.5a1.4 1.4 0 1 0 2.8 0a1.4 1.4 0 1 0-2.8 0")],
        App => p![
            s("M6.5 4h11A2.5 2.5 0 0 1 20 6.5v11a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 17.5v-11A2.5 2.5 0 0 1 6.5 4z"),
            s("M4 8.5h16"),
        ],
        Windows => p![f("M4 4h7.5v7.5H4z"), f("M12.5 4H20v7.5h-7.5z"), f("M4 12.5h7.5V20H4z"), f("M12.5 12.5H20V20h-7.5z")],
        // Eight teeth (tops on r 9.5, valleys on r 7) around a hole. The outline
        // starts at a tooth's corner, not mid-curve, so its closing join can't spike.
        Gear => p![
            s("M10.25 5.22L10.43 2.63A9.5 9.5 0 0 1 13.57 2.63L13.75 5.22A7 7 0 0 1 15.55 5.97L17.52 4.27A9.5 9.5 0 0 1 19.73 6.48L18.03 8.45A7 7 0 0 1 18.78 10.25L21.37 10.43A9.5 9.5 0 0 1 21.37 13.57L18.78 13.75A7 7 0 0 1 18.03 15.55L19.73 17.52A9.5 9.5 0 0 1 17.52 19.73L15.55 18.03A7 7 0 0 1 13.75 18.78L13.57 21.37A9.5 9.5 0 0 1 10.43 21.37L10.25 18.78A7 7 0 0 1 8.45 18.03L6.48 19.73A9.5 9.5 0 0 1 4.27 17.52L5.97 15.55A7 7 0 0 1 5.22 13.75L2.63 13.57A9.5 9.5 0 0 1 2.63 10.43L5.22 10.25A7 7 0 0 1 5.97 8.45L4.27 6.48A9.5 9.5 0 0 1 6.48 4.27L8.45 5.97A7 7 0 0 1 10.25 5.22Z"),
            s("M9 12a3 3 0 1 0 6 0a3 3 0 1 0-6 0"),
        ],
        // A circle (r 8) less a second one (r 7) up and to the right: both arcs meet where the circles cross.
        Moon => p![s("M19.77 13.9A8 8 0 1 1 10.1 4.23A7 7 0 0 0 19.77 13.9Z")],
        Script => p![s("M10 6.5h10"), s("M10 12h10"), s("M10 17.5h6.5"), f("M3.6 9.3v5.4a.8.8 0 0 0 1.2.7l4.3-2.7a.8.8 0 0 0 0-1.4L4.8 8.6a.8.8 0 0 0-1.2.7z")],
        Stop => p![f("M8.2 6.5h7.6a1.7 1.7 0 0 1 1.7 1.7v7.6a1.7 1.7 0 0 1-1.7 1.7H8.2a1.7 1.7 0 0 1-1.7-1.7V8.2a1.7 1.7 0 0 1 1.7-1.7z")],
        Record => p![f("M6.5 12a5.5 5.5 0 1 0 11 0a5.5 5.5 0 1 0-11 0z")],
        Download => p![s("M12 4v10.5"), s("M7.5 10L12 14.5 16.5 10"), s("M4.5 16v2.5A1.5 1.5 0 0 0 6 20h12a1.5 1.5 0 0 0 1.5-1.5V16")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[Icon] = &[
        Icon::Play, Icon::Pause, Icon::Next, Icon::Prev, Icon::Music, Icon::Lyrics, Icon::PopOut, Icon::PopIn, Icon::MiniPlayer, Icon::ExitMiniPlayer, Icon::Timer, Icon::Coffee,
        Icon::Tray, Icon::ListChecks, Icon::Check, Icon::CheckCircle, Icon::Circle, Icon::Plus, Icon::Minus,
        Icon::Close, Icon::Clipboard, Icon::Copy, Icon::Search, Icon::Globe, Icon::ArrowRight, Icon::ArrowLeft, Icon::ChevronLeft, Icon::ChevronRight, Icon::Refresh, Icon::ExternalLink, Icon::ResumePage, Icon::Clock, Icon::Cpu, Icon::Memory, Icon::Disk,
        Icon::Battery, Icon::Bolt, Icon::Bluetooth, Icon::Headphones, Icon::Volume, Icon::VolumeLow, Icon::VolumeMute,
        Icon::Pin, Icon::PinFill, Icon::KeepVisible, Icon::Folder, Icon::File, Icon::Trash, Icon::Undo, Icon::Reset,
        Icon::Sliders, Icon::Filter, Icon::VsCode, Icon::Eyedropper, Icon::Open, Icon::Phone,
        Icon::Gear,
        Icon::Gauge, Icon::App, Icon::Windows, Icon::Moon, Icon::Stop, Icon::Script, Icon::Record, Icon::Download,Icon::Gpu,Icon::Upload,
    ];

    #[test]
    fn every_icon_parses() {
        for &i in ALL {
            for p in parts(i) {
                lumo_core::svgpath::parse(p.d).unwrap_or_else(|e| panic!("{i:?}: {}", e.0));
            }
        }
    }
}
