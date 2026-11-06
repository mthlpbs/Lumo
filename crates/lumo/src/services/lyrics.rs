//! Local music can try sidecar and embedded lyrics, then LRCLIB (https://lrclib.net).
//! Music streaming services use LRCLIB directly. Other online lookups can use
//! Apple Music when the user saved their token, with a disk cache.
//!
//! LRCLIB is a free, open lyrics database (no key). One worker thread does
//! all the work; [`LyricsService::request`] only swaps the queued job and
//! returns. The latest request wins: a newer track replaces any queued job,
//! aborts the running lookup at its next step, and results for a track that
//! is no longer the newest are dropped.
//!
//! Lookup for a track:
//! 1. clean the metadata (`lrclib::normalize_query`);
//! 2. with an Apple Music token: the cache `lyrics\am-<cache_key>.json`, then
//!    Apple's own synced lyrics (`services::applemusic`); anything short of
//!    synced lyrics from Apple falls through to LRCLIB;
//! 3. the disk cache `lyrics\<cache_key>.json` (hits, and misses for 3 days
//!    unless a better lookup has shipped since: `CacheEntry::LOOKUP`);
//! 4. `/api/get` with title, artist, album and duration;
//! 5. `/api/get` without the album;
//! 6. `/api/search` by title + primary artist, then `lrclib::best_match`
//!    (also after a 5xx from `/api/get`; a miss then is retried, not cached);
//! 7. for a browser tab, `/api/search` again reading the title as
//!    "Artist - Song", then as "Song - Artist" (`lrclib::title_guesses`).
//!    A 5xx from one search doesn't stop the next.
//!
//! A plain-only answer doesn't end steps 4-7: it is kept while the later
//! steps look for a synced copy of the song.
//!
//! Synced lyrics whose recording is much longer or shorter than what plays
//! (a music video's intro, a radio edit) are shown unsynced.
//!
//! Politeness: at least 1 s between requests, `429 Retry-After` honoured for
//! every later request, a descriptive User-Agent, and at most a couple of
//! delayed retries after network errors.

use crate::events::{AppEvent, AppleMusicStatus, EventSink, LyricsResult};
use crate::services::applemusic::{AppleMusic, Check, Miss};
use crate::services::http::{self, Client, FetchError, MtaGuard};
use crate::state::MediaSession;
use lumo_core::lrclib::{self, LrclibRecord, TrackQuery};
use lumo_core::nowplaying;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pure::{duration_secs, is_fresh, lyrics_for, prune_victims, retry_delay, usable, CacheEntry};

// LRCLIB asks apps to say who they are.
const USER_AGENT: &str = concat!("Lumo/", env!("CARGO_PKG_VERSION"), " (+https://github.com/mthlpbs/lumo)");
const SOURCE: &str = "LRCLIB";
const APPLE_SOURCE: &str = "Apple Music";
/// Minimum gap between two requests to LRCLIB.
const MIN_GAP: Duration = Duration::from_secs(1);
/// 429 without a usable `Retry-After`.
const DEFAULT_BACKOFF: Duration = Duration::from_secs(60);
/// Upper bound for any server-requested back-off.
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);
/// Waits up to this long happen inline; longer ones fail the lookup now and
/// retry it later, so the UI isn't left on "loading".
const MAX_INLINE_WAIT: Duration = Duration::from_secs(10);
/// Delayed retries after transient failures (while the track is still current).
const MAX_RETRIES: u32 = 2;
/// Cache size bounds: prune to `CACHE_KEEP` files once over `CACHE_MAX`.
const CACHE_MAX: usize = 2000;
const CACHE_KEEP: usize = 1600;
/// Re-check the cache size after this many writes (long-running sessions).
const PRUNE_EVERY_WRITES: u32 = 200;

const MSG_BUSY: &str = "LRCLIB is busy right now";

#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::events::SendHwnd;
    use lumo_core::nowplaying::MusicSource;
    use lumo_core::timeline::Timeline;

    struct Library(PathBuf);
    impl Library {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("lumo-lyrics-routing-{}-{stamp}", std::process::id()));
            fs::create_dir_all(path.join("cache")).unwrap();
            Self(path)
        }
    }
    impl Drop for Library { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    fn session(id: &str, name: &str) -> MediaSession {
        MediaSession {
            app_id: id.into(), app_name: name.into(), music_source: nowplaying::music_source(id, name, Some(true)),
            title: "Paper Boats".into(), artist: "The Lanterns".into(), album: "Original test fixture".into(),
            art: None, art_color: None, timeline: Timeline::new(0.0, 201.0, 0.0, true),
            can_prev: true, can_next: true, can_play_pause: true, can_seek: true,
            track_key: MediaSession::make_track_key("Paper Boats", "The Lanterns", "Original test fixture"),
        }
    }

    fn worker(library: &Library, shared: Arc<Shared>) -> Worker {
        let (tx, _rx) = std::sync::mpsc::channel();
        Worker { shared, sink: EventSink::new(tx, SendHwnd(0)), cache_dir: library.0.join("cache"),
            apple: AppleMusic::new(), client: None, last_request: None, blocked_until: None, writes: 0 }
    }

    fn cache_song(worker: &Worker, job: &Job) {
        // Original placeholder text, not song lyrics; no network needed.
        let record = LrclibRecord { id: 1, track_name: job.query.title.clone(), artist_name: job.query.artist.clone(),
            album_name: Some(job.query.album.clone()), duration: Some(201.0), instrumental: false,
            plain_lyrics: None, synced_lyrics: Some("[00:01]Cached first\n[00:02]Cached second\n[00:03]Cached third\n[00:04]Cached fourth".into()) };
        let path = worker.cache_dir.join(format!("{}.json", lrclib::cache_key(&lrclib::normalize_query(&job.query))));
        write_cache(&path, &CacheEntry::new(Some(record), crate::clock::unix_now()));
    }

    fn assert_source(result: Result<LyricsResult, Halt>, expected: &str, local: bool) {
        match result {
            Ok(LyricsResult::Found { source, source_file, .. }) => {
                assert_eq!(source, expected);
                assert_eq!(source_file.is_some(), local);
            }
            _ => panic!("expected lyrics from {expected}"),
        }
    }

    #[test]
    fn downloaded_lyrics_save_to_the_chosen_folder_and_are_reused_for_local_music() {
        let library=Library::new();let downloads=Library::new();
        let service=LyricsService{shared:Arc::new(Shared::default())};
        let mut worker=worker(&library,service.shared.clone());
        let local=session("MusicBee.exe","MusicBee");
        fs::write(library.0.join("01 - Paper Boats.mp3"),[]).unwrap();
        let path=downloads.0.join("01 - Paper Boats.lrc");
        service.request(&local,Some(library.0.clone()),Some(downloads.0.clone()),false, true);
        let job=service.shared.lock().pending.take().unwrap();cache_song(&worker,&job);
        assert_source(worker.lookup(&job),SOURCE,false);assert!(!path.exists(),"saving is explicitly switchable");
        service.request(&local,Some(library.0.clone()),Some(downloads.0.clone()),true, true);
        let job=service.shared.lock().pending.take().unwrap();
        assert_source(worker.lookup(&job),SOURCE,false);assert!(path.exists());
        assert!(lumo_core::local_lyrics::load_file(&path).unwrap().lyrics.synced);
        service.request(&local,Some(library.0.clone()),Some(downloads.0.clone()),false, true);
        let job=service.shared.lock().pending.take().unwrap();
        assert_source(worker.lookup(&job),"Downloaded from LRCLIB",true);
        fs::write(&path,"Personal lyrics").unwrap();
        service.request(&session("Spotify.exe","Spotify"),Some(library.0.clone()),Some(downloads.0.clone()),true, true);
        let job=service.shared.lock().pending.take().unwrap();
        assert!(job.local_dir.is_none() && job.saved_dir.is_none());
        assert_source(worker.lookup(&job),SOURCE,false);
        assert_eq!(fs::read_to_string(&path).unwrap(),"Personal lyrics","streaming must neither read nor replace local lyrics");
        service.request(&local,None,None,true, true);
        let job=service.shared.lock().pending.take().unwrap();assert!(job.save_dir.is_none());
        service.cancel();assert!(matches!(worker.lookup(&job),Err(Halt::Abandoned)));
        assert!(worker.client.is_none(),"downloads from the existing cache need no additional network request");
    }

    #[test]
    fn default_music_folder_saves_missing_lyrics_and_folder_errors_do_not_break_playback() {
        let library=Library::new();let service=LyricsService{shared:Arc::new(Shared::default())};
        let mut worker=worker(&library,service.shared.clone());
        let local=session("MusicBee.exe","MusicBee");
        fs::write(library.0.join("01 - Paper Boats.mp3"),[]).unwrap();
        service.request(&local,Some(library.0.clone()),Some(library.0.clone()),true, true);
        let job=service.shared.lock().pending.take().unwrap();cache_song(&worker,&job);
        assert_source(worker.lookup(&job),SOURCE,false);
        assert!(library.0.join("01 - Paper Boats.lrc").exists());
        let invalid=library.0.join("not-a-folder");fs::write(&invalid,"Keep this file").unwrap();
        service.request(&session("Spotify.exe","Spotify"),None,Some(invalid.clone()),true, true);
        let job=service.shared.lock().pending.take().unwrap();
        assert_source(worker.lookup(&job),SOURCE,false);
        assert_eq!(fs::read_to_string(invalid).unwrap(),"Keep this file");
    }

    #[test]
    #[ignore="requires an existing local audio/LRC pair in LUMO_SIDECAR_AUDIO; read-only"]
    fn existing_decorated_sidecar_is_loaded_without_network() {
        let audio=PathBuf::from(std::env::var_os("LUMO_SIDECAR_AUDIO").expect("existing audio file required"));
        let root=audio.parent().unwrap().to_path_buf();
        let tags=file_tags(&audio).expect("existing audio metadata");
        let matched=lumo_core::local_lyrics::files::find(&root,&tags.title,&tags.artist,&tags.album,||true,file_tags).unwrap();
        assert_eq!(matched,Some(audio.clone()));
        let sidecar=audio.with_extension("lrc");
        let local=lumo_core::local_lyrics::load_file(&sidecar).unwrap();
        assert!(local.lyrics.synced && local.lyrics.text_lines()>1);
        let library=Library::new();let service=LyricsService{shared:Arc::new(Shared::default())};let mut worker=worker(&library,service.shared.clone());
        let mut current=session("MusicBee.exe","MusicBee");current.title=tags.title;current.artist=tags.artist;current.album=tags.album;
        current.track_key=MediaSession::make_track_key(&current.title,&current.artist,&current.album);
        service.request(&current,Some(root),None,false,true);
        let job=service.shared.lock().pending.take().unwrap();cache_song(&worker,&job);
        match worker.lookup(&job){
            Ok(LyricsResult::Found{source,source_file,lyrics,..})=>{
                assert_eq!(source,"Local files");assert_eq!(source_file,Some(sidecar.to_string_lossy().into_owned()));assert_eq!(lyrics,local.lyrics);
            }
            _=>panic!("the existing sidecar must win over the online cache"),
        }
        assert!(worker.client.is_none(),"no network client should be created for an existing sidecar");
    }

    #[test]
    #[ignore="requires independently encoded audio fixtures in LUMO_FILENAME_FIXTURES"]
    fn real_encoded_tracks_resolve_original_filenames_and_reuse_downloads() {
        let root=PathBuf::from(std::env::var_os("LUMO_FILENAME_FIXTURES").expect("fixture folder required"));
        for ext in ["mp3","flac","m4a","opus","ogg"] {
            let music=root.join(ext);let song=music.join(format!("01 - original recording.{ext}"));
            let tags=file_tags(&song).expect("Windows should expose music properties");
            assert_eq!(tags.title,"Paper Boats","{ext}: title");assert_eq!(tags.artist,"The Lanterns","{ext}: artist");
            let downloads=music.join("downloads");fs::create_dir_all(&downloads).unwrap();
            let library=Library::new();let service=LyricsService{shared:Arc::new(Shared::default())};let mut worker=worker(&library,service.shared.clone());
            service.request(&session("MusicBee.exe","MusicBee"),Some(music.clone()),Some(downloads.clone()),true,true);
            let job=service.shared.lock().pending.take().unwrap();cache_song(&worker,&job);
            assert_source(worker.lookup(&job),SOURCE,false);
            let sidecar=downloads.join("01 - original recording.lrc");assert!(sidecar.exists(),"{ext}: exact basename");
            assert!(lumo_core::local_lyrics::load_file(&sidecar).unwrap().downloaded);
            service.request(&session("MusicBee.exe","MusicBee"),Some(music),Some(downloads),false,true);
            let job=service.shared.lock().pending.take().unwrap();assert_source(worker.lookup(&job),"Downloaded from LRCLIB",true);
            assert!(worker.client.is_none());
        }
    }

    #[test]
    fn embedded_local_lyrics_win_over_cached_online_lyrics_but_streaming_skips_them() {
        let library=Library::new();
        let text=b"\x03eng\0[00:01]Embedded first\n[00:02]Embedded second";
        let mut frame=b"USLT".to_vec();frame.extend_from_slice(&(text.len() as u32).to_be_bytes());frame.extend_from_slice(&[0,0]);frame.extend_from_slice(text);
        let mut audio=b"ID3\x03\0\0".to_vec();audio.extend_from_slice(&[0,0,0,frame.len() as u8]);audio.extend(frame);
        let path=library.0.join("Paper Boats.mp3");fs::write(&path,audio).unwrap();
        let service=LyricsService{shared:Arc::new(Shared::default())};
        service.request(&session("MusicBee.exe","MusicBee"),Some(library.0.clone()),None, false, true);
        let mut job=service.shared.lock().pending.take().unwrap();
        let mut worker=worker(&library,service.shared.clone());cache_song(&worker,&job);
        assert_source(worker.lookup(&job),"Embedded lyrics",true);
        assert_source(worker.lookup(&job),"Embedded lyrics",true);
        service.request(&session("Spotify.exe","Spotify"),Some(library.0.clone()),Some(path.clone()), false, true);
        let job=service.shared.lock().pending.take().unwrap();cache_song(&worker,&job);
        assert_source(worker.lookup(&job),SOURCE,false);
        // A local file without any tags still falls through without network work.
        fs::write(path,b"No tags here").unwrap();
        service.request(&session("MusicBee.exe","MusicBee"),Some(library.0.clone()),None, false, true);
        let job=service.shared.lock().pending.take().unwrap();
        assert_source(worker.lookup(&job),SOURCE,false);
        assert!(worker.client.is_none());
    }

    #[test]
    fn local_files_win_and_missing_or_unusable_files_fall_back_to_lrclib() {
        let library = Library::new();
        let service = LyricsService { shared: Arc::new(Shared::default()) };
        let local = library.0.join("Paper Boats.lrc");
        fs::write(&local, "[00:01]Local first\n[00:02]Local second").unwrap();
        service.request(&session("MusicBee.exe", "MusicBee"), Some(library.0.clone()), None, false, true);
        let mut job = service.shared.lock().pending.take().unwrap();
        let mut worker = worker(&library, service.shared.clone());
        cache_song(&worker, &job);
        assert_source(worker.lookup(&job), "Local files", true);
        fs::remove_file(&local).unwrap();
        assert_source(worker.lookup(&job), SOURCE, false);
        fs::write(&local, "").unwrap();
        assert_source(worker.lookup(&job), SOURCE, false);
        assert_source(worker.lookup(&job), SOURCE, false);
        job.local_dir = Some(library.0.join("missing-folder"));
        assert_source(worker.lookup(&job), SOURCE, false);
        assert!(worker.client.is_none(), "cached fallback must not create a network client");
        service.cancel();
        assert!(matches!(worker.lookup(&job), Err(Halt::Abandoned)));
    }

    #[test]
    fn streaming_jobs_never_read_local_paths_and_use_lrclib() {
        let library = Library::new();
        let local = library.0.join("Paper Boats.lrc");
        fs::write(&local, "[00:01]Wrong local recording").unwrap();
        let service = LyricsService { shared: Arc::new(Shared::default()) };
        let mut worker = worker(&library, service.shared.clone());
        for (id, name) in [("Spotify.exe", "Spotify"), ("Deezer.exe", "Deezer"), ("Chrome", "YouTube Music")] {
            let s = session(id, name);
            assert_eq!(s.music_source, Some(MusicSource::Streaming));
            service.request(&s, Some(library.0.clone()), Some(local.clone()), false, true);
            let job = service.shared.lock().pending.take().unwrap();
            assert!(job.local_dir.is_none() && job.saved_dir.is_none() && job.lrclib_only);
            cache_song(&worker, &job);
            assert_source(worker.lookup(&job), SOURCE, false);
        }
        assert!(worker.client.is_none());
    }
}

pub struct LyricsService {
    shared: Arc<Shared>,
}

impl LyricsService {
    pub fn start(sink: EventSink) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("lumo-lyrics".into())
            .spawn(move || worker_main(worker, sink))
            .map_err(|e| io::Error::other(format!("could not start the lyrics thread: {e}")))?;
        Ok(Self { shared })
    }

    /// Queue a lookup for `s` and return immediately (UI thread safe). The
    /// answer arrives as `AppEvent::Lyrics` with `s.track_key`.
    pub fn request(&self, s: &MediaSession, local_dir: Option<PathBuf>, download_dir: Option<PathBuf>, auto_save: bool, use_local:bool) -> u64 {
        let match_dir=local_dir.clone();
        let local_dir = local_dir.filter(|_| use_local && s.music_source == Some(nowplaying::MusicSource::Local));
        let saved_dir = local_dir.as_ref().and_then(|dir|download_dir.as_ref().filter(|saved|*saved!=dir)).cloned();
        let mut job = Job {
            id: 0,
            lrclib_only: s.music_source.is_some(),
            local_dir,
            match_dir,
            saved_dir,
            save_dir: download_dir.filter(|_|auto_save),
            track_key: s.track_key.clone(),
            query: TrackQuery {
                title: s.title.clone(),
                artist: s.artist.clone(),
                album: s.album.clone(),
                duration_s: duration_secs(s.timeline.duration_s),
            },
            browser: nowplaying::is_browser(&s.app_id),
            attempt: 0,
        };
        let id;
        {
            let mut q = self.shared.lock();
            q.newest_id = q.newest_id.wrapping_add(1).max(1);
            id = q.newest_id;
            job.id = id;
            q.pending = Some(job);
            q.retry = None;
        }
        self.shared.cv.notify_all();
        id
    }

    pub fn cancel(&self) {
        let mut q = self.shared.lock();
        q.newest_id = q.newest_id.wrapping_add(1).max(1);
        q.pending = None;
        q.retry = None;
        self.shared.cv.notify_all();
    }

    /// The Apple Music token on disk was saved or removed: the worker reloads
    /// it, checks it with Apple and reports `AppEvent::AppleMusic`.
    pub fn apple_token_changed(&self) {
        self.shared.lock().apple_changed = true;
        self.shared.cv.notify_all();
    }
}

impl Drop for LyricsService {
    fn drop(&mut self) {
        // The worker exits at its next wait; a request in flight finishes
        // (bounded by the HTTP timeout) on its own. Never join from the UI thread.
        self.shared.lock().shutdown = true;
        self.shared.cv.notify_all();
    }
}

// ------------------------------------------------------------------ queue

#[derive(Clone, Debug)]
struct Job {
    id: u64,
    /// Streaming and local-file fallback must go straight to LRCLIB.
    lrclib_only: bool,
    local_dir: Option<PathBuf>,
    match_dir: Option<PathBuf>,
    saved_dir: Option<PathBuf>,
    save_dir: Option<PathBuf>,
    track_key: String,
    query: TrackQuery,
    /// A browser tab: the title is a video title ("Artist - Song").
    browser: bool,
    /// 0 for a fresh request, n for the n-th delayed retry.
    attempt: u32,
}

/// What the worker should do next.
enum Work {
    Lookup(Job),
    /// Reload and check the Apple Music token.
    AppleToken,
}

#[derive(Default)]
struct Queue {
    /// The next job; a new request replaces it (latest wins).
    pending: Option<Job>,
    /// A delayed retry of the current track, run once its time comes.
    retry: Option<(Instant, Job)>,
    /// Track key of the newest request; anything else is stale.
    newest_id: u64,
    /// The Apple Music token was saved or removed (handled before any job).
    apple_changed: bool,
    shutdown: bool,
}

#[derive(Default)]
struct Shared {
    q: Mutex<Queue>,
    cv: Condvar,
}

impl Shared {
    /// The queue holds plain data, so a panic elsewhere can't leave it
    /// inconsistent: recover from poisoning instead of panicking.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.q.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wait_for<'a>(&self, g: MutexGuard<'a, Queue>, timeout: Option<Duration>) -> MutexGuard<'a, Queue> {
        match timeout {
            Some(d) => match self.cv.wait_timeout(g, d) {
                Ok((g, _)) => g,
                Err(p) => p.into_inner().0,
            },
            None => self.cv.wait(g).unwrap_or_else(|p| p.into_inner()),
        }
    }
}

// ----------------------------------------------------------------- worker

/// Why a lookup stopped without an answer.
enum Halt {
    /// Superseded by a newer track, or shutting down: say nothing.
    Abandoned,
    /// Show `msg`; retry at `retry_at` while the track is still current.
    Failed { msg: String, retry_at: Option<Instant> },
}

/// Only requested local candidates reach this path. Built-in readers handle
/// common tags; Windows' installed property handlers cover other formats.
fn load_embedded(path: &std::path::Path) -> io::Result<Option<lumo_core::local_lyrics::LocalLyrics>> {
    if let Some(found)=lumo_core::local_lyrics::load_embedded_file(path)? {return Ok(Some(found));}
    use windows::core::{GUID,HSTRING};
    use windows::Win32::System::Com::{CoInitializeEx,CoTaskMemFree,CoUninitialize,COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::{IShellItem2,SHCreateItemFromParsingName,PropertiesSystem::PROPERTYKEY};
    // System.Music.Lyrics: learn.microsoft.com/windows/win32/properties/props-system-music-lyrics
    const KEY:PROPERTYKEY=PROPERTYKEY{fmtid:GUID::from_u128(0x56a3372e_ce9c_11d2_9f0e_006097c686f6),pid:12};
    let initialized=unsafe{CoInitializeEx(None,COINIT_MULTITHREADED)}.is_ok();
    let result=(||unsafe{
        let item:IShellItem2=SHCreateItemFromParsingName(&HSTRING::from(path.to_string_lossy().as_ref()),None).ok()?;
        let value=item.GetString(&KEY).ok()?;
        let text=value.to_string().ok();
        CoTaskMemFree(Some(value.0 as *const _));
        let text=text.filter(|s|s.len() as u64<=lumo_core::local_lyrics::MAX_FILE_BYTES)?;
        let lyrics=lumo_core::lrc::parse_lrc(&text);
        (!lyrics.is_empty()).then(||lumo_core::local_lyrics::LocalLyrics{path:path.to_path_buf(),lyrics,embedded:true,downloaded:false})
    })();
    if initialized {unsafe{CoUninitialize();}}
    Ok(result)
}

enum Fetched {
    Found(LrclibRecord),
    Missing,
}

fn local_result(found:lumo_core::local_lyrics::LocalLyrics)->LyricsResult {
    let source=if found.embedded {"Embedded lyrics"} else if found.downloaded {"Downloaded from LRCLIB"} else {"Local files"};
    LyricsResult::Found{lyrics:found.lyrics,source:source.into(),source_file:Some(found.path.to_string_lossy().into_owned()),ruby:Vec::new()}
}

fn file_tags(path:&Path)->Option<lumo_core::local_lyrics::files::Tags> {
    // Only a requested filename association enters Windows' metadata handler.
    // Reuse completion notifications and bounded waits, not a new poll/thread.
    let _mta=super::http::MtaGuard::new();
    let native=(||{
    let open=windows::Storage::StorageFile::GetFileFromPathAsync(&windows::core::HSTRING::from(path.to_string_lossy().as_ref())).ok()?;
    let file=super::media::wait(&open,Duration::from_millis(150)).ok()?;
    let read=file.Properties().ok()?.GetMusicPropertiesAsync().ok()?;
    let properties=super::media::wait(&read,Duration::from_millis(150)).ok()?;
    Some(lumo_core::local_lyrics::files::Tags{title:properties.Title().ok()?.to_string(),artist:properties.Artist().ok()?.to_string(),album:properties.Album().ok()?.to_string()})
    })();
    native.filter(|tags|!tags.title.trim().is_empty()).or_else(||lumo_core::local_lyrics::audio_tags(path).ok().flatten())
}

fn worker_main(shared: Arc<Shared>, sink: EventSink) {
    // Declared first so it is dropped last, after the worker's HttpClient.
    let _com = MtaGuard::new();
    let cache_dir = crate::paths::lyrics_cache_dir();
    prune_cache(&cache_dir);
    let apple = AppleMusic::new();
    let mut w = Worker { shared, sink, cache_dir, apple, client: None, last_request: None, blocked_until: None, writes: 0 };
    while let Some(work) = w.next_job() {
        match work {
            Work::Lookup(job) => w.process(job),
            Work::AppleToken => w.apple_token_changed(),
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    sink: EventSink,
    cache_dir: PathBuf,
    apple: AppleMusic,
    /// Created on first use; reused so connections are pooled.
    client: Option<Client>,
    /// When the last request to LRCLIB finished.
    last_request: Option<Instant>,
    /// No requests before this (server asked us to back off).
    blocked_until: Option<Instant>,
    /// Cache writes since the last prune.
    writes: u32,
}

impl Worker {
    /// Sleep until there is work: a changed Apple Music token first, then a
    /// queued job or a delayed retry that is due; `None` on shutdown.
    fn next_job(&self) -> Option<Work> {
        let mut q = self.shared.lock();
        loop {
            if q.shutdown {
                return None;
            }
            if std::mem::take(&mut q.apple_changed) {
                return Some(Work::AppleToken);
            }
            if let Some(job) = q.pending.take() {
                return Some(Work::Lookup(job));
            }
            let timeout = match &q.retry {
                Some((at, _)) => {
                    let now = Instant::now();
                    if *at <= now {
                        return q.retry.take().map(|(_, job)| Work::Lookup(job));
                    }
                    Some(*at - now)
                }
                None => None,
            };
            q = self.shared.wait_for(q, timeout);
        }
    }

    fn is_current(&self, job: &Job) -> bool {
        let q = self.shared.lock();
        !q.shutdown && q.newest_id == job.id
    }

    fn deliver(&self, job: &Job, result: LyricsResult) {
        if self.is_current(job) {
            self.sink.send(AppEvent::Lyrics { track_key: job.track_key.clone(), request_id: job.id, result });
        }
    }

    fn process(&mut self, job: Job) {
        match self.lookup(&job) {
            Ok(LyricsResult::Found { lyrics, source, source_file, .. }) => {
                // Readings for Japanese / Korean / Chinese lines, worked out
                // here so the UI thread never waits on the analyzer.
                let read = super::romanize::pronounce(&lyrics);
                self.deliver(&job, LyricsResult::Found { lyrics, source, source_file, ruby: read.ruby })
            }
            Ok(result) => self.deliver(&job, result),
            Err(Halt::Abandoned) => {}
            Err(Halt::Failed { msg, retry_at }) => {
                self.deliver(&job, LyricsResult::Error(msg));
                if let Some(at) = retry_at {
                    self.schedule_retry(job, at);
                }
            }
        }
    }

    fn schedule_retry(&self, job: Job, at: Instant) {
        if job.attempt >= MAX_RETRIES {
            return;
        }
        let mut q = self.shared.lock();
        if q.shutdown || q.newest_id != job.id || q.pending.is_some() {
            return;
        }
        q.retry = Some((at, Job { attempt: job.attempt + 1, ..job }));
        // The worker itself is the only waiter and it is not waiting right
        // now, so no notify is needed.
    }

    /// The user saved or removed their Apple Music token: reload it and tell
    /// the UI whether Apple accepts it.
    fn apple_token_changed(&mut self) {
        self.apple.reload_token();
        if !self.apple.has_token() {
            return;
        }
        let shared = self.shared.clone();
        let live = move || !shared.lock().shutdown;
        let status = match self.apple.check(&live) {
            Check::Accepted => AppleMusicStatus::Accepted,
            Check::Rejected => AppleMusicStatus::Rejected,
            Check::Unreachable => AppleMusicStatus::Unreachable,
        };
        self.sink.send(AppEvent::AppleMusic(status));
    }

    fn lookup(&mut self, job: &Job) -> Result<LyricsResult, Halt> {
        let audio_file=if job.local_dir.is_some() || job.save_dir.is_some() {
            let deadline=Instant::now()+Duration::from_secs(2);
            job.match_dir.as_ref().and_then(|dir|lumo_core::local_lyrics::files::find(dir,&job.query.title,&job.query.artist,&job.query.album,||self.is_current(job) && Instant::now()<deadline,file_tags).ok().flatten())
        } else {None};
        if !self.is_current(job){return Err(Halt::Abandoned);}
        if job.local_dir.is_some() {
            if let Some(audio)=&audio_file {
                let mut sidecars=vec![audio.with_extension("lrc")];
                if let Some(name)=audio.with_extension("lrc").file_name() {
                    for dir in job.local_dir.iter().chain(job.saved_dir.iter()) {sidecars.push(dir.join(name));}
                }
                for path in sidecars {
                    if let Ok(found)=lumo_core::local_lyrics::load_file(&path){return Ok(local_result(found));}
                }
                if let Ok(Some(found))=load_embedded(audio){return Ok(local_result(found));}
            }
        }
        for dir in job.local_dir.iter().chain(job.saved_dir.iter()) {
            let found = lumo_core::local_lyrics::lookup_with(dir, &job.query.title, &job.query.artist, &job.query.album, || self.is_current(job), load_embedded);
            match found {
                Ok(Some(found)) => return Ok(local_result(found)),
                Ok(None) => {},
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(Halt::Abandoned),
                Err(e) => crate::pwarn!("lyrics: local file unavailable ({e}); trying LRCLIB"),
            }
        }
        if !self.is_current(job) { return Err(Halt::Abandoned); }
        let q = lrclib::normalize_query(&job.query);
        if q.title.trim().is_empty() {
            // Nothing to search for; answer so the UI doesn't sit on "loading".
            return Ok(LyricsResult::NotFound);
        }
        if !job.lrclib_only && self.apple.enabled() {
            if let Some(result) = self.lookup_apple(job, &q)? {
                return Ok(result);
            }
        }
        let result=self.lookup_lrclib(job, &q)?;
        if self.is_current(job) {
            if let (Some(dir),Some(audio),LyricsResult::Found{lyrics,..})=(&job.save_dir,&audio_file,&result) {
                let root=if job.match_dir.as_ref()==Some(dir){audio.parent().unwrap_or(dir)}else{dir};
                if let Err(error)=lumo_core::local_lyrics::downloads::save(root,audio,&job.query.artist,&job.query.title,lyrics) {
                    crate::pwarn!("lyrics: couldn't save downloaded lyrics ({error})");
                }
            }
        }
        Ok(result)
    }

    /// Apple Music's synced lyrics; `Ok(None)` hands the song to LRCLIB.
    fn lookup_apple(&mut self, job: &Job, q: &TrackQuery) -> Result<Option<LyricsResult>, Halt> {
        let key = format!("am-{}", lrclib::cache_key(q));
        let path = self.cache_dir.join(format!("{key}.json"));
        let cached = read_cache(&path);
        if let Some(entry) = &cached {
            if is_fresh(entry, crate::clock::unix_now()) {
                return Ok(entry.record.as_ref().map(|r| to_result(Some(r), job, APPLE_SOURCE)));
            }
        }
        let (shared, request_id) = (self.shared.clone(), job.id);
        let live = move || {
            let q = shared.lock();
            !q.shutdown && q.newest_id == request_id
        };
        match self.apple.find(q, &live) {
            Ok(Some(record)) => {
                crate::pinfo!("lyrics {key}: synced");
                self.store(&path, &CacheEntry::new(Some(record.clone()), crate::clock::unix_now()));
                Ok(Some(to_result(Some(&record), job, APPLE_SOURCE)))
            }
            Ok(None) => {
                crate::pinfo!("lyrics {key}: no synced lyrics on Apple Music");
                self.store(&path, &CacheEntry::new(None, crate::clock::unix_now()));
                Ok(None)
            }
            Err(Miss::Abandoned) => Err(Halt::Abandoned),
            Err(miss) => {
                match miss {
                    Miss::Rejected => self.sink.send(AppEvent::AppleMusic(AppleMusicStatus::Rejected)),
                    Miss::NoAccess => self.sink.send(AppEvent::AppleMusic(AppleMusicStatus::NoAccess)),
                    Miss::Unreachable(why) => crate::pwarn!("lyrics: Apple Music unavailable ({why}); asking LRCLIB"),
                    Miss::Abandoned => {}
                }
                // An expired hit still beats asking LRCLIB.
                Ok(cached.as_ref().and_then(|e| e.record.as_ref()).map(|r| to_result(Some(r), job, APPLE_SOURCE)))
            }
        }
    }

    fn lookup_lrclib(&mut self, job: &Job, q: &TrackQuery) -> Result<LyricsResult, Halt> {
        let key = lrclib::cache_key(q);
        let path = self.cache_dir.join(format!("{key}.json"));
        let cached = read_cache(&path);
        if let Some(entry) = &cached {
            if is_fresh(entry, crate::clock::unix_now()) {
                return Ok(to_result(entry.record.as_ref(), job, SOURCE));
            }
        }
        match self.fetch(job, q) {
            Ok(Fetched::Found(record)) => {
                crate::pinfo!(
                    "lyrics {key}: {}",
                    if record.instrumental {
                        "instrumental"
                    } else if record.has_synced() {
                        "synced"
                    } else {
                        "plain"
                    }
                );
                self.store(&path, &CacheEntry::new(Some(record.clone()), crate::clock::unix_now()));
                Ok(to_result(Some(&record), job, SOURCE))
            }
            Ok(Fetched::Missing) => {
                crate::pinfo!("lyrics {key}: not on LRCLIB");
                self.store(&path, &CacheEntry::new(None, crate::clock::unix_now()));
                Ok(LyricsResult::NotFound)
            }
            Err(Halt::Failed { msg, retry_at }) => match cached.as_ref().and_then(|e| e.record.as_ref()) {
                // An expired hit beats an error.
                Some(record) => Ok(to_result(Some(record), job, SOURCE)),
                None => Err(Halt::Failed { msg, retry_at }),
            },
            Err(Halt::Abandoned) => Err(Halt::Abandoned),
        }
    }

    fn store(&mut self, path: &Path, entry: &CacheEntry) {
        write_cache(path, entry);
        self.writes += 1;
        if self.writes >= PRUNE_EVERY_WRITES {
            self.writes = 0;
            prune_cache(&self.cache_dir);
        }
    }

    fn fetch(&mut self, job: &Job, q: &TrackQuery) -> Result<Fetched, Halt> {
        // Set when /api/get or a search failed on LRCLIB's side. The other
        // lookups still run, but a miss is then no longer a sure "no lyrics"
        // and isn't cached.
        let mut server_failed = None;
        // Plain lyrics found on the way. One signature can hold a plain-only
        // copy while another copy of the song is synced, so the looser
        // lookups still run before settling for these.
        let mut plain: Option<LrclibRecord> = None;
        // /api/get needs an artist; try the full signature, then without album.
        if !q.artist.is_empty() {
            let mut urls = vec![lrclib::get_url(q)];
            if !q.album.is_empty() {
                urls.push(lrclib::get_url(&TrackQuery { album: String::new(), ..q.clone() }));
            }
            for url in urls {
                let resp = self.call(job, &url)?;
                match resp.status {
                    200 => match lrclib::parse_get(&resp.body) {
                        Ok(record) if complete(&record) => return Ok(Fetched::Found(record)),
                        Ok(record) if usable(&record) => {
                            plain.get_or_insert(record);
                        }
                        Ok(_) => {}
                        Err(e) => return Err(self.bad_reply(job, "/api/get", &e)),
                    },
                    // No such signature: try the next, looser one.
                    404 => {}
                    // LRCLIB rejected the query shape (e.g. no duration); the
                    // album-less /api/get would be rejected too, so search.
                    400 => break,
                    // A signature LRCLIB doesn't hold sends it to outside
                    // sources, which can answer 503 while busy; the search
                    // only reads its own database and usually still works.
                    s if s >= 500 => {
                        server_failed = Some(self.http_error(job, s));
                        break;
                    }
                    s => return Err(self.http_error(job, s)),
                }
            }
        }
        let mut searches = vec![q.clone()];
        // A browser tab whose channel isn't the artist (a lyric-video
        // channel, a record label): read the title as "Artist - Song", then
        // as "Song - Artist", and search again.
        if job.browser {
            searches.extend(lrclib::title_guesses(q));
        }
        for sq in &searches {
            if let Some(record) = self.search(job, sq, &mut server_failed)? {
                if complete(&record) {
                    return Ok(Fetched::Found(record));
                }
                plain.get_or_insert(record);
            }
        }
        match plain {
            Some(record) => Ok(Fetched::Found(record)),
            None => server_failed.map_or(Ok(Fetched::Missing), Err),
        }
    }

    /// `/api/search` by title + primary artist; the best usable match. A
    /// 5xx is noted in `server_failed` and counts as no match here: LRCLIB
    /// can answer 503 to a search it holds nothing for (a video title read
    /// the wrong way) while the next reading of the title works.
    fn search(&mut self, job: &Job, q: &TrackQuery, server_failed: &mut Option<Halt>) -> Result<Option<LrclibRecord>, Halt> {
        let search = TrackQuery {
            title: q.title.clone(),
            artist: lrclib::primary_artist(&q.artist),
            album: String::new(),
            duration_s: q.duration_s,
        };
        let resp = self.call(job, &lrclib::search_url(&search))?;
        match resp.status {
            200 => match lrclib::parse_search(&resp.body) {
                Ok(records) => Ok(lrclib::best_match(&records, q).filter(|r| usable(r)).cloned()),
                Err(e) => Err(self.bad_reply(job, "/api/search", &e)),
            },
            400 | 404 => Ok(None),
            s if s >= 500 => {
                let failed = self.http_error(job, s);
                server_failed.get_or_insert(failed);
                Ok(None)
            }
            s => Err(self.http_error(job, s)),
        }
    }

    /// One paced request. 429s set the shared back-off; a short one is waited
    /// out and the request repeated once.
    fn call(&mut self, job: &Job, url: &str) -> Result<http::Response, Halt> {
        for _ in 0..2 {
            self.pace(job)?;
            let client = match self.client.take() {
                Some(c) => c,
                None => Client::new(USER_AGENT).map_err(|e| {
                    crate::pwarn!("lyrics: {e}");
                    Halt::Failed { msg: "Lyrics are unavailable".into(), retry_at: Some(Instant::now() + retry_delay(job.attempt)) }
                })?,
            };
            let res = client.get(url, http::DEFAULT_TIMEOUT);
            self.client = Some(client);
            self.last_request = Some(Instant::now());
            match res {
                Ok(r) if r.status == 429 => {
                    let wait = r.retry_after_s.map(Duration::from_secs).unwrap_or(DEFAULT_BACKOFF).min(MAX_BACKOFF);
                    crate::pwarn!("lyrics: LRCLIB rate limit, backing off {} s", wait.as_secs());
                    self.blocked_until = Some(Instant::now() + wait);
                    // `pace` waits a short back-off out, or fails with a retry.
                }
                Ok(r) => return Ok(r),
                Err(e) => {
                    crate::pwarn!("lyrics: request failed: {e}");
                    let msg = match e {
                        FetchError::Timeout => "LRCLIB didn't answer in time",
                        FetchError::Network(_) => "Can't reach LRCLIB",
                        FetchError::Other(_) => "Lyrics are unavailable",
                    };
                    return Err(Halt::Failed { msg: msg.into(), retry_at: Some(Instant::now() + retry_delay(job.attempt)) });
                }
            }
        }
        Err(self.busy())
    }

    /// Wait for the rate limit / back-off. Aborts when the job goes stale or
    /// the service stops; fails (with a retry) when the wait would be long.
    fn pace(&self, job: &Job) -> Result<(), Halt> {
        let mut until = self.last_request.map(|t| t + MIN_GAP);
        if let Some(b) = self.blocked_until {
            until = Some(until.map_or(b, |u| u.max(b)));
        }
        let Some(until) = until else {
            return if self.is_current(job) { Ok(()) } else { Err(Halt::Abandoned) };
        };
        if until.saturating_duration_since(Instant::now()) > MAX_INLINE_WAIT {
            return Err(self.busy());
        }
        let mut q = self.shared.lock();
        loop {
            if q.shutdown || q.newest_id != job.id {
                return Err(Halt::Abandoned);
            }
            let now = Instant::now();
            if now >= until {
                return Ok(());
            }
            q = self.shared.wait_for(q, Some(until - now));
        }
    }

    fn busy(&self) -> Halt {
        let at = self.blocked_until.unwrap_or_else(Instant::now).max(Instant::now() + MIN_GAP);
        Halt::Failed { msg: MSG_BUSY.into(), retry_at: Some(at) }
    }

    fn http_error(&self, job: &Job, status: u16) -> Halt {
        crate::pwarn!("lyrics: LRCLIB answered HTTP {status}");
        // Server trouble is worth a later retry; other client errors are not.
        let retry_at = (status >= 500).then(|| Instant::now() + retry_delay(job.attempt));
        Halt::Failed { msg: format!("LRCLIB error {status}"), retry_at }
    }

    /// A 200 that isn't LRCLIB JSON (captive portal, proxy page, API change):
    /// never cache it as a miss.
    fn bad_reply(&self, job: &Job, endpoint: &str, err: &str) -> Halt {
        crate::pwarn!("lyrics: unreadable {endpoint} reply: {err}");
        Halt::Failed {
            msg: "Unexpected reply from LRCLIB".into(),
            retry_at: Some(Instant::now() + retry_delay(job.attempt)),
        }
    }
}

/// A record that answers the lookup for good: synced lyrics, or a definite
/// "instrumental".
fn complete(r: &LrclibRecord) -> bool {
    r.instrumental || r.has_full_synced()
}

fn to_result(record: Option<&LrclibRecord>, job: &Job, source: &str) -> LyricsResult {
    if record.is_some_and(|r| r.instrumental) {
        return LyricsResult::Instrumental;
    }
    match record.and_then(|r| lyrics_for(r, job.query.duration_s)) {
        Some(lyrics) => LyricsResult::Found { lyrics, source: source.into(), source_file: None, ruby: Vec::new() },
        None => LyricsResult::NotFound,
    }
}

// ------------------------------------------------------------------ cache

fn read_cache(path: &Path) -> Option<CacheEntry> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            crate::pwarn!("lyrics cache: cannot read {}: {e}", path.display());
            // Invalid UTF-8 means a damaged file; anything else may be transient.
            if e.kind() == io::ErrorKind::InvalidData {
                let _ = fs::remove_file(path);
            }
            return None;
        }
    };
    match serde_json::from_str::<CacheEntry>(&text) {
        Ok(entry) if entry.v == CacheEntry::VERSION => Some(entry),
        Ok(_) => None,
        Err(e) => {
            crate::pwarn!("lyrics cache: dropping unreadable {}: {e}", path.display());
            let _ = fs::remove_file(path);
            None
        }
    }
}

fn write_cache(path: &Path, entry: &CacheEntry) {
    if let Err(e) = lumo_core::store::write_json_atomic(path, entry) {
        crate::pwarn!("lyrics cache: cannot write {}: {e}", path.display());
    }
}

/// Remove leftovers of interrupted writes and keep the folder bounded
/// (oldest entries go first).
fn prune_cache(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("tmp") => {
                let _ = fs::remove_file(&path);
            }
            Some("json") => {
                let modified = entry.metadata().and_then(|m| m.modified()).ok();
                files.push((modified, path));
            }
            _ => {}
        }
    }
    let victims = prune_victims(files, CACHE_MAX, CACHE_KEEP);
    if !victims.is_empty() {
        crate::pinfo!("lyrics cache: pruning {} old entries", victims.len());
    }
    for path in victims {
        let _ = fs::remove_file(path);
    }
}

/// OS-free helpers (unit-tested on any host).
mod pure {
    use lumo_core::lrc::{parse_lrc, LyricLine, Lyrics};
    use lumo_core::lrclib::{timing_fits, LrclibRecord, MIN_LINES};
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    /// Misses are re-checked after 3 days (LRCLIB grows every day).
    pub const NEGATIVE_TTL_S: i64 = 3 * 24 * 3600;
    /// Plain-only hits are re-checked after 30 days in case synced lyrics appeared.
    pub const PLAIN_TTL_S: i64 = 30 * 24 * 3600;

    /// One file in the lyrics cache.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct CacheEntry {
        /// Format version.
        #[serde(default)]
        pub v: u32,
        /// The lookup that wrote it ([`CacheEntry::LOOKUP`]); 0 in files from
        /// before it was recorded.
        #[serde(default)]
        pub lookup: u32,
        /// Unix seconds when LRCLIB was asked.
        pub fetched_at: i64,
        /// The matched record; `None` records a miss.
        #[serde(default)]
        pub record: Option<LrclibRecord>,
    }

    impl CacheEntry {
        pub const VERSION: u32 = 1;
        /// Raised whenever the lookup learns to find songs it used to miss
        /// (2: subtitle tags and "Song - Artist" video titles), so misses
        /// from an older lookup are asked again instead of kept for 3 days.
        pub const LOOKUP: u32 = 2;

        pub fn new(record: Option<LrclibRecord>, now: i64) -> Self {
            Self { v: Self::VERSION, lookup: Self::LOOKUP, fetched_at: now, record }
        }
    }

    /// Whether `e` can be used without asking LRCLIB again.
    pub fn is_fresh(e: &CacheEntry, now: i64) -> bool {
        if let Some(r) = &e.record {
            // Synced lyrics and instrumental flags don't go stale.
            if r.has_synced() || r.instrumental {
                return true;
            }
        } else if e.lookup < CacheEntry::LOOKUP {
            // A miss from a lookup that knew fewer ways to find the song.
            return false;
        }
        let age = now - e.fetched_at;
        if age < -24 * 3600 {
            // Written "in the future": the clock was wrong; don't trust it.
            return false;
        }
        let ttl = if e.record.is_some() { PLAIN_TTL_S } else { NEGATIVE_TTL_S };
        age < ttl
    }

    /// Whether a record answers the lookup (lyrics or a definite "instrumental").
    pub fn usable(r: &LrclibRecord) -> bool {
        r.instrumental || lyrics_from_record(r).is_some()
    }

    /// The lyrics to show for a record: synced when available, else plain.
    /// `None` for instrumentals and records without real text (fewer than
    /// `MIN_LINES` lines: a title, a lone "♪").
    pub fn lyrics_from_record(r: &LrclibRecord) -> Option<Lyrics> {
        if r.instrumental {
            return None;
        }
        let synced = r.synced_lyrics.as_deref().filter(|s| !s.trim().is_empty()).map(parse_lrc);
        if let Some(l) = &synced {
            if l.synced && l.text_lines() >= MIN_LINES {
                return synced;
            }
        }
        if let Some(text) = r.plain_lyrics.as_deref().filter(|s| !s.trim().is_empty()) {
            let l = trim_blank_edges(Lyrics::plain(text));
            if l.text_lines() >= MIN_LINES {
                return Some(l);
            }
        }
        // "Synced" text without usable timestamps still reads as plain lyrics.
        synced.map(trim_blank_edges).filter(|l| l.text_lines() >= MIN_LINES)
    }

    /// The lyrics to show while a `playing_s`-second track plays. Synced
    /// lyrics stay synced only when the record is about as long; otherwise
    /// (a music video's intro, a radio edit) they would light up the wrong
    /// lines, so they're shown unsynced.
    pub fn lyrics_for(r: &LrclibRecord, playing_s: u32) -> Option<Lyrics> {
        let l = lyrics_from_record(r)?;
        if !l.synced || timing_fits(r.duration, playing_s) {
            return Some(l);
        }
        if let Some(text) = r.plain_lyrics.as_deref() {
            let plain = trim_blank_edges(Lyrics::plain(text));
            if plain.text_lines() >= MIN_LINES {
                return Some(plain);
            }
        }
        let lines = l.lines.into_iter().map(|x| LyricLine { time_s: 0.0, text: x.text }).collect();
        Some(trim_blank_edges(Lyrics { lines, synced: false }))
    }

    /// Drop blank rows at the start and end of unsynced lyrics (blank rows
    /// in between are stanza breaks and stay).
    fn trim_blank_edges(mut l: Lyrics) -> Lyrics {
        if !l.synced {
            while l.lines.last().is_some_and(|x| x.text.trim().is_empty()) {
                l.lines.pop();
            }
            let lead = l.lines.iter().take_while(|x| x.text.trim().is_empty()).count();
            l.lines.drain(..lead);
        }
        l
    }

    /// Whole seconds for LRCLIB's duration match; 0 when unknown.
    pub fn duration_secs(d: f64) -> u32 {
        if d.is_finite() && d > 0.5 {
            d.round().min(u32::MAX as f64) as u32
        } else {
            0
        }
    }

    /// Delay before the n-th delayed retry (0-based attempt that just failed).
    pub fn retry_delay(attempt: u32) -> Duration {
        Duration::from_secs(if attempt == 0 { 30 } else { 120 })
    }

    /// Which cache files to delete: none while `files.len() <= max`, else the
    /// oldest ones (unknown times first) down to `keep`.
    pub fn prune_victims(mut files: Vec<(Option<SystemTime>, PathBuf)>, max: usize, keep: usize) -> Vec<PathBuf> {
        if files.len() <= max {
            return Vec::new();
        }
        files.sort_by_key(|(t, _)| *t);
        let excess = files.len() - keep.min(files.len());
        files.into_iter().take(excess).map(|(_, p)| p).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Fixtures are original placeholder text, not real lyrics.
        fn rec(synced: Option<&str>, plain: Option<&str>, instrumental: bool) -> LrclibRecord {
            LrclibRecord {
                id: 1,
                track_name: "Paper Boats".into(),
                artist_name: "The Lanterns".into(),
                album_name: None,
                duration: Some(201.0),
                instrumental,
                plain_lyrics: plain.map(Into::into),
                synced_lyrics: synced.map(Into::into),
            }
        }

        const SYNCED: &str = "[00:01.00]first row\n[00:04.50]second row\n[00:08.00]third row\n[00:12.00]fourth row\n";
        const PLAIN: &str = "first row\nsecond row\nthird row\nfourth row";

        #[test]
        fn prefers_synced_then_plain() {
            let r = rec(Some(SYNCED), Some(PLAIN), false);
            let l = lyrics_from_record(&r).unwrap();
            assert!(l.synced);
            assert_eq!(l.lines.len(), 4);
            assert!((l.lines[1].time_s - 4.5).abs() < 1e-9);

            let r = rec(None, Some("\n\nfirst row\n\nsecond row\nthird row\nfourth row\n\n"), false);
            let l = lyrics_from_record(&r).unwrap();
            assert!(!l.synced);
            let rows: Vec<&str> = l.lines.iter().map(|x| x.text.as_str()).collect();
            assert_eq!(rows, vec!["first row", "", "second row", "third row", "fourth row"]);

            // Blank or stub synced text falls back to plain.
            let r = rec(Some("   "), Some(PLAIN), false);
            assert!(!lyrics_from_record(&r).unwrap().synced);
            let r = rec(Some("[00:00.00]Paper Boats"), Some(PLAIN), false);
            assert!(!lyrics_from_record(&r).unwrap().synced);
            // "Synced" text without timestamps still shows as plain lyrics.
            let r = rec(Some(PLAIN), None, false);
            let l = lyrics_from_record(&r).unwrap();
            assert!(!l.synced);
            assert_eq!(l.lines[0].text, "first row");
        }

        #[test]
        fn instrumental_empty_and_stub_records_have_no_lyrics() {
            assert!(lyrics_from_record(&rec(Some(SYNCED), None, true)).is_none());
            assert!(usable(&rec(None, None, true)));
            assert!(lyrics_from_record(&rec(None, None, false)).is_none());
            assert!(!usable(&rec(None, Some("  \n "), false)));
            assert!(usable(&rec(None, Some(PLAIN), false)));
            // Fewer than MIN_LINES lines of text: a stub, not lyrics.
            assert!(!usable(&rec(None, Some("only row"), false)));
            assert!(!usable(&rec(Some("[00:00.00]Paper Boats"), None, false)));
        }

        #[test]
        fn synced_only_when_the_lengths_match() {
            // rec() is 201 s long.
            let r = rec(Some(SYNCED), Some(PLAIN), false);
            assert!(lyrics_for(&r, 203).unwrap().synced);
            assert!(lyrics_for(&r, 0).unwrap().synced);
            // A music video 40 s longer: the plain text, unsynced.
            let l = lyrics_for(&r, 241).unwrap();
            assert!(!l.synced);
            assert_eq!(l.lines.len(), 4);
            // Without plain text, the synced rows lose their times.
            let l = lyrics_for(&rec(Some(SYNCED), None, false), 160).unwrap();
            assert!(!l.synced);
            assert!(l.lines.iter().all(|x| x.time_s == 0.0));
            assert_eq!(l.lines[3].text, "fourth row");
        }

        #[test]
        fn freshness_rules() {
            let now = 1_800_000_000;
            let day = 24 * 3600;
            let miss = CacheEntry::new(None, now - 2 * day);
            assert!(is_fresh(&miss, now));
            let miss = CacheEntry::new(None, now - 3 * day);
            assert!(!is_fresh(&miss, now));
            let synced = CacheEntry::new(Some(rec(Some("[00:01.00]x"), None, false)), now - 900 * day);
            assert!(is_fresh(&synced, now));
            let instrumental = CacheEntry::new(Some(rec(None, None, true)), now - 900 * day);
            assert!(is_fresh(&instrumental, now));
            let plain = CacheEntry::new(Some(rec(None, Some("row"), false)), now - 10 * day);
            assert!(is_fresh(&plain, now));
            let plain = CacheEntry::new(Some(rec(None, Some("row"), false)), now - 31 * day);
            assert!(!is_fresh(&plain, now));
            // Timestamps far in the future are distrusted.
            let future = CacheEntry::new(None, now + 2 * day);
            assert!(!is_fresh(&future, now));
            // Misses from an older lookup are asked again; its hits stay.
            let old_miss = CacheEntry { lookup: CacheEntry::LOOKUP - 1, ..CacheEntry::new(None, now - day) };
            assert!(!is_fresh(&old_miss, now));
            let old_synced = CacheEntry { lookup: 0, ..CacheEntry::new(Some(rec(Some("[00:01.00]x"), None, false)), now - day) };
            assert!(is_fresh(&old_synced, now));
        }

        #[test]
        fn cache_entry_roundtrips() {
            let e = CacheEntry::new(Some(rec(Some("[00:01.00]x"), Some("x"), false)), 42);
            let json = serde_json::to_string(&e).unwrap();
            let back: CacheEntry = serde_json::from_str(&json).unwrap();
            assert_eq!(back, e);
            // Files from before the lookup was recorded read as lookup 0.
            let miss: CacheEntry = serde_json::from_str(r#"{"v":1,"fetched_at":7,"record":null}"#).unwrap();
            assert_eq!(miss, CacheEntry { lookup: 0, ..CacheEntry::new(None, 7) });
        }

        #[test]
        fn durations_round_or_zero() {
            assert_eq!(duration_secs(201.4), 201);
            assert_eq!(duration_secs(201.5), 202);
            assert_eq!(duration_secs(0.0), 0);
            assert_eq!(duration_secs(0.4), 0);
            assert_eq!(duration_secs(-3.0), 0);
            assert_eq!(duration_secs(f64::NAN), 0);
            assert_eq!(duration_secs(f64::INFINITY), 0);
        }

        #[test]
        fn retry_backoff_grows() {
            assert!(retry_delay(1) > retry_delay(0));
        }

        #[test]
        fn prunes_oldest_first() {
            use std::time::UNIX_EPOCH;
            let at = |s: u64| Some(UNIX_EPOCH + Duration::from_secs(s));
            let files = vec![
                (at(30), PathBuf::from("c")),
                (at(10), PathBuf::from("a")),
                (None, PathBuf::from("unknown")),
                (at(20), PathBuf::from("b")),
            ];
            assert!(prune_victims(files.clone(), 4, 2).is_empty());
            assert_eq!(prune_victims(files, 3, 2), vec![PathBuf::from("unknown"), PathBuf::from("a")]);
        }
    }
}
