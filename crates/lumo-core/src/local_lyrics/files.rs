//! Associate a track with a real file in the selected library, only on request.
use super::{extension, linked, normalized, name_key, rating, stem_key, MAX_DEPTH, MAX_ENTRIES};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_METADATA_FILES: usize = 256;
#[derive(Default)]
pub struct Tags { pub title:String, pub artist:String, pub album:String }

/// Prefer an unambiguous tag match. Filename matches remain useful for files
/// without readable tags. A large library does not trigger unlimited metadata
/// reads: exact/decorated filename candidates are checked first, at most 256.
pub fn find(root:&Path,title:&str,artist:&str,album:&str,live:impl Fn()->bool,
    mut tags:impl FnMut(&Path)->Option<Tags>)->io::Result<Option<PathBuf>> {
    let expected_rating=rating(title);
    let title=name_key(title);let artist=normalized(artist);let album=normalized(album);
    if title.is_empty(){return Ok(None);}
    let full=[format!("{artist}{title}"),format!("{title}{artist}")];
    let mut pending=vec![(root.to_path_buf(),0usize)];let mut files=Vec::new();let mut named=Vec::new();let mut visited=0;
    let mut large=false;
    while let Some((dir,depth))=pending.pop() {
        if !live(){return Err(io::ErrorKind::Interrupted.into());}
        let entries=match fs::read_dir(&dir){Ok(v)=>v,Err(e) if dir==root=>return Err(e),Err(_)=>continue};
        for entry in entries {
            if !live(){return Err(io::ErrorKind::Interrupted.into());}
            visited+=1;if visited>MAX_ENTRIES{return Ok(None);}
            let Ok(entry)=entry else{continue};let path=entry.path();
            let Ok(meta)=fs::symlink_metadata(&path) else{continue};if linked(&meta){continue;}
            if meta.is_dir(){if depth<MAX_DEPTH{pending.push((path,depth+1));}continue;}
            if !meta.is_file() || extension(&path)!=Some(2){continue;}
            let Some(stem)=path.file_stem().and_then(|s|s.to_str()) else{continue};
            let raw=name_key(stem);let key=stem_key(stem);
            if expected_rating.zip(rating(stem)).is_some_and(|(expected,found)|expected!=found){continue;}
            let score=if !artist.is_empty() && (full.contains(&raw)||full.contains(&key)){300}
                else if raw==title || key==title{200}
                else if !artist.is_empty() && raw.contains(&title) && raw.contains(&artist){100} else{0};
            if score>0 {
                named.push((score,path.clone()));if named.len()>MAX_METADATA_FILES{return Ok(None);}
            }
            if files.len()<MAX_METADATA_FILES{files.push((score,path));}else{large=true;}
        }
    }
    let candidates=if !named.is_empty(){named}else if large{return Ok(None);}else{files};
    let mut matches=Vec::new();
    for (filename_score,path) in candidates {
        if !live(){return Err(io::ErrorKind::Interrupted.into());}
        let score=match tags(&path).filter(|m|!m.title.trim().is_empty()) {
            Some(m)=>{
                if name_key(&m.title)!=title || (!artist.is_empty() && normalized(&m.artist)!=artist){continue;}
                if !album.is_empty() && !m.album.trim().is_empty() && normalized(&m.album)!=album{continue;}
                let found_rating=rating(&m.title).or_else(||path.file_stem().and_then(|s|s.to_str()).and_then(rating));
                if expected_rating.zip(found_rating).is_some_and(|(expected,found)|expected!=found){continue;}
                1000+if expected_rating.is_some() && found_rating==expected_rating {2000}else{0}
            }
            None if filename_score>=200=>filename_score+if expected_rating.is_some() && path.file_stem().and_then(|s|s.to_str()).and_then(rating)==expected_rating {2000}else{0},
            _=>continue,
        };
        matches.push((score,path));
    }
    matches.sort_by(|a,b|b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    if let Some(best)=matches.first(){
        if matches.get(1).is_some_and(|other|other.0==best.0){return Ok(None);}
        return Ok(Some(best.1.clone()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64,Ordering};
    struct Folder(PathBuf);
    impl Folder{fn new()->Self{static N:AtomicU64=AtomicU64::new(0);let p=std::env::temp_dir().join(format!("lumo-song-match-{}-{}",std::process::id(),N.fetch_add(1,Ordering::Relaxed)));fs::create_dir_all(&p).unwrap();Self(p)}}
    impl Drop for Folder{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    #[test]
    fn tags_resolve_the_real_filename_and_duplicates_are_not_guessed(){
        let f=Folder::new();let path=f.0.join("01 - actual recording (2026).opus");fs::write(&path,[]).unwrap();
        let read=|_:&Path|Some(Tags{title:"Paper Boats".into(),artist:"The Lanterns".into(),album:"Harbor".into()});
        assert_eq!(find(&f.0,"Paper Boats","The Lanterns","Harbor",||true,read).unwrap(),Some(path));
        fs::write(f.0.join("second-copy.mp3"),[]).unwrap();
        assert!(find(&f.0,"Paper Boats","The Lanterns","Harbor",||true,read).unwrap().is_none());
        assert_eq!(find(&f.0,"Paper Boats","The Lanterns","Harbor",||false,read).unwrap_err().kind(),io::ErrorKind::Interrupted);
    }
    #[test]
    fn explicit_badges_find_decorated_files_in_large_libraries_without_scanning_all_tags(){
        let f=Folder::new();
        for i in 0..300{fs::write(f.0.join(format!("Unrelated {i}.mp3")),[]).unwrap();}
        let explicit=f.0.join("The Lanterns - Paper Boats (Explicit).m4a");fs::write(&explicit,[]).unwrap();
        let clean=f.0.join("The Lanterns - Paper Boats (Clean).m4a");fs::write(&clean,[]).unwrap();
        let mut reads=0;
        let found=find(&f.0,"Paper Boats 🅴","The Lanterns","Harbor",||true,|path|{
            reads+=1;Some(Tags{title:if path==explicit {"Paper Boats 🅴"} else {"Paper Boats"}.into(),artist:"The Lanterns".into(),album:"Harbor".into()})
        }).unwrap();
        assert_eq!(found,Some(explicit));assert_eq!(reads,1,"only the compatible filename needs metadata");
    }

    #[test]
    fn filenames_are_a_fallback_but_conflicting_tags_are_rejected(){
        let f=Folder::new();let path=f.0.join("01 - The Lanterns - Paper Boats.mp3");fs::write(&path,[]).unwrap();
        assert_eq!(find(&f.0,"Paper Boats","The Lanterns","",||true,|_|None).unwrap(),Some(path));
        assert!(find(&f.0,"Paper Boats","The Lanterns","",||true,|_|Some(Tags{title:"Another song".into(),..Default::default()})).unwrap().is_none());
    }
}
