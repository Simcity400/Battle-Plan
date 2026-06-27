//! Font registry: maps a logical `.gui` font name (`hoi_18mbs`, `hoi_16mbs`) to
//! a loaded BMFont, via `interface/core.gfx`'s `bitmapfont { name=… fontfiles={…}
//! languages={…} }` blocks. We pick the **Latin/English** set: the block with NO
//! `languages` restriction (the default), ignoring `_cryllic`/japanese variants.
//!
//! The first listed `fontfiles` path (sans extension) names the `.fnt`/atlas
//! pair; the resolver finds the real file. Fonts load lazily and are cached.

use std::collections::HashMap;
use std::path::PathBuf;

use assets::resolver::GameResolver;
use assets::BmFont;

/// name -> the resolved `.fnt` path (Latin set).
pub struct FontMap {
    fnt_paths: HashMap<String, PathBuf>,
}

impl FontMap {
    /// Parse `interface/core.gfx` for `bitmapfont` blocks. Resolved through the
    /// game resolver so a mod override of core.gfx wins.
    pub fn from_core_gfx(resolver: &GameResolver) -> FontMap {
        let mut fnt_paths = HashMap::new();
        if let Some(core) = resolver.resolve("interface/core.gfx") {
            if let Ok(text) = std::fs::read_to_string(&core) {
                for (name, file) in parse_bitmapfonts(&text) {
                    // First (default/Latin) definition wins; skip language variants.
                    let entry = fnt_paths.entry(name).or_insert_with(|| PathBuf::new());
                    if entry.as_os_str().is_empty() {
                        // The fontfile is named without extension: resolve `.fnt`.
                        let rel = format!("{file}.fnt");
                        if let Some(p) = resolver.resolve(&rel) {
                            *entry = p;
                        }
                    }
                }
            }
        }
        FontMap { fnt_paths }
    }

    /// The resolved `.fnt` path for a logical font name, if known.
    pub fn fnt_path(&self, name: &str) -> Option<&PathBuf> {
        self.fnt_paths.get(name)
    }
}

/// Parse `bitmapfont { name=… fontfiles={ "a" "b" } languages={…} }` blocks.
/// Returns (name, first_fontfile_without_ext) for blocks WITHOUT a `languages`
/// restriction (the Latin/English default). Language-restricted blocks are
/// skipped so we never pick the Cyrillic/Japanese atlas for an English replay.
fn parse_bitmapfonts(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(rel) = text[i..].find("bitmapfont") {
        let start = i + rel;
        // find the opening brace after `bitmapfont`
        let brace = match text[start..].find('{') {
            Some(b) => start + b,
            None => break,
        };
        // find the matching close brace
        let mut depth = 0i32;
        let mut j = brace;
        let mut end = brace;
        while j < bytes.len() {
            match bytes[j] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = j;
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let block = &text[brace + 1..end];
        i = end + 1;

        // Skip language-restricted blocks.
        if block.contains("languages") {
            continue;
        }
        let name = match capture_value(block, "name") {
            Some(n) => n,
            None => continue,
        };
        let file = match first_fontfile(block) {
            Some(f) => f,
            None => continue,
        };
        out.push((name, file));
    }
    out
}

/// `name = "hoi_18mbs"` -> `hoi_18mbs`.
fn capture_value(block: &str, key: &str) -> Option<String> {
    let pos = block.find(key)?;
    let after = &block[pos + key.len()..];
    let eq = after.find('=')?;
    let rest = &after[eq + 1..];
    let q0 = rest.find('"')?;
    let q1 = rest[q0 + 1..].find('"')?;
    Some(rest[q0 + 1..q0 + 1 + q1].to_string())
}

/// First quoted path inside `fontfiles = { "a" "b" }`, extension stripped.
fn first_fontfile(block: &str) -> Option<String> {
    let pos = block.find("fontfiles")?;
    let after = &block[pos..];
    let open = after.find('{')?;
    let close = after[open..].find('}')?;
    let inner = &after[open + 1..open + close];
    let q0 = inner.find('"')?;
    let q1 = inner[q0 + 1..].find('"')?;
    let path = &inner[q0 + 1..q0 + 1 + q1];
    // strip a trailing extension if present
    let path = match path.rfind('.') {
        Some(d) if path[d..].len() <= 5 => &path[..d],
        _ => path,
    };
    Some(path.to_string())
}

/// A loaded font + its page atlas textures (uploaded to the GPU).
pub struct LoadedFont {
    pub font: BmFont,
    pub pages: Vec<render::Texture>,
}

/// Cache of logical-name -> loaded font (+ pages), loaded lazily on first use.
pub struct FontCache<'a> {
    map: &'a FontMap,
    resolver: &'a GameResolver,
    fallback_fnt: Option<PathBuf>,
    loaded: HashMap<String, LoadedFont>,
}

impl<'a> FontCache<'a> {
    pub fn new(map: &'a FontMap, resolver: &'a GameResolver) -> FontCache<'a> {
        // A guaranteed-present fallback for any font we cannot resolve.
        let fallback_fnt = resolver
            .resolve("gfx/fonts/hoi_18mbs.fnt")
            .or_else(|| resolver.resolve("gfx/fonts/garamond_14.fnt"));
        FontCache {
            map,
            resolver,
            fallback_fnt,
            loaded: HashMap::new(),
        }
    }

    /// Get (loading if needed) the font for a logical name. Falls back to a
    /// known-present font so text never silently disappears.
    pub fn get(&mut self, renderer: &render::Renderer, name: &str) -> Option<&LoadedFont> {
        if !self.loaded.contains_key(name) {
            let fnt = self
                .map
                .fnt_path(name)
                .cloned()
                .or_else(|| self.resolver.resolve(&format!("gfx/fonts/{name}.fnt")))
                .or_else(|| self.fallback_fnt.clone())?;
            let loaded = load_font(renderer, &fnt)?;
            self.loaded.insert(name.to_string(), loaded);
        }
        self.loaded.get(name)
    }
}

fn load_font(renderer: &render::Renderer, fnt_path: &std::path::Path) -> Option<LoadedFont> {
    let font = BmFont::load(fnt_path).ok()?;
    let max_page = font.page_paths.keys().copied().max().unwrap_or(0) as usize;
    let mut pages = Vec::with_capacity(max_page + 1);
    for id in 0..=max_page {
        let id = id as u16;
        let tex = font
            .page_paths
            .get(&id)
            .and_then(|p| renderer.load_dds(p).ok())
            .unwrap_or_else(|| renderer.upload_rgba8_image(1, 1, &[255, 255, 255, 255]));
        pages.push(tex);
    }
    if pages.is_empty() {
        pages.push(renderer.upload_rgba8_image(1, 1, &[255, 255, 255, 255]));
    }
    Some(LoadedFont { font, pages })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bitmapfont_block_skipping_language_variants() {
        let gfx = r#"
        bitmapfont = {
            name = "hoi_18mbs"
            fontfiles = {
                "gfx/fonts/hoi_18mbs"
                "gfx/fonts/hoi_18mbs_cryllic"
            }
            color = 0xffffffff
        }
        bitmapfont = {
            name = "hoi_18mbs"
            fontfiles = { "gfx/fonts/japanese/hoi_18mbs" }
            languages = { "l_japanese" }
        }
        "#;
        let v = parse_bitmapfonts(gfx);
        // only the non-language block is kept
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, "hoi_18mbs");
        assert_eq!(v[0].1, "gfx/fonts/hoi_18mbs");
    }
}
