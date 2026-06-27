//! BMFont `.fnt` (AngelCode text format) parser for the game's `gfx/fonts/*.fnt`.
//!
//! the game ships text-format `.fnt` files: `info` / `common` / `page` / `char` /
//! `kerning` lines. Glyphs reference a page (atlas index); each glyph carries a
//! source rect (x,y,w,h on the page), placement offsets (xoffset,yoffset), the
//! pen advance (xadvance), and a channel mask (`chnl`, optional). We also read
//! `lineHeight` / `base` and kerning pairs.
//!
//! String measurement honours xoffset/yoffset/xadvance + kerning and measures
//! the RESOLVED string the caller passes (after `$VAR$`/`§colour`/`£icon`
//! expansion happens upstream) — never the raw markup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub id: u32,
    pub page: u16,
    /// Source rect on the atlas page, in pixels.
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub xoffset: i16,
    pub yoffset: i16,
    pub xadvance: i16,
    /// BMFont channel mask: 1=blue 2=green 4=red 8=alpha 15=all. the game atlases
    /// store the glyph (and its baked outline/shadow) in the channels named here;
    /// 15 (or absent) => sample straight RGBA.
    pub chnl: u8,
}

#[derive(Clone, Debug)]
pub struct BmFont {
    pub face: String,
    pub size: i32,
    pub line_height: i32,
    pub base: i32,
    pub scale_w: u32,
    pub scale_h: u32,
    /// page id -> atlas file name as written in the `.fnt` (e.g. "Arial.dds").
    pub page_files: HashMap<u16, String>,
    /// Resolved absolute atlas paths per page (filled by [`resolve_pages`]).
    pub page_paths: HashMap<u16, PathBuf>,
    pub glyphs: HashMap<u32, Glyph>,
    /// (first, second) -> kerning amount (px).
    pub kernings: HashMap<(u32, u32), i16>,
}

#[derive(Debug)]
pub enum FontError {
    Io(String),
    Parse(String),
    MissingPage(String),
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::Io(s) => write!(f, "font io: {}", s),
            FontError::Parse(s) => write!(f, "font parse: {}", s),
            FontError::MissingPage(s) => write!(f, "font page not found: {}", s),
        }
    }
}
impl std::error::Error for FontError {}

/// Tokenise one `.fnt` line into key=value pairs, honouring quoted values that
/// may contain spaces (`file="Arial 12.dds"`, `face="Garamond"`).
fn parse_tokens(line: &str) -> (&str, Vec<(String, String)>) {
    let line = line.trim();
    let (tag, rest) = match line.split_once(char::is_whitespace) {
        Some((t, r)) => (t, r),
        None => (line, ""),
    };
    let mut pairs = Vec::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // skip whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // key
        let kstart = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = rest[kstart..i].to_string();
        if i >= bytes.len() || bytes[i] != b'=' {
            // a lone flag without '=' — skip
            continue;
        }
        i += 1; // skip '='
                // value (possibly quoted)
        let val;
        if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let vstart = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            val = rest[vstart..i].to_string();
            if i < bytes.len() {
                i += 1; // closing quote
            }
        } else {
            let vstart = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            val = rest[vstart..i].to_string();
        }
        if !key.is_empty() {
            pairs.push((key, val));
        }
    }
    (tag, pairs)
}

fn geti(pairs: &[(String, String)], key: &str) -> Option<i64> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.parse::<i64>().ok())
}
fn gets<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Parse a `.fnt` from its text bytes. Does NOT resolve page atlas files;
/// call [`BmFont::resolve_pages`] with the `.fnt`'s directory afterwards.
pub fn parse_fnt(text: &str) -> Result<BmFont, FontError> {
    let mut font = BmFont {
        face: String::new(),
        size: 0,
        line_height: 0,
        base: 0,
        scale_w: 0,
        scale_h: 0,
        page_files: HashMap::new(),
        page_paths: HashMap::new(),
        glyphs: HashMap::new(),
        kernings: HashMap::new(),
    };
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let (tag, p) = parse_tokens(raw);
        match tag {
            "info" => {
                if let Some(f) = gets(&p, "face") {
                    font.face = f.to_string();
                }
                if let Some(s) = geti(&p, "size") {
                    font.size = s as i32;
                }
            }
            "common" => {
                font.line_height = geti(&p, "lineHeight").unwrap_or(0) as i32;
                font.base = geti(&p, "base").unwrap_or(0) as i32;
                font.scale_w = geti(&p, "scaleW").unwrap_or(0) as u32;
                font.scale_h = geti(&p, "scaleH").unwrap_or(0) as u32;
            }
            "page" => {
                let id = geti(&p, "id").unwrap_or(0) as u16;
                if let Some(file) = gets(&p, "file") {
                    font.page_files.insert(id, file.to_string());
                }
            }
            "char" => {
                let id = geti(&p, "id").unwrap_or(0) as u32;
                let g = Glyph {
                    id,
                    page: geti(&p, "page").unwrap_or(0) as u16,
                    x: geti(&p, "x").unwrap_or(0) as u16,
                    y: geti(&p, "y").unwrap_or(0) as u16,
                    width: geti(&p, "width").unwrap_or(0) as u16,
                    height: geti(&p, "height").unwrap_or(0) as u16,
                    xoffset: geti(&p, "xoffset").unwrap_or(0) as i16,
                    yoffset: geti(&p, "yoffset").unwrap_or(0) as i16,
                    xadvance: geti(&p, "xadvance").unwrap_or(0) as i16,
                    chnl: geti(&p, "chnl").unwrap_or(15) as u8,
                };
                font.glyphs.insert(id, g);
            }
            "kerning" => {
                let first = geti(&p, "first").unwrap_or(0) as u32;
                let second = geti(&p, "second").unwrap_or(0) as u32;
                let amount = geti(&p, "amount").unwrap_or(0) as i16;
                font.kernings.insert((first, second), amount);
            }
            _ => {}
        }
    }
    if font.glyphs.is_empty() {
        return Err(FontError::Parse("no glyphs in .fnt".into()));
    }
    Ok(font)
}

impl BmFont {
    /// Resolve each page's atlas file relative to `fnt_dir`. the game `.fnt`s
    /// sometimes name the page `"Arial.dds"` while the real file is
    /// `Arial12.dds`; we fall back to `<fnt_stem>.dds` / `.tga`, and try the
    /// page filename's stem with alternate extensions.
    pub fn resolve_pages(&mut self, fnt_path: &Path) -> Result<(), FontError> {
        let dir = fnt_path.parent().unwrap_or_else(|| Path::new("."));
        let fnt_stem = fnt_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // the game `.fnt`s frequently omit the `page` line entirely; the atlas is
        // then `<fnt_stem>.dds`/`.tga` and the only page is id 0. Synthesize it
        // so glyphs referencing page 0 still resolve.
        if self.page_files.is_empty() {
            self.page_files.insert(0, format!("{fnt_stem}.dds"));
        }
        for (&id, file) in self.page_files.clone().iter() {
            let mut found: Option<PathBuf> = None;
            // 1) exactly as written
            let cand = dir.join(file);
            if cand.is_file() {
                found = Some(cand);
            }
            // 2) page stem with alternate extensions
            if found.is_none() {
                let stem = Path::new(file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file);
                for ext in ["dds", "tga", "png"] {
                    let c = dir.join(format!("{stem}.{ext}"));
                    if c.is_file() {
                        found = Some(c);
                        break;
                    }
                }
            }
            // 3) the .fnt stem with alternate extensions (the game's common case)
            if found.is_none() {
                for ext in ["dds", "tga", "png"] {
                    let c = dir.join(format!("{fnt_stem}.{ext}"));
                    if c.is_file() {
                        found = Some(c);
                        break;
                    }
                }
            }
            match found {
                Some(p) => {
                    self.page_paths.insert(id, p);
                }
                None => {
                    return Err(FontError::MissingPage(format!(
                        "page {id} ({file}) near {}",
                        fnt_path.display()
                    )))
                }
            }
        }
        Ok(())
    }

    /// Load a `.fnt` from disk and resolve its pages.
    pub fn load(fnt_path: &Path) -> Result<BmFont, FontError> {
        let text = std::fs::read_to_string(fnt_path)
            .map_err(|e| FontError::Io(format!("{}: {e}", fnt_path.display())))?;
        let mut f = parse_fnt(&text)?;
        f.resolve_pages(fnt_path)?;
        Ok(f)
    }

    pub fn kerning(&self, a: u32, b: u32) -> i16 {
        self.kernings.get(&(a, b)).copied().unwrap_or(0)
    }

    /// Width in pixels of the RESOLVED string (post markup-expansion). Uses
    /// xadvance + kerning between successive glyphs. Missing glyphs contribute
    /// nothing (matching the engine's silent drop). This is the value wrap /
    /// truncate / centring should use — not the raw markup length.
    pub fn measure(&self, s: &str) -> i32 {
        let mut width = 0i32;
        let mut prev: Option<u32> = None;
        for ch in s.chars() {
            let cp = ch as u32;
            if let Some(g) = self.glyphs.get(&cp) {
                if let Some(p) = prev {
                    width += self.kerning(p, cp) as i32;
                }
                width += g.xadvance as i32;
                prev = Some(cp);
            }
        }
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"info face="Test" size=14
common lineHeight=14 base=11 scaleW=256 scaleH=256 pages=1
page id=0 file="atlas.dds"
chars count=3
char id=65 x=0 y=0 width=8 height=10 xoffset=1 yoffset=2 xadvance=9 page=0 chnl=15
char id=86 x=8 y=0 width=8 height=10 xoffset=0 yoffset=2 xadvance=7 page=0 chnl=15
char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=3 page=0 chnl=15
kerning first=65 second=86 amount=-2
"#;

    #[test]
    fn parses_glyphs_and_kerning() {
        let f = parse_fnt(SAMPLE).unwrap();
        assert_eq!(f.line_height, 14);
        assert_eq!(f.base, 11);
        assert_eq!(f.glyphs.len(), 3);
        let a = f.glyphs[&65];
        assert_eq!((a.xoffset, a.yoffset, a.xadvance), (1, 2, 9));
        assert_eq!(f.kerning(65, 86), -2);
    }

    #[test]
    fn measure_uses_advance_plus_kerning_not_raw_len() {
        let f = parse_fnt(SAMPLE).unwrap();
        // "AV" => 9 + (-2 kern) + 7 = 14
        assert_eq!(f.measure("AV"), 14);
        // a space => xadvance 3
        assert_eq!(f.measure(" "), 3);
        // unknown codepoint contributes nothing (engine drops it)
        assert_eq!(f.measure("A\u{2603}V"), 14);
    }
}
