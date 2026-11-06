//! One requested LRCLIB result, one optional file write. No scans or timers.
use super::{invalid, MAX_FILE_BYTES};
use crate::lrc::Lyrics;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn header(text: &str) -> String {
    text.chars().filter(|c|!c.is_control()).map(|c|match c {'['=>'(',']'=>')',_=>c}).collect()
}

/// Never overwrite an existing sidecar. Empty/instrumental results create
/// nothing. The caller keeps showing lyrics even when a folder is unavailable.
pub fn save(root: &Path, song_file:&Path, artist: &str, title: &str, lyrics: &Lyrics) -> io::Result<Option<PathBuf>> {
    if title.trim().is_empty() || lyrics.is_empty() {return Ok(None);}
    if super::extension(song_file)!=Some(2) || !fs::symlink_metadata(song_file).is_ok_and(|m|m.is_file() && !super::linked(&m)) {return Ok(None);}
    let sidecar=song_file.with_extension("lrc");
    let Some(name)=sidecar.file_name() else{return Ok(None);};
    let path=root.join(name);
    let mut text=format!("[ar:{}]\n[ti:{}]\n[re:Lumo LRCLIB]\n",header(artist),header(title));
    for line in &lyrics.lines {
        if lyrics.synced {
            if !line.time_s.is_finite() || line.time_s<0.0 {return Err(invalid("Invalid lyric timestamp."));}
            let ms=(line.time_s*1000.0).round().min(u64::MAX as f64) as u64;
            text.push_str(&format!("[{:02}:{:02}.{:03}]",ms/60000,ms/1000%60,ms%1000));
        }
        text.push_str(&line.text.replace('\r',""));text.push('\n');
        if text.len() as u64>MAX_FILE_BYTES {return Err(invalid("Downloaded lyrics exceed 1 MB."));}
    }
    let mut file=match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file)=>file,
        Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>return Ok(None),
        Err(e)=>return Err(e),
    };
    if let Err(error)=file.write_all(text.as_bytes()) {
        drop(file);
        // Only this newly created file is removed after an incomplete write.
        let _=fs::remove_file(&path);
        return Err(error);
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lrc::parse_lrc;
    use std::sync::atomic::{AtomicU64,Ordering};
    struct Folder(PathBuf);
    impl Folder {
        fn new()->Self {
            static NEXT:AtomicU64=AtomicU64::new(0);
            let path=std::env::temp_dir().join(format!("lumo-lyric-save-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
            fs::create_dir_all(&path).unwrap();Self(path)
        }
    }
    impl Drop for Folder {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    #[test]
    fn synced_downloads_roundtrip_and_never_replace_an_existing_file() {
        let folder=Folder::new();let lyrics=parse_lrc("[00:01.250]First\n[01:02.345]日本語");
        let song=folder.0.join("01 - real recording (2026).mp3");fs::write(&song,[]).unwrap();
        let path=save(&folder.0,&song,"The Lanterns","Paper Boats",&lyrics).unwrap().unwrap();
        assert_eq!(path.file_name().unwrap(),"01 - real recording (2026).lrc");
        assert_eq!(super::super::load_file(&path).unwrap().lyrics,lyrics);
        assert!(super::super::load_file(&path).unwrap().downloaded);
        fs::write(&path,"A hand-edited sidecar").unwrap();
        assert!(save(&folder.0,&song,"The Lanterns","Paper Boats",&lyrics).unwrap().is_none());
        assert_eq!(fs::read_to_string(path).unwrap(),"A hand-edited sidecar");
    }
    #[test]
    fn plain_downloads_keep_stanzas_and_empty_results_write_nothing() {
        let folder=Folder::new();let lyrics=Lyrics::plain("First\n\nSecond");
        let song=folder.0.join("original name.opus");fs::write(&song,[]).unwrap();
        let path=save(&folder.0,&song,"","Song",&lyrics).unwrap().unwrap();
        assert_eq!(path.file_name().unwrap(),"original name.lrc");
        assert_eq!(super::super::load_file(&path).unwrap().lyrics,lyrics);
        assert!(save(&folder.0,&song,"Artist","Empty",&Lyrics::default()).unwrap().is_none());
        assert!(save(&folder.0,&song,"Artist","",&lyrics).unwrap().is_none());
        assert_eq!(fs::read_dir(&folder.0).unwrap().count(),2);
    }
    #[test]
    fn missing_audio_files_do_not_create_invented_filenames() {
        let folder=Folder::new();let lyrics=Lyrics::plain("Words");
        assert!(save(&folder.0,&folder.0.join("missing.mp3"),"Artist","Title",&lyrics).unwrap().is_none());
        assert_eq!(fs::read_dir(&folder.0).unwrap().count(),0);
    }
}
