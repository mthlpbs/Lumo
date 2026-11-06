//! Bounded, display-only Markdown for chat. No HTML execution or external loads.
//! Common chat blocks and GFM tables; unsupported constructs remain readable text.
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const CODE: u8 = 4;
pub const STRIKE: u8 = 8;
pub const LINK: u8 = 16;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub len: u32,
    pub flags: u8,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub start: u32,
    pub len: u32,
    pub url: String,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inline {
    pub utf16_len: u32,
    pub text: String,
    pub spans: Vec<Span>,
    pub links: Vec<Link>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Inline),
    Heading(u8, Inline),
    List {
        marker: String,
        depth: u8,
        content: Inline,
    },
    Quote(Inline),
    Code {
        language: String,
        content: Inline,
    },
    Rule,
    Table {
        rows: Vec<Vec<Inline>>,
        align: Vec<Align>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
}

pub fn safe_link(url: &str) -> bool {
    let s = url.trim();
    (s.starts_with("https://") || s.starts_with("http://") || s.starts_with("mailto:"))
        && !s.chars().any(char::is_control)
}
fn append(out: &mut Inline, text: &str, flags: u8) {
    let start = out.utf16_len;
    out.text.push_str(text);
    let len = text.encode_utf16().count() as u32;
    out.utf16_len += len;
    if flags != 0 && len > 0 {
        out.spans.push(Span { start, len, flags });
    }
}
fn entity(s: &str) -> Option<(String, usize)> {
    let end = s
        .char_indices()
        .take(13)
        .find(|(_, c)| *c == ';')
        .map(|(n, _)| n)?;
    let code = &s[1..end];
    let c = match code {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => {
            let n = code
                .strip_prefix("#x")
                .or_else(|| code.strip_prefix("#X"))
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .or_else(|| code.strip_prefix('#').and_then(|s| s.parse().ok()))?;
            char::from_u32(n)?
        }
    };
    Some((c.to_string(), end + 1))
}
fn find(s: &str, needle: &str, budget: &mut usize) -> Option<usize> {
    let found = s.find(needle);
    let work = found.map_or(s.len(), |n| n + needle.len());
    if work > *budget {
        *budget = 0;
        return None;
    }
    *budget -= work;
    found
}
fn inline_into(mut s: &str, flags: u8, depth: u8, out: &mut Inline, budget: &mut usize) {
    if depth > 12 {
        append(out, s, flags);
        return;
    }
    while !s.is_empty() {
        if *budget == 0 {
            append(out, s, flags);
            return;
        }
        if let Some(rest) = s.strip_prefix('\\') {
            if let Some(c) = rest.chars().next().filter(|c| c.is_ascii_punctuation()) {
                append(out, &c.to_string(), flags);
                s = &rest[c.len_utf8()..];
                continue;
            }
        }
        if s.starts_with('&') {
            if let Some((text, n)) = entity(s) {
                append(out, &text, flags);
                s = &s[n..];
                continue;
            }
        }
        if s.starts_with('`') {
            let n = s.bytes().take_while(|b| *b == b'`').count();
            let marker = &s[..n];
            if let Some(end) = find(&s[n..], marker, budget) {
                let code = s[n..n + end].replace('\n', " ");
                append(out, code.trim_matches(' '), flags | CODE);
                s = &s[n + end + n..];
                continue;
            }
        }
        let image = s.starts_with("![");
        let link = if image { &s[1..] } else { s };
        if let Some(rest) = link.strip_prefix('[') {
            if let Some(end) = find(rest, "]", budget).filter(|end| rest[*end..].starts_with("]("))
            {
                if let Some(close) = find(&rest[end + 2..], ")", budget) {
                    let raw = rest[end + 2..end + 2 + close].trim();
                    let url = raw
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_matches(['<', '>']);
                    let start = out.utf16_len;
                    inline_into(
                        &rest[..end],
                        flags | if !image && safe_link(url) { LINK } else { 0 },
                        depth + 1,
                        out,
                        budget,
                    );
                    let len = out.utf16_len - start;
                    if !image && safe_link(url) {
                        out.links.push(Link {
                            start,
                            len,
                            url: url.into(),
                        });
                    }
                    s = &rest[end + 3 + close..];
                    continue;
                }
            }
        }
        if s.starts_with('<') {
            if let Some(end) = find(s, ">", budget) {
                let url = &s[1..end];
                if safe_link(url) {
                    let start = out.utf16_len;
                    append(out, url, flags | LINK);
                    out.links.push(Link {
                        start,
                        len: url.encode_utf16().count() as u32,
                        url: url.into(),
                    });
                    s = &s[end + 1..];
                    continue;
                }
            }
        }
        let mut consumed = false;
        for (mark, bit) in [
            ("***", BOLD | ITALIC),
            ("___", BOLD | ITALIC),
            ("**", BOLD),
            ("__", BOLD),
            ("~~", STRIKE),
            ("*", ITALIC),
            ("_", ITALIC),
        ] {
            if let Some(rest) = s.strip_prefix(mark) {
                if rest.starts_with(char::is_whitespace) {
                    continue;
                }
                if mark == "_" && out.text.chars().last().is_some_and(char::is_alphanumeric) {
                    continue;
                }
                if let Some(end) = find(rest, mark, budget).filter(|n| *n > 0) {
                    if rest[..end].ends_with(char::is_whitespace) {
                        continue;
                    }
                    inline_into(&rest[..end], flags | bit, depth + 1, out, budget);
                    s = &rest[end + mark.len()..];
                    consumed = true;
                    break;
                }
            }
        }
        if consumed {
            continue;
        }
        // Copy a plain run, rather than allocating one span for every character.
        let n = s
            .char_indices()
            .skip(1)
            .find(|(_, c)| matches!(c, '\\' | '&' | '`' | '[' | '!' | '<' | '*' | '_' | '~'))
            .map(|(n, _)| n)
            .unwrap_or(s.len());
        append(out, &s[..n], flags);
        s = &s[n..];
    }
}
pub fn inline(s: &str) -> Inline {
    let mut out = Inline::default();
    let mut budget = s.len().saturating_mul(24).max(256);
    inline_into(s, 0, 0, &mut out, &mut budget);
    out
}
fn cells(line: &str) -> Vec<String> {
    let line = line.trim().trim_start_matches('|').trim_end_matches('|');
    let mut cells = vec![];
    let mut cell = String::new();
    let mut escaped = false;
    let mut code = false;
    for c in line.chars() {
        if escaped {
            cell.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            cell.push(c);
            continue;
        }
        if c == '`' {
            code = !code;
        }
        if c == '|' && !code {
            cells.push(cell.trim().to_owned());
            cell.clear();
        } else {
            cell.push(c);
        }
    }
    cells.push(cell.trim().to_owned());
    cells.truncate(12);
    cells
}
fn table_align(line: &str) -> Option<Vec<Align>> {
    let cs = cells(line);
    if cs.is_empty() {
        return None;
    }
    let mut out = vec![];
    for c in cs {
        let d = c.trim_matches(':');
        if d.len() < 3 || !d.chars().all(|c| c == '-') {
            return None;
        }
        out.push(if c.starts_with(':') && c.ends_with(':') {
            Align::Center
        } else if c.ends_with(':') {
            Align::Right
        } else {
            Align::Left
        });
    }
    Some(out)
}
fn rule(s: &str) -> bool {
    let chars: Vec<_> = s.chars().filter(|c| !c.is_whitespace()).collect();
    chars.len() >= 3 && matches!(chars[0], '-' | '*' | '_') && chars.iter().all(|c| *c == chars[0])
}
fn list(s: &str) -> Option<(String, &str)> {
    for mark in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(mark) {
            return Some(("•".into(), rest));
        }
    }
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    if n > 0 && n <= 9 && (s[n..].starts_with(". ") || s[n..].starts_with(") ")) {
        return Some((s[..n + 1].to_owned(), &s[n + 2..]));
    }
    None
}
fn starts_block(s: &str) -> bool {
    s.is_empty()
        || s.starts_with(['#', '>'])
        || s.starts_with("```")
        || s.starts_with("~~~")
        || s.starts_with("$$")
        || s.starts_with("\\[")
        || rule(s)
        || list(s).is_some()
}
pub fn parse(source: &str) -> Document {
    let bounded: String = source.chars().take(128 * 1024).collect();
    let lines: Vec<_> = bounded.lines().take(8192).collect();
    let mut doc = Document::default();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let s = raw.trim();
        if s.is_empty() {
            i += 1;
            continue;
        }
        if s.starts_with("$$") || s.starts_with("\\[") {
            let (open, close) = if s.starts_with("$$") {
                ("$$", "$$")
            } else {
                ("\\[", "\\]")
            };
            let rest = &s[open.len()..];
            let mut formula = String::new();
            let mut trailing = "";
            if let Some(end) = rest.find(close) {
                formula.push_str(rest[..end].trim());
                trailing = rest[end + close.len()..].trim();
                i += 1;
            } else {
                formula.push_str(rest.trim());
                i += 1;
                while i < lines.len() {
                    let line = lines[i];
                    i += 1;
                    if !formula.is_empty() {
                        formula.push('\n');
                    }
                    if let Some(end) = line.find(close) {
                        formula.push_str(line[..end].trim_end());
                        trailing = line[end + close.len()..].trim();
                        break;
                    }
                    formula.push_str(line);
                }
            }
            let mut content = Inline::default();
            append(&mut content, formula.trim_matches('\n'), CODE);
            doc.blocks.push(Block::Code {
                language: "LaTeX".into(),
                content,
            });
            if !trailing.is_empty() {
                doc.blocks.push(Block::Paragraph(inline(trailing)));
            }
            continue;
        }
        if s.starts_with("```") || s.starts_with("~~~") {
            let mark = s.chars().next().unwrap();
            let n = s.chars().take_while(|c| *c == mark).count();
            let language = s[n..].trim().chars().take(40).collect();
            i += 1;
            let mut text = String::new();
            let mut first = true;
            while i < lines.len() {
                if lines[i]
                    .trim_start()
                    .chars()
                    .take_while(|c| *c == mark)
                    .count()
                    >= n
                {
                    i += 1;
                    break;
                }
                if !first {
                    text.push('\n');
                }
                first = false;
                text.push_str(lines[i]);
                i += 1;
            }
            let mut content = Inline::default();
            append(&mut content, &text, CODE);
            doc.blocks.push(Block::Code { language, content });
            continue;
        }
        if rule(s) {
            doc.blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        let hashes = s.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && s[hashes..].starts_with(char::is_whitespace) {
            doc.blocks.push(Block::Heading(
                hashes as u8,
                inline(s[hashes..].trim().trim_end_matches('#').trim_end()),
            ));
            i += 1;
            continue;
        }
        if i + 1 < lines.len() && s.contains('|') {
            if let Some(align) = table_align(lines[i + 1]) {
                let mut rows = vec![cells(s).iter().map(|s| inline(s)).collect()];
                i += 2;
                while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                    let mut row: Vec<_> = cells(lines[i])
                        .iter()
                        .take(align.len())
                        .map(|s| inline(s))
                        .collect();
                    row.resize_with(align.len(), Inline::default);
                    rows.push(row);
                    i += 1;
                }
                doc.blocks.push(Block::Table { rows, align });
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix('>') {
            doc.blocks.push(Block::Quote(inline(rest.trim_start())));
            i += 1;
            continue;
        }
        if let Some((mut marker, rest)) = list(s) {
            let depth = ((raw.len() - raw.trim_start().len()) / 2).min(8) as u8;
            let rest = if let Some(rest) = rest
                .strip_prefix("[x] ")
                .or_else(|| rest.strip_prefix("[X] "))
            {
                marker = "☑".into();
                rest
            } else if let Some(rest) = rest.strip_prefix("[ ] ") {
                marker = "☐".into();
                rest
            } else {
                rest
            };
            doc.blocks.push(Block::List {
                marker,
                depth,
                content: inline(rest),
            });
            i += 1;
            continue;
        }
        if i + 1 < lines.len() {
            let next = lines[i + 1].trim();
            if next.len() >= 3 && (next.chars().all(|c| c == '=') || next.chars().all(|c| c == '-'))
            {
                doc.blocks.push(Block::Heading(
                    if next.starts_with('=') { 1 } else { 2 },
                    inline(s),
                ));
                i += 2;
                continue;
            }
        }
        let mut text = s.to_owned();
        i += 1;
        while i < lines.len() && !starts_block(lines[i].trim()) {
            if i + 1 < lines.len() && lines[i].contains('|') && table_align(lines[i + 1]).is_some()
            {
                break;
            }
            // Preserve supplied lyric lines and deliberate chat line breaks.
            text.push('\n');
            text.push_str(lines[i].trim());
            i += 1;
        }
        doc.blocks.push(Block::Paragraph(inline(&text)));
    }
    doc
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_math_preserves_copyable_latex_without_markdown_escapes() {
        let d = parse("Before\n\n$$\\frac{a_b}{c^2}$$\n\n\\[\n\\sum_{i=1}^{n} x_i\n\\]\n\nAfter");
        assert!(
            matches!(&d.blocks[1],Block::Code{language,content} if language=="LaTeX" && content.text==r"\frac{a_b}{c^2}")
        );
        assert!(
            matches!(&d.blocks[2],Block::Code{language,content} if language=="LaTeX" && content.text==r"\sum_{i=1}^{n} x_i")
        );
        assert!(matches!(&d.blocks[3],Block::Paragraph(content) if content.text=="After"));
    }
    #[test]
    fn song_structure_and_production_table_render_as_blocks() {
        let d=parse("# 🎧 Electric Echoes\n\n**Inspired** by synth pop\n\n---\n\n- Catchy *hook*\n\n| Element | Settings |\n|---|---|\n| Bass | Smooth sub-bass |\n\n> Soft pads");
        assert!(matches!(d.blocks[0], Block::Heading(1, _)));
        assert!(d
            .blocks
            .iter()
            .any(|b| matches!(b,Block::Table{rows,..} if rows.len()==2)));
        assert!(d.blocks.iter().any(|b| matches!(b, Block::List { .. })));
        assert!(d.blocks.iter().any(|b| matches!(b, Block::Rule)));
    }
    #[test]
    fn inline_styles_use_utf16_ranges_and_keep_lyrics_markers() {
        let i = inline("🎧 **Big** and *soft* `a_b` [Verse 1] [docs](https://example.org)");
        assert_eq!(i.text, "🎧 Big and soft a_b [Verse 1] docs");
        assert_eq!(
            i.spans[0],
            Span {
                start: 3,
                len: 3,
                flags: BOLD
            }
        );
        assert_eq!(i.links[0].url, "https://example.org");
        assert!(i.spans.iter().any(|s| s.flags == CODE));
    }
    #[test]
    fn code_and_unsafe_links_do_not_execute_or_load() {
        let d =
            parse("```rust\nlet a = **b;\n```\n\n[bad](javascript:alert)\n<script>data</script>");
        assert!(matches!(&d.blocks[0],Block::Code{content,..} if content.text=="let a = **b;"));
        let d = parse("```python\n\n  value = 1\n\n```");
        assert!(matches!(&d.blocks[0],Block::Code{content,..} if content.text=="\n  value = 1\n"));
        let i = inline("[bad](file:///secret)");
        assert!(i.links.is_empty());
        assert!(!safe_link("https://example.org\nattack"));
    }
    #[test]
    fn table_escaped_pipes_and_alignment() {
        let d = parse("|Name|Value|\n|:---|---:|\n|a\\|b|`x|y`|");
        assert!(
            matches!(&d.blocks[0],Block::Table{rows,align} if rows[1][0].text=="a|b" && rows[1][1].text=="x|y" && align[1]==Align::Right)
        );
    }
    #[test]
    fn malformed_links_have_a_bounded_scan_budget() {
        let source = "[".repeat(120000) + "](unfinished";
        let i = inline(&source);
        assert_eq!(i.text, source);
    }
    #[test]
    fn unclosed_markers_and_fences_remain_readable() {
        assert_eq!(
            inline("unclosed ** text_with_underscores").text,
            "unclosed ** text_with_underscores"
        );
        assert!(
            matches!(&parse("~~~\n*literal*").blocks[0],Block::Code{content,..} if content.text=="*literal*")
        );
    }
}
