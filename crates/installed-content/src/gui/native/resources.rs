//! The one native resource path GUI consumers draw through: the content stack's sprite
//! registry, fonts and colour tables in the game's load order, the texture a sprite's name loads
//! (or the reason none draws), its frames' queried integer extent and floating geometry, and the
//! effects sprites name.

use std::{collections::BTreeMap, io};

use super::{
    fonts::{self, FontEvent, Fonts, TextColors},
    sprites::{self, Plain, SpriteFacts, Sprites},
};
use crate::{
    ResolvedLayers,
    crt::truncate_f32,
    image::{dds, native},
};

/// The fallback name retried when the requested sprite is undefined.
pub const FALLBACK_SPRITE: &[u8] = b"GFX_default_fallback_texture";

/// The texture loader's file-size limit; larger files do not load.
const FILE_LIMIT: usize = 16 << 20;

/// The sprite instance selected from a name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lookup<'a> {
    /// The latest declaration of the name.
    Defined(&'a SpriteFacts),
    /// The name is undefined; the fallback sprite's declaration.
    Fallback(&'a SpriteFacts),
    /// Neither requested nor fallback sprite is defined: no drawable.
    Nothing,
}

/// A texture as the game loads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    /// The path read (a layer-relative path), as text for messages: the read itself keeps its
    /// bytes.
    pub path: String,
    /// The texture's own extent, which every size and UV divides.
    pub width: u32,
    pub height: u32,
    pub image: dds::Texture,
}

/// Why a defined sprite has no drawable texture. Every variant but [`NoTexture::Undefined`] and
/// [`NoTexture::Device`] is the game's own outcome: sprite draw then returns
/// without drawing, and every size it reports divides 0-by-0 extents. Paths are text for
/// messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoTexture {
    /// No texture name (an empty `textureFile` is never registered).
    Empty,
    /// A name without a `.`: texture extension resolver returns null.
    NoExtension(Vec<u8>),
    /// None of the resolver's candidates exists in any layer.
    Missing(Vec<String>),
    /// The file exceeds the texture loader's 16 MiB limit.
    TooLarge { path: String, bytes: usize },
    /// Every texture image decoder refuses the file ([`native::Failure::Undecodable`]), with
    /// each step's reason.
    Undecodable { path: String, reason: String },
    /// What the game draws from the file is not determined by it
    /// ([`native::Failure::Undefined`]: uninitialised or out-of-bounds memory): a consumer must
    /// refuse what it would draw with it, not draw it as missing.
    Undefined { path: String, reason: String },
    /// What the game draws from the file depends on the graphics device
    /// ([`native::Failure::Device`]), which the recording does not hold: refused by name.
    Device { path: String, reason: String },
    /// The caller's decoded-texel budget refused the file's extent before decoding
    /// ([`stack_texture_within`], [`native::Failure::OverBudget`]); the game would load it.
    OverBudget { path: String, reason: String },
}

impl NoTexture {
    /// Whether the game itself has no texture here (so drawing nothing is exact), as opposed to
    /// an outcome the file or the recording does not determine.
    pub fn native_failure(&self) -> bool {
        !matches!(
            self,
            Self::Undefined { .. } | Self::Device { .. } | Self::OverBudget { .. }
        )
    }
}

impl std::fmt::Display for NoTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("names no texture"),
            Self::NoExtension(name) => write!(
                f,
                "names texture {:?}, which has no extension, so the texture extension resolver \
                 resolves nothing",
                String::from_utf8_lossy(name)
            ),
            Self::Missing(tried) => write!(f, "names a texture no layer holds (tried {tried:?})"),
            Self::TooLarge { path, bytes } => write!(
                f,
                "texture {path} is {bytes} bytes, past texture file loader's 16 MiB, so it does \
                 not load"
            ),
            Self::Undecodable { path, reason } => {
                write!(f, "texture {path} does not load in the game: {reason}")
            }
            Self::Undefined { path, reason } => {
                write!(
                    f,
                    "texture {path} does not determine what the game draws: {reason}"
                )
            }
            Self::Device { path, reason } => write!(
                f,
                "texture {path} draws as the recording client's graphics device decides, which \
                 is not recorded: {reason}"
            ),
            Self::OverBudget { path, reason } => {
                write!(f, "texture {path} is not decoded here: {reason}")
            }
        }
    }
}

/// The content stack's GUI resources, read once in the game's load order.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    /// Every `.gfx` file read, in load order.
    pub files: Vec<(String, Sprites)>,
    /// Each name's latest declaration: (file, index).
    sprites: BTreeMap<Vec<u8>, (usize, usize)>,
    /// The registered fonts, overrides applied as read.
    pub fonts: Fonts,
    /// The global `textcolors` table: the last block read; empty (every code
    /// looks free) when no file declares one.
    pub text_colors: TextColors,
    /// The global `cursor_offset` and `selection_offset`, the last of each.
    pub cursor_offset: [i32; 2],
    pub selection_offset: [i32; 2],
}

impl Registry {
    /// Reads every `interface/**/*.gfx` of the stack in the game's order
    /// ([`sprites::gfx_order`]).
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        Ok(Self::from_files(sprites::read_layers(layers)?))
    }

    /// The registry over files already read, in load order.
    pub fn from_files(files: Vec<(String, Sprites)>) -> Self {
        let mut sprites = BTreeMap::new();
        let mut text_colors = TextColors::new();
        let mut cursor_offset = [0; 2];
        let mut selection_offset = [0; 2];
        for (file, (_, read)) in files.iter().enumerate() {
            for (index, sprite) in read.sprites.iter().enumerate() {
                sprites.insert(sprite.name.clone(), (file, index));
            }
            if let Some(table) = &read.text_colors {
                text_colors.clone_from(table);
            }
            if let Some(offset) = read.cursor_offset {
                cursor_offset = offset;
            }
            if let Some(offset) = read.selection_offset {
                selection_offset = offset;
            }
        }
        let fonts = Fonts::new(
            files
                .iter()
                .flat_map(|(_, read)| read.fonts.iter())
                .collect::<Vec<&FontEvent>>(),
        );
        Self {
            files,
            sprites,
            fonts,
            text_colors,
            cursor_offset,
            selection_offset,
        }
    }

    /// The latest sprite declaration of `name`, compared byte for byte.
    pub fn sprite(&self, name: &[u8]) -> Option<&SpriteFacts> {
        self.sprites
            .get(crate::parser::c_string(name))
            .map(|(file, index)| &self.files[*file].1.sprites[*index])
    }

    /// The requested name, else the fallback sprite, else nothing.
    pub fn create(&self, name: &[u8]) -> Lookup<'_> {
        match (self.sprite(name), self.sprite(FALLBACK_SPRITE)) {
            (Some(sprite), _) => Lookup::Defined(sprite),
            (None, Some(fallback)) => Lookup::Fallback(fallback),
            (None, None) => Lookup::Nothing,
        }
    }

    /// Every refusal of every file and declaration: the sprite files', their sprites' and every
    /// registered font's.
    pub fn refusals(&self) -> Vec<&super::Refusal> {
        let mut refusals = sprites::refusals(&self.files);
        refusals.extend(
            self.fonts
                .fonts()
                .iter()
                .flat_map(|font| &font.facts.refusals),
        );
        refusals
    }
}

/// The candidate byte path opened by the virtual file system, with separators collapsed.
fn vfs_path(candidate: &[u8]) -> Vec<u8> {
    candidate
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(&b'/')
}

/// Load a sprite texture by trying extension candidates and taking the first existing file. The
/// texture loader passes the authored name and client's `decode` state to
/// [`native::decode_texture_image`]. Read each candidate's bytes through `read`; return
/// `Err(io)` only for failures other than a missing file.
pub fn texture(
    name: &[u8],
    read: &dyn Fn(&[u8]) -> io::Result<Vec<u8>>,
    decode: native::Decode,
) -> io::Result<Result<Texture, NoTexture>> {
    texture_within(name, read, decode, None)
}

/// [`texture`], refusing an image past `max_texels` from its header
/// ([`native::decode_texture_image_within`]).
fn texture_within(
    name: &[u8],
    read: &dyn Fn(&[u8]) -> io::Result<Vec<u8>>,
    decode: native::Decode,
    max_texels: Option<u64>,
) -> io::Result<Result<Texture, NoTexture>> {
    let name = crate::parser::c_string(name);
    if name.is_empty() {
        return Ok(Err(NoTexture::Empty));
    }
    let candidates = fonts::texture_candidates(name);
    if candidates.is_empty() {
        return Ok(Err(NoTexture::NoExtension(name.to_vec())));
    }
    let mut tried = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let bytes_path = vfs_path(candidate);
        let path = String::from_utf8_lossy(&bytes_path).into_owned();
        tried.push(String::from_utf8_lossy(candidate).into_owned());
        let bytes = match read(&bytes_path) {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if bytes.len() > FILE_LIMIT {
            return Ok(Err(NoTexture::TooLarge {
                path,
                bytes: bytes.len(),
            }));
        }
        let decoded = match max_texels {
            Some(max_texels) => {
                native::decode_texture_image_within(name, &bytes, decode, max_texels)
            }
            None => native::decode_texture_image(name, &bytes, decode),
        };
        return Ok(match decoded {
            Ok(image) => Ok(Texture {
                path,
                width: image.width,
                height: image.height,
                image,
            }),
            Err(native::Failure::Undecodable(reason)) => {
                Err(NoTexture::Undecodable { path, reason })
            }
            Err(native::Failure::Undefined(reason)) => Err(NoTexture::Undefined { path, reason }),
            Err(native::Failure::Device(reason)) => Err(NoTexture::Device { path, reason }),
            Err(native::Failure::OverBudget(reason)) => Err(NoTexture::OverBudget { path, reason }),
        });
    }
    Ok(Err(NoTexture::Missing(tried)))
}

/// [`texture`] for a GUI sprite or font, through the content stack by each candidate's bytes
/// (uncovered textures unverified, as `ResolvedLayers::read_uncovered` states). A cube map is
/// refused: texture creation makes it a cube texture, which a GUI draw samples through
/// a two-dimensional binding.
pub fn stack_texture(
    layers: &ResolvedLayers,
    name: &[u8],
    decode: native::Decode,
) -> io::Result<Result<Texture, NoTexture>> {
    stack_texture_within(layers, name, decode, None)
}

/// [`stack_texture`] for a caller that must bound memory: an image whose decoded texels would
/// exceed `max_texels` is refused from its header, before it is decoded
/// ([`NoTexture::OverBudget`]).
pub fn stack_texture_within(
    layers: &ResolvedLayers,
    name: &[u8],
    decode: native::Decode,
    max_texels: Option<u64>,
) -> io::Result<Result<Texture, NoTexture>> {
    let read = |path: &[u8]| layers.read_uncovered_path(path);
    Ok(
        texture_within(name, &read, decode, max_texels)?.and_then(|texture| {
            if texture.image.faces == 1 {
                return Ok(texture);
            }
            Err(NoTexture::Undefined {
                path: texture.path,
                reason:
                    "a cube-map DDS: texture creation makes it a cube texture, which a GUI draw \
                 samples through a two-dimensional binding"
                        .to_owned(),
            })
        }),
    )
}

/// A frame's queried integer extent: `cvttss2si((1.0f / columns) * texture width)`, and the
/// same vertically. A texture that does not load gives 0 by 0; zero frames give `i32::MIN`. The
/// sprite size query then scales this extent.
pub fn frame_extent(plain: &Plain, texture: [u32; 2]) -> [i32; 2] {
    let grid = plain.grid();
    [0, 1].map(|axis| truncate_f32((1.0 / grid[axis] as f32) * texture[axis] as f32))
}

/// The floating frame extent before world scaling: `(1.0f / columns) * width`, and the same
/// vertically times the V scale; no truncation.
pub fn frame_geometry(plain: &Plain, texture: [u32; 2]) -> [f32; 2] {
    let grid = plain.grid();
    [
        (1.0 / grid[0] as f32) * texture[0] as f32,
        (1.0 / grid[1] as f32) * texture[1] as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::native::sprites::{SpriteKind, read_sprites};

    fn registry(files: &[(&str, &[u8])]) -> Registry {
        Registry::from_files(
            files
                .iter()
                .map(|(path, bytes)| ((*path).to_owned(), read_sprites(bytes)))
                .collect(),
        )
    }

    /// The latest declaration wins across files; an undefined name takes the fallback sprite,
    /// and a defined sprite without a texture is not the fallback.
    #[test]
    fn lookups_distinguish_undefined_from_textureless() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let registry = registry(&[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_a\" noOfFrames = 2 } \
                  spriteType = { name = \"GFX_default_fallback_texture\" textureFile = \"x.dds\" } }",
            ),
            (
                "interface/b.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_a\" noOfFrames = 0x3 } }",
            ),
        ]);
        let Lookup::Defined(a) = registry.create(b"GFX_a") else {
            panic!();
        };
        assert_eq!(a.kind.plain().unwrap().frames, 3);
        assert!(a.kind.plain().unwrap().texture.is_empty());
        assert!(matches!(registry.create(b"GFX_b"), Lookup::Fallback(_)));
        let missing = |_: &[u8]| Err(io::Error::from(io::ErrorKind::NotFound));
        let texture = |name: &[u8], read: &dyn Fn(&[u8]) -> io::Result<Vec<u8>>| {
            texture(name, read, native::Decode::default())
        };
        assert_eq!(texture(b"", &missing).unwrap(), Err(NoTexture::Empty));
        assert_eq!(
            texture(b"gfx//interface//x", &missing).unwrap(),
            Err(NoTexture::NoExtension(b"gfx//interface//x".to_vec()))
        );
        assert!(matches!(
            texture(b"gfx\\x.png", &missing).unwrap(),
            Err(NoTexture::Missing(tried)) if tried == ["gfx/x.dds", "gfx/x.tga", "gfx/x.png"]
        ));
        // A name whose bytes are not UTF-8 is read by exactly those bytes, never through a lossy
        // spelling.
        let png = crate::image::native::tests_support::png_rgba(1, 1, &[9, 8, 7, 255]);
        let read = |path: &[u8]| match path {
            b"gfx/\x80.dds" => Ok(png.clone()),
            _ => Err(io::Error::from(io::ErrorKind::NotFound)),
        };
        let loaded = texture(b"gfx/\x80.png", &read).unwrap().unwrap();
        assert_eq!(loaded.image.base().data, [9, 8, 7, 255]);
        assert!(matches!(
            texture("gfx/\u{FFFD}.png".as_bytes(), &read).unwrap(),
            Err(NoTexture::Missing(_))
        ));
    }

    /// Registry refusals include every registered font as well as sprites: a text color code
    /// causes the text color reader to index past its table.
    #[test]
    fn refusals_include_the_fonts() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let registry = registry(&[(
            "interface/fonts.gfx",
            b"bitmapfonts = { bitmapfont = { name = \"f\" path = \"gfx/fonts/f\" \
              textcolors = { \xC0 = { 1 2 3 } } } }",
        )]);
        let refusals = registry.refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(refusals[0].reason.contains("text color reader"));
    }

    /// A mod sprite's texture resolves through the stack and decodes in the game's order: a
    /// `.png` it names, a `.tga` name whose file holds PNG bytes, a `.dds` name found as a PNG
    /// under the name itself, a JPEG under a `.dds` name. A three-byte file makes FreeImage's
    /// ICO validate compare uninitialised stack, an outcome the file does not determine; bytes
    /// every decoder refuses, an absent file and an oversized file are the game's own failures.
    #[test]
    fn mod_textures_decode_in_the_native_order() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let png = crate::image::native::tests_support::png_rgba(2, 1, &rgba);
        let jpeg = include_bytes!("../../image/native/testdata/baseline_420.jpg").to_vec();
        let undefined = vec![0, 0, 1];
        let garbage = vec![
            0x13, 7, 0x55, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ];
        let files: BTreeMap<&[u8], Vec<u8>> = [
            (&b"gfx/mod/icon.png"[..], png.clone()),
            (b"gfx/mod/renamed.tga", png.clone()),
            (b"gfx/mod/photo.dds", jpeg),
            (b"gfx/mod/tiny.dds", undefined),
            (b"gfx/mod/noise.dds", garbage),
            (b"gfx/mod/huge.dds", vec![0; FILE_LIMIT + 1]),
        ]
        .into_iter()
        .collect();
        let texture = |name: &[u8], read: &dyn Fn(&[u8]) -> io::Result<Vec<u8>>| {
            texture(name, read, native::Decode::default())
        };
        let read = |path: &[u8]| {
            files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        };
        let registry = registry(&[(
            "interface/mod.gfx",
            b"spriteTypes = { spriteType = { name = \"GFX_mod\" textureFile = \"gfx/mod/icon.png\" } }",
        )]);
        let Lookup::Defined(sprite) = registry.create(b"GFX_mod") else {
            panic!();
        };
        let loaded = texture(&sprite.kind.plain().unwrap().texture, &read)
            .unwrap()
            .unwrap();
        assert_eq!(
            (loaded.path.as_str(), loaded.width, loaded.height),
            ("gfx/mod/icon.png", 2, 1)
        );
        assert_eq!(loaded.image.base().data, rgba);
        let renamed = texture(b"gfx/mod/renamed.tga", &read).unwrap().unwrap();
        assert_eq!(renamed.image.base().data, rgba);
        let photo = texture(b"gfx/mod/photo.png", &read).unwrap().unwrap();
        assert_eq!(
            (photo.path.as_str(), photo.width, photo.height),
            ("gfx/mod/photo.dds", 13, 11)
        );
        let tiny = texture(b"gfx/mod/tiny.png", &read).unwrap().unwrap_err();
        assert!(matches!(&tiny, NoTexture::Undefined { path, .. } if path == "gfx/mod/tiny.dds"));
        assert!(!tiny.native_failure());
        for (name, native) in [
            (&b"gfx/mod/noise.dds"[..], "Undecodable"),
            (b"gfx/mod/absent.png", "Missing"),
            (b"gfx/mod/huge.dds", "TooLarge"),
        ] {
            let why = texture(name, &read).unwrap().unwrap_err();
            assert!(why.native_failure(), "{why}");
            assert!(format!("{why:?}").starts_with(native), "{why:?}");
        }
    }

    /// The queried extent truncates the reciprocal product; the geometry keeps its fraction;
    /// a vertical strip divides the height.
    #[test]
    fn frames_have_an_integer_extent_and_a_floating_geometry() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let read = read_sprites(
            b"spriteTypes = { spriteType = { name = \"h\" noOfFrames = 3 } \
              spriteType = { name = \"v\" noOfFrames = 3 framedirectiony = yes } \
              spriteType = { name = \"z\" noOfFrames = 0 } }",
        );
        let plain = |index: usize| match &read.sprites[index].kind {
            SpriteKind::Plain(plain) => plain.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(frame_extent(&plain(0), [10, 8]), [3, 8]);
        assert_eq!(
            frame_geometry(&plain(0), [10, 8]),
            [(1.0f32 / 3.0) * 10.0, 8.0]
        );
        assert_eq!(frame_extent(&plain(1), [10, 9]), [10, 3]);
        assert_eq!(frame_extent(&plain(2), [10, 8]), [i32::MIN, 8]);
        assert_eq!(frame_extent(&plain(0), [0, 0]), [0, 0]);
    }
}
