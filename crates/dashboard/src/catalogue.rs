//! Content facts every product shares: the recording's localisation in the
//! user's game language and its interface sprites, read once per source and
//! folders, plus the texture decoding every image reply uses. Everything is
//! resolved through the recorded content stack; nothing names a vanilla asset.

use crate::{Source, content, invalid, products::ContentOptions};
use base64::Engine;
use installed_content::{
    ResolvedLayers,
    content::ResolvedContent,
    gfx::SpriteSet,
    image::Rgba8Image,
    localisation::{Argument, Localisation},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, OnceLock},
};

/// How the game's view of a name treats `[...]` scripted text, settled per
/// view by tracing it to CTextBase::ProcessString (1412B9CA0), the only
/// evaluator: a text box itself (`CInstantTextBox::text_setter` 1422B59E0)
/// stores the string and the text layout (1422894D0) draws `[` and `]` as
/// glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The name reaches its text box through the plain localiser (142230F20)
    /// only: brackets are glyphs. Party names (14118F860), character names
    /// (140FB5FD0, 14128CDE0), ideology names (140616BC0), technology names
    /// (140EC9B40 -> 140BBAA50), faction names (140D77360), trait names in
    /// trait lists (1416B51C0, 1416B40A0), idea slot names (140FBD700),
    /// character seat names (141D09EF0) and idea category names (1406708A0).
    Plain,
    /// The view runs 1412B9CA0 in a country's event scope. Idea names
    /// (140FBCF90 -> 1410CF560), idea descriptions (140FBAE40), focus names
    /// and descriptions (1402D2250, 1402D21E0), the idea tooltip's modifier
    /// lines with their postfixes and custom tooltips (140FBC770 -> 14054E520
    /// -> 1410CF560) and every name in the stability breakdown (1406EBBE0 ->
    /// 1410CF390). With the country ([`localised_in`]) the brackets are
    /// evaluated ([`crate::scripted_text`]); without it a closed `[...]` is
    /// refused. A `[` with no `]` after it makes 1412B9CA0 leave (1412B9E55)
    /// before it appends the text before it, so the text is dropped.
    Scripted,
}

/// [`localised_with`] with no arguments.
pub fn localised(localisation: &Localisation, key: &str, view: View) -> Result<String, String> {
    localised_with(localisation, key, &[], view)
}

/// `key` through the game's localiser (`installed_content::localisation`, 142230F20:
/// its substitution, depth and 0x1800-byte buffer; a key the content lacks is
/// copied as its own bytes), then read as `view` shows it ([`View`]) and made
/// plain text ([`plain_text`]). The error names the key and why.
pub fn localised_with(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    view: View,
) -> Result<String, String> {
    let text = localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|refusal| refusal.to_string())?;
    let text =
        scripted(&text, view).map_err(|reason| format!("localisation {key} holds {reason}"))?;
    plain_text(&text).map_err(|reason| format!("localisation {key} holds {reason}"))
}

/// The text as `view` shows its `[`s ([`View`]).
fn scripted(text: &str, view: View) -> Result<String, String> {
    let Some(open) = text.find('[') else {
        return Ok(text.to_owned());
    };
    let closed = text[open + 1..].find(']');
    match (view, closed) {
        (View::Plain, _) => Ok(text.to_owned()),
        (View::Scripted, None) => Ok(String::new()),
        (View::Scripted, Some(length)) => Err(format!(
            "the scripted expression {:?}, which the game evaluates for the country the text is shown for, and this product does not pass that country",
            &text[open..open + length + 2],
        )),
    }
}

/// [`localised_with`] in a Scripted view evaluated for a country: the
/// brackets go through [`crate::scripted_text::ScriptedText`], then the
/// text is made plain.
pub fn localised_in(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
    scripted: &crate::scripted_text::ScriptedText<'_>,
) -> Result<String, String> {
    let text = localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|refusal| refusal.to_string())?;
    let text = scripted
        .expand(&text)
        .map_err(|reason| format!("localisation {key} holds {reason}"))?;
    plain_text(&text).map_err(|reason| format!("localisation {key} holds {reason}"))
}

/// Stored game text (`installed_content::localisation`) as plain text, read as the
/// game's text layout reads it (`installed_content::localisation::pieces`): colour
/// codes are removed, inline images (a text icon or an inline flag) are left
/// out of the plain text, and the 0x14 and 0x1A spacing controls, which have
/// no plain-text form, are refused by name.
pub fn plain_text(text: &str) -> Result<String, String> {
    use installed_content::localisation::{Piece, pieces};
    let mut clean = String::new();
    for piece in pieces(text) {
        match piece {
            Piece::Text(text) => clean.push_str(text),
            Piece::Colour(_) | Piece::Icon(_) | Piece::Flag(_) => (),
            Piece::Control14 | Piece::Control1A(_) => {
                return Err(format!(
                    "{}, which plain text cannot show",
                    piece.describe()
                ));
            }
        }
    }
    Ok(clean)
}

/// Texts not shown, by reason, with what each names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refusals(BTreeMap<String, BTreeSet<String>>);

impl Refusals {
    /// Counts `identity` under `reason` once, however often it is met.
    pub fn note(&mut self, reason: impl Into<String>, identity: impl Into<String>) {
        self.0
            .entry(reason.into())
            .or_default()
            .insert(identity.into());
    }

    /// One warning per reason, naming every `what` it affects.
    pub fn lines(&self, what: &str) -> Vec<String> {
        self.0
            .iter()
            .map(|(reason, identities)| {
                format!(
                    "{} {what} not shown: {reason}; {}",
                    identities.len(),
                    identities.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            })
            .collect()
    }
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
    /// The recording client's game language (the replay identity's, without
    /// its `l_`): the language of text the game stored already localised.
    pub recorded_language: String,
    /// Every key whose text [`Catalogue::localised_with`] or
    /// [`Catalogue::localise_in`] refused so far, by reason: a fact of the
    /// content, reported by [`Catalogue::refusal_lines`].
    refused: Mutex<Refusals>,
    /// Scripted localisation names and the recursion limit, read on first
    /// use ([`Catalogue::scripted_content`]).
    scripted: OnceLock<Result<crate::scripted_text::ScriptedContent, String>>,
}

impl Catalogue {
    fn load(source: &Source, options: &ContentOptions) -> io::Result<Self> {
        let mut verification = Vec::new();
        let replay = source.replay();
        content::check_replay(&replay, options.allow_unverified, &mut verification)?;
        let language = &replay.identity.language;
        let recorded_language = language.strip_prefix("l_").unwrap_or(language).to_owned();
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
            recorded_language,
            refused: Mutex::default(),
            scripted: OnceLock::new(),
        })
    }

    /// [`localised_in`], noting a refusal: a key the content lacks is its own
    /// bytes, as the game's localiser copies it.
    pub fn localise_in(
        &self,
        key: &str,
        scripted: &crate::scripted_text::ScriptedText<'_>,
    ) -> Option<String> {
        localised_in(&self.localisation, key, &[], scripted)
            .inspect_err(|reason| self.refuse(reason, key))
            .ok()
    }

    /// [`localised_with`], noting a refusal: a key the content lacks is its
    /// own bytes, as the game's localiser copies it.
    pub fn localised_with(
        &self,
        key: &str,
        arguments: &[(&str, Argument<'_>)],
        view: View,
    ) -> Option<String> {
        localised_with(&self.localisation, key, arguments, view)
            .inspect_err(|reason| self.refuse(reason, key))
            .ok()
    }

    /// [`Catalogue::localised_with`] with no arguments.
    pub fn localised(&self, key: &str, view: View) -> Option<String> {
        self.localised_with(key, &[], view)
    }

    /// The content the scripted text evaluator needs
    /// ([`crate::scripted_text::ScriptedContent::read`]), read once. Its
    /// reading's warnings join [`Catalogue::refusal_lines`].
    pub fn scripted_content(&self) -> Result<&crate::scripted_text::ScriptedContent, String> {
        self.scripted
            .get_or_init(|| {
                crate::scripted_text::ScriptedContent::read(
                    Arc::clone(&self.content),
                    self.recorded_language.clone(),
                )
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Notes a text of `identity` not shown for `reason`.
    pub fn refuse(&self, reason: &str, identity: &str) {
        self.refused
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .note(reason, identity);
    }

    /// The texts refused so far, one warning per reason, and the warnings
    /// of the scripted localisation read once scripted text was evaluated.
    pub fn refusal_lines(&self) -> Vec<String> {
        let mut lines = self
            .refused
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lines("texts are");
        if let Some(Ok(scripted)) = self.scripted.get() {
            lines.extend(scripted.names.warnings.iter().cloned());
            if let Some(Ok(environment)) = scripted.environment.get() {
                lines.extend(environment.warnings.iter().cloned());
            }
        }
        lines
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
    texture_frame_png(layers, path, frames, 0, max_edge)
}

/// One authored frame (zero-based) of a horizontal sprite sheet, as
/// [`texture_png`] reads the first.
pub fn texture_frame_png(
    layers: &ResolvedLayers,
    path: &str,
    frames: u32,
    frame: u32,
    max_edge: u32,
) -> io::Result<String> {
    let image = read_texture(layers, path)?;
    if frames == 0 || image.width % frames != 0 || frame >= frames {
        return Err(invalid("Invalid sprite frames"));
    }
    let frame_width = image.width / frames;
    let left = frame * frame_width;
    let scale = (frame_width.max(image.height) as f64 / f64::from(max_edge)).max(1.0);
    let width = (frame_width as f64 / scale).round().max(1.0) as u32;
    let height = (image.height as f64 / scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let at = ((y * image.height / height * image.width + left + x * frame_width / width)
                * 4) as usize;
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
pub(crate) mod tests {
    use super::*;

    fn fixture() -> Localisation {
        loaded(
            &(String::from(concat!(
                "l_english:\n",
                " NAME:0 \"§YReal name§!\\nSecond line\"\n",
                " NESTED:0 \"Leader of $NAME$\"\n",
                " SCOPED:0 \"[ROOT.GetName]\"\n",
                " BROKEN:0 \"a$MISSING$b\"\n",
                " ICON:0 \"£pol_power 100§§ @GER£x\"\n",
                " GAP:0 \"a¤b\"\n",
                " BRACKET:0 \"A]B\"\n",
                " OPEN:0 \"A[B\"\n",
                " PRICE:0 \"Price $$5\"\n",
                " LONG:0 \"",
            )) + &"é".repeat(0xC00)
                + "x\"\n"),
        )
    }

    /// Localisation as the game content loads it: `yml` is one file of a
    /// folder layer, read by `installed_content::localisation::read`.
    pub(crate) fn loaded(yml: &str) -> Localisation {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4_dashboard_localisation_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("localisation/test_l_english.yml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("\u{feff}{yml}")).unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let localisation = installed_content::localisation::read(&layers, "english").unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        localisation
    }

    /// A content folder written for a test, removed when dropped.
    pub(crate) struct Folder(pub std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A catalogue over one content folder holding `files` (path, text), read
    /// by the same readers [`Catalogue::load`] uses, in English. The folder
    /// lives as long as the returned guard.
    pub(crate) fn folder_catalogue(files: &[(&str, &str)]) -> (Catalogue, Folder) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4_dashboard_catalogue_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let localisation = installed_content::localisation::read(&layers, "english").unwrap();
        let sprites = installed_content::gfx::read(&layers).unwrap();
        let catalogue = Catalogue {
            content: Arc::new(ResolvedContent {
                layers,
                stats: Default::default(),
            }),
            localisation: Arc::new(localisation),
            sprites,
            verification: Vec::new(),
            warnings: Vec::new(),
            recorded_language: "english".into(),
            refused: Mutex::default(),
            scripted: OnceLock::new(),
        };
        (catalogue, Folder(root))
    }

    /// Stored controls: colour codes and inline images leave the plain text,
    /// a doubled `§` is the character, and a spacing control is refused.
    #[test]
    fn plain_text_reads_the_stored_controls() {
        let loc = fixture();
        assert_eq!(localised(&loc, "ICON", View::Plain), Ok("100§ ".into()));
        assert_eq!(
            localised(&loc, "GAP", View::Plain),
            Err(
                "localisation GAP holds the control byte 0x14 (an authored `¤`), which plain text cannot show".into()
            )
        );
        assert_eq!(localised(&loc, "GAP", View::Plain).ok(), None);
    }

    #[test]
    fn localised_text_removes_colour_and_expands_references() {
        let loc = fixture();
        assert_eq!(
            localised(&loc, "NAME", View::Plain),
            Ok("Real name\nSecond line".into())
        );
        assert_eq!(
            localised(&loc, "NESTED", View::Plain),
            Ok("Leader of Real name\nSecond line".into())
        );
    }

    /// The shared localiser's rules, not a private engine's: `$$` is one `$`,
    /// a nested key the content lacks writes nothing (142230940), a `]` alone
    /// is a glyph, and the 0x1800-byte buffer (142230F20) cuts a long text.
    #[test]
    fn text_follows_the_games_localiser() {
        let loc = fixture();
        assert_eq!(localised(&loc, "PRICE", View::Plain), Ok("Price $5".into()));
        assert_eq!(localised(&loc, "BROKEN", View::Plain), Ok("ab".into()));
        assert_eq!(localised(&loc, "BRACKET", View::Plain), Ok("A]B".into()));
        // 0xC00 two-byte characters fill the buffer exactly; the `x` is cut.
        assert_eq!(localised(&loc, "LONG", View::Plain), Ok("é".repeat(0xC00)));
        // A key the content lacks is copied as its bytes.
        assert_eq!(
            localised(&loc, "Not a key", View::Plain).as_deref(),
            Ok("Not a key")
        );
        assert_eq!(
            localised(&loc, "ABSENT", View::Plain).as_deref(),
            Ok("ABSENT")
        );
    }

    /// 1412B9CA0 is the only evaluator of `[...]`: a plain view shows the
    /// brackets as glyphs; a scripted view needs a scope for a closed
    /// expression and drops the text at an unclosed `[` (1412B9E55).
    #[test]
    fn brackets_follow_the_view() {
        let loc = fixture();
        assert_eq!(
            localised(&loc, "SCOPED", View::Plain),
            Ok("[ROOT.GetName]".into())
        );
        assert_eq!(localised(&loc, "OPEN", View::Plain), Ok("A[B".into()));
        // Without the country it is shown for, a closed expression is refused.
        assert_eq!(
            localised(&loc, "SCOPED", View::Scripted),
            Err(
                "localisation SCOPED holds the scripted expression \"[ROOT.GetName]\", which the game evaluates for the country the text is shown for, and this product does not pass that country".into()
            )
        );
        assert_eq!(localised(&loc, "OPEN", View::Scripted), Ok(String::new()));
        assert_eq!(localised(&loc, "BRACKET", View::Scripted), Ok("A]B".into()));
    }

    #[test]
    fn a_refusal_is_counted_once_per_identity() {
        let mut refusals = Refusals::default();
        refusals.note("why", "B");
        refusals.note("why", "A");
        refusals.note("why", "A");
        assert_eq!(
            refusals.lines("names are"),
            ["2 names are not shown: why; A, B"]
        );
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
