use super::*;
use crate::map::native_text::Glyphs;
use std::path::Path;

/// A 32-bit uncompressed Targa of `width` by `height` transparent texels.
fn tga(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&width.to_le_bytes());
    bytes[14..16].copy_from_slice(&height.to_le_bytes());
    bytes[16] = 32;
    bytes[17] = 8;
    bytes.resize(18 + usize::from(width) * usize::from(height) * 4, 0);
    bytes
}

/// A `.fnt` of the given `char` lines, `lineHeight` `line`.
fn fnt(line: i32, chars: &[(char, i32)]) -> Vec<u8> {
    let mut text = format!(
        "info face=\"x\" size={line}\ncommon lineHeight={line} base={line} scaleW=999 scaleH=999 pages=1\n\
         page id=0 file=\"x.tga\"\nchars count={}\n",
        chars.len()
    );
    for (ch, advance) in chars {
        text.push_str(&format!(
            "char id={} x=1 y=2 width=3 height=4 xoffset=0 yoffset=0 xadvance={advance} page=0 chnl=15\n",
            u32::from(*ch)
        ));
    }
    text.into_bytes()
}

/// A content stack of one folder holding `files`.
fn stack(name: &str, files: &[(&str, Vec<u8>)]) -> (std::path::PathBuf, ResolvedLayers) {
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_native_resources_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(Path::new(&root), &[][..])]).unwrap();
    (root, layers)
}

fn loaded(
    layers: &ResolvedLayers,
    language: &[u8],
    assets: &Assets,
) -> (GuiResources, Vec<dds::Texture>) {
    let mut resources = GuiResources::new(
        Registry::load(layers).unwrap(),
        language,
        [5632, 2048],
        Decode::default(),
    );
    let mut images = Vec::new();
    resources.load(layers, assets, &mut images).unwrap();
    (resources, images)
}

#[test]
fn lazy_plain_resources_share_native_metadata_without_loading_texture() {
    let (root, layers) = stack(
        "lazy-plain",
        &[
            (
                "interface/lazy.gfx",
                br#"spriteTypes = {
            spriteType = { name = "GFX_portrait" textureFile = "gfx/portrait.tga"
                noOfFrames = 3 framedirectiony = yes anchor = { x = 0.25 y = 0.75 }
                transparencecheck = yes }
        }"#
                .to_vec(),
            ),
            ("gfx/portrait.tga", tga(6, 9)),
        ],
    );
    let mut lazy = GuiResources::new(
        Registry::load(&layers).unwrap(),
        b"l_english",
        [5632, 2048],
        Decode::default(),
    );
    lazy.load_lazy_plain(&layers, b"GFX_portrait").unwrap();
    assert!(lazy.textures.is_empty());
    assert_eq!(lazy.sheet(b"GFX_portrait").unwrap().texture, NO_TEXTURE);
    let deferred = lazy.sprite(b"GFX_portrait").unwrap();
    let assets = Assets {
        sprites: [b"GFX_portrait".to_vec()].into(),
        ..Assets::default()
    };
    lazy.sprites.remove(b"GFX_portrait".as_slice());
    let mut images = Vec::new();
    lazy.load(&layers, &assets, &mut images).unwrap();
    let mut expected = lazy.sprite(b"GFX_portrait").unwrap();
    expected.texture = [0; 2];
    assert_eq!(deferred, expected);
    assert_eq!(images.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

/// A mod's font: `path` appends and `fontfiles` replaces in order, the first file holding a
/// glyph wins, a page that does not load is named when a glyph on it draws, a language
/// override selects its own family, and an unknown name takes `GetFontOrFallback`'s font.
#[test]
fn fonts_load_through_the_native_registry() {
    let (root, layers) = stack(
        "fonts",
        &[
            (
                "interface/fonts.gfx",
                b"bitmapfonts = { bitmapfont = { name = \"mod_font\" path = \"gfx/fonts/a\" \
                  fontfiles = { \"gfx/fonts/b\" } path = \"gfx/fonts/c\" color = 0xff102030 } \
                  bitmapfont_override = { name = \"mod_font\" fontfiles = { \"gfx/fonts/k\" } \
                  languages = { \"l_korean\" } } }"
                    .to_vec(),
            ),
            ("gfx/fonts/a.fnt", fnt(30, &[('A', 30)])),
            ("gfx/fonts/b.fnt", fnt(10, &[('A', 5)])),
            ("gfx/fonts/b.tga", tga(8, 16)),
            ("gfx/fonts/c.fnt", fnt(20, &[('A', 9), ('B', 6)])),
            ("gfx/fonts/k.fnt", fnt(12, &[('A', 7)])),
            ("gfx/fonts/k.tga", tga(4, 4)),
        ],
    );
    let assets = Assets {
        sprites: Default::default(),
        fonts: [b"mod_font".to_vec(), b"missing_font".to_vec()].into(),
        textures: Default::default(),
    };
    let (english, images) = loaded(&layers, b"l_english", &assets);
    let font = english.font(b"mod_font").unwrap();
    assert_eq!(font.facts.color, 0xFF10_2030);
    // `a` was replaced by `fontfiles`; `b` holds `A` first, and its line height is the first
    // file's.
    assert_eq!(font.own('A').unwrap().advance, 5);
    assert_eq!(font.own('B').unwrap().advance, 6);
    assert_eq!(font.line_height(), 10);
    let a = font.quad('A').unwrap();
    assert_eq!(a.page.size, [8, 16]);
    assert_eq!(a.uv, [1.0 / 8.0, 2.0 / 16.0, 4.0 / 8.0, 6.0 / 16.0]);
    assert_eq!(images.len(), 1);
    // A page whose texture does not load keeps its glyphs over the null texture's 0 by 0.
    let b = font.quad('B').unwrap();
    assert_eq!(b.null_page.as_deref(), Some(b"gfx/fonts/c.tga".as_slice()));
    assert_eq!((b.page.texture, b.page.size), (NO_TEXTURE, [0, 0]));
    assert!(b.uv[0].is_infinite());
    // The one font is every unknown name's fallback.
    assert!(std::ptr::eq(english.font(b"missing_font").unwrap(), font));
    let (korean, _) = loaded(&layers, b"l_korean", &assets);
    assert_eq!(
        korean.font(b"mod_font").unwrap().own('A').unwrap().advance,
        7
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The loaded resources as a layout reads them, over one GUI document's templates.
struct Drawn<'a> {
    resources: &'a GuiResources,
    gui: &'a installed_content::gui::native::Gui,
    localisation: installed_content::localisation::Localisation,
}

impl crate::map::native_gui::Resources for Drawn<'_> {
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.resources.sprite(name)
    }
    fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        self.resources.font(name)
    }
    fn localisation(&self) -> &installed_content::localisation::Localisation {
        &self.localisation
    }
    fn text_colors(&self) -> &TextColors {
        self.resources.text_colors()
    }
    fn text_effect(&self) -> Result<Effect, String> {
        self.resources.text_effect()
    }
    fn effect(&self, file: &str) -> Result<Effect, String> {
        self.resources.effect(file)
    }
    fn shield_flag(&self, _: bool, _: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        Ok(None)
    }
    fn text_flag(&self, _: &str) -> Result<Option<Texture>, String> {
        Ok(None)
    }
    fn gui_type(&self, name: &[u8]) -> Option<&installed_content::gui::native::Element> {
        self.gui.template(name)
    }
    fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        self.resources.texture_file(path)
    }
    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.resources.sprite_animations(sprite)
    }
}

/// A mod whose GUI and GFX name a sprite with the same non-UTF-8 bytes: collected, loaded
/// and drawn under those bytes (`sprite_functions::lookup@140A2D0D0` compares them), not
/// under their lossy spelling, which another declaration holds.
#[test]
fn byte_names_match_end_to_end() {
    let (root, layers) = stack(
        "bytes",
        &[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_\x80a\" texturefile = \"gfx/a.tga\" } \
                  spriteType = { name = \"GFX_\xEF\xBF\xBDa\" texturefile = \"gfx/b.tga\" } \
                  spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" } }"
                    .to_vec(),
            ),
            ("gfx/a.tga", tga(6, 2)),
            ("gfx/b.tga", tga(4, 4)),
        ],
    );
    let gui = installed_content::gui::native::read(
        b"guiTypes = { containerWindowType = { name = \"w\" size = { width = 50 height = 50 } \
          iconType = { name = \"i\" spriteType = \"GFX_\x80a\" } } }",
    );
    let window = gui.template("w").unwrap();
    let assets = crate::map::native_gui::assets_in(window, &|_| None, &Default::default());
    assert!(assets.sprites.contains(b"GFX_\x80a".as_slice()));
    let (resources, _) = loaded(&layers, b"l_english", &assets);
    assert_eq!(resources.sprite(b"GFX_\x80a").unwrap().texture, [6, 2]);
    let drawn = Drawn {
        resources: &resources,
        gui: &gui,
        localisation: Default::default(),
    };
    let mut instance =
        crate::map::native_gui::lifecycle::Instance::construct(window, [800, 600], &drawn).unwrap();
    instance.gui_show(window, window).unwrap();
    let first = crate::map::native_gui::lifecycle::Tick { time: 0.0, dt: 0.0 };
    instance.tick(window, &drawn, first).unwrap();
    let layout = instance.layout(window, &drawn, None).unwrap().unwrap();
    assert!(matches!(
        &layout.parts[0].kind,
        crate::map::native_gui::PartKind::Sprite { sprite, .. } if sprite == b"GFX_\x80a"
    ));
    assert_eq!(resources.sheet(b"GFX_\x80a").unwrap().size, [6, 2]);
    let _ = std::fs::remove_dir_all(root);
}

/// A texture whose bytes do not determine what the game draws (FreeImage's format sniff
/// reading past a three-byte file) refuses what draws it, where a texture the game fails to
/// load draws nothing: a sprite, a font page, an edit box's texture file.
#[test]
fn undetermined_textures_are_refused_not_dropped() {
    // Three bytes: FreeImage's ICO validate compares six (uninitialised memory past them).
    let jpeg = vec![0, 0, 1];
    let (root, layers) = stack(
        "unimplemented",
        &[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_photo\" texturefile = \"gfx/photo.jpg\" } \
                  spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" } } \
                  bitmapfonts = { bitmapfont = { name = \"f\" path = \"gfx/fonts/j\" } }"
                    .to_vec(),
            ),
            ("gfx/photo.jpg", jpeg.clone()),
            ("gfx/b.tga", tga(4, 4)),
            ("gfx/fonts/j.fnt", fnt(10, &[('A', 5)])),
            ("gfx/fonts/j.tga", jpeg.clone()),
            ("gfx/edit.dds", jpeg),
        ],
    );
    let assets = Assets {
        sprites: [b"GFX_photo".to_vec()].into(),
        fonts: [b"f".to_vec()].into(),
        textures: [b"gfx/edit.dds".to_vec()].into(),
    };
    let (resources, _) = loaded(&layers, b"l_english", &assets);
    let photo = resources.sprite(b"GFX_photo").unwrap();
    assert_eq!(photo.refusals.len(), 1);
    assert_eq!(photo.refusals[0].0, RefusalKind::UndefinedRead);
    let font = resources.font(b"f").unwrap();
    assert_eq!(font.refusals[0].0, RefusalKind::UndefinedRead);
    assert!(matches!(
        resources.texture_file(b"gfx/edit.dds"),
        Some(TextureFile::Refused(RefusalKind::UndefinedRead, _))
    ));
    let _ = std::fs::remove_dir_all(root);
}

/// A 16-bit page stays at full precision; shader-pipeline handles device filtering.
#[test]
fn a_sixteen_bit_texture_is_loaded_at_full_precision() {
    let mut dds = vec![0_u8; 128];
    dds[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (12, 1),
        (16, 1),
        (28, 1),
        (76, 32),
        (80, 0x40),
        (88, 16),
        (92, 0xF800),
        (96, 0x7E0),
        (100, 0x1F),
    ] {
        dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dds.extend(0x0800_u16.to_le_bytes());
    let (root, layers) = stack(
        "platform",
        &[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_565\" texturefile = \"gfx/r.dds\" } \
                  spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" } }"
                    .to_vec(),
            ),
            ("gfx/r.dds", dds),
            ("gfx/b.tga", tga(4, 4)),
        ],
    );
    let assets = Assets {
        sprites: [b"GFX_565".to_vec()].into(),
        ..Assets::default()
    };
    let (resources, images) = loaded(&layers, b"l_english", &assets);
    assert!(resources.sprite(b"GFX_565").unwrap().refusals.is_empty());
    let image = images
        .iter()
        .find(|image| image.format == dds::TextureFormat::B5G6R5)
        .unwrap();
    assert_eq!(
        image
            .format
            .sample_16(u16::from_le_bytes(
                image.mips[0][0].data[..2].try_into().unwrap()
            ))
            .unwrap()[0],
        1. / 31.
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Two sprites sharing a texture whose draw the file does not determine are refused alike,
/// whichever loads it first and whether they load for the same window or for separate ones
/// (a second [`GuiResources::load`] finds the texture already loaded); a font page on it is
/// refused too.
#[test]
fn a_shared_refused_texture_refuses_every_user() {
    let (root, layers) = stack(
        "shared",
        &[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_one\" texturefile = \"gfx/x.dds\" } \
                  spriteType = { name = \"GFX_two\" texturefile = \"gfx/x.dds\" noOfFrames = 2 } \
                  spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" } } \
                  bitmapfonts = { bitmapfont = { name = \"f\" path = \"gfx/x\" } }"
                    .to_vec(),
            ),
            // Three bytes: FreeImage's ICO validate compares six.
            ("gfx/x.dds", vec![0, 0, 1]),
            ("gfx/b.tga", tga(4, 4)),
            ("gfx/x.fnt", fnt(10, &[('A', 5)])),
        ],
    );
    let window = |sprite: &[u8]| Assets {
        sprites: [sprite.to_vec()].into(),
        ..Assets::default()
    };
    let mut seen = Vec::new();
    for order in [[&b"GFX_one"[..], b"GFX_two"], [b"GFX_two", b"GFX_one"]] {
        // Separate windows: one load each.
        let mut resources = GuiResources::new(
            Registry::load(&layers).unwrap(),
            b"l_english",
            [5632, 2048],
            Decode::default(),
        );
        let mut images = Vec::new();
        for sprite in order {
            resources
                .load(&layers, &window(sprite), &mut images)
                .unwrap();
        }
        // One window: both in one load.
        let both = Assets {
            sprites: order.iter().map(|name| name.to_vec()).collect(),
            fonts: [b"f".to_vec()].into(),
            ..Assets::default()
        };
        let (together, _) = loaded(&layers, b"l_english", &both);
        for resources in [&resources, &together] {
            let one = resources.sprite(b"GFX_one").unwrap().refusals;
            let two = resources.sprite(b"GFX_two").unwrap().refusals;
            assert_eq!(one.len(), 1, "{one:?}");
            assert_eq!(one[0].0, RefusalKind::UndefinedRead);
            assert_eq!(one, two);
            seen.push(one);
        }
        assert_eq!(together.font(b"f").unwrap().refusals, seen[seen.len() - 1]);
    }
    assert!(seen.windows(2).all(|pair| pair[0] == pair[1]));
    let _ = std::fs::remove_dir_all(root);
}

/// A mod's sprites: the latest declaration's texture and frames, a texture that does not
/// load named (the plain draw then returns), an undefined name left to the fallback, and an
/// missing effect as the native null record and a missing backend header named explicitly.
#[test]
fn sprites_load_through_the_native_registry() {
    let (root, layers) = stack(
        "sprites",
        &[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_mod\" texturefile = \"gfx/a.dds\" } }"
                    .to_vec(),
            ),
            (
                "interface/b.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_mod\" texturefile = \"gfx/b.tga\" \
                  noOfFrames = 3 effectFile = \"gfx/FX/none.lua\" } \
                  spriteType = { name = \"GFX_gone\" texturefile = \"gfx/gone.dds\" \
                  effectFile = \"gfx/FX/mod.lua\" } \
                  spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" } }"
                    .to_vec(),
            ),
            ("gfx/b.tga", tga(6, 2)),
            ("gfx/FX/mod.shader", b"Effect Up { PixelShader = \"x\" }".to_vec()),
        ],
    );
    let assets = Assets {
        sprites: [b"GFX_mod".as_slice(), b"GFX_gone", b"GFX_undefined"]
            .map(<[u8]>::to_vec)
            .into(),
        fonts: Default::default(),
        textures: Default::default(),
    };
    let (resources, images) = loaded(&layers, b"l_english", &assets);
    let info = resources.sprite(b"GFX_mod").unwrap();
    assert_eq!((info.texture, info.frames, info.missing), ([6, 2], 3, None));
    assert!(!info.effect.unwrap().has_technique("Up"));
    assert_eq!(resources.sheet(b"GFX_mod").unwrap().size, [6, 2]);
    // One texture, loaded once for both sprites that name it.
    assert_eq!(images.len(), 1);
    let gone = resources.sprite(b"GFX_gone").unwrap();
    assert!(gone.missing.unwrap().contains("gfx/gone.dds"));
    assert_eq!(gone.hit, Hit::Through);
    assert!(gone.effect.unwrap_err().contains("defines_hlsl_dx11.fxh"));
    assert!(resources.sprite(b"GFX_undefined").is_none());
    // A lazily loaded sprite keeps no records; a texture file is answered once loaded, and
    // not before.
    assert_eq!(resources.sprite_animations(b"GFX_mod"), Some(Vec::new()));
    assert!(matches!(
        resources.texture_file(b"gfx/b.tga"),
        Some(TextureFile::Loaded(texture)) if texture.size == [6, 2]
    ));
    assert_eq!(resources.texture_file(b"gfx/other.dds"), None);
    assert!(resources.sprite(FALLBACK_SPRITE).is_some());
    assert!(resources.registry().sprite(b"GFX_undefined").is_none());
    // The effects the game's code names are missing here; the content declares no `textcolors`.
    assert!(!resources.text_effect().unwrap().has_technique("Text"));
    assert!(
        !resources
            .effect(SHIELD_OVERLAY_EFFECT)
            .unwrap()
            .has_technique("Up")
    );
    assert!(resources.effect("gfx/FX/other.lua").is_err());
    assert!(resources.text_colors().is_empty());
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn arbitrary_effects_load_lazily_without_a_model_or_compiler() {
    let (root, layers) = stack(
        "lazy-effect",
        &[
            (
                "gfx/FX/mod-owned.shader",
                br#"
VertexStruct I { float3 p : POSITION; };
VertexStruct O { float4 p : SV_POSITION; };
VertexShader = { MainCode V [[ O main(I v) { O o; o.p=float4(v.p,1); return o; } ]] }
PixelShader = { MainCode P [[ float4 main(O v) : SV_TARGET { return float4(0,1,0,1); } ]] }
Effect Up { VertexShader = V PixelShader = P }
"#
                .to_vec(),
            ),
            ("gfx/FX/defines_hlsl_dx11.fxh", Vec::new()),
        ],
    );
    let mut resources = GuiResources::new(
        Registry::load(&layers).unwrap(),
        b"l_english",
        [5632, 2048],
        Decode::default(),
    );
    let effect = resources
        .effect_result(&layers, b"gfx/FX/mod-owned.lua", false)
        .unwrap();
    assert!(effect.has_technique("Up"));
    // The renderer's globals come first (1423F4EC0), then the GUI's own defines.
    assert_eq!(
        effect.compiled.as_ref().unwrap().defines,
        super::super::native_shader::library::globals([5632, 2048], Decode::default())
    );
    let masked = resources
        .effect_result(&layers, b"gfx/FX/mod-owned.lua", true)
        .unwrap();
    let defines = &masked.compiled.as_ref().unwrap().defines;
    assert_eq!(
        (defines[0].as_str(), defines.last().unwrap().as_str()),
        ("MAP_SIZE_X 5632.000", "MASKING")
    );
    assert_eq!(
        resources
            .compiler
            .as_ref()
            .unwrap()
            .cache
            .diagnostics()
            .compiled,
        0
    );
    let again = resources
        .effect_result(&layers, b"gfx/FX/mod-owned.lua", false)
        .unwrap();
    assert_eq!(effect, again);
    assert!(resources.compiler.as_ref().unwrap().refused.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

/// The client's one `force_pow2_textures` reaches both readers of `get_force_power_of_two
/// @1423F07B0`: the textures' decode and the effects' power-of-two globals (`140A4DDE0`).
#[test]
fn force_pow2_textures_sets_the_decode_and_the_effect_globals() {
    let (root, layers) = stack(
        "pow2-globals",
        &[
            (
                "gfx/FX/mod-owned.shader",
                b"Effect Up { VertexShader = V PixelShader = P }\n".to_vec(),
            ),
            ("gfx/FX/defines_hlsl_dx11.fxh", Vec::new()),
        ],
    );
    for (settings, pow2) in [
        ("force_pow2_textures=no", ["1.000"; 4]),
        (
            "force_pow2_textures=yes",
            ["0.917", "0.667", "0.688", "1.000"],
        ),
    ] {
        let decode = installed_content::settings::SettingsKeys::read(settings.as_bytes()).texture_decode();
        let mut resources = GuiResources::new(
            Registry::load(&layers).unwrap(),
            b"l_english",
            [5632, 2048],
            decode,
        );
        assert_eq!(resources.decode, decode);
        let effect = resources
            .effect_result(&layers, b"gfx/FX/mod-owned.lua", false)
            .unwrap();
        let defines = &effect.compiled.as_ref().unwrap().defines;
        let value = |name: &str| {
            defines
                .iter()
                .find_map(|define| define.strip_prefix(name)?.strip_prefix(' '))
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            ["FOW_POW2_X", "FOW_POW2_Y", "MAP_POW2_X", "MAP_POW2_Y"].map(value),
            pow2,
            "{settings}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}
