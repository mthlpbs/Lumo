//! Bounded metadata reads only; never decode audio or load a whole song.
//! ID3: id3.org/id3v2.3.0 and id3v2.4.0-structure; FLAC: RFC 9639;
//! Ogg comments: xiph.org/vorbis/doc/v-comment.html (also OpusTags).
use super::{invalid, MAX_FILE_BYTES};
use crate::lrc::{parse_lrc, LyricLine, Lyrics};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const MAX_TAG: usize = 8 * 1024 * 1024;
const MAX_PARTS: usize = 4096;

pub(super) fn read(path: &Path) -> io::Result<Option<Lyrics>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < 4 { return Ok(None); }
    let mut magic = [0; 4]; file.read_exact(&mut magic)?;
    file.rewind()?;
    if &magic[..3] == b"ID3" { id3(&mut file) }
    else if &magic == b"fLaC" { flac(&mut file) }
    else if &magic == b"OggS" { ogg(&mut file) }
    else if matches!(path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase().as_str(), "m4a"|"mp4"|"m4b") {
        mp4(&mut file, 0, len, 0, false, &mut 0)
    } else { Ok(None) }
}

fn bytes(file: &mut File, len: usize, limit: usize) -> io::Result<Vec<u8>> {
    if len > limit { return Err(invalid("Embedded metadata exceeds the read limit.")); }
    let mut data = vec![0; len]; file.read_exact(&mut data)?; Ok(data)
}
fn be(data: &[u8]) -> Option<usize> { data.iter().try_fold(0usize, |n,&b| n.checked_mul(256)?.checked_add(b as usize)) }
fn sync(data: &[u8]) -> Option<usize> { data.iter().try_fold(0usize, |n,&b| if b<128 { n.checked_mul(128)?.checked_add(b as usize) } else { None }) }
fn take<'a>(data: &mut &'a [u8], n: usize) -> Option<&'a [u8]> { let out=data.get(..n)?; *data=&data[n..]; Some(out) }
fn le_field<'a>(data: &mut &'a [u8]) -> Option<&'a [u8]> {
    let n=u32::from_le_bytes(take(data,4)?.try_into().ok()?) as usize; take(data,n)
}
fn text_lyrics(text: &str) -> Option<Lyrics> {
    if text.len() as u64 > MAX_FILE_BYTES { return None; }
    let text=text.trim_matches('\0').trim_start_matches('\u{feff}');
    let lyrics=parse_lrc(text);
    (!lyrics.is_empty()).then_some(lyrics)
}
fn choose(best: &mut Option<Lyrics>, next: Option<Lyrics>) {
    if let Some(next)=next { if best.is_none() || (next.synced && !best.as_ref().unwrap().synced) { *best=Some(next); } }
}
fn unsync(data: &[u8]) -> Vec<u8> {
    data.iter().enumerate().filter_map(|(i,&b)| (!(b==0 && i>0 && data[i-1]==255)).then_some(b)).collect()
}
fn string(data: &[u8], enc: u8, little: bool) -> Option<String> {
    let text=match enc {
        0 => data.iter().map(|&b|char::from(b)).collect(),
        3 => std::str::from_utf8(data).ok()?.to_owned(),
        1|2 => {
            let (data,little)=if data.starts_with(&[255,254]) {(&data[2..],true)} else if data.starts_with(&[254,255]) {(&data[2..],false)} else {(data,enc==1 && little)};
            if data.len()%2!=0 { return None; }
            String::from_utf16(&data.chunks_exact(2).map(|p| if little {u16::from_le_bytes([p[0],p[1]])} else {u16::from_be_bytes([p[0],p[1]])}).collect::<Vec<_>>()).ok()?
        }
        _ => return None,
    };
    Some(text.trim_matches('\0').to_owned())
}
fn terminated<'a>(data: &mut &'a [u8], enc: u8) -> Option<&'a [u8]> {
    let width=if enc==1 || enc==2 {2} else {1};
    let end=data.chunks_exact(width).position(|p|p.iter().all(|&b|b==0))?*width;
    let value=take(data,end)?; take(data,width)?; Some(value)
}
fn lyric_key(key: &str) -> bool { matches!(key.to_ascii_uppercase().as_str(), "LYRICS"|"UNSYNCEDLYRICS"|"UNSYNCED LYRICS"|"SYNCEDLYRICS"|"SYNCED LYRICS") }

fn id3(file: &mut File) -> io::Result<Option<Lyrics>> {
    let mut header=[0;10]; file.read_exact(&mut header)?;
    let ver=header[3]; if !(2..=4).contains(&ver) || (ver==2 && header[5]&0x40!=0) { return Ok(None); }
    let len=sync(&header[6..]).ok_or_else(||invalid("Invalid ID3 size."))?;
    let raw=bytes(file,len,MAX_TAG)?;
    let decoded;
    let mut data=if ver<4 && header[5]&0x80!=0 { decoded=unsync(&raw); decoded.as_slice() } else {raw.as_slice()};
    if ver>=3 && header[5]&0x40!=0 {
        let size=if ver==4 {sync(data.get(..4).unwrap_or(&[]))} else {be(data.get(..4).unwrap_or(&[])).and_then(|s|s.checked_add(4))};
        let Some(size)=size.filter(|&s|s>=4 && s<=data.len()) else {return Err(invalid("Invalid ID3 extended header."));};
        data=&data[size..];
    }
    let mut best=None;
    for _ in 0..MAX_PARTS {
        let head=if ver==2 {6} else {10};
        if data.len()<head || data[0]==0 {break;}
        let id=&data[..if ver==2 {3} else {4}];
        if !id.iter().all(|b|b.is_ascii_uppercase() || b.is_ascii_digit()) {break;}
        let size=if ver==2 {be(&data[3..6])} else if ver==4 {sync(&data[4..8])} else {be(&data[4..8])};
        let Some(size)=size else {break;};
        let flags=if ver==2 {0} else {data[9]};
        data=&data[head..];
        let Some(mut body)=take(&mut data,size) else {return Err(invalid("Truncated ID3 frame."));};
        if (ver==3 && flags&0xc0!=0) || (ver==4 && flags&0x0c!=0) {continue;}
        if !matches!(id,b"USLT"|b"ULT"|b"SYLT"|b"SLT"|b"TXXX"|b"TXX") {continue;}
        if body.len() as u64>MAX_FILE_BYTES {continue;}
        let decoded;
        if ver==4 && (header[5]&0x80!=0 || flags&2!=0) {decoded=unsync(body);body=&decoded;}
        if (ver==3 && flags&0x20!=0) || (ver==4 && flags&0x40!=0) {let Some(rest)=body.get(1..) else {continue;};body=rest;}
        if ver==4 && flags&1!=0 {let Some(rest)=body.get(4..) else {continue;};body=rest;}
        choose(&mut best,id3_lyrics(id,body));
    }
    Ok(best)
}
fn id3_lyrics(id: &[u8], mut body: &[u8]) -> Option<Lyrics> {
    let enc=*take(&mut body,1)?.first()?;
    let timed=matches!(id,b"SYLT"|b"SLT");
    let custom=matches!(id,b"TXXX"|b"TXX");
    if !custom {take(&mut body,3)?;}
    if timed {
        let format=take(&mut body,2)?;
        if format[0]!=2 {return None;} // MPEG-frame timestamps need an audio decoder; use other tags/LRCLIB.
    }
    let little=body.starts_with(&[255,254]);
    let desc=string(terminated(&mut body,enc)?,enc,little)?;
    if custom && !lyric_key(&desc) {return None;}
    if !timed {return text_lyrics(&string(body,enc,little)?);}
    let mut parts=Vec::new();
    while !body.is_empty() && parts.len()<MAX_PARTS {
        let text=string(terminated(&mut body,enc)?,enc,little)?;
        let time=be(take(&mut body,4)?)? as f64/1000.0;
        parts.push((time,text));
    }
    let newline=parts.iter().any(|(_,s)|s.contains(['\r','\n']));
    let mut lines:Vec<LyricLine>=Vec::new();
    for (time,text) in parts {
        if !newline {if !text.trim().is_empty() {lines.push(LyricLine{time_s:time,text});} continue;}
        for (i,part) in text.replace("\r\n","\n").replace('\r',"\n").split('\n').enumerate() {
            if i>0 || lines.is_empty() {lines.push(LyricLine{time_s:time,text:String::new()});}
            lines.last_mut()?.text.push_str(part);
        }
    }
    lines.retain(|l|!l.text.trim().is_empty());
    lines.sort_by(|a,b|a.time_s.total_cmp(&b.time_s));
    (!lines.is_empty()).then_some(Lyrics{lines,synced:true})
}

fn comments(mut data: &[u8]) -> Option<Lyrics> {
    le_field(&mut data)?; // Vendor.
    let count=u32::from_le_bytes(take(&mut data,4)?.try_into().ok()?) as usize;
    let mut best=None;
    for _ in 0..count.min(MAX_PARTS) {
        let field=std::str::from_utf8(le_field(&mut data)?).ok()?;
        if let Some((key,value))=field.split_once('=') {if lyric_key(key) {choose(&mut best,text_lyrics(value));}}
    }
    best
}
fn flac(file: &mut File) -> io::Result<Option<Lyrics>> {
    file.seek(SeekFrom::Start(4))?;
    for _ in 0..MAX_PARTS {
        let mut header=[0;4];file.read_exact(&mut header)?;
        let len=be(&header[1..]).unwrap();
        if header[0]&127==4 {return Ok(comments(&bytes(file,len,MAX_TAG)?));}
        if header[0]&128!=0 {break;}
        file.seek(SeekFrom::Current(len as i64))?;
    }
    Ok(None)
}
fn ogg(file: &mut File) -> io::Result<Option<Lyrics>> {
    Ok(ogg_comments(file)?.as_deref().and_then(comments))
}
fn ogg_comments(file: &mut File) -> io::Result<Option<Vec<u8>>> {
    let mut packet=Vec::new(); let mut serial=None; let mut packets=0;
    for _ in 0..256 {
        let mut header=[0;27];file.read_exact(&mut header)?;
        if &header[..4]!=b"OggS" || header[4]!=0 {return Err(invalid("Invalid Ogg page."));}
        let stream=&header[14..18];
        let selected=serial.get_or_insert_with(||stream.to_vec())==stream;
        let sizes=bytes(file,header[26] as usize,255)?;
        if !selected {file.seek(SeekFrom::Current(sizes.iter().map(|&b|b as i64).sum()))?;continue;}
        for size in sizes {
            if packet.len()+size as usize>MAX_TAG {return Err(invalid("Embedded Ogg tags exceed the read limit."));}
            let at=packet.len();packet.resize(at+size as usize,0);file.read_exact(&mut packet[at..])?;
            if size<255 {
                if let Some(tags)=packet.strip_prefix(b"OpusTags").or_else(||packet.strip_prefix(b"\x03vorbis")) {return Ok(Some(tags.to_vec()));}
                packets+=1;
                if packets>=3 {return Ok(None);}
                packet.clear();
            }
        }
    }
    Ok(None)
}

pub(super) fn audio_tags(path:&Path)->io::Result<Option<super::files::Tags>> {
    let mut file=File::open(path)?;let mut magic=[0;4];
    if file.read_exact(&mut magic).is_err() || &magic!=b"OggS" {return Ok(None);}
    file.rewind()?;
    let Some(data)=ogg_comments(&mut file)? else{return Ok(None);};
    let mut data=data.as_slice();
    let Some(_)=le_field(&mut data) else{return Ok(None);};
    let Some(count)=take(&mut data,4).and_then(|v|v.try_into().ok()).map(u32::from_le_bytes) else{return Ok(None);};
    let mut tags=super::files::Tags::default();
    for _ in 0..(count as usize).min(MAX_PARTS) {
        let Some(field)=le_field(&mut data).and_then(|v|std::str::from_utf8(v).ok()) else{return Ok(None);};
        if let Some((key,value))=field.split_once('=') {
            match key.to_ascii_uppercase().as_str(){"TITLE"=>tags.title=value.into(),"ARTIST"=>{if !tags.artist.is_empty(){tags.artist.push_str("; ");}tags.artist.push_str(value);},"ALBUM"=>tags.album=value.into(),_=>{}}
        }
    }
    Ok((!tags.title.is_empty()).then_some(tags))
}
fn mp4(file: &mut File, mut at:u64, end:u64, depth:usize, lyric:bool, count:&mut usize) -> io::Result<Option<Lyrics>> {
    if depth>8 {return Err(invalid("Embedded MP4 metadata is too deeply nested."));}
    while at.checked_add(8).is_some_and(|n|n<=end) {
        *count+=1;if *count>MAX_PARTS {return Err(invalid("Too many MP4 metadata atoms."));}
        file.seek(SeekFrom::Start(at))?;
        let mut h=[0;8];file.read_exact(&mut h)?;
        let small=u32::from_be_bytes(h[..4].try_into().unwrap());
        let (size,head)=if small==1 {let mut b=[0;8];file.read_exact(&mut b)?;(u64::from_be_bytes(b),16)} else if small==0 {(end-at,8)} else {(small as u64,8)};
        let Some(next)=at.checked_add(size).filter(|&n|size>=head && n<=end) else {return Err(invalid("Invalid MP4 metadata atom."));};
        let kind=&h[4..];
        if lyric && kind==b"data" {
            let len=usize::try_from(size-head).map_err(|_|invalid("Invalid MP4 metadata size."))?;
            let data=bytes(file,len,MAX_FILE_BYTES as usize+8)?;
            if data.len()>=8 {
                let enc=be(&data[..4]).unwrap();
                if let Some(text)=match enc {1=>string(&data[8..],3,false),2=>string(&data[8..],2,false),_=>None} {
                    if let Some(lyrics)=text_lyrics(&text) {return Ok(Some(lyrics));}
                }
            }
        } else if matches!(kind,b"moov"|b"udta"|b"meta"|b"ilst"|b"\xa9lyr") {
            let start=at+head+if kind==b"meta" {4} else {0};
            if start<=next {if let Some(lyrics)=mp4(file,start,next,depth+1,kind==b"\xa9lyr",count)? {return Ok(Some(lyrics));}}
        }
        at=next;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64,Ordering};
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(ext:&str,data:&[u8])->Self {
            static NEXT:AtomicU64=AtomicU64::new(0);
            let path=std::env::temp_dir().join(format!("lumo-embedded-{}-{}.{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed),ext));
            std::fs::write(&path,data).unwrap();Self(path)
        }
        fn lyrics(&self)->Lyrics {read(&self.0).unwrap().unwrap()}
    }
    impl Drop for Fixture {fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}
    fn syn(n:usize)->[u8;4] {[(n>>21) as u8&127,(n>>14) as u8&127,(n>>7) as u8&127,n as u8&127]}
    fn tag(ver:u8,frames:&[(&[u8],Vec<u8>)])->Vec<u8> {
        let mut data=Vec::new();
        for (id,body) in frames {
            data.extend_from_slice(id);
            if ver==2 {data.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);}
            else {data.extend_from_slice(&if ver==4 {syn(body.len())} else {(body.len() as u32).to_be_bytes()});data.extend_from_slice(&[0,0]);}
            data.extend_from_slice(body);
        }
        let mut out=vec![b'I',b'D',b'3',ver,0,0];out.extend_from_slice(&syn(data.len()));out.extend(data);out
    }
    fn uslt(enc:u8,text:&str)->Vec<u8> {
        let mut data=vec![enc,b'e',b'n',b'g'];
        if enc==1 {data.extend_from_slice(&[255,254,0,0,255,254]);for w in text.encode_utf16(){data.extend_from_slice(&w.to_le_bytes());}}
        else if enc==2 {data.extend_from_slice(&[0,0]);for w in text.encode_utf16(){data.extend_from_slice(&w.to_be_bytes());}}
        else {data.push(0);data.extend_from_slice(text.as_bytes());}
        data
    }
    #[test]
    fn id3_versions_and_unicode_encodings_keep_lrc_timestamps() {
        for ver in [2,3,4] {for enc in [1,2,3] {
            let data=tag(ver,&[(if ver==2 {b"ULT"} else {b"USLT"},uslt(enc,"[00:01.25]日本語\n[00:05]Second"))]);
            let lyrics=Fixture::new("mp3",&data).lyrics();
            assert!(lyrics.synced);assert_eq!(lyrics.lines[0].text,"日本語");assert_eq!(lyrics.lines[0].time_s,1.25);
        }}
        let data=tag(3,&[(b"USLT",vec![0,b'e',b'n',b'g',0,b'C',b'a',b'f',0xe9])]);
        let lyrics=Fixture::new("mp3",&data).lyrics();assert!(!lyrics.synced);assert_eq!(lyrics.lines[0].text,"Café");
    }
    #[test]
    fn sylt_prefers_timed_lyrics_and_joins_syllables_at_line_breaks() {
        let mut sylt=vec![3,b'e',b'n',b'g',2,1,0];
        for (ms,text) in [(1000u32,"Paper "),(1400,"boats"),(5000,"\nSail home")] {
            sylt.extend_from_slice(text.as_bytes());sylt.push(0);sylt.extend_from_slice(&ms.to_be_bytes());
        }
        let data=tag(4,&[(b"USLT",uslt(3,"Plain words")),(b"SYLT",sylt)]);
        let lyrics=Fixture::new("mp3",&data).lyrics();assert!(lyrics.synced);assert_eq!(lyrics.lines.len(),2);
        assert_eq!(lyrics.lines[0].text,"Paper boats");assert_eq!(lyrics.lines[1].time_s,5.0);
        let mut custom=vec![3];custom.extend_from_slice(b"LYRICS\0[00:02]Custom tag");
        assert!(Fixture::new("mp3",&tag(3,&[(b"TXXX",custom)])).lyrics().synced);
    }
    fn comment(text:&str)->Vec<u8> {
        let field=format!("LYRICS={text}");let mut data=vec![0;4];data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&(field.len() as u32).to_le_bytes());data.extend_from_slice(field.as_bytes());data
    }
    fn page(serial:u32,flags:u8,sizes:&[u8],payload:&[u8])->Vec<u8> {
        let mut data=b"OggS".to_vec();data.extend_from_slice(&[0,flags]);data.extend_from_slice(&[0;8]);data.extend_from_slice(&serial.to_le_bytes());
        data.extend_from_slice(&[0;8]);data.push(sizes.len() as u8);data.extend_from_slice(sizes);data.extend_from_slice(payload);data
    }
    #[test]
    fn flac_opus_and_vorbis_comments_read_synced_and_plain_lyrics() {
        let text="[00:01]First\n[00:02]Second";let tags=comment(text);
        let mut flac=b"fLaC".to_vec();flac.push(0x84);flac.extend_from_slice(&(tags.len() as u32).to_be_bytes()[1..]);flac.extend_from_slice(&tags);
        assert!(Fixture::new("flac",&flac).lyrics().synced);
        for (ext,ident,head) in [("opus",b"OpusHead".as_slice(),b"OpusTags".as_slice()),("ogg",b"\x01vorbis",b"\x03vorbis")] {
            let mut file=page(1,2,&[ident.len() as u8],ident);
            let mut packet=head.to_vec();packet.extend_from_slice(&comment(&format!("{text}\n{}","x".repeat(300))));
            file.extend(page(1,0,&[255],&packet[..255]));
            // Another logical stream must not contaminate a continued packet.
            file.extend(page(2,2,&[3],b"bad"));
            file.extend(page(1,1,&[(packet.len()-255) as u8],&packet[255..]));
            let lyrics=Fixture::new(ext,&file).lyrics();assert!(lyrics.synced);assert_eq!(lyrics.lines[1].text,"Second");
        }
        assert!(!comments(&comment("First\n\nSecond")).unwrap().synced);
    }
    fn atom(kind:&[u8;4],body:&[u8])->Vec<u8> {let mut out=((body.len()+8) as u32).to_be_bytes().to_vec();out.extend_from_slice(kind);out.extend_from_slice(body);out}
    #[test]
    fn m4a_reads_lyrics_after_skipping_audio_atoms() {
        let mut data=vec![0,0,0,1,0,0,0,0];data.extend_from_slice(b"[00:02]First\n[00:04]Second");
        let item=atom(b"\xa9lyr",&atom(b"data",&data));
        let mut meta=vec![0;4];meta.extend(atom(b"ilst",&item));
        let mut file=atom(b"ftyp",b"M4A \0\0\0\0");file.extend(atom(b"mdat",&vec![0;1024]));file.extend(atom(b"moov",&atom(b"udta",&atom(b"meta",&meta))));
        let lyrics=Fixture::new("m4a",&file).lyrics();assert!(lyrics.synced);assert_eq!(lyrics.lines[0].time_s,2.0);
    }
    #[test]
    fn malformed_lengths_and_large_tags_fail_without_reading_audio() {
        let mut oversized=b"ID3\x04\0\0".to_vec();oversized.extend_from_slice(&syn(MAX_TAG+1));
        assert!(read(&Fixture::new("mp3",&oversized).0).is_err());
        let good=tag(4,&[(b"USLT",uslt(3,"[00:01]First"))]);
        for len in 4..good.len() {assert!(read(&Fixture::new("mp3",&good[..len]).0).is_err());}
        assert!(read(&Fixture::new("m4a",b"\0\0\0\x04moov").0).is_err());
        assert!(read(&Fixture::new("flac",b"fLaC\x84\xff\xff\xff").0).is_err());
    }
}
