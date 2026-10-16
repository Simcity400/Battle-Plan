use super::*;
use installed_content::image::native::Decode;
use viewer_foundation::map::lazy_icons::flag_set;
use viewer_foundation::map::sprites::Interface;
use viewer_foundation::map::test_support::gui as tests;
use viewer_foundation::map::test_support::image::tga;
use viewer_foundation::map::test_support::unit_types::subunits as synthetic_subunits;

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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

#[test]
fn combat_shortcut_assets_request_the_first_keycap_without_localisation() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let windows = tests::windows(
        br#"guiTypes={containerWindowType={name=landcombatview
            containerWindowType={name=header
                buttonType={name=close shortcut=ESCAPE shortcut=F12}}}}"#,
    );
    let assets = tooltip_assets(&windows, &Default::default());
    assert!(assets.sprites.contains(b"GFX_ESCAPE".as_slice()));
    assert!(!assets.sprites.contains(b"GFX_F12".as_slice()));
}

#[test]
fn combat_fonts_select_the_native_language_family() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    let interface = Interface {
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
    let flags = flag_set(
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_resources_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx")).unwrap();
    // The native resolver sniffs bytes and prefers the same stem's `.dds` file.
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    let warnings = sprite_warnings(&gui, &assets.sprites, true);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("1 undefined_read"), "{warnings:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_resources_preserve_producer_dimensions_and_named_refusals() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_unusable_sheets_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx/interface")).unwrap();
    std::fs::write(root.join("gfx/one.tga"), tga(10, 4)).unwrap();
    std::fs::write(root.join("gfx/interface/covered.tga"), tga(10, 4)).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let read = installed_content::gui::native::sprites::read_sprites(
        br#"spriteTypes = {
                spriteType = { name = "GFX_negative" textureFile = "gfx/one.tga" noOfFrames = -2 }
                spriteType = { name = "GFX_covered" textureFile = "gfx/interface/covered.tga" }
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
            b"GFX_covered".to_vec(),
        ]
        .into(),
        ..native_gui::Assets::default()
    };
    gui.load(&layers, &assets, &mut Vec::new()).unwrap();
    // Only the read outside coverage is named; the covered and the missing texture are not.
    assert_eq!(
        gui.uncovered_note("Combat UI").as_deref(),
        Some(
            "Combat UI: 1 font/texture file(s) are outside the content coverage policy and are not hash-verified: gfx/one.tga."
        )
    );
    assert_eq!(
        prepared_size(&gui, b"GFX_resized", &gui.sprite(b"GFX_resized").unwrap()),
        Some([40.0, 20.0])
    );
    for (name, kind) in [
        (b"GFX_negative".as_slice(), "negative_frame_count"),
        (b"GFX_missing", "missing_texture"),
    ] {
        assert!(prepared_size(&gui, name, &gui.sprite(name).unwrap()).is_none());
        let warnings = sprite_warnings(&gui, &[name.to_vec()].into(), true);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains(&format!("1 {kind}")), "{warnings:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn content_preload_uses_the_same_native_glyph_names_and_type_masks() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (assets, refusals) = unit_assets(&synthetic_subunits());
    assert!(refusals.is_empty());
    assert!(
        assets
            .sprites
            .contains(b"GFX_unit_Foot_icon_small".as_slice())
    );
    assert!(
        assets
            .sprites
            .contains(b"GFX_unit_Horse_icon_small".as_slice())
    );
    assert!(
        assets
            .sprites
            .contains(b"GFX_ship_Boat_icon_small".as_slice())
    );
    assert!(
        !assets
            .sprites
            .contains(b"GFX_unit_Boat_icon_small".as_slice())
    );
}
