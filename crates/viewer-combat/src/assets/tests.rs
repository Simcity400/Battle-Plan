use super::*;
use content_reader_gui::gui::native::sprites::Sprites;
use content_reader_image::native::Decode;
use viewer_native_gui::lazy_icons::flag_set;
use viewer_native_gui::sprites::Interface;
use viewer_native_gui::test_support::gui as tests;
use viewer_runtime::test_support::image::tga;
use viewer_foundation::test_support::unit_types::subunits as synthetic_subunits;

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
    let regions = content_reader_regions::regions::read(&layers).unwrap();
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
    content_reader_script::test_game::enter();
    let windows = NativeWindows::of(gui::native::Guis::new(vec![
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
    ]))
    .unwrap();
    let assets = native_window_assets(&windows, &Default::default());
    assert!(assets.sprites.contains(b"GFX_mod".as_slice()));
    assert!(assets.sprites.contains(b"GFX_mod_slider".as_slice()));
    assert!(!assets.sprites.contains(b"GFX_old".as_slice()));
}

#[test]
fn every_window_the_map_opens_loads_its_own_and_its_entries_sprites() {
    use viewer_engagements::combat_selection::Selection;
    content_reader_script::test_game::enter();
    let opened: Vec<&[u8]> = [
        Selection::Land(0),
        Selection::Naval(0),
        Selection::NavalResult(0),
        Selection::AirRegion(0),
        Selection::State(0),
        Selection::NavalRegion(0),
    ]
    .map(|selection| crate::gui::window_name(selection).as_bytes())
    .into_iter()
    .chain(crate::state_view::TEMPLATES.iter().copied())
    .chain(crate::naval_region_view::TEMPLATES.iter().copied())
    .collect();
    let sprite = |name: &[u8]| format!("GFX_own_{}", String::from_utf8_lossy(name));
    let mut declaration = String::from(
        "guiTypes = { containerWindowType = { name = unit_counter_navy } containerWindowType = { name = naval_combat_mapicon }",
    );
    for name in &opened {
        declaration.push_str(&format!(
            " containerWindowType = {{ name = {} iconType = {{ name = icon spriteType = {} }} }}",
            String::from_utf8_lossy(name),
            sprite(name)
        ));
    }
    declaration.push('}');
    let windows = NativeWindows::of(gui::native::Guis::new(vec![(
        "windows.gui".into(),
        gui::native::read(declaration.as_bytes()),
    )]))
    .unwrap();
    let assets = native_window_assets(&windows, &Default::default());
    for name in opened {
        assert!(
            assets.sprites.contains(sprite(name).as_bytes()),
            "{} is not loaded",
            sprite(name)
        );
    }
    assert!(assets.sprites.contains(b"GFX_onmap_victorypoints_strip".as_slice()));
}

#[test]
fn combat_shortcut_assets_request_the_first_keycap_without_localisation() {
    content_reader_script::test_game::enter();
    let windows = tests::windows(
        br#"guiTypes={containerWindowType={name=landcombatview
            containerWindowType={name=header
                buttonType={name=close shortcut=ESCAPE shortcut=F12}}}}"#,
    );
    let assets = tooltip_assets(&windows, &Default::default());
    assert!(assets.sprites.contains(b"GFX_ESCAPE".as_slice()));
    assert!(!assets.sprites.contains(b"GFX_F12".as_slice()));
}

pub(crate) fn synthetic_assets(
    name: &str,
    children: &str,
    write: impl FnOnce(&std::path::Path),
) -> (std::path::PathBuf, Assets, Arc<LazyTextures>) {
    use engine_executable::{executable_tables, modifier_keys};
    use std::sync::OnceLock;

    static TABLES: OnceLock<executable_tables::Tables> = OnceLock::new();
    let tables = *TABLES.get_or_init(|| {
        content_reader_script::test_game::executable()
            .pinned_modifiers([
                (modifier_keys::NAVAL_RETREAT_SPEED, "naval_retreat_speed"),
                (modifier_keys::SUB_RETREAT_SPEED, "sub_retreat_speed"),
                (
                    modifier_keys::NAVAL_RETREAT_SPEED_AFTER_INITIAL_COMBAT,
                    "naval_retreat_speed_after_initial_combat",
                ),
                (modifier_keys::CONVOY_RETREAT_SPEED, "convoy_retreat_speed"),
            ])
            .build()
    });
    executable_tables::enter(Some(tables));
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_assets_{name}_{}_{:?}",
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
    write(&root);
    let shared = Arc::new(ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap());
    let registry = Registry::load(&shared).unwrap();
    let interface = Interface {
        sprites: content_reader_gfx::gfx::SpriteSet::new(&registry.files),
        registry: Arc::new(registry),
        decode: Decode::default(),
    };
    let mut declaration = String::from("guiTypes = {");
    for name in WINDOW_TEMPLATES
        .iter()
        .copied()
        .chain([b"unit_counter_navy".as_slice(), b"naval_combat_mapicon"])
    {
        let text = if name == b"landcombatview" {
            &format!(
                r#"instantTextBoxType = {{ name = "fixture_text" font = "fixture_font" }} {children}"#
            )
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
    lazy.set_background(false);
    let flags = flag_set(
        &shared,
        content_reader_gui::flags::FlagSet::Normal,
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
    (root, assets, lazy)
}

#[test]
fn combat_fonts_select_the_native_language_family() {
    let (root, assets, _) = synthetic_assets("font_language", "", |_| {});
    assert_eq!(
        assets.gui.font(b"fixture_font").unwrap().glyphs.line_height,
        Some(23)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_portraits_publish_loaded_and_missing_resources() {
    use crate::{binding::Bindings, gui_resources};
    use viewer_native_gui::native_gui::Resources;

    let (root, assets, lazy) = synthetic_assets(
        "lazy_portrait_purity",
        r#"iconType = { name = static_icon spriteType = GFX_static }"#,
        |root| {
            std::fs::create_dir_all(root.join("gfx/leaders")).unwrap();
            std::fs::write(root.join("gfx/leaders/lazy.tga"), tga(8, 6)).unwrap();
            std::fs::write(root.join("gfx/static.tga"), tga(4, 2)).unwrap();
            std::fs::write(
                root.join("interface/portraits.gfx"),
                br#"spriteTypes = {
                    spriteType = { name = GFX_lazy textureFile = "gfx/leaders/lazy.tga" }
                    spriteType = { name = GFX_lazy_missing textureFile = "gfx/leaders/missing.tga" }
                    spriteType = { name = GFX_static textureFile = "gfx/static.tga" }
                }"#,
            )
            .unwrap();
        },
    );
    let inventory = gui_resources::GuiAssets::new(&assets, &Bindings::default());
    let localisation = Default::default();
    let resources = gui_resources::GuiResources::new(
        &assets,
        &assets.native_windows,
        &localisation,
        &assets.flags,
        &assets.flags,
        &inventory,
    );
    for name in [b"GFX_lazy".as_slice(), b"GFX_lazy_missing"] {
        assert!(
            assets
                .lazy_sprites
                .contains_key(std::str::from_utf8(name).unwrap())
        );
        assert!(resources.sprite_info(name).unwrap().missing.is_some());
    }
    assert_eq!(
        assets.portraits.peek("gfx/leaders/lazy.tga"),
        Lookup::Pending
    );
    let version = lazy.version();
    assert!(lazy.settle());
    assert!(lazy.version() > version);
    assert!(matches!(
        assets.portraits.peek("gfx/leaders/lazy.tga"),
        Lookup::Loaded { size: [8, 6], .. }
    ));
    assert_eq!(
        assets.portraits.peek("gfx/leaders/missing.tga"),
        Lookup::Failed
    );
    let loaded = resources.sprite_info(b"GFX_lazy").unwrap();
    assert_eq!(loaded.texture, [8, 6]);
    assert!(loaded.missing.is_none());
    assert!(
        resources
            .sprite_info(b"GFX_lazy_missing")
            .unwrap()
            .missing
            .is_some()
    );
    for name in [b"GFX_lazy".as_slice(), b"GFX_lazy_missing"] {
        assert!(
            assets
                .lazy_sprites
                .contains_key(std::str::from_utf8(name).unwrap())
        );
    }
    for name in [b"GFX_static".as_slice(), b"GFX_absent", b"\xff"] {
        assert!(
            std::str::from_utf8(name)
                .ok()
                .is_none_or(|name| !assets.lazy_sprites.contains_key(name))
        );
    }
    assert!(resources.sprite_info(b"GFX_static").is_some());
    assert!(resources.sprite_info(b"GFX_absent").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_resources_use_native_texture_resolution_and_shared_pages() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_resources_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx")).unwrap();
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
fn combat_resources_name_corrupt_textures() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_refused_texture_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx")).unwrap();
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
    assert!(warnings[0].contains("gfx/portrait.dds does not load"), "{warnings:?}");
    assert!(warnings[0].contains("1 missing_texture"), "{warnings:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_resources_preserve_producer_dimensions_and_named_refusals() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_combat_unusable_sheets_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx/interface")).unwrap();
    std::fs::write(root.join("gfx/one.tga"), tga(10, 4)).unwrap();
    std::fs::write(root.join("gfx/interface/covered.tga"), tga(10, 4)).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let read = content_reader_gui::gui::native::sprites::read_sprites(
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
    content_reader_script::test_game::enter();
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

#[test]
fn sprites_the_state_view_names_from_its_content_load_with_the_windows() {
    let (root, assets, _) = synthetic_assets("content_named", "", |root| {
        for directory in ["common/occupation_laws", "common/dynamic_modifiers", "gfx/bound"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        std::fs::write(root.join("common/occupation_laws/laws.txt"), b"bound_law = { icon = GFX_bound_law }").unwrap();
        std::fs::write(root.join("common/dynamic_modifiers/modifiers.txt"), b"bound_modifier = { icon = GFX_bound_modifier }").unwrap();
        std::fs::write(root.join("gfx/bound/one.tga"), tga(4, 4)).unwrap();
        std::fs::write(
            root.join("interface/bound.gfx"),
            br#"spriteTypes = {
                spriteType = { name = "GFX_bound_law" textureFile = "gfx/bound/one.tga" }
                spriteType = { name = "GFX_bound_modifier" textureFile = "gfx/bound/one.tga" }
                spriteType = { name = "GFX_unbound" textureFile = "gfx/bound/one.tga" }
            }"#,
        )
        .unwrap();
    });
    assert!(assets.lazy_sprites.contains_key("GFX_bound_law"));
    assert!(assets.lazy_sprites.contains_key("GFX_bound_modifier"));
    assert_eq!(assets.portraits.peek("gfx/bound/one.tga"), Lookup::Pending);
    assert!(assets.has_sprite("GFX_bound_law"));
    assert!(assets.has_sprite("GFX_bound_modifier"));
    assert!(!assets.has_sprite("GFX_unbound"));
    assert!(assets.warnings.iter().any(|warning| warning.contains("building special icons")), "{:?}", assets.warnings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn combat_dynamic_sprites_defer_until_drawn_or_queried_and_keep_alpha_hit_eager() {
    use crate::{binding::Bindings, gui_resources};

    let (root, assets, lazy) = synthetic_assets("dynamic_lazy", "", |root| {
        std::fs::create_dir_all(root.join("gfx/combat")).unwrap();
        std::fs::create_dir_all(root.join("gfx/interface/landcombat/tactics")).unwrap();
        std::fs::write(root.join("gfx/combat/ship.tga"), tga(12, 8)).unwrap();
        std::fs::write(root.join("gfx/combat/cold.tga"), tga(10, 6)).unwrap();
        std::fs::write(root.join("gfx/combat/alpha.tga"), tga(6, 6)).unwrap();
        std::fs::write(
            root.join("gfx/interface/landcombat/tactics/fixture.tga"),
            tga(4, 4),
        )
        .unwrap();
        std::fs::write(
            root.join("common/combat_tactics.txt"),
            b"fixture = { picture = fixture }",
        )
        .unwrap();
        std::fs::write(
            root.join("interface/dynamic.gfx"),
            br#"spriteTypes = {
                spriteType = { name = "GFX_navalcombat_ship_icon_destroyer" textureFile = "gfx/combat/ship.tga" noOfFrames = 2 }
                spriteType = { name = "GFX_navalcombat_ship_icon_missing" textureFile = "gfx/combat/missing.tga" noOfFrames = 2 }
                spriteType = { name = "GFX_terrain_cold" textureFile = "gfx/combat/cold.tga" }
                spriteType = { name = "GFX_terrain_alpha" textureFile = "gfx/combat/alpha.tga" transparencecheck = yes }
                spriteType = { name = "GFX_mapmode_buttons_default_medium" textureFile = "gfx/combat/ship.tga" }
            }"#,
        )
        .unwrap();
    });
    assert!(assets.lazy_sprites.contains_key("GFX_navalcombat_ship_icon_destroyer"));
    assert!(assets.lazy_sprites.contains_key("GFX_navalcombat_ship_icon_missing"));
    assert!(assets.lazy_sprites.contains_key("GFX_terrain_cold"));
    assert!(!assets.lazy_sprites.contains_key("GFX_terrain_alpha"));
    assert!(!assets.lazy_sprites.contains_key("GFX_mapmode_buttons_default_medium"));
    assert!(!assets.lazy_sprites.contains_key("combat_tactic:fixture"));
    assert_eq!(assets.portraits.peek("gfx/combat/ship.tga"), Lookup::Pending);
    assert_eq!(assets.portraits.peek("gfx/combat/cold.tga"), Lookup::Pending);
    assert_eq!(assets.portraits.peek("gfx/combat/missing.tga"), Lookup::Pending);

    assert_eq!(assets.sprite_size("GFX_navalcombat_ship_icon_destroyer"), Some([6.0, 8.0]));
    assert_eq!(assets.sprite_size("GFX_navalcombat_ship_icon_missing"), None);
    assert!(assets.has_sprite("GFX_navalcombat_ship_icon_destroyer"));
    assert!(!assets.has_sprite("GFX_navalcombat_ship_icon_missing"));
    assert_eq!(
        assets.drawable_sprite(b"GFX_navalcombat_ship_icon_destroyer").map(|info| info.texture),
        Some([12, 8])
    );
    assert!(assets.drawable_sprite(b"GFX_navalcombat_ship_icon_missing").is_none());

    assert_eq!(assets.portraits.peek("gfx/combat/cold.tga"), Lookup::Pending);
    let mut bindings = Bindings::default();
    crate::binding::sprite(&mut bindings.values, "icon", "GFX_terrain_cold".into());
    assets.acquire_sprite("GFX_terrain_cold");
    let mut inventory = gui_resources::GuiAssets::new(&assets, &bindings);
    inventory.refresh_portraits(&assets, 0.0, lazy.version());
    let localisation = Default::default();
    let resources = gui_resources::GuiResources::new(
        &assets,
        &assets.native_windows,
        &localisation,
        &assets.flags,
        &assets.flags,
        &inventory,
    );
    let sheet = resources
        .sheet(b"GFX_terrain_cold")
        .expect("acquired lazy sprite draws on the first frame");
    assert_ne!(sheet.texture, NO_TEXTURE);
    assert_eq!((sheet.size, sheet.frames), ([10, 6], 1));
    std::fs::remove_dir_all(root).unwrap();
}
