//! Local sidecar and embedded lyrics. Lookups run on the lyrics worker only when requested;
//! there is no library index, watcher or background rescan.

use crate::lrc::{parse_lrc, Lyrics};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
mod embedded;
pub mod downloads;
pub mod files;

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const MAX_CANDIDATES: usize = 256;
const MAX_DEPTH: usize = 32;

#[derive(Debug)]
pub struct LocalLyrics {
    pub path: PathBuf,
    pub lyrics: Lyrics,
    pub embedded: bool,
    pub downloaded: bool,
}

/// Local files are an opt-in first choice for local music only. Missing files
/// fall through to LRCLIB; streaming services always keep their online source.
pub fn for_player(enabled: bool, source: Option<crate::nowplaying::MusicSource>) -> bool {
    enabled && source == Some(crate::nowplaying::MusicSource::Local)
}

fn normalized(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Players can display an explicit-content badge where filenames use a
/// bracketed label. Rust considers the boxed E alphabetic, so punctuation
/// filtering alone does not remove it. Compare the name and rating separately.
fn name_key(s:&str)->String {
    let mut text=String::with_capacity(s.len());let mut rest=s;
    while let Some(start)=rest.find(['(','[']) {
        text.push_str(&rest[..start]);let close=if rest.as_bytes()[start]==b'(' {')'} else {']'};
        let Some(end)=rest[start+1..].find(close).map(|i|start+1+i) else{text.push_str(&rest[start..]);rest="";break;};
        if rating_label(&rest[start+1..end]).is_none(){text.push_str(&rest[start..=end]);}
        rest=&rest[end+1..];
    }
    text.push_str(rest);
    text.chars().filter(|c|!matches!(c,'🅴'|'🄴') && c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}
fn rating_label(label:&str)->Option<bool> {
    match label.trim().to_ascii_lowercase().as_str(){"explicit"|"explicit version"|"explicit lyrics"=>Some(true),"clean"|"clean version"|"clean lyrics"=>Some(false),_=>None}
}
fn rating(s:&str)->Option<bool> {
    if s.contains(['🅴','🄴']){return Some(true);}
    let mut rest=s;
    while let Some(start)=rest.find(['(','[']) {
        let close=if rest.as_bytes()[start]==b'(' {')'} else {']'};
        let Some(end)=rest[start+1..].find(close).map(|i|start+1+i) else{break;};
        if let Some(value)=rating_label(&rest[start+1..end]){return Some(value);}
        rest=&rest[end+1..];
    }
    None
}

fn stem_key(s: &str) -> String {
    // Common album sidecars are named "01 - Artist - Title" or "01. Title".
    let rest = s.trim_start_matches(|c: char| c.is_ascii_digit());
    let s = if rest.len() != s.len() && rest.starts_with([' ', '.', '-', '_']) {
        rest.trim_start_matches([' ', '.', '-', '_'])
    } else {
        s
    };
    name_key(s)
}

fn extension(path: &Path) -> Option<u8> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "lrc" => Some(0),
        "txt" | "lyrics" => Some(1),
        "mp3" | "flac" | "m4a" | "m4b" | "mp4" | "ogg" | "oga" | "opus" | "aac" | "wma" | "wav" | "aif" | "aiff" | "ape" | "wv" => Some(2),
        _ => None,
    }
}

/// Load one explicitly selected lyric file, with UTF-8 or BOM-marked UTF-16.
/// Timestamped LRC stays synced; plain text keeps its stanza breaks.
pub fn load_file(path: &Path) -> io::Result<LocalLyrics> {
    let format = extension(path).ok_or_else(|| invalid("Choose an .lrc, .txt or .lyrics file."))?;
    if format==2 { return load_embedded_file(path)?.ok_or_else(||invalid("No embedded lyrics found in this song.")); }
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES {
        return Err(invalid("The lyric file is larger than 1 MB."));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("The lyric file is larger than 1 MB."));
    }
    let text = decode(&bytes)?;
    let lyrics = if format == 0 { parse_lrc(&text) } else { Lyrics::plain(&text) };
    if lyrics.is_empty() {
        return Err(invalid("The selected file contains no lyrics."));
    }
    let downloaded=text.lines().any(|line|line.trim()=="[re:Lumo LRCLIB]");
    Ok(LocalLyrics { path: path.to_path_buf(), lyrics, embedded: false, downloaded })
}

/// Read common audio tags without decoding or modifying the song.
pub fn load_embedded_file(path: &Path) -> io::Result<Option<LocalLyrics>> {
    embedded::read(path).map(|lyrics|lyrics.map(|lyrics|LocalLyrics{path:path.to_path_buf(),lyrics,embedded:true,downloaded:false}))
}

pub fn audio_tags(path:&Path)->io::Result<Option<files::Tags>> {embedded::audio_tags(path)}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn decode(bytes: &[u8]) -> io::Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err(invalid("The UTF-16 lyric file is incomplete."));
        }
        let little = bytes[0] == 0xff;
        let words: Vec<u16> = bytes[2..].chunks_exact(2).map(|b| {
            if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) }
        }).collect();
        String::from_utf16(&words).map_err(|_| invalid("The lyric file is not valid UTF-16."))
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        String::from_utf8(bytes.to_vec()).map_err(|_| invalid("Save the lyric file as UTF-8 or UTF-16."))
    }
}

fn linked(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions are reparse points too: never walk outside the selected tree.
        if metadata.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

/// Match artist/title filenames, then title filenames in artist/album folders.
/// Synced sidecars win over plain sidecars for the same match. Cancelling a
/// track stops the walk at its next entry. Limits bound very large libraries.
pub fn lookup(root: &Path, title: &str, artist: &str, album: &str, live: impl Fn() -> bool) -> io::Result<Option<LocalLyrics>> {
    lookup_with(root,title,artist,album,live,load_embedded_file)
}

/// A platform may also consult its installed metadata handlers for other formats.
pub fn lookup_with(root: &Path, title: &str, artist: &str, album: &str, live: impl Fn() -> bool,
    embedded: impl Fn(&Path)->io::Result<Option<LocalLyrics>>) -> io::Result<Option<LocalLyrics>> {
    let expected_rating=rating(title);
    let title = name_key(title);
    if title.is_empty() { return Ok(None); }
    let artist = normalized(artist);
    let album = normalized(album);
    let full = [format!("{artist}{title}"), format!("{title}{artist}")];
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut candidates = Vec::new();
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        if !live() { return Err(io::ErrorKind::Interrupted.into()); }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if dir == root => return Err(e),
            Err(_) => continue,
        };
        for entry in entries {
            if !live() { return Err(io::ErrorKind::Interrupted.into()); }
            visited += 1;
            if visited > MAX_ENTRIES {
                return Err(invalid("Choose a smaller Music folder."));
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else { continue };
            if linked(&metadata) { continue; }
            if metadata.is_dir() {
                if depth < MAX_DEPTH { pending.push((path, depth + 1)); }
                continue;
            }
            if !metadata.is_file() { continue; }
            let Some(format) = extension(&path) else { continue };
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let key = stem_key(stem);
            let raw_key = name_key(stem);
            let file_rating=rating(stem);
            if expected_rating.zip(file_rating).is_some_and(|(expected,found)|expected!=found){continue;}
            let folders: Vec<String> = dir.strip_prefix(root).unwrap_or(&dir).components()
                .filter_map(|c| c.as_os_str().to_str().map(normalized)).collect();
            let mut score = if !artist.is_empty() && (full.contains(&key) || full.contains(&raw_key)) {
                300
            } else if key == title || raw_key == title {
                if !artist.is_empty() && folders.contains(&artist) { 200 } else { 100 }
            } else { continue };
            if !album.is_empty() && folders.contains(&album) { score += 10; }
            if expected_rating.is_some() && file_rating==expected_rating {score+=20;}
            candidates.push((score, format, path));
            if candidates.len() > MAX_CANDIDATES {
                return Err(invalid("Several files match this song. Use artist and album folders."));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    if let Some(best) = candidates.first() {
        let best_rating=best.2.file_stem().and_then(|s|s.to_str()).and_then(rating);
        if expected_rating.is_none() && candidates.iter().skip(1).any(|other|other.0==best.0 && other.2.file_stem().and_then(|s|s.to_str()).and_then(rating)!=best_rating) {
            return Err(invalid("Several editions match this song. Its explicit or clean version could not be identified."));
        }
        if best.0 < 200 && candidates.iter().any(|other| other.0 == best.0 && other.2.parent() != best.2.parent()) {
            return Err(invalid("Several files match this title. Use Artist - Title filenames."));
        }
    }
    let mut last_error = None;
    for (_, format, path) in candidates {
        if !live() { return Err(io::ErrorKind::Interrupted.into()); }
        match if format==2 {embedded(&path)} else {load_file(&path).map(Some)} {
            Ok(Some(found)) => return Ok(Some(found)),
            Ok(None) => {},
            Err(e) => last_error = Some(e),
        }
    }
    match last_error { Some(e) => Err(e), None => Ok(None) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Library(PathBuf);
    impl Library {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("lumo-local-lyrics-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn put(&self, name: &str, text: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }
    }
    impl Drop for Library { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn explicit_badges_and_bracketed_labels_match_without_mixing_editions() {
        let lib=Library::new();
        let explicit=lib.put("The Lanterns - Paper Boats (Explicit).lrc","[00:01]Explicit fixture\n[00:02]Second fixture");
        let clean=lib.put("The Lanterns - Paper Boats (Clean).lrc","[00:01]Clean fixture\n[00:02]Second fixture");
        assert_eq!(lookup(&lib.0,"Paper Boats 🅴","The Lanterns","",||true).unwrap().unwrap().path,explicit);
        assert_eq!(lookup(&lib.0,"Paper Boats (Clean)","The Lanterns","",||true).unwrap().unwrap().path,clean);
        assert!(lookup(&lib.0,"Paper Boats","The Lanterns","",||true).is_err(),"an unknown edition must not choose between explicit and clean");
        assert!(lookup(&lib.0,"Paper Boats (Live)","The Lanterns","",||true).unwrap().is_none(),"live/remix titles are not decorative labels");
    }

    #[test]
    fn matching_song_files_supply_embedded_lyrics_after_sidecars() {
        let lib=Library::new();
        let text=b"\x03eng\0[00:01]Embedded first\n[00:02]Embedded second";
        let mut frame=b"USLT".to_vec();frame.extend_from_slice(&(text.len() as u32).to_be_bytes());frame.extend_from_slice(&[0,0]);frame.extend_from_slice(text);
        let mut audio=b"ID3\x03\0\0".to_vec();audio.extend_from_slice(&[0,0,0,frame.len() as u8]);audio.extend(frame);
        let song=lib.put("The Lanterns/Album/01 - Paper Boats.mp3",audio);
        let sidecar=lib.put("The Lanterns/Album/01 - Paper Boats.lrc","[00:01]Sidecar");
        assert!(!lookup(&lib.0,"Paper Boats","The Lanterns","Album",||true).unwrap().unwrap().embedded);
        fs::remove_file(sidecar).unwrap();
        let found=lookup(&lib.0,"Paper Boats","The Lanterns","Album",||true).unwrap().unwrap();
        assert!(found.embedded && found.lyrics.synced);assert_eq!(found.path,song);assert_eq!(found.lyrics.lines[0].text,"Embedded first");
    }

    #[test]
    fn local_mode_is_opt_in_and_does_not_replace_browser_lyrics() {
        use crate::nowplaying::MusicSource::{Local, Streaming};
        assert!(!for_player(false, Some(Local)));
        assert!(for_player(true, Some(Local)));
        assert!(!for_player(true, Some(Streaming)));
        assert!(!for_player(true, None));
    }

    #[test]
    fn nested_artist_matches_prefer_lrc_and_keep_unicode() {
        let lib = Library::new();
        lib.put("Björk/Album/01 - Jóga.txt", "plain line");
        let synced = lib.put("Björk/Album/01 - Jóga.LRC", "[00:01.25]first line\n[00:05]second line");
        lib.put("Another artist/Jóga.lrc", "[00:00]wrong artist");
        let found = lookup(&lib.0, "Jóga", "BJÖRK", "Album", || true).unwrap().unwrap();
        assert_eq!(found.path, synced);
        assert!(found.lyrics.synced);
        assert_eq!(found.lyrics.lines[0].time_s, 1.25);
    }

    #[test]
    fn artist_title_beats_an_unrelated_title_and_plain_keeps_stanzas() {
        let lib = Library::new();
        lib.put("Paper Boats.lrc", "[00:01]other song");
        let plain = lib.put("The Lanterns - Paper Boats.lyrics", "first\n\nsecond");
        let found = lookup(&lib.0, "Paper Boats", "The Lanterns", "", || true).unwrap().unwrap();
        assert_eq!(found.path, plain);
        assert!(!found.lyrics.synced);
        assert_eq!(found.lyrics.lines[1].text, "");
        assert!(lookup(&lib.0, "Missing", "", "", || true).unwrap().is_none());
        lib.put("1979.lrc", "[00:01]a numeric title");
        lib.put("7 Years.txt", "a title with a leading number");
        assert!(lookup(&lib.0, "1979", "", "", || true).unwrap().is_some());
        assert!(lookup(&lib.0, "7 Years", "", "", || true).unwrap().is_some());
    }

    #[test]
    fn duplicate_titles_require_a_choice_and_cancellation_stops_a_scan() {
        let lib = Library::new();
        let path = lib.put("One/Song.lrc", "[00:01]one");
        lib.put("Two/Song.lrc", "[00:01]two");
        assert!(lookup(&lib.0, "Song", "", "", || true).is_err());
        assert_eq!(load_file(&path).unwrap().lyrics.lines[0].text, "one");
        assert_eq!(lookup(&lib.0, "Song", "", "", || false).unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn utf_boms_and_file_limits() {
        let lib = Library::new();
        let utf8 = lib.put("UTF8.txt", "\u{feff}words");
        assert_eq!(load_file(&utf8).unwrap().lyrics.lines[0].text, "words");
        for little in [true, false] {
            let mut bytes = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
            for word in "[00:02]日本語".encode_utf16() {
                bytes.extend_from_slice(&if little { word.to_le_bytes() } else { word.to_be_bytes() });
            }
            let file = lib.put("UTF16.lrc", bytes);
            assert_eq!(load_file(&file).unwrap().lyrics.lines[0].text, "日本語");
        }
        let huge = lib.put("Huge.txt", vec![b'x'; MAX_FILE_BYTES as usize + 1]);
        assert!(load_file(&huge).is_err());
        assert!(load_file(&lib.put("Invalid.txt", [0xff])).is_err());
        assert!(load_file(&lib.put("Empty.lrc", "[ti:Song]")).is_err());
    }
}
