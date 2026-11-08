//! Local search preferences, strict name matching and bounded usage history.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub horizontal_scroll: bool,
    pub apps: bool,
    pub settings: bool,
    pub web: bool,
    pub engine: Engine,
    pub keep_open: bool,
    pub resume_page: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            horizontal_scroll: true,
            apps: true,
            settings: true,
            web: false,
            engine: Engine::Bing,
            keep_open: false,
            resume_page: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    #[default]
    Bing,
    Google,
    DuckDuckGo,
    Brave,
}
impl Engine {
    pub const ALL: [Self; 4] = [Self::Bing, Self::Google, Self::DuckDuckGo, Self::Brave];
    pub fn name(self) -> &'static str {
        match self {
            Self::Bing => "Bing",
            Self::Google => "Google",
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Brave => "Brave",
        }
    }
    pub fn url(self, query: &str) -> Option<String> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        let base = match self {
            Self::Bing => "https://www.bing.com/search?q=",
            Self::Google => "https://www.google.com/search?q=",
            Self::DuckDuckGo => "https://duckduckgo.com/?q=",
            Self::Brave => "https://search.brave.com/search?q=",
        };
        let mut url = base.to_string();
        for b in q.chars().take(128).collect::<String>().bytes() {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                url.push(b as char);
            } else {
                use std::fmt::Write;
                let _ = write!(url, "%{b:02X}");
            }
        }
        Some(url)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    App,
    Settings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowsSetting {
    Home,
    Bluetooth,
    Display,
    Sound,
    Network,
    Apps,
    Startup,
    Updates,
    Personalization,
    Storage,
    Wifi,
    Notifications,
    Mouse,
    Keyboard,
    Printers,
    Accounts,
    DateTime,
    Language,
    Accessibility,
    Privacy,
    Camera,
    Microphone,
    Power,
    About,
    Background,
    LockScreen,
    Colors,
    Taskbar,
    DefaultApps,
    OptionalFeatures,
    Recovery,
    Clipboard,
    Multitasking,
    Proxy,
    VPN,
}
impl WindowsSetting {
    pub const ALL: [Self; 35] = [
        Self::Home,
        Self::Bluetooth,
        Self::Display,
        Self::Sound,
        Self::Network,
        Self::Apps,
        Self::Startup,
        Self::Updates,
        Self::Personalization,
        Self::Storage,
        Self::Wifi,
        Self::Notifications,
        Self::Mouse,
        Self::Keyboard,
        Self::Printers,
        Self::Accounts,
        Self::DateTime,
        Self::Language,
        Self::Accessibility,
        Self::Privacy,
        Self::Camera,
        Self::Microphone,
        Self::Power,
        Self::About,
        Self::Background,
        Self::LockScreen,
        Self::Colors,
        Self::Taskbar,
        Self::DefaultApps,
        Self::OptionalFeatures,
        Self::Recovery,
        Self::Clipboard,
        Self::Multitasking,
        Self::Proxy,
        Self::VPN,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Windows Settings",
            Self::Bluetooth => "Bluetooth & devices",
            Self::Display => "Display",
            Self::Sound => "Sound",
            Self::Network => "Network & internet",
            Self::Apps => "Installed apps",
            Self::Startup => "Startup apps",
            Self::Updates => "Windows Update",
            Self::Personalization => "Personalization",
            Self::Storage => "Storage",
            Self::Wifi => "Wi-Fi",
            Self::Notifications => "Notifications",
            Self::Mouse => "Mouse",
            Self::Keyboard => "Keyboard",
            Self::Printers => "Printers & scanners",
            Self::Accounts => "Accounts",
            Self::DateTime => "Date & time",
            Self::Language => "Language & region",
            Self::Accessibility => "Accessibility",
            Self::Privacy => "Privacy",
            Self::Camera => "Camera privacy",
            Self::Microphone => "Microphone privacy",
            Self::Power => "Power & battery",
            Self::About => "About your PC",
            Self::Background => "Desktop background",
            Self::LockScreen => "Lock screen",
            Self::Colors => "Colors",
            Self::Taskbar => "Taskbar",
            Self::DefaultApps => "Default apps",
            Self::OptionalFeatures => "Optional features",
            Self::Recovery => "Recovery",
            Self::Clipboard => "Clipboard",
            Self::Multitasking => "Multitasking",
            Self::Proxy => "Proxy",
            Self::VPN => "VPN",
        }
    }
    pub fn uri(self) -> &'static str {
        match self {
            Self::Home => "ms-settings:",
            Self::Bluetooth => "ms-settings:bluetooth",
            Self::Display => "ms-settings:display",
            Self::Sound => "ms-settings:sound",
            Self::Network => "ms-settings:network",
            Self::Apps => "ms-settings:appsfeatures",
            Self::Startup => "ms-settings:startupapps",
            Self::Updates => "ms-settings:windowsupdate",
            Self::Personalization => "ms-settings:personalization",
            Self::Storage => "ms-settings:storagesense",
            Self::Wifi => "ms-settings:network-wifi",
            Self::Notifications => "ms-settings:notifications",
            Self::Mouse => "ms-settings:mousetouchpad",
            Self::Keyboard => "ms-settings:easeofaccess-keyboard",
            Self::Printers => "ms-settings:printers",
            Self::Accounts => "ms-settings:yourinfo",
            Self::DateTime => "ms-settings:dateandtime",
            Self::Language => "ms-settings:regionlanguage",
            Self::Accessibility => "ms-settings:easeofaccess",
            Self::Privacy => "ms-settings:privacy",
            Self::Camera => "ms-settings:privacy-webcam",
            Self::Microphone => "ms-settings:privacy-microphone",
            Self::Power => "ms-settings:powersleep",
            Self::About => "ms-settings:about",
            Self::Background => "ms-settings:personalization-background",
            Self::LockScreen => "ms-settings:lockscreen",
            Self::Colors => "ms-settings:colors",
            Self::Taskbar => "ms-settings:taskbar",
            Self::DefaultApps => "ms-settings:defaultapps",
            Self::OptionalFeatures => "ms-settings:optionalfeatures",
            Self::Recovery => "ms-settings:recovery",
            Self::Clipboard => "ms-settings:clipboard",
            Self::Multitasking => "ms-settings:multitasking",
            Self::Proxy => "ms-settings:network-proxy",
            Self::VPN => "ms-settings:network-vpn",
        }
    }
    pub fn keywords(self) -> &'static str {
        match self {
            Self::Wifi => "wifi wireless internet",
            Self::Display => "screen resolution brightness monitor",
            Self::Sound => "audio volume speaker headphones",
            Self::Storage => "disk space drives",
            Self::Apps => "uninstall programs applications",
            Self::Power => "sleep energy battery saver",
            Self::About => "system information specifications",
            Self::Updates => "update upgrade",
            Self::Accounts => "user password sign in",
            Self::Printers => "printing scanning",
            Self::Accessibility => "ease access",
            _ => "",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    App(Application),
    Windows(WindowsSetting),
    Web { query: String, url: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub group: Group,
    pub target: Target,
    title_key: String,
    words: String,
}
impl Entry {
    pub fn new(
        id: String,
        title: String,
        subtitle: String,
        keywords: &str,
        group: Group,
        target: Target,
    ) -> Self {
        Self {
            id,
            title_key: normalize(&title),
            words: normalize(keywords),
            title,
            subtitle,
            group,
            target,
        }
    }
    pub fn enabled(&self, cfg: &Config) -> bool {
        match self.group {
            Group::App => cfg.apps,
            Group::Settings => cfg.settings,
        }
    }
}
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}
fn score_keys(title: &str, words: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return None;
    }
    if title == query {
        return Some(0);
    }
    if title.starts_with(query) {
        return Some(5);
    }
    let mut score = 10;
    for token in query.split_whitespace() {
        score += if title
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word.starts_with(token))
        {
            5
        } else if title.contains(token) {
            15
        } else if words.split_whitespace().any(|word| word.starts_with(token)) {
            30
        } else {
            return None;
        };
    }
    Some(score)
}
pub fn ranked(entries: &[Entry], query: &str, group: Option<Group>, cfg: &Config) -> Vec<usize> {
    let query = normalize(&query.trim().chars().take(128).collect::<String>());
    let mut found: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.enabled(cfg) && group.is_none_or(|g| e.group == g))
        .filter_map(|(i, e)| score_keys(&e.title_key, &e.words, &query).map(|s| (s, i)))
        .collect();
    found.sort_by_key(|&(s, i)| (s, i));
    found.into_iter().take(100).map(|(_, i)| i).collect()
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub count: u32,
    pub last: u64,
}
pub fn record_usage(usage: &mut BTreeMap<String, Usage>, id: String, now: u64) {
    if !usage.contains_key(&id) && usage.len() >= 128 {
        if let Some(key) = usage
            .iter()
            .min_by_key(|(_, u)| (u.count, u.last))
            .map(|(k, _)| k.clone())
        {
            usage.remove(&key);
        }
    }
    let u = usage.entry(id).or_default();
    u.count = u.count.saturating_add(1);
    u.last = now;
}
pub fn frequent(entries: &[Entry], cfg: &Config, usage: &BTreeMap<String, Usage>) -> Vec<usize> {
    let mut out: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            usage
                .get(&e.id)
                .filter(|u| u.count > 0 && e.enabled(cfg))
                .map(|u| (i, u.count, u.last))
        })
        .collect();
    out.sort_by_key(|&(i, count, last)| (std::cmp::Reverse(count), std::cmp::Reverse(last), i));
    out.into_iter().take(100).map(|(i, _, _)| i).collect()
}
/// One opening per physical horizontal gesture. Inertia and direction changes
/// cannot repeatedly reopen it after dismissal; a 350 ms quiet gap rearms it.
#[derive(Default)]
pub struct Gesture {
    last: Option<f64>,
    total: i32,
    fired: bool,
}
impl Gesture {
    pub fn feed(&mut self, delta: i16, now: f64) -> bool {
        if delta == 0 {
            return false;
        }
        if self.last.is_none_or(|t| now - t > 0.35) {
            self.total = 0;
            self.fired = false;
        }
        self.last = Some(now);
        if self.fired {
            return false;
        }
        if self.total.signum() != i32::from(delta).signum() {
            self.total = 0;
        }
        self.total = self.total.saturating_add(i32::from(delta));
        if self.total.abs() < 40 {
            return false;
        }
        self.fired = true;
        true
    }
}

/// An opaque Windows catalog item. It cannot be constructed from a command,
/// path or URI typed into Search. The native worker supplies its ID-list bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Application(Vec<u8>);
impl std::fmt::Debug for Application {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Application({} bytes)", self.0.len())
    }
}
impl Application {
    pub fn from_id_list(bytes: Vec<u8>) -> Option<Self> {
        if bytes.len() < 4 || bytes.len() > 16384 {
            return None;
        }
        let mut offset = 0;
        loop {
            let length =
                u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize;
            if length == 0 {
                return (offset > 0 && offset + 2 == bytes.len()).then_some(Self(bytes));
            }
            if length < 2 {
                return None;
            }
            offset = offset.checked_add(length)?;
        }
    }
    pub fn id_list(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_file_preferences_are_ignored_without_changing_web_preferences() {
        let cfg: Config = serde_json::from_str(
            r#"{"files":true,"locations":["C:\\Old"],"web":true,"engine":"Google"}"#,
        )
        .unwrap();
        assert!(cfg.web);
        assert_eq!(cfg.engine, Engine::Google);
        let saved = serde_json::to_value(cfg).unwrap();
        assert!(saved.get("files").is_none() && saved.get("locations").is_none());
    }
    fn entry(name: &str) -> Entry {
        Entry::new(
            name.into(),
            name.into(),
            "Installed application".into(),
            "",
            Group::App,
            Target::Windows(WindowsSetting::Home),
        )
    }
    #[test]
    fn names_match_without_fuzzy_or_generic_metadata_false_positives() {
        let items = vec![
            entry("Visual Studio Code"),
            entry("Café Notes"),
            entry("Media Player"),
        ];
        let cfg = Config::default();
        assert_eq!(ranked(&items, "studio code", None, &cfg), vec![0]);
        assert_eq!(ranked(&items, "cafe", None, &cfg), vec![1]);
        assert_eq!(ranked(&items, "media", None, &cfg), vec![2]);
        for q in [
            "",
            "vsc",
            "cne",
            "installed application",
            "missing-987",
            "media browser",
        ] {
            assert!(ranked(&items, q, None, &cfg).is_empty(), "{q}");
        }
    }
    #[test]
    fn frequent_is_empty_until_used_and_respects_sources() {
        let items = vec![entry("A"), entry("B")];
        let mut usage = BTreeMap::new();
        let mut cfg = Config::default();
        assert!(frequent(&items, &cfg, &usage).is_empty());
        record_usage(&mut usage, "B".into(), 1);
        record_usage(&mut usage, "A".into(), 2);
        record_usage(&mut usage, "B".into(), 3);
        assert_eq!(frequent(&items, &cfg, &usage), vec![1, 0]);
        cfg.apps = false;
        assert!(frequent(&items, &cfg, &usage).is_empty());
        for i in 0..200 {
            record_usage(&mut usage, format!("{i}"), i);
        }
        assert!(usage.len() <= 128);
    }
    #[test]
    fn web_urls_encode_input_and_do_not_accept_commands() {
        for engine in Engine::ALL {
            let url = engine.url("a &b=\"猫\"").unwrap();
            assert!(url.starts_with("https://"));
            assert!(url.contains("%26b%3D%22%E7%8C%AB%22"));
            assert!(!url.contains(' '));
            assert!(engine.url(" ").is_none());
        }
    }
    #[test]
    fn gesture_suppresses_inertia() {
        for delta in [-20, 20] {
            let mut g = Gesture::default();
            assert!(!g.feed(delta, 0.0));
            assert!(g.feed(delta, 0.02));
            assert!(!g.feed(-120, 0.1));
            assert!(g.feed(120, 0.8));
        }
    }
    #[test]
    fn application_ids_reject_plain_commands() {
        assert!(Application::from_id_list(vec![4, 0, 10, 20, 0, 0]).is_some());
        for b in [
            vec![0, 0],
            vec![8, 0, 10, 20, 0, 0],
            b"cmd.exe /c whoami".to_vec(),
        ] {
            assert!(Application::from_id_list(b).is_none());
        }
    }
}
