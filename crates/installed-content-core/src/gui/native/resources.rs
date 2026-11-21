
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

pub const FALLBACK_SPRITE: &[u8] = b"GFX_default_fallback_texture";

pub const SHIELD_OVERLAY_EFFECT: &str = "gfx/FX/buttonstate.shader";

const FILE_LIMIT: usize = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lookup<'a> {
    Defined(&'a SpriteFacts),
    Fallback(&'a SpriteFacts),
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub image: dds::Texture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoTexture {
    Empty,
    NoExtension(Vec<u8>),
    Missing(Vec<String>),
    TooLarge { path: String, bytes: usize },
    Undecodable { path: String, reason: String },
    Undefined { path: String, reason: String },
    Device { path: String, reason: String },
    OverBudget { path: String, reason: String },
}

impl NoTexture {
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

#[derive(Clone, Debug, Default)]
pub struct Registry {
    pub files: Vec<(String, Sprites)>,
    sprites: BTreeMap<Vec<u8>, (usize, usize)>,
    pub fonts: Fonts,
    pub text_colors: TextColors,
    pub cursor_offset: [i32; 2],
    pub selection_offset: [i32; 2],
}

impl Registry {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.product("interface registry", 1, &[], |layers| {
            Ok(Self::from_files(sprites::read_layers(layers)?))
        }).map(|registry| registry.as_ref().clone())
    }

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

    pub fn sprite(&self, name: &[u8]) -> Option<&SpriteFacts> {
        self.sprites
            .get(crate::parser::c_string(name))
            .map(|(file, index)| &self.files[*file].1.sprites[*index])
    }

    pub fn create(&self, name: &[u8]) -> Lookup<'_> {
        match (self.sprite(name), self.sprite(FALLBACK_SPRITE)) {
            (Some(sprite), _) => Lookup::Defined(sprite),
            (None, Some(fallback)) => Lookup::Fallback(fallback),
            (None, None) => Lookup::Nothing,
        }
    }

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

fn vfs_path(candidate: &[u8]) -> Vec<u8> {
    candidate
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(&b'/')
}

pub fn texture<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<crate::content::Bytes>>(
    name: &[u8],
    read: &dyn Fn(&[u8]) -> io::Result<B>,
    decode: native::Decode,
) -> io::Result<Result<Texture, NoTexture>> {
    texture_within(name, read, decode, None)
}

fn texture_within<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<crate::content::Bytes>>(
    name: &[u8],
    read: &dyn Fn(&[u8]) -> io::Result<B>,
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
        let bytes: crate::content::Bytes = match read(&bytes_path) {
            Ok(bytes) => bytes.into(),
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
        let mut parameters = vec![u8::from(decode.force_pow2_textures),
            u8::from(name.len() > 3 && name[name.len() - 4..].eq_ignore_ascii_case(b".tga")),
            u8::from(max_texels.is_some())];
        parameters.extend_from_slice(&max_texels.unwrap_or_default().to_le_bytes());
        let decoded = crate::content::Generation::derive(&bytes, "native texture decoding", 1, &parameters, || {
            Ok(match max_texels {
                Some(max_texels) => native::decode_texture_image_within(name, &bytes, decode, max_texels),
                None => native::decode_texture_image(name, &bytes, decode),
            })
        })?;
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

pub fn stack_texture_or_recorded(
    layers: &ResolvedLayers,
    recorded: Option<&ResolvedLayers>,
    name: &[u8],
    decode: native::Decode,
) -> io::Result<Result<Texture, NoTexture>> {
    let shown = stack_texture(layers, name, decode);
    let why = match &shown {
        Ok(Ok(_)) => None,
        Ok(Err(why)) => Some(why.to_string()),
        Err(error) => Some(error.to_string()),
    };
    if let Some(why) = why
        && !crate::content::is_strict()
        && let Some(recorded) = recorded
        && let Ok(Ok(texture)) = stack_texture(recorded, name, decode)
    {
        layers.note_overlay_refusal(format!("texture {} ({why})", String::from_utf8_lossy(name)));
        return Ok(Ok(texture));
    }
    shown
}

pub fn stack_texture(
    layers: &ResolvedLayers,
    name: &[u8],
    decode: native::Decode,
) -> io::Result<Result<Texture, NoTexture>> {
    stack_texture_within(layers, name, decode, None)
}

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

pub fn frame_extent(plain: &Plain, texture: [u32; 2]) -> [i32; 2] {
    let grid = plain.grid();
    [0, 1].map(|axis| truncate_f32((1.0 / grid[axis] as f32) * texture[axis] as f32))
}

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

    #[test]
    fn lookups_distinguish_undefined_from_textureless() {
        engine_executable::executable_tables::initialize_for_tests()
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

    #[test]
    fn refusals_include_the_fonts() {
        engine_executable::executable_tables::initialize_for_tests()
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

    #[test]
    fn mod_textures_decode_in_the_native_order() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let png = crate::image::native::tests_support::png_rgba(2, 1, &rgba);
        let jpeg =
            include_bytes!("../../../../installed-content-image/src/native/testdata/baseline_420.jpg")
                .to_vec();
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

    #[test]
    fn frames_have_an_integer_extent_and_a_floating_geometry() {
        engine_executable::executable_tables::initialize_for_tests()
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

    #[test]
    fn bc7_force_pow2_refuses_a_zero_padded_mip_side_by_file_name() {
        use crate::gui::native::resources::{NoTexture, texture};
        use crate::image::native::{Decode, decode_texture_image, tests_support::dx10};

        for dxgi in [98, 99] {
            let mut file = dx10(10, 2, 4, dxgi);
            for (level, size) in [48, 32, 16, 16].into_iter().enumerate() {
                file.extend(std::iter::repeat_n(level as u8 + 1, size));
            }
            let name = b"gfx/thin-bc7.dds";
            let unpadded = decode_texture_image(name, &file, Decode::default()).unwrap();
            let sides: Vec<_> = unpadded.mips[0]
                .iter()
                .map(|m| (m.width, m.height))
                .collect();
            assert_eq!(sides, [(10, 2), (5, 1), (2, 1), (1, 1)]);
            let padded = texture(
                name,
                &|path| {
                    assert_eq!(path, name);
                    Ok(file.clone())
                },
                Decode {
                    force_pow2_textures: true,
                },
            )
            .unwrap();
            assert!(matches!(padded, Err(NoTexture::Undefined { path, reason })
                if path == "gfx/thin-bc7.dds"
                    && reason.contains("pads a 10x2 image to 16x2")
                    && reason.contains("level 2 is 4x0")
                    && reason.contains("power-of-two padding")
                    && reason.contains("texture creation")));
        }
    }
}
