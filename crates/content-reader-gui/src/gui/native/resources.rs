use std::{collections::BTreeMap, io};

use super::{
    fonts::{FontEvent, Fonts, TextColors},
    sprites::{self, Plain, SpriteFacts, Sprites},
};
use content_reader_image::native;
use content_reader_layers::{ResolvedLayers, content::ArtFileSource};
use content_reader_layers::textures::texture_within;
pub use content_reader_layers::textures::{NoTexture, Texture, texture};
use content_reader_parser::crt::truncate_f32;

pub const FALLBACK_SPRITE: &[u8] = b"GFX_default_fallback_texture";

pub const SHIELD_OVERLAY_EFFECT: &str = "gfx/FX/buttonstate.shader";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lookup<'a> {
    Defined(&'a SpriteFacts),
    Fallback(&'a SpriteFacts),
    Nothing,
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
        layers
            .product("interface registry", 1, &[], |layers| {
                Ok(Self::from_files(sprites::read_layers(layers)?))
            })
            .map(|registry| registry.as_ref().clone())
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

    pub fn sprite_file(&self, name: &[u8]) -> Option<&str> {
        self.sprites
            .get(content_reader_parser::parser::c_string(name))
            .map(|(file, _)| self.files[*file].0.as_str())
    }

    pub fn sprite(&self, name: &[u8]) -> Option<&SpriteFacts> {
        self.sprites
            .get(content_reader_parser::parser::c_string(name))
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
        && !content_reader_layers::content::is_strict()
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
    let unmounted = std::cell::RefCell::new(None);
    let read = |path: &[u8]| {
        if unmounted.borrow().is_some() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        match layers.art_file_source(path)? {
            ArtFileSource::Mounted { path, .. } => layers.read_uncovered_path(&path),
            ArtFileSource::Unmounted { layer, reason } => {
                let layer = layers.layers[layer]
                    .display_name()
                    .map_or_else(|| layers.layers[layer].id.clone(), str::to_owned);
                *unmounted.borrow_mut() = Some(NoTexture::Undefined {
                    path: String::from_utf8_lossy(path).into_owned(),
                    reason: format!("the art of layer '{layer}' is not mounted ({reason})"),
                });
                Err(io::Error::from(io::ErrorKind::NotFound))
            }
            ArtFileSource::Missing => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    };
    let found = texture_within(name, &read, decode, max_texels)?;
    if let Some(refusal) = unmounted.into_inner() {
        return Ok(Err(refusal));
    }
    Ok(
        found.and_then(|texture| {
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
        content_reader_script::test_game::enter();
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
        let png = content_reader_image::native::tests_support::png_rgba(1, 1, &[9, 8, 7, 255]);
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
    fn a_texture_under_an_unmounted_mod_layer_is_refused_not_taken_from_below() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "battleplan-unmounted-art-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let base = root.join("base");
        let addon = root.join("mod");
        std::fs::create_dir_all(base.join("gfx")).unwrap();
        std::fs::create_dir_all(&addon).unwrap();
        let png = content_reader_image::native::tests_support::png_rgba(1, 1, &[9, 8, 7, 255]);
        std::fs::write(base.join("gfx/plate.png"), &png).unwrap();
        let mounted = ResolvedLayers::from_folders(&[(&base, &[][..]), (&addon, &[][..])]).unwrap();
        let decode = native::Decode::default();
        assert!(stack_texture(&mounted, b"gfx/plate.png", decode).unwrap().is_ok());
        let mut layers = mounted.layers;
        layers[1].art_unmounted = Some("its art folder is missing".to_owned());
        let unmounted = ResolvedLayers::from_test_layers(layers).unwrap();
        let refused = stack_texture(&unmounted, b"gfx/plate.png", decode)
            .unwrap()
            .unwrap_err();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            matches!(&refused, NoTexture::Undefined { reason, .. } if reason.contains("its art folder is missing")),
            "{refused:?}"
        );
    }

    #[test]
    fn refusals_include_the_fonts() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let png = content_reader_image::native::tests_support::png_rgba(2, 1, &rgba);
        let jpeg =
            include_bytes!("../../../../content-reader-image/src/native/testdata/baseline_420.jpg")
                .to_vec();
        let undefined = b"DDS ".to_vec();
        let short = vec![0, 0, 1];
        let garbage = vec![
            0x13, 7, 0x55, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ];
        let files: BTreeMap<&[u8], Vec<u8>> = [
            (&b"gfx/mod/icon.png"[..], png.clone()),
            (b"gfx/mod/renamed.tga", png),
            (b"gfx/mod/photo.dds", jpeg),
            (b"gfx/mod/tiny.dds", undefined),
            (b"gfx/mod/short.dds", short),
            (b"gfx/mod/noise.dds", garbage),
            (
                b"gfx/mod/huge.dds",
                vec![0; content_reader_layers::textures::TEXTURE_FILE_LIMIT + 1],
            ),
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
        let short = texture(b"gfx/mod/short.png", &read).unwrap().unwrap_err();
        assert!(
            matches!(&short, NoTexture::Undecodable { path, .. } if path == "gfx/mod/short.dds")
        );
        assert!(short.native_failure());
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
        content_reader_script::test_game::enter();
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
        use content_reader_image::native::{Decode, decode_texture_image, tests_support::dx10};

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
