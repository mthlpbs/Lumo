//! Update state for Settings > About. Each requested network job ends;
//! recurring checks are opt-in and rest while the session is away.
use crate::icons::Icon;
use crate::state::{AppState, InlineAction, Notice, NoticeAction, UpdateStatus};
use crate::version::VERSION;
use lumo_core::geom::{palette, Color};
use lumo_core::store::{load_json, save_json_atomic};
use lumo_core::update::{self, Download, Memory, Release};
use std::path::PathBuf;

pub struct Updater {
    path: PathBuf,
    memory: Memory,
    due: Option<u64>,
    setup: Setup,
    setups: PathBuf,
    requested_download: bool,
}
#[derive(Clone, Debug, PartialEq)]
enum Setup { None, Fetching, Ready(PathBuf), Failed }

impl Updater {
    pub fn start(path: PathBuf, st: &mut AppState, now: u64) -> Self {
        Self::open(path, setup_dir(), st, now)
    }
    fn open(path: PathBuf, setups: PathBuf, st: &mut AppState, now: u64) -> Self {
        let (mut memory, _) = load_json::<Memory>(&path);
        memory.latest = memory.latest.filter(Release::allowed);
        memory.ran = VERSION.into();
        if !st.settings.automatic_updates { memory.install_on_start.clear(); }
        let due = Some(memory.next_check(now));
        let u = Self { path, memory, due, setup: Setup::None, setups, requested_download: false };
        u.show(st);
        st.update.status = if st.update.offer.is_some() { UpdateStatus::Available } else { UpdateStatus::Idle };
        u.clear_old_setups(st);
        save(&u.path, &u.memory);
        u
    }
    pub fn due_in(&self, st: &AppState, now: u64) -> Option<u64> {
        if !(st.settings.update_check || st.settings.automatic_updates) || self.busy() { return None; }
        self.due.map(|due| due.saturating_sub(now))
    }
    fn busy(&self) -> bool { self.due.is_none() || matches!(self.setup, Setup::Fetching) }
    /// Explicit checks work with both background switches off.
    pub fn begin(&mut self, st: &mut AppState) -> bool {
        if self.busy() { return false; }
        self.due = None;
        st.update.status = UpdateStatus::Checking;
        true
    }
    pub fn checked(&mut self, st: &mut AppState, result: Result<Option<Release>, String>, now: u64) {
        self.due = Some(now + if result.is_ok() { update::CHECK_EVERY_S } else { update::RETRY_S });
        match result {
            Ok(release) => {
                if self.memory.latest != release || matches!(self.setup, Setup::Failed) { self.setup = Setup::None; }
                self.memory.latest = release;
                self.memory.checked = now;
                self.show(st);
                st.update.status = if st.update.ready { UpdateStatus::Ready } else if st.update.offer.is_some() { UpdateStatus::Available } else { UpdateStatus::Current };
                self.clear_old_setups(st);
            }
            Err(error) => st.update.status = UpdateStatus::Failed(error),
        }
        save(&self.path, &self.memory);
    }
    pub fn switched(&mut self, st: &mut AppState, now: u64) {
        if !st.settings.automatic_updates {
            self.memory.install_on_start.clear();
            save(&self.path, &self.memory);
        }
        if !self.busy() && (st.settings.update_check || st.settings.automatic_updates) {
            self.due = Some(now + update::SWITCHED_ON_S);
        }
        self.show(st);
        if st.settings.automatic_updates && st.update.ready { self.install_on_next_start(st); }
    }
    pub fn request_download(&mut self, st: &mut AppState) {
        if self.busy() || st.update.offer.is_none() { return; }
        self.requested_download = true;
        if !matches!(self.setup, Setup::Ready(_)) { self.setup = Setup::None; }
    }
    pub fn want_setup(&mut self, st: &mut AppState) -> Option<(Download, PathBuf)> {
        if !st.settings.automatic_updates && !self.requested_download { return None; }
        let d = st.update.offer.as_ref()?.download()?.clone();
        if !d.allowed() || matches!(self.setup, Setup::Fetching | Setup::Ready(_) | Setup::Failed) { return None; }
        let path = self.setups.join(d.file_name());
        self.setup = Setup::Fetching;
        self.requested_download = false;
        st.update.status = UpdateStatus::Downloading;
        self.show(st);
        Some((d, path))
    }
    pub fn fetched(&mut self, st: &mut AppState, result: Result<PathBuf, String>) {
        match result {
            Ok(path) if st.update.offer.as_ref().and_then(Release::download).is_some_and(|d| path == self.setups.join(d.file_name())) => {
                self.setup = Setup::Ready(path);
                st.update.status = UpdateStatus::Ready;
                if st.settings.automatic_updates { self.install_on_next_start(st); }
            }
            Ok(_) => { self.setup = Setup::Failed; st.update.status = UpdateStatus::Failed("This download no longer matches the update. Check again.".into()); }
            Err(error) => { self.setup = Setup::Failed; st.update.status = UpdateStatus::Failed(error); }
        }
        self.show(st);
    }
    pub fn ready_setup(&self, st: &AppState) -> Option<(PathBuf, Download)> {
        let d = st.update.offer.as_ref()?.download()?;
        match &self.setup {
            Setup::Ready(path) if *path == self.setups.join(d.file_name()) && d.allowed() => Some((path.clone(), d.clone())),
            _ => None,
        }
    }
    pub fn installation_failed(&mut self, st: &mut AppState, error: String) {
        self.setup = Setup::Failed;
        st.update.status = UpdateStatus::Failed(error);
        self.show(st);
    }
    pub fn install_on_next_start(&mut self, st: &mut AppState) {
        if let Some(release) = st.update.offer.as_ref() {
            self.memory.install_on_start = release.version.clone();
            save(&self.path, &self.memory);
        }
        self.show(st);
    }
    fn clear_old_setups(&self, st: &AppState) {
        let keep = st.update.offer.as_ref().and_then(Release::download).map(Download::file_name);
        if let Ok(entries) = std::fs::read_dir(&self.setups) {
            for entry in entries.flatten() {
                if keep != entry.file_name().to_str() { let _ = std::fs::remove_file(entry.path()); }
            }
        }
    }
    pub fn announce(&mut self, st: &mut AppState, free: bool) -> bool {
        if !free || !st.settings.update_check || !self.memory.should_announce(VERSION) { return false; }
        let Some(release) = st.update.offer.clone() else { return false };
        st.notice = Some(new_version_notice(&release, st.accent, st.now + 15.0));
        self.memory.announced = release.version;
        save(&self.path, &self.memory);
        true
    }
    fn show(&self, st: &mut AppState) {
        st.update.offer = self.memory.offer(VERSION).cloned();
        st.update.checked = self.memory.checked;
        st.update.ready = self.ready_setup(st).is_some();
        st.update.next_start = st.update.offer.as_ref().is_some_and(|r| r.version == self.memory.install_on_start);
        st.update.sheet = false;
    }
}
fn setup_dir() -> PathBuf { crate::paths::cache_dir().join("update") }

pub fn pending_install() -> bool {
    if !update::ENABLED || crate::autostart::packaged() { return false; }
    let (settings, _) = load_json::<lumo_core::settings::Settings>(&crate::paths::settings_path());
    if !settings.automatic_updates { return false; }
    let path = crate::paths::data_dir().join("update.json");
    let (mut memory, _) = load_json::<Memory>(&path);
    let Some(download) = memory.pending_install(VERSION).filter(|r| r.allowed()).and_then(Release::download).cloned() else { return false };
    if crate::services::power::query().saver { return false; }
    memory.install_on_start.clear();
    save(&path, &memory);
    match crate::services::update::install(&setup_dir().join(download.file_name()), &download) {
        Ok(()) => true,
        Err(error) => { crate::pwarn!("updates: couldn't install on start ({error})"); false }
    }
}
pub fn new_version_notice(release: &Release, accent: Color, until: f64) -> Notice {
    Notice { icon: Icon::Download, color: accent, title: "Update available".into(), title_dim: release.version.clone(),
        subtitle: "Open Settings > About to update Lumo".into(), until, action: Some(NoticeAction::OpenUpdate), inline: Some(InlineAction::Hide) }
}
pub fn updated_notice(headline: Option<String>, until: f64) -> Notice {
    Notice { icon: Icon::CheckCircle, color: palette::GREEN, title: format!("Updated to {VERSION}"), title_dim: String::new(),
        subtitle: headline.unwrap_or_else(|| "Lumo is up to date".into()), until, action: Some(NoticeAction::OpenUpdate), inline: None }
}
fn save(path: &PathBuf, memory: &Memory) {
    if let Err(error) = save_json_atomic(path, memory) { crate::pwarn!("updates: couldn't save {} ({error})", path.display()); }
}
pub fn unix_now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

#[cfg(test)]
mod tests {
    use super::*;
    use lumo_core::{settings::Settings, store::Store};
    fn fixture(name: &str) -> (PathBuf, AppState, Updater) {
        let dir = std::env::temp_dir().join(format!("lumo-update-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(dir.join("setups")).unwrap();
        let version = format!("{VERSION}.1");
        let bytes = b"test installer";
        let download = Download { url: format!("https://github.com/mthlpbs/lumo/releases/download/v{version}/Lumo-Setup-{version}.exe"), size: bytes.len() as u64, sha256: lumo_core::sha256::hex(&lumo_core::sha256::sha256(bytes)) };
        let latest = Release { version: version.clone(), date: String::new(), headline: "Next version".into(), notes: format!("https://github.com/mthlpbs/lumo/releases/tag/v{version}"), windows: Some(download) };
        save(&dir.join("update.json"), &Memory { latest: Some(latest), ..Memory::default() });
        let mut st = AppState::new(Settings::default(), Store::default());
        let u = Updater::open(dir.join("update.json"), dir.join("setups"), &mut st, 10);
        (dir, st, u)
    }
    #[test]
    fn manual_check_works_off_and_duplicate_checks_are_ignored() {
        let (dir, mut st, mut u) = fixture("manual");
        assert_eq!(u.due_in(&st, 20), None);
        assert!(u.begin(&mut st));
        assert!(!u.begin(&mut st));
        u.checked(&mut st, Ok(None), 30);
        assert_eq!(st.update.status, UpdateStatus::Current);
        assert!(st.update.offer.is_none());
        assert_eq!(u.due_in(&st, 31), None);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn notifications_do_not_download_and_are_announced_once() {
        let (dir, mut st, mut u) = fixture("notify");
        st.settings.update_check = true;
        assert!(u.want_setup(&mut st).is_none());
        assert!(u.announce(&mut st, true));
        assert!(!u.announce(&mut st, true));
        assert!(u.due_in(&st, 20).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn automatic_download_is_once_and_switching_off_cancels_next_launch() {
        let (dir, mut st, mut u) = fixture("auto");
        st.settings.automatic_updates = true;
        let (_, path) = u.want_setup(&mut st).unwrap();
        assert!(u.want_setup(&mut st).is_none());
        u.fetched(&mut st, Ok(path));
        assert!(st.update.ready && st.update.next_start);
        st.settings.automatic_updates = false;
        u.switched(&mut st, 40);
        assert!(!st.update.next_start);
        assert_eq!(u.due_in(&st, 41), None);
        let (memory, _) = load_json::<Memory>(&dir.join("update.json"));
        assert!(memory.install_on_start.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn manual_download_can_retry_without_enabling_background_work() {
        let (dir, mut st, mut u) = fixture("retry");
        u.request_download(&mut st);
        assert!(u.want_setup(&mut st).is_some());
        u.fetched(&mut st, Err("offline".into()));
        assert!(u.want_setup(&mut st).is_none());
        u.request_download(&mut st);
        let (download, path) = u.want_setup(&mut st).unwrap();
        std::fs::write(&path, b"test installer").unwrap();
        u.fetched(&mut st, Ok(path.clone()));
        assert!(st.update.ready && !st.update.next_start);
        std::fs::write(&path, b"changed").unwrap();
        assert!(crate::services::update::install(&path, &download).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
