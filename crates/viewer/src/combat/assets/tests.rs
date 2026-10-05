use super::*;
use installed_content::image::native::Decode;

#[test]
fn custom_sea_declarations_follow_effective_strategic_templates() {
    let root = std::env::temp_dir().join(format!("hoi4_custom_sea_{}", std::process::id()));
    let directory = root.join("map/strategicregions");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("a.txt"),
            b"strategic_region = { id = 1 name = ONE provinces = { 1 } static_modifiers = { x = y } }\nstrategic_region = { id = 2 name = TWO provinces = { 2 } static_modifiers = { x = y } }").unwrap();
    std::fs::write(
        directory.join("z.txt"),
        b"strategic_region = { id = 1 name = REPLACED provinces = { 1 } }",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let regions = installed_content::regions::read(&layers).unwrap();
    assert_eq!(
        regions.regions.get(&1).unwrap().static_modifier_names,
        Vec::<String>::new()
    );
    assert_eq!(
        regions.regions.get(&2).unwrap().static_modifier_names,
        ["x"]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn combat_native_assets_follow_global_templates_and_scrollbars() {
    let windows = NativeWindows::of(gui::native::Guis {
        files: vec![
            (
                "first.gui".into(),
                gui::native::read(
                    br#"guiTypes = {
                        containerWindowType = { name = "unit_counter_navy" }
                        containerWindowType = { name = "naval_combat_mapicon" }
                        containerWindowType = { name = "landcombatview"
                            iconType = { name = "old" spriteType = "GFX_old" }
                        }
                    }"#,
                ),
            ),
            (
                "mod.gui".into(),
                gui::native::read(
                    br#"@art = "GFX_mod"
                    guiTypes = {
                        containerWindowType = { name = "landcombatview"
                            verticalScrollbar = "mod_scroll"
                            iconType = { name = "new" spriteType = @art }
                        }
                        extendedScrollbarType = { name = "mod_scroll"
                            slider = { name = "s" spriteType = "GFX_mod_slider" }
                        }
                    }"#,
                ),
            ),
        ],
    })
    .unwrap();
    let assets = native_window_assets(&windows, &Default::default());
    assert!(assets.sprites.contains(b"GFX_mod".as_slice()));
    assert!(assets.sprites.contains(b"GFX_mod_slider".as_slice()));
    assert!(!assets.sprites.contains(b"GFX_old".as_slice()));
}

fn tga(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&width.to_le_bytes());
    bytes[14..16].copy_from_slice(&height.to_le_bytes());
    bytes[16] = 32;
    bytes[17] = 8;
    bytes.resize(18 + usize::from(width) * usize::from(height) * 4, 255);
    bytes
}

#[test]
fn combat_inline_sprite_substitution_renders_through_loaded_inventory() {
    use crate::combat::{binding::Bindings, gui_resources, tooltip_painter};
    use crate::map::{native_gui::Resources, sprites::Batch};

    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_inline_sprite_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let mut declaration = String::from("guiTypes = {");
    for name in WINDOW_TEMPLATES
        .iter()
        .copied()
        .chain([b"unit_counter_navy".as_slice(), b"naval_combat_mapicon"])
    {
        let text = if name == b"landcombatview" {
            r#"instantTextBoxType = { name = "fixture_text" font = "fixture_font" }"#
        } else {
            ""
        };
        declaration.push_str(&format!(
            "containerWindowType = {{ name = \"{}\" {text} }}\n",
            std::str::from_utf8(name).unwrap()
        ));
    }
    declaration.push_str(
        r#"textBoxType = { name = "ToolTip" font = "fixture_font"
            maxWidth = 200 borderSize = { x = 1 y = 1 } } }"#,
    );
    for (path, bytes) in [
        ("interface/fixture.gui", declaration.into_bytes()),
        ("interface/fixture.gfx", br#"spriteTypes = {
            spriteType = { name = "GFX_fixture_substituted" textureFile = "gfx/icon.tga" }
        }
        bitmapfonts = { bitmapfont = { name = "fixture_font" path = "gfx/fonts/fixture" } }"#.to_vec()),
        ("map/default.map", br#"definitions = "x" provinces = "x" terrain = "x"
            rivers = "x" heightmap = "x" tree_definition = "x""#.to_vec()),
        ("common/terrain/fixture.txt", b"categories = { fixture = {} }".to_vec()),
        ("common/combat_tactics.txt", Vec::new()),
        ("localisation/english/fixture.yml", "\u{feff}l_english:\n fixture_unlisted_modifier:0 \"A £$fixture_icon_name$\"\n fixture_icon_name:0 \"fixture_substituted\"\n".as_bytes().to_vec()),
        ("gfx/icon.tga", tga(4, 4)),
        ("gfx/fonts/fixture.tga", tga(2, 2)),
        ("gfx/fonts/fixture.fnt", b"info face=\"fixture\" size=4\ncommon lineHeight=4 base=3 scaleW=2 scaleH=2 pages=1\npage id=0 file=\"fixture.tga\"\nchars count=1\nchar id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0 chnl=15\n".to_vec()),
        ("gfx/FX/defines_hlsl_dx11.fxh", Vec::new()),
        ("gfx/FX/text.shader", b"Effect Text { }".to_vec()),
        ("gfx/FX/buttonstate.shader", b"Effect Up { }".to_vec()),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let shared = Arc::new(ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap());
    let windows = NativeWindows::load_at(&shared, [640, 480]).unwrap();
    let registry = Registry::load(&shared).unwrap();
    let interface = crate::map::sprites::Interface {
        sprites: installed_content::gfx::SpriteSet::new(&registry.files),
        registry,
        decode: Decode::default(),
    };
    let lazy = LazyTextures::new();
    let flags = crate::map::counters::flag_set(
        &shared,
        installed_content::flags::FlagSet::Normal,
        [1, 1],
        |_| {},
        &lazy,
    )
    .unwrap();
    let mut images = Vec::new();
    let mut combat = Assets::load(
        &shared,
        &BTreeMap::new(),
        &Defines::new().unwrap(),
        &interface,
        &flags,
        ("english", [10, 10], &windows),
        &mut images,
        &lazy,
    )
    .unwrap();
    let localisation = installed_content::localisation::read(&shared, "english").unwrap();
    // The same inventory and load calls MapIcons::load_gui uses; no test-only merge.
    let (extra, _) = combat.localisation_assets(&localisation);
    combat
        .load_gui(&shared, &localisation, &extra, &mut images)
        .unwrap();
    let inventory = gui_resources::GuiAssets::new(&combat, &Bindings::default());
    let resources = gui_resources::GuiResources::new(
        &combat,
        &windows,
        &localisation,
        &flags,
        &flags,
        &inventory,
    );
    let text = localisation.localise(b"fixture_unlisted_modifier").unwrap();
    let defines = Defines::new().unwrap();
    defines
        .load(
            "fixture",
            b"NDefines = { NInterface = {
        TOOLTIP_SCREEN_LEFT_OFFSET_X = 0, TOOLTIP_SCREEN_TOP_OFFSET_Y = 0,
        TOOLTIP_SCREEN_RIGHT_OFFSET_X = 0, TOOLTIP_SCREEN_BOTTOM_OFFSET_Y = 0
    }, NGraphics = { TOOLTIP_SHOW_DELAY = 0, TOOLTIP_DELAYED_DELAY = 0, TOOLTIP_HIDE_DELAY = 0 } }",
        )
        .unwrap();
    let mut batch = Batch::default();
    let refused = tooltip_painter::draw(
        &resources,
        &mut batch,
        &text,
        [0.0; 2],
        [640.0, 480.0],
        &tooltip_painter::TooltipDefines::read(&defines).unwrap(),
        0.0,
    );
    std::fs::remove_dir_all(root).unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    assert!(!batch.is_empty(), "the combat popup must draw");
    assert!(resources.sprite(b"GFX_fixture_substituted").is_some());
    let icon = combat
        .gui
        .sheet(b"GFX_fixture_substituted")
        .unwrap()
        .texture;
    assert!(
        batch
            .quads
            .iter()
            .filter_map(|quad| quad.exact.as_ref()?.native.as_ref()?.bake.as_ref())
            .flat_map(|bake| &bake.draws)
            .any(|draw| draw.textures.contains(&(0, Some(icon)))),
        "the popup bake must draw its substituted inline sprite"
    );
}

#[test]
fn combat_fonts_select_the_native_language_family() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_font_language_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    for directory in ["interface", "gfx/fonts", "map", "common/terrain"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    std::fs::write(
        root.join("map/default.map"),
        br#"definitions = "x" provinces = "x"
            terrain = "x" rivers = "x" heightmap = "x" tree_definition = "x""#,
    )
    .unwrap();
    std::fs::write(
        root.join("common/terrain/fixture.txt"),
        b"categories = { fixture = {} }",
    )
    .unwrap();
    std::fs::write(root.join("common/combat_tactics.txt"), b"").unwrap();
    std::fs::write(
        root.join("interface/fonts.gfx"),
        br#"bitmapfonts = {
            bitmapfont = { name = "fixture_font" path = "gfx/fonts/base" }
            bitmapfont_override = { name = "fixture_font" fontfiles = { "gfx/fonts/native" }
                languages = { "l_fixture" } }
        }"#,
    )
    .unwrap();
    for (name, height) in [("base", 11), ("native", 23)] {
        std::fs::write(root.join(format!("gfx/fonts/{name}.fnt")), format!(
                "info face=\"fixture\" size={height}\ncommon lineHeight={height} base=5 scaleW=2 scaleH=2 pages=1\n\
                 page id=0 file=\"x.tga\"\nchars count=1\n\
                 char id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0 chnl=15\n"
            )).unwrap();
        std::fs::write(root.join(format!("gfx/fonts/{name}.tga")), tga(2, 2)).unwrap();
    }
    let shared = Arc::new(ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap());
    let registry = Registry::load(&shared).unwrap();
    let interface = crate::map::sprites::Interface {
        sprites: installed_content::gfx::SpriteSet::new(&registry.files),
        registry,
        decode: Decode::default(),
    };
    let mut declaration = String::from("guiTypes = {");
    for name in WINDOW_TEMPLATES
        .iter()
        .copied()
        .chain([b"unit_counter_navy".as_slice(), b"naval_combat_mapicon"])
    {
        let text = if name == b"landcombatview" {
            r#"instantTextBoxType = { name = "fixture_text" font = "fixture_font" }"#
        } else {
            ""
        };
        declaration.push_str(&format!(
            "containerWindowType = {{ name = \"{}\" {text} }}\n",
            std::str::from_utf8(name).unwrap()
        ));
    }
    declaration.push('}');
    let windows = NativeWindows::read(declaration.as_bytes(), [640, 480]).unwrap();
    let lazy = LazyTextures::new();
    let flags = crate::map::counters::flag_set(
        &shared,
        installed_content::flags::FlagSet::Normal,
        [1, 1],
        |_| {},
        &lazy,
    )
    .unwrap();
    let assets = Assets::load(
        &shared,
        &BTreeMap::new(),
        &Defines::new().unwrap(),
        &interface,
        &flags,
        ("fixture", [10, 10], &windows),
        &mut Vec::new(),
        &lazy,
    )
    .unwrap();
    assert_eq!(
        assets.gui.font(b"fixture_font").unwrap().glyphs.line_height,
        Some(23)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_resources_use_native_texture_resolution_and_shared_pages() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_resources_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx")).unwrap();
    // The native resolver sniffs bytes and prefers the same stem's .dds.
    std::fs::write(root.join("gfx/portrait.tga"), tga(8, 6)).unwrap();
    std::fs::write(root.join("gfx/portrait.dds"), tga(10, 4)).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let read = Sprites {
        sprites: vec![
            texture_sprite("combat_portrait:test".into(), "gfx/portrait.tga".into()),
            texture_sprite("combat_tactic:test".into(), "gfx/portrait.tga".into()),
        ],
        ..Sprites::default()
    };
    let mut gui = GuiResources::new(
        Registry::from_files(vec![("runtime".into(), read)]),
        b"l_english",
        [10, 10],
        Decode::default(),
    );
    let assets = native_gui::Assets {
        sprites: [
            b"combat_portrait:test".to_vec(),
            b"combat_tactic:test".to_vec(),
        ]
        .into(),
        ..native_gui::Assets::default()
    };
    let mut images = Vec::new();
    gui.load(&layers, &assets, &mut images).unwrap();
    let portrait = gui.sprite(b"combat_portrait:test").unwrap();
    assert_eq!(portrait.texture, [10, 4]);
    assert_eq!(
        gui.sheet(b"combat_portrait:test").unwrap().texture,
        gui.sheet(b"combat_tactic:test").unwrap().texture
    );
    assert_eq!(images.len(), 1);
    assert_eq!(
        resources::stack_texture(&layers, b"gfx/portrait.tga", Decode::default())
            .unwrap()
            .unwrap()
            .image
            .width,
        10
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_resources_name_undefined_texture_reads() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_refused_texture_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx")).unwrap();
    // FreeImage's ICO sniff reads six bytes from this three-byte file.
    std::fs::write(root.join("gfx/portrait.dds"), [0, 0, 1]).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let read = Sprites {
        sprites: vec![texture_sprite(
            "combat_portrait:test".into(),
            "gfx/portrait.dds".into(),
        )],
        ..Sprites::default()
    };
    let mut gui = GuiResources::new(
        Registry::from_files(vec![("runtime".into(), read)]),
        b"l_english",
        [10, 10],
        Decode::default(),
    );
    let assets = native_gui::Assets {
        sprites: [b"combat_portrait:test".to_vec()].into(),
        ..native_gui::Assets::default()
    };
    gui.load(&layers, &assets, &mut Vec::new()).unwrap();
    let warnings = sprite_warnings(&gui, &assets.sprites);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("1 undefined_read"), "{warnings:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_resources_preserve_producer_dimensions_and_named_refusals() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_unusable_sheets_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx")).unwrap();
    std::fs::write(root.join("gfx/one.tga"), tga(10, 4)).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let read = installed_content::gui::native::sprites::read_sprites(
        br#"spriteTypes = {
                spriteType = { name = "GFX_negative" textureFile = "gfx/one.tga" noOfFrames = -2 }
                progressBarType = { name = "GFX_missing" textureFile1 = "gfx/one.tga"
                    textureFile2 = "gfx/missing.tga" size = { x = 10 y = 4 } }
                resizeableSpriteType = { name = "GFX_resized" textureFile = "gfx/one.tga"
                    size = { x = 40 y = 20 } }
            }"#,
    );
    let mut gui = GuiResources::new(
        Registry::from_files(vec![("runtime".into(), read)]),
        b"l_english",
        [10, 10],
        Decode::default(),
    );
    let assets = native_gui::Assets {
        sprites: [
            b"GFX_negative".to_vec(),
            b"GFX_missing".to_vec(),
            b"GFX_resized".to_vec(),
        ]
        .into(),
        ..native_gui::Assets::default()
    };
    gui.load(&layers, &assets, &mut Vec::new()).unwrap();
    assert_eq!(
        prepared_size(&gui, b"GFX_resized", &gui.sprite(b"GFX_resized").unwrap()),
        Some([40.0, 20.0])
    );
    for (name, kind) in [
        (b"GFX_negative".as_slice(), "negative_frame_count"),
        (b"GFX_missing", "missing_texture"),
    ] {
        assert!(prepared_size(&gui, name, &gui.sprite(name).unwrap()).is_none());
        let warnings = sprite_warnings(&gui, &[name.to_vec()].into());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains(&format!("1 {kind}")), "{warnings:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
