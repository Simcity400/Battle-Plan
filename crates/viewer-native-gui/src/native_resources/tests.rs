use super::*;
use crate::native_text::Glyphs;
use std::path::Path;
use std::path::PathBuf;
use std::collections::BTreeSet;

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

fn stack(name: &str, files: &[(&str, Vec<u8>)]) -> (PathBuf, ResolvedLayers) {
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
) -> (GuiResources, Vec<viewer_gpu::textures::GuiSource>) {
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
    content_reader_script::test_game::enter();
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
    assert!(lazy.sprites.is_empty());
    assert!(lazy.sprite_diagnostics.is_empty());
    let slot = &lazy.lazy_plain[b"GFX_portrait".as_slice()].prepared;
    assert!(slot.get().is_none());
    assert!(lazy.sprite_alpha(b"GFX_portrait").is_none());
    assert!(slot.get().is_none());
    assert_eq!(lazy.sheet(b"GFX_portrait").unwrap().texture, NO_TEXTURE);
    assert!(slot.get().is_some());
    let deferred = lazy.sprite(b"GFX_portrait").unwrap();
    let assets = Assets {
        sprites: [b"GFX_portrait".to_vec()].into(),
        ..Assets::default()
    };
    lazy.lazy_plain.remove(b"GFX_portrait".as_slice());
    let mut images = Vec::new();
    lazy.load(&layers, &assets, &mut images).unwrap();
    let mut expected = lazy.sprite(b"GFX_portrait").unwrap();
    expected.texture = [0; 2];
    assert_eq!(deferred, expected);
    assert_eq!(images.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fonts_load_through_the_native_registry() {
    content_reader_script::test_game::enter();
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
    assert_eq!(font.own('A').unwrap().advance, 5);
    assert_eq!(font.own('B').unwrap().advance, 6);
    assert_eq!(font.line_height(), 10);
    let a = font.quad('A').unwrap();
    assert_eq!(a.page.size, [8, 16]);
    assert_eq!(a.uv, [1.0 / 8.0, 2.0 / 16.0, 4.0 / 8.0, 6.0 / 16.0]);
    assert_eq!(images.len(), 1);
    let b = font.quad('B').unwrap();
    assert_eq!(b.null_page.as_deref(), Some(b"gfx/fonts/c.tga".as_slice()));
    assert_eq!((b.page.texture, b.page.size), (NO_TEXTURE, [0, 0]));
    assert!(b.uv[0].is_infinite());
    assert!(std::ptr::eq(english.font(b"missing_font").unwrap(), font));
    let (korean, _) = loaded(&layers, b"l_korean", &assets);
    assert_eq!(
        korean.font(b"mod_font").unwrap().own('A').unwrap().advance,
        7
    );
    let _ = std::fs::remove_dir_all(root);
}

struct Drawn<'a> {
    resources: &'a GuiResources,
    gui: &'a content_reader_gui::gui::native::Gui,
    localisation: content_reader_localisation::localisation::Localisation,
}

impl crate::native_gui::Resources for Drawn<'_> {
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.resources.sprite(name)
    }
    fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        self.resources.font(name)
    }
    fn localisation(&self) -> &content_reader_localisation::localisation::Localisation {
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
    fn gui_type(&self, name: &[u8]) -> Option<&content_reader_gui::gui::native::Element> {
        self.gui.template(name)
    }
    fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        self.resources.texture_file(path)
    }
    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.resources.sprite_animations(sprite)
    }
}

#[test]
fn byte_names_match_end_to_end() {
    content_reader_script::test_game::enter();
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
    let gui = content_reader_gui::gui::native::read(
        b"guiTypes = { containerWindowType = { name = \"w\" size = { width = 50 height = 50 } \
          iconType = { name = \"i\" spriteType = \"GFX_\x80a\" } } }",
    );
    let window = gui.template("w").unwrap();
    let assets = crate::native_gui::assets_in(window, &|_| None, &Default::default());
    assert!(assets.sprites.contains(b"GFX_\x80a".as_slice()));
    let (resources, _) = loaded(&layers, b"l_english", &assets);
    assert_eq!(resources.sprite(b"GFX_\x80a").unwrap().texture, [6, 2]);
    let drawn = Drawn {
        resources: &resources,
        gui: &gui,
        localisation: Default::default(),
    };
    let mut instance =
        crate::native_gui::lifecycle::Instance::construct(window, [800, 600], &drawn).unwrap();
    instance.gui_show(window, window).unwrap();
    let first = crate::native_gui::lifecycle::Tick { time: 0.0, dt: 0.0 };
    instance.tick(window, &drawn, first).unwrap();
    let layout = instance.layout(window, &drawn, None).unwrap().unwrap();
    assert!(matches!(
        &layout.parts[0].kind,
        crate::native_gui::PartKind::Sprite { sprite, .. } if sprite == b"GFX_\x80a"
    ));
    assert_eq!(resources.sheet(b"GFX_\x80a").unwrap().size, [6, 2]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn undetermined_textures_are_refused_not_dropped() {
    content_reader_script::test_game::enter();
    let mut truncated_tga = tga(1, 1);
    truncated_tga.truncate(18);
    let (root, layers) = stack(
        "unimplemented",
        &[
            (
                "interface/a.gfx",
                b"spriteTypes = { spriteType = { name = \"GFX_photo\" texturefile = \"gfx/photo.dds\" } \
                  spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" } } \
                  bitmapfonts = { bitmapfont = { name = \"f\" path = \"gfx/fonts/j\" } }"
                    .to_vec(),
            ),
            ("gfx/photo.dds", b"DDS ".to_vec()),
            ("gfx/b.tga", tga(4, 4)),
            ("gfx/fonts/j.fnt", fnt(10, &[('A', 5)])),
            ("gfx/fonts/j.tga", truncated_tga),
            ("gfx/edit.dds", b"DDS ".to_vec()),
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

#[test]
fn a_sixteen_bit_texture_is_loaded_at_full_precision() {
    content_reader_script::test_game::enter();
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
    let source = images
        .iter()
        .find(|image| image.meta().format == dds::TextureFormat::B5G6R5)
        .unwrap();
    assert!(!source.is_held());
    let image = source.decode().unwrap();
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

#[test]
fn a_shared_refused_texture_refuses_every_user() {
    content_reader_script::test_game::enter();
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
            ("gfx/x.dds", b"DDS ".to_vec()),
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

#[test]
fn sprites_load_through_the_native_registry() {
    content_reader_script::test_game::enter();
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
    assert_eq!(images.len(), 1);
    let gone = resources.sprite(b"GFX_gone").unwrap();
    assert!(gone.missing.unwrap().contains("gfx/gone.dds"));
    assert_eq!(gone.hit, Hit::Through);
    assert!(gone.effect.unwrap_err().contains("defines_hlsl_dx11.fxh"));
    assert!(resources.sprite(b"GFX_undefined").is_none());
    assert_eq!(resources.sprite_animations(b"GFX_mod"), Some(Vec::new()));
    assert!(matches!(
        resources.texture_file(b"gfx/b.tga"),
        Some(TextureFile::Loaded(texture)) if texture.size == [6, 2]
    ));
    assert_eq!(resources.texture_file(b"gfx/other.dds"), None);
    assert!(resources.sprite(FALLBACK_SPRITE).is_some());
    assert!(resources.registry().sprite(b"GFX_undefined").is_none());
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
    content_reader_script::test_game::enter();
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
    assert_eq!(
        effect.compiled.as_ref().unwrap().defines,
        shader_pipeline::library::globals([5632, 2048], Decode::default())
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

#[test]
fn force_pow2_textures_sets_the_decode_and_the_effect_globals() {
    content_reader_script::test_game::with(|game| game.word("force_pow2_textures"));
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
    for (settings, force_pow2_textures, pow2) in [
        ("force_pow2_textures=no", false, ["1.000"; 4]),
        (
            "force_pow2_textures=yes",
            true,
            ["0.917", "0.667", "0.688", "1.000"],
        ),
    ] {
        let decode =
            content_reader_settings::settings::SettingsKeys::read(settings.as_bytes()).texture_decode();
        assert_eq!(decode.force_pow2_textures, force_pow2_textures);
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

#[test]
fn a_font_page_the_mods_break_or_lack_is_named_and_the_recordings_drawn() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_font_pages_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let sprites = "spriteTypes = { spriteType = { name = \"GFX_default_fallback_texture\" \
                   texturefile = \"gfx/b.tga\" } }";
    write(
        "base/interface/a.gfx",
        format!(
            "{sprites} bitmapfonts = {{ bitmapfont = {{ name = \"g\" path = \"gfx/fonts/j\" }} }}"
        )
        .as_bytes(),
    );
    write("base/gfx/b.tga", &tga(4, 4));
    write("base/gfx/fonts/j.fnt", &fnt(10, &[('A', 5)]));
    write("base/gfx/fonts/j.tga", &tga(4, 4));
    write(
        "art/interface/a.gfx",
        format!(
            "{sprites} bitmapfonts = {{ bitmapfont = {{ name = \"g\" path = \"gfx/fonts/j\" }} \
             bitmapfont = {{ name = \"f\" path = \"gfx/fonts/m\" }} }}"
        )
        .as_bytes(),
    );
    write("art/gfx/fonts/j.tga", b"broken");
    write("art/gfx/fonts/m.fnt", &fnt(10, &[('A', 5)]));
    let art = root.join("art").display().to_string().replace('\\', "/");
    write(
        "user/mod/art.mod",
        format!("name=\"Art\"\npath=\"{art}\"\n").as_bytes(),
    );
    std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
    let options = content_reader_layers::ResolveOptions {
        game_dir: root.join("steamapps/common/Game"),
        user_dir: root.join("user"),
        cache_dir: root.join("cache"),
    };
    let base = root.join("base");
    let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
    let selected = [content_reader_layers::content::visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    }];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel).unwrap();
    let presentation = layers.presentation();
    let assets = Assets {
        sprites: Default::default(),
        fonts: [b"f".to_vec(), b"g".to_vec()].into(),
        textures: Default::default(),
    };
    let (resources, _) = loaded(presentation, b"l_english", &assets);
    let _ = std::fs::remove_dir_all(&root);
    assert!(resources.font(b"g").unwrap().pages[0].is_some());
    assert!(resources.font(b"f").unwrap().pages[0].is_none());
    let refused = presentation.refused();
    assert!(refused.contains("font page gfx/fonts/m.tga"), "{refused:?}");
    assert!(
        !refused.contains("font page gfx/fonts/j.tga"),
        "{refused:?}"
    );
    let refusals = presentation.read_warnings().overlay_refusals().unwrap();
    assert!(refusals.contains("texture gfx/fonts/j.tga ("), "{refusals}");
}

#[test]
fn what_the_mods_cannot_draw_is_the_recordings_own_declaration_with_its_registry() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_recorded_declarations_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let fallback = "spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" }";
    write(
        "base/interface/a.gfx",
        format!(
            "spriteTypes = {{ {fallback} spriteType = {{ name = \"GFX_x\" texturefile = \"gfx/x.tga\" }}              spriteType = {{ name = \"GFX_kept\" texturefile = \"gfx/b.tga\" }} }}              bitmapfonts = {{ bitmapfont = {{ name = \"g\" path = \"gfx/fonts/j\" }} }}"
        )
        .as_bytes(),
    );
    write("base/gfx/b.tga", &tga(4, 4));
    write("base/gfx/x.tga", &tga(8, 2));
    write("base/gfx/fonts/j.fnt", &fnt(10, &[('A', 5)]));
    write("base/gfx/fonts/j.tga", &tga(4, 4));
    write(
        "art/interface/a.gfx",
        format!(
            "spriteTypes = {{ {fallback} spriteType = {{ name = \"GFX_x\" texturefile = \"gfx/mod_x.tga\" }}              spriteType = {{ name = \"GFX_kept\" texturefile = \"gfx/kept.tga\" }} }}              bitmapfonts = {{ bitmapfont = {{ name = \"g\" path = \"gfx/fonts/mod_j\" }} }}"
        )
        .as_bytes(),
    );
    write("art/gfx/mod_x.tga", b"broken");
    write("art/gfx/kept.tga", &tga(6, 6));
    write("art/gfx/fonts/mod_j.fnt", &fnt(10, &[('A', 5)]));
    write("art/gfx/fonts/mod_j.tga", b"broken");
    let art = root.join("art").display().to_string().replace('\\', "/");
    write(
        "user/mod/art.mod",
        format!("name=\"Art\"
path=\"{art}\"
").as_bytes(),
    );
    std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
    let options = content_reader_layers::ResolveOptions {
        game_dir: root.join("steamapps/common/Game"),
        user_dir: root.join("user"),
        cache_dir: root.join("cache"),
    };
    let base = root.join("base");
    let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
    let selected = [content_reader_layers::content::visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    }];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel).unwrap();
    let presentation = layers.presentation();
    let assets = Assets {
        sprites: [b"GFX_x".to_vec(), b"GFX_kept".to_vec()].into(),
        fonts: [b"g".to_vec()].into(),
        textures: Default::default(),
    };
    let draw = |recorded: bool| {
        let mut resources = GuiResources::new(
            Registry::load(presentation).unwrap(),
            b"l_english",
            [5632, 2048],
            Decode::default(),
        );
        if recorded {
            resources = resources.with_recorded_registry(Registry::load(&layers.recorded_only()).unwrap());
        }
        let mut images = Vec::new();
        let (loaded, fallbacks) = content_reader_layers::content::observe_overlay_refusals(|| {
            resources.load(presentation, &assets, &mut images)
        });
        loaded.unwrap();
        (resources, fallbacks)
    };
    let (map, _) = draw(false);
    assert!(map.sprite(b"GFX_x").unwrap().missing.is_some(), "the map's retry by path");
    assert!(map.font(b"g").unwrap().pages[0].is_none());
    let (window, fallbacks) = draw(true);
    let _ = std::fs::remove_dir_all(&root);
    let x = window.sprite(b"GFX_x").unwrap();
    assert_eq!((x.missing.clone(), x.texture), (None, [8, 2]), "the recording's own GFX_x");
    assert_eq!(window.sprite(b"GFX_kept").unwrap().texture, [6, 6], "the mod's own");
    assert!(window.font(b"g").unwrap().pages[0].is_some(), "the recording's own font");
    assert!(fallbacks.iter().any(|why| why.starts_with("sprite GFX_x (")), "{fallbacks:?}");
    assert!(fallbacks.iter().any(|why| why.starts_with("font g (")), "{fallbacks:?}");
    assert!(!fallbacks.iter().any(|why| why.contains("kept")), "{fallbacks:?}");
}

fn mounted(
    name: &str,
    base: &[(&str, Vec<u8>)],
    art: &[(&str, Vec<u8>)],
) -> (PathBuf, ResolvedLayers) {
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_mounted_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    for (path, bytes) in base {
        write(&format!("base/{path}"), bytes);
    }
    for (path, bytes) in art {
        write(&format!("art/{path}"), bytes);
    }
    let folder = root.join("art").display().to_string().replace('\\', "/");
    write(
        "user/mod/art.mod",
        format!("name=\"Art\"\npath=\"{folder}\"\n").as_bytes(),
    );
    std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
    let options = content_reader_layers::ResolveOptions {
        game_dir: root.join("steamapps/common/Game"),
        user_dir: root.join("user"),
        cache_dir: root.join("cache"),
    };
    let mut layers = ResolvedLayers::from_folders(&[(root.join("base").as_path(), &[][..])]).unwrap();
    let selected = [content_reader_layers::content::visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    }];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel).unwrap();
    (root, layers)
}

fn with_recorded(
    layers: &ResolvedLayers,
    assets: &Assets,
) -> (io::Result<()>, GuiResources, BTreeSet<String>) {
    let presentation = layers.presentation();
    let mut resources = GuiResources::new(
        Registry::load(presentation).unwrap(),
        b"l_english",
        [5632, 2048],
        Decode::default(),
    )
    .with_recorded_registry(Registry::load(&layers.recorded_only()).unwrap());
    let mut images = Vec::new();
    let (loaded, fallbacks) = content_reader_layers::content::observe_overlay_refusals(|| {
        resources.load(presentation, assets, &mut images)
    });
    (loaded, resources, fallbacks)
}

fn sprites(names: &[&str]) -> Assets {
    Assets {
        sprites: names.iter().map(|name| name.as_bytes().to_vec()).collect(),
        fonts: Default::default(),
        textures: Default::default(),
    }
}

fn fonts(names: &[&str]) -> Assets {
    Assets {
        sprites: Default::default(),
        fonts: names.iter().map(|name| name.as_bytes().to_vec()).collect(),
        textures: Default::default(),
    }
}

fn held_unreadable(path: &Path) -> std::fs::File {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap()
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0)).unwrap();
        std::fs::File::open(path.parent().unwrap()).unwrap()
    }
}

const FALLBACK_DECLARATION: &str =
    "spriteType = { name = \"GFX_default_fallback_texture\" texturefile = \"gfx/b.tga\" }";

#[test]
fn a_mod_read_that_fails_is_the_attempts_failure() {
    content_reader_script::test_game::enter();
    let (root, layers) = mounted(
        "failed_read",
        &[
            (
                "interface/a.gfx",
                format!(
                    "spriteTypes = {{ {FALLBACK_DECLARATION} \
                     spriteType = {{ name = \"GFX_x\" texturefile = \"gfx/x.tga\" }} }}"
                )
                .into_bytes(),
            ),
            ("gfx/b.tga", tga(4, 4)),
            ("gfx/x.tga", tga(8, 2)),
        ],
        &[
            (
                "interface/a.gfx",
                format!(
                    "spriteTypes = {{ {FALLBACK_DECLARATION} \
                     spriteType = {{ name = \"GFX_x\" texturefile = \"gfx/mod_x.tga\" }} \
                     spriteType = {{ name = \"GFX_mod\" texturefile = \"gfx/mod_x.tga\" }} }}"
                )
                .into_bytes(),
            ),
            ("gfx/mod_x.tga", tga(6, 6)),
        ],
    );
    let held = held_unreadable(&root.join("art/gfx/mod_x.tga"));
    let (loaded, resources, fallbacks) = with_recorded(&layers, &sprites(&["GFX_x"]));
    let (only_mod, _, _) = with_recorded(&layers, &sprites(&["GFX_mod"]));
    drop(held);
    let _ = std::fs::remove_dir_all(&root);
    loaded.unwrap();
    let x = resources.sprite(b"GFX_x").unwrap();
    assert_eq!((x.missing.clone(), x.texture), (None, [8, 2]), "the recording's own GFX_x");
    assert!(fallbacks.iter().any(|why| why.starts_with("sprite GFX_x (")), "{fallbacks:?}");
    assert!(only_mod.is_err(), "a sprite only the mods declare keeps their outcome");
}

#[test]
fn fonts_fall_back_to_the_recordings_own_of_their_name_only() {
    content_reader_script::test_game::enter();
    let (root, layers) = mounted(
        "font_names",
        &[
            (
                "interface/a.gfx",
                format!(
                    "spriteTypes = {{ {FALLBACK_DECLARATION} }} bitmapfonts = {{ \
                     bitmapfont = {{ name = \"font_a\" path = \"gfx/fonts/a\" }} \
                     bitmapfont = {{ name = \"font_b\" path = \"gfx/fonts/b\" }} }}"
                )
                .into_bytes(),
            ),
            ("gfx/b.tga", tga(4, 4)),
            ("gfx/fonts/a.fnt", fnt(10, &[('A', 5)])),
            ("gfx/fonts/a.tga", tga(4, 4)),
            ("gfx/fonts/b.fnt", fnt(12, &[('B', 7)])),
            ("gfx/fonts/b.tga", tga(4, 4)),
        ],
        &[
            (
                "interface/a.gfx",
                format!(
                    "spriteTypes = {{ {FALLBACK_DECLARATION} }} bitmapfonts = {{ \
                     bitmapfont = {{ name = \"font_m\" path = \"gfx/fonts/m\" }} }}"
                )
                .into_bytes(),
            ),
            ("gfx/fonts/m.fnt", fnt(10, &[('M', 5)])),
            ("gfx/fonts/m.tga", b"broken".to_vec()),
        ],
    );
    let (loaded, resources, fallbacks) =
        with_recorded(&layers, &fonts(&["font_a", "font_b", "font_m"]));
    let _ = std::fs::remove_dir_all(&root);
    loaded.unwrap();
    for name in ["font_a", "font_b"] {
        let font = resources.font(name.as_bytes()).unwrap();
        assert_eq!(font.facts.name, name.as_bytes(), "the recording's own {name}");
        assert!(font.pages[0].is_some());
        assert!(
            fallbacks.iter().any(|why| why.starts_with(&format!("font {name} ("))),
            "{fallbacks:?}"
        );
    }
    let only_mod = resources.font(b"font_m").unwrap();
    assert_eq!(only_mod.facts.name, b"font_m", "the mods' own");
    assert!(only_mod.pages[0].is_none());
    assert!(!fallbacks.iter().any(|why| why.starts_with("font font_m (")), "{fallbacks:?}");
}

#[test]
fn only_the_kept_attempts_notes_are_named() {
    content_reader_script::test_game::enter();
    let shader = |technique: &str| {
        format!(
            "VertexStruct I {{ float3 p : POSITION;\n}};\n\
             VertexStruct O {{ float4 p : SV_POSITION;\n}};\n\
             VertexShader = {{ MainCode V [[ O main(I v) {{ O o; o.p=float4(v.p,1); return o; }} ]] }}\n\
             PixelShader = {{ MainCode P [[ float4 main(O v) : SV_TARGET {{ return float4(0,1,0,1); }} ]] }}\n\
             Effect {technique} {{ VertexShader = V PixelShader = P }}\n"
        )
        .into_bytes()
    };
    let (root, layers) = mounted(
        "attempt_notes",
        &[
            (
                "interface/a.gfx",
                format!(
                    "spriteTypes = {{ {FALLBACK_DECLARATION} \
                     spriteType = {{ name = \"GFX_x\" texturefile = \"gfx/x.tga\" effectFile = \"gfx/FX/own.lua\" }} }}"
                )
                .into_bytes(),
            ),
            ("gfx/b.tga", tga(4, 4)),
            ("gfx/x.tga", tga(8, 2)),
            ("gfx/FX/own.shader", shader("Up")),
            ("gfx/FX/shared.shader", shader("Up")),
            ("gfx/FX/defines_hlsl_dx11.fxh", Vec::new()),
        ],
        &[
            (
                "interface/a.gfx",
                format!(
                    "spriteTypes = {{ {FALLBACK_DECLARATION} \
                     spriteType = {{ name = \"GFX_x\" texturefile = \"gfx/mod_x.tga\" effectFile = \"gfx/FX/shared.lua\" }} \
                     spriteType = {{ name = \"GFX_kept\" texturefile = \"gfx/b.tga\" effectFile = \"gfx/FX/shared.lua\" }} }}"
                )
                .into_bytes(),
            ),
            ("gfx/mod_x.tga", b"broken".to_vec()),
            ("gfx/FX/shared.shader", b"Code [[ \x80 ]]".to_vec()),
        ],
    );
    let presentation = layers.presentation();
    let mut resources = GuiResources::new(
        Registry::load(presentation).unwrap(),
        b"l_english",
        [5632, 2048],
        Decode::default(),
    )
    .with_recorded_registry(Registry::load(&layers.recorded_only()).unwrap());
    let mut images = Vec::new();
    let mut load = |names: &[&str]| {
        content_reader_layers::content::observe_overlay_refusals(|| {
            resources.load(presentation, &sprites(names), &mut images)
        })
    };
    let (first, recorded) = load(&["GFX_x"]);
    let (second, kept) = load(&["GFX_kept"]);
    let _ = std::fs::remove_dir_all(&root);
    first.unwrap();
    second.unwrap();
    assert!(recorded.iter().any(|why| why.starts_with("sprite GFX_x (")), "{recorded:?}");
    assert!(!recorded.iter().any(|why| why.starts_with("shader ")), "{recorded:?}");
    assert!(kept.iter().any(|why| why.starts_with("shader ")), "{kept:?}");
}

#[test]
fn transparency_masks_keep_mip_zero_alpha_one_and_shared_texture_identity() {
    content_reader_script::test_game::enter();
    let mut image = tga(4, 1);
    image[17] = 0x28;
    for (pixel, alpha) in [0, 1, 128, 255].into_iter().enumerate() {
        image[18 + pixel * 4 + 3] = alpha;
    }
    let (root, layers) = stack(
        "alpha",
        &[
            (
                "interface/alpha.gfx",
                br#"spriteTypes = {
            spriteType = { name = alpha textureFile = "gfx/alpha.tga" transparencecheck = yes }
            spriteType = { name = alias textureFile = "gfx/alpha.tga" transparencecheck = yes }
            spriteType = { name = missing textureFile = "gfx/missing.tga" transparencecheck = yes }
        }"#
                .to_vec(),
            ),
            ("gfx/alpha.tga", image),
        ],
    );
    let assets = Assets {
        sprites: [b"alpha".to_vec(), b"alias".to_vec(), b"missing".to_vec()].into(),
        ..Assets::default()
    };
    let (resources, _) = loaded(&layers, b"l_english", &assets);
    let alpha = resources.sprite_alpha(b"alpha").unwrap();
    assert_eq!(&*alpha.transparent, &[1, 0, 0, 0]);
    assert_eq!(alpha.width, 4);
    assert!(Arc::ptr_eq(
        &alpha.transparent,
        &resources.sprite_alpha(b"alias").unwrap().transparent
    ));
    assert_eq!(resources.sprite(b"missing").unwrap().hit, Hit::Through);
    assert!(resources.sprite_alpha(b"missing").is_none());
    std::fs::remove_dir_all(root).unwrap();
}
