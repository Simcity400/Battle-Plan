//! Content facts every product shares: the recording's localisation in the
//! user's game language and its interface sprites, read once per source and
//! folders, plus the texture decoding every image reply uses. Everything is
//! resolved through the recorded content stack; nothing names a vanilla asset.

use crate::{Source, content, invalid, products::ContentOptions};
use base64::Engine;
use installed_content::{
    ResolvedLayers, content::ResolvedContent, gfx::SpriteSet, image::Rgba8Image,
    localisation::Localisation,
};
use std::{io, sync::Arc};

/// What a localisation key resolves to. Text needing runtime scope (`[ROOT.GetName]`)
/// is kept apart from a missing key, so neither is shown as the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Localised {
    Text(String),
    NeedsScope,
    Missing,
}

impl Localised {
    pub fn text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::NeedsScope | Self::Missing => None,
        }
    }
}

/// Longest text accepted, before and after expanding `$key$` references.
const MAX_TEXT: usize = 8192;

/// The localised text for `key`.
pub fn localise(localisation: &Localisation, key: &str) -> Localised {
    match localisation.get(key) {
        Some(text) => resolve_text(text, localisation),
        None => Localised::Missing,
    }
}

/// Expands `$key$` references, removes `§` colour codes and turns `\n` into line
/// breaks. Text that still needs scripted scope or an unknown reference is not
/// passed off as a resolved name.
pub fn resolve_text(text: &str, localisation: &Localisation) -> Localised {
    let mut result = text.to_owned();
    for _ in 0..8 {
        let mut changed = false;
        let parts = result
            .split('$')
            .enumerate()
            .map(|(i, part)| {
                if i % 2 == 1 {
                    if let Some(value) = localisation.get(part) {
                        changed = true;
                        value.to_owned()
                    } else {
                        format!("${part}$")
                    }
                } else {
                    part.to_owned()
                }
            })
            .collect::<String>();
        result = parts;
        if !changed || result.len() > MAX_TEXT {
            break;
        }
    }
    if result.len() > MAX_TEXT {
        return Localised::Missing;
    }
    if result.contains(['[', ']']) {
        return Localised::NeedsScope;
    }
    if result.contains('$') {
        return Localised::Missing;
    }
    Localised::Text(plain_text(&result).replace("\\n", "\n"))
}

/// Game text without its `§X` colour codes (each `§` and the letter after it).
pub fn plain_text(text: &str) -> String {
    let mut chars = text.chars();
    let mut clean = String::new();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            clean.push(c);
        }
    }
    clean
}

/// One source's shared content facts for one set of folders.
pub struct Catalogue {
    pub content: Arc<ResolvedContent>,
    /// Shared with country naming (`content::localisation`): parsed once per source.
    pub localisation: Arc<Localisation>,
    pub sprites: SpriteSet,
    /// Notes from verifying the recording, such as unverified content allowed by the user.
    pub verification: Vec<String>,
    /// Standing notes about the recorded content, such as unparsed lines.
    pub warnings: Vec<String>,
}

impl Catalogue {
    fn load(source: &Source, options: &ContentOptions) -> io::Result<Self> {
        let mut verification = Vec::new();
        content::check_replay(
            &source.replay(),
            options.allow_unverified,
            &mut verification,
        )?;
        let mut warnings = Vec::new();
        let content = source.content(&options.options)?;
        let localisation = content::localisation(source, options)?;
        let sprites = installed_content::gfx::read(&content.layers)?;
        for (label, count) in [
            ("unparsed localisation lines", localisation.unparsed.len()),
            ("duplicate localisation keys", localisation.duplicates.len()),
            ("recovered sprite files", sprites.recovered.len()),
        ] {
            if count > 0 {
                warnings.push(format!("Recorded content contains {count} {label}."));
            }
        }
        Ok(Self {
            content,
            localisation,
            sprites,
            verification,
            warnings,
        })
    }

    pub fn localise(&self, key: &str) -> Localised {
        localise(&self.localisation, key)
    }
}

/// The shared catalogue for these folders, loaded on first use and replaced when
/// the folders, the cache folder, the game language or the unverified-content
/// choice change.
pub fn catalogue(source: &Source, options: &ContentOptions) -> io::Result<Arc<Catalogue>> {
    type Cached = Option<(content::LocalisationKey, Arc<Catalogue>)>;
    let key = content::localisation_key(options)?;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, catalogue)) = state.as_ref()
        && *cached == key
    {
        return Ok(Arc::clone(catalogue));
    }
    let catalogue = Arc::new(Catalogue::load(source, options)?);
    *state = Some((key, Arc::clone(&catalogue)));
    Ok(catalogue)
}

/// Largest texture file or edge read for an image reply.
const MAX_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXTURE_EDGE: u32 = 2048;

/// Decodes a DDS or TGA texture from the content layers within the default
/// budget. Textures sit outside content hash coverage.
pub fn read_texture(layers: &ResolvedLayers, path: &str) -> io::Result<Rgba8Image> {
    read_texture_within(layers, path, MAX_TEXTURE_BYTES, MAX_TEXTURE_EDGE)
}

/// Decodes a DDS or TGA texture, refusing a file over `max_bytes` or an image
/// with a side over `max_edge` before decoding it.
pub fn read_texture_within(
    layers: &ResolvedLayers,
    path: &str,
    max_bytes: usize,
    max_edge: u32,
) -> io::Result<Rgba8Image> {
    let bytes = layers.read_uncovered(path)?;
    if bytes.len() > max_bytes {
        return Err(invalid("Artwork exceeds input budget"));
    }
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".dds") {
        // Check dimensions before decoding the container's authored mip levels.
        if bytes.len() < 128
            || u32::from_le_bytes(bytes[12..16].try_into().unwrap()) > max_edge
            || u32::from_le_bytes(bytes[16..20].try_into().unwrap()) > max_edge
        {
            return Err(invalid("Artwork dimensions exceed budget"));
        }
        installed_content::image::rgba::dds_base(&installed_content::image::dds::decode(&bytes)?)
    } else if lower.ends_with(".tga") {
        if bytes.len() < 18
            || u16::from_le_bytes(bytes[12..14].try_into().unwrap()) as u32 > max_edge
            || u16::from_le_bytes(bytes[14..16].try_into().unwrap()) as u32 > max_edge
        {
            return Err(invalid("Artwork dimensions exceed budget"));
        }
        installed_content::image::tga::decode(&bytes)
    } else {
        Err(invalid("Unsupported artwork format"))
    }
}

/// A texture's first authored frame (the normal state; a sprite sheet is never
/// stretched), point-sampled down to `max_edge`, as a PNG data URL.
pub fn texture_png(
    layers: &ResolvedLayers,
    path: &str,
    frames: u32,
    max_edge: u32,
) -> io::Result<String> {
    let image = read_texture(layers, path)?;
    if frames == 0 || image.width % frames != 0 {
        return Err(invalid("Invalid sprite frames"));
    }
    let frame_width = image.width / frames;
    let scale = (frame_width.max(image.height) as f64 / f64::from(max_edge)).max(1.0);
    let width = (frame_width as f64 / scale).round().max(1.0) as u32;
    let height = (image.height as f64 / scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let at =
                ((y * image.height / height * image.width + x * frame_width / width) * 4) as usize;
            pixels.extend_from_slice(&image.pixels[at..at + 4]);
        }
    }
    png_data_url(&Rgba8Image {
        width,
        height,
        pixels,
    })
}

/// Straight-alpha RGBA as a PNG data URL, the form every image reply uses.
pub fn png_data_url(image: &Rgba8Image) -> io::Result<String> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.pixels)?;
    }
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encoded)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Localisation {
        [
            ("NAME".to_owned(), "§YReal name§!\\nSecond line".to_owned()),
            ("NESTED".to_owned(), "Leader of $NAME$".to_owned()),
            ("SCOPED".to_owned(), "[ROOT.GetName]".to_owned()),
            ("BROKEN".to_owned(), "$MISSING$".to_owned()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn localised_text_removes_colour_and_expands_references() {
        let loc = fixture();
        assert_eq!(
            localise(&loc, "NAME"),
            Localised::Text("Real name\nSecond line".into())
        );
        assert_eq!(
            localise(&loc, "NESTED"),
            Localised::Text("Leader of Real name\nSecond line".into())
        );
    }

    #[test]
    fn scripted_scope_and_missing_keys_stay_apart() {
        let loc = fixture();
        assert_eq!(localise(&loc, "SCOPED"), Localised::NeedsScope);
        assert_eq!(localise(&loc, "BROKEN"), Localised::Missing);
        assert_eq!(localise(&loc, "ABSENT"), Localised::Missing);
        assert_eq!(
            resolve_text(&"x".repeat(MAX_TEXT + 1), &loc),
            Localised::Missing
        );
        assert_eq!(Localised::NeedsScope.text(), None);
    }

    #[test]
    fn png_data_urls_round_trip_their_size() {
        let url = png_data_url(&Rgba8Image {
            width: 3,
            height: 2,
            pixels: vec![255; 24],
        })
        .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (3, 2));
    }
}
