use lumo_core::lrc::Lyrics;
use lumo_core::romanize::{self, Ruby, Script};
const HAN_PIECE_MAX: usize = 8;
#[derive(Default)]
pub struct Pronounced { pub ruby: Vec<Vec<Ruby>> }
pub fn pronounce(lyrics: &Lyrics) -> Pronounced {
    let texts: Vec<&str> = lyrics.lines.iter().map(|l| l.text.trim()).collect();
    let ruby = match romanize::song_script(&texts) {
        Some(Script::Korean) => texts.iter().map(|t| romanize::korean_ruby(t)).collect(),
        Some(Script::Chinese) => texts.iter().map(|t| split_long(romanize::chinese_ruby(t))).collect(),
        _ => Vec::new(),
    };
    Pronounced { ruby }
}
fn split_long(pieces: Vec<Ruby>) -> Vec<Ruby> {
    let mut out = Vec::with_capacity(pieces.len());
    for p in pieces {
        let chars: Vec<char> = p.text.chars().collect();
        let syllables: Vec<&str> = p.roman.split(' ').filter(|s| !s.is_empty()).collect();
        if chars.len() <= HAN_PIECE_MAX || syllables.len() != chars.len() {
            out.push(p);
            continue;
        }
        for (cs, ss) in chars.chunks(HAN_PIECE_MAX).zip(syllables.chunks(HAN_PIECE_MAX)) {
            out.push(Ruby { text: cs.iter().collect(), roman: ss.join(" ") });
        }
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn japanese_songs_have_no_readings() {
        assert!(pronounce(&lumo_core::lrc::parse_lrc("[00:00.00]君の名前" )).ruby.is_empty());
    }
    #[test]
    fn korean_and_chinese_songs_keep_their_readings() {
        for text in ["[00:00.00]안녕하세요", "[00:00.00]你好"] {
            let read = pronounce(&lumo_core::lrc::parse_lrc(text));
            assert!(read.ruby.iter().flatten().any(|r| !r.roman.is_empty()));
        }
    }
}
