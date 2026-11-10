//! Local search preferences, strict name matching and bounded usage history.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub horizontal_scroll: bool,
    pub apps: bool,
    pub settings: bool,
    pub services: bool,
    pub powertoys: bool,
    #[serde(alias="app_actions")]
    pub vscode_commands: bool,
    pub web: bool,
    pub engine: Engine,
    pub keep_open: bool,
    pub resume_page: bool,
    pub browser_wide: bool,
    pub browser_height: u16,
    pub google_suggestions: bool,
    pub mini_width: u16,
    pub mini_height: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            horizontal_scroll: true,
            apps: true,
            settings: true,
            services: false,
            powertoys: true,
            vscode_commands: true,
            web: false,
            engine: Engine::Bing,
            keep_open: false,
            resume_page: false,
            browser_wide: false,
            browser_height: 680,
            google_suggestions: false,
            mini_width: 560,
            mini_height: 680,
        }
    }
}
impl Config {
    pub fn sanitize(&mut self) {
        self.browser_height=self.browser_height.clamp(360,1200);
        self.mini_width=self.mini_width.clamp(280,8192);
        self.mini_height=self.mini_height.clamp(200,8192);
    }
}

pub const MAX_SUGGESTIONS: usize = 4;
pub fn google_suggestions_url(query:&str)->Option<String> {
    let query=query.trim();
    if query.chars().count()<2 || query.chars().any(char::is_control){return None;}
    let search=Engine::Google.url(query)?;
    Some(format!("https://suggestqueries.google.com/complete/search?client=firefox&ie=UTF-8&oe=UTF-8&q={}",search.split_once("?q=")?.1))
}
pub fn parse_google_suggestions(body:&str,query:&str)->Vec<String> {
    if body.len()>32768{return vec![];}
    let Ok(value)=serde_json::from_str::<serde_json::Value>(body)else{return vec![];};
    if value.get(0).and_then(|v|v.as_str())!=Some(query){return vec![];}
    let Some(items)=value.get(1).and_then(|v|v.as_array())else{return vec![];};
    let mut out:Vec<String>=vec![];
    for item in items.iter().take(20) {
        let Some(text)=item.as_str().map(str::trim)else{continue;};
        if text.is_empty() || text.chars().count()>128 || text.chars().any(char::is_control) || out.iter().any(|s|s.eq_ignore_ascii_case(text)){continue;}
        out.push(text.into());if out.len()==MAX_SUGGESTIONS{break;}
    }
    out
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
    Service,
    PowerToys,
    VSCode,
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
    Service(Service),
    Launch {path:String,arguments:String},
    PowerToy {event:String},
    AppCommand {window:u64,pid:u32,keys:Vec<u16>},
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {pub name:String,pub state:u32,pub accepts_stop:bool}
impl Service {
    pub fn status(&self)->&'static str{match self.state{1=>"Stopped",2=>"Starting",3=>"Stopping",4=>"Running",5=>"Continuing",6=>"Pausing",7=>"Paused",_=>"Unknown"}}
    pub fn can_start(&self)->bool{self.state==1}
    pub fn can_stop(&self)->bool{matches!(self.state,4|7)&&self.accepts_stop}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub group: Group,
    pub target: Target,
    /// Optional local tool executable read by the on-demand icon worker.
    pub icon_path: Option<String>,
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
            icon_path: None,
        }
    }
    pub fn enabled(&self, cfg: &Config) -> bool {
        match self.group {
            Group::App => cfg.apps,
            Group::Settings => cfg.settings,
            Group::Service => cfg.services,
            Group::PowerToys => cfg.powertoys,
            Group::VSCode => cfg.vscode_commands,
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
/// Suppress short inertial tails, while deliberate reversals and sustained
/// motion can start another action without requiring a finger lift.
#[derive(Default)]
pub struct Gesture {
    last: Option<f64>,
    total: i32,
    fired: bool,
    fired_at: f64,
    direction: i32,
}
impl Gesture {
    pub fn feed(&mut self, delta: i16, now: f64) -> bool {
        if delta == 0 || !now.is_finite() {
            return false;
        }
        if self.last.is_none_or(|t| now < t || now - t > 0.35) {
            self.total = 0;
            self.fired = false;
        }
        self.last = Some(now);
        if self.fired {
            let reverse=i32::from(delta).signum()!=self.direction;
            let ready=now-self.fired_at>=if reverse{0.20}else{0.65};
            if !ready {self.total=0;return false;}
            if self.total.signum()!=i32::from(delta).signum(){self.total=0;}
            self.total=self.total.saturating_add(i32::from(delta));
            if self.total.abs()<if reverse{60}else{180}{return false;}
            self.total=0;self.direction=i32::from(delta).signum();self.fired_at=now;
            return true;
        }
        if self.total.signum() != i32::from(delta).signum() {
            self.total = 0;
        }
        self.total = self.total.saturating_add(i32::from(delta));
        if self.total.abs() < 40 {
            return false;
        }
        self.fired = true;
        self.fired_at=now;self.direction=i32::from(delta).signum();self.total=0;
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
    #[test]
    fn service_controls_and_retired_preferences_are_safe(){
        let stopped=Service{name:"fixture".into(),state:1,accepts_stop:false};assert!(stopped.can_start()&&!stopped.can_stop());
        let running=Service{state:4,accepts_stop:true,..stopped.clone()};assert!(running.can_stop()&&!running.can_start());
        assert!(!Service{accepts_stop:false,..running}.can_stop());
        for state in [2,3,5,6]{let service=Service{state,..stopped.clone()};assert!(!service.can_start()&&!service.can_stop());}
        let config:Config=serde_json::from_str(r#"{"app_actions":false,"browser_content":true,"custom_actions":[{"name":"old","target":"vscode://settings/editor.fontSize"}]}"#).unwrap();
        assert!(!config.vscode_commands && !config.services);
        let saved=serde_json::to_string(&config).unwrap();
        for retired in ["app_actions","browser_content","custom_actions"]{assert!(!saved.contains(retired));}
        assert_eq!(serde_json::from_str::<Config>(&saved).unwrap(),config);
        assert!(Config::default().vscode_commands);

    }
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
    fn suggestions_are_opt_in_bounded_and_match_the_requested_query(){
        let old:Config=serde_json::from_str(r#"{"web":true}"#).unwrap();
        assert!(!old.google_suggestions && !old.browser_wide);
        let retired:Config=serde_json::from_str(r#"{"desktop_sites":true}"#).unwrap();
        assert_eq!(retired,Config::default());
        assert!(google_suggestions_url("x").is_none());assert!(google_suggestions_url("ab\ncd").is_none());
        assert!(google_suggestions_url("cat & dog").unwrap().ends_with("q=cat%20%26%20dog"));
        let body=r#"["cat",["cat food","CAT FOOD","cat videos","cat\ncommand","cat toys","cats","cat six"],[]]"#;
        assert_eq!(parse_google_suggestions(body,"cat"),["cat food","cat videos","cat toys","cats"]);
        assert!(parse_google_suggestions(body,"dog").is_empty());
        for malformed in ["null","{}",r#"["cat",{}]"#,"not json"]{assert!(parse_google_suggestions(malformed,"cat").is_empty());}
        let mut c=old;c.mini_width=0;c.mini_height=65535;c.browser_height=1;c.sanitize();
        assert_eq!((c.mini_width,c.mini_height,c.browser_height),(280,8192,360));
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
    fn gesture_rearms_without_lifting_after_deliberate_reversal_or_continued_motion() {
        let mut g=Gesture::default();assert!(g.feed(80,0.0));
        for (d,t) in [(80,0.02),(40,0.06),(20,0.12),(10,0.18)]{assert!(!g.feed(d,t));}
        assert!(!g.feed(-30,0.24));assert!(g.feed(-40,0.26),"direction reversal needs no quiet gap");
        assert!(!g.feed(-80,0.3));assert!(!g.feed(-10,0.45));assert!(!g.feed(-10,0.6));assert!(!g.feed(-10,0.75));
        assert!(!g.feed(-80,0.93));assert!(!g.feed(-80,0.95));assert!(g.feed(-40,0.97),"a renewed sustained stroke can fire while contact continues");
        assert!(!g.feed(120,f64::NAN));
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
