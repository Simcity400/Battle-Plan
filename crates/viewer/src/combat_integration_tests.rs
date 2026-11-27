use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_gui::gui::native::resources::Registry;
use content_reader_image::native::Decode;
use std::time::Duration;
use std::time::Instant;
use std::{collections::BTreeMap, sync::Arc};
use viewer_engagements::combat_selection::Selection;
use viewer_native_gui::{
    gui_resources::GuiAssets,
    lazy_icons::{LazyTextures, flag_set},
    native_gui::{self, Resources},
    native_resources::GuiResources,
    native_windows::NativeWindows,
    sprites::{Batch, CHILD_SLOTS, Interface, NO_TEXTURE},
    tooltip_painter,
};
use viewer_runtime::test_support::image::tga;
use {viewer_combat::Assets, viewer_combat::Window, viewer_combat::window_layer};
use {
    viewer_combat::test_support::Bindings, viewer_combat::test_support::CombatGuiAssets,
    viewer_combat::test_support::CombatGuiResources, viewer_combat::test_support::GuiWindows,
    viewer_combat::test_support::NULL_PICTURE, viewer_combat::test_support::WINDOW_TEMPLATES,
    viewer_combat::test_support::bind_authored, viewer_combat::test_support::bind_sprite,
};

#[test]
fn combat_inline_air_and_close_tooltips_use_the_runtime_resources() {
    viewer_foundation::test_support::enter_fixed_modifier_tables();
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
            r#"instantTextBoxType = { name = "fixture_text" font = "fixture_font" }
                buttonType = { name = "fixture_missing" shortcut = "FIXTURE_MISSING" }
                buttonType = { name = "fixture_texture" shortcut = "FIXTURE_TEXTURE" }"#
        } else if name == b"navalcombatview" {
            r#"size = { width = 80 height = 50 }
                containerWindowType = { name = header_container size = { width = 80 height = 50 }
                    buttonType = { name = close spriteType = GFX_fixture_substituted
                        buttonFont = fixture_font position = { x = 10 y = 10 }
                        shortcut = "ESCAPE" } }"#
        } else {
            ""
        };
        declaration.push_str(&format!(
            "containerWindowType = {{ name = \"{}\" {text} }}\n",
            std::str::from_utf8(name).unwrap()
        ));
    }
    declaration.push_str(
        r#"containerWindowType = { name = "victory_point_mapicon" size = { width = 20 height = 10 }
            iconType = { name = icon spriteType = GFX_fixture_late }
            instantTextBoxType = { name = text font = fixture_font maxWidth = 18 text = A }
        }
        textBoxType = { name = "ToolTip" font = "fixture_font"
            maxWidth = 200 borderSize = { x = 1 y = 1 } } }"#,
    );
    for (path, bytes) in [
        ("interface/fixture.gui", declaration.into_bytes()),
        ("interface/fixture.gfx", br#"spriteTypes = {
            spriteType = { name = "GFX_fixture_substituted" textureFile = "gfx/icon.tga" }
            spriteType = { name = "GFX_fixture_portrait" textureFile = "gfx/leaders/fixture.tga" }
            spriteType = { name = "GFX_fixture_late" textureFile = "gfx/late.tga" }
            spriteType = { name = "GFX_ESCAPE" textureFile = "gfx/icon.tga" }
            spriteType = { name = "GFX_FIXTURE_TEXTURE" textureFile = "gfx/missing.tga" }
            spriteType = { name = "GFX_navalcombat_air_icon_fixture_bomber" textureFile = "gfx/icon.tga" noOfFrames = 2 }
            spriteType = { name = "GFX_navalcombat_air_icon_fixture_fighter" textureFile = "gfx/icon.tga" noOfFrames = 2 }
        }
        bitmapfonts = { bitmapfont = { name = "fixture_font" path = "gfx/fonts/fixture" } }"#.to_vec()),
        ("map/default.map", br#"definitions = "x" provinces = "x" terrain = "x"
            rivers = "x" heightmap = "x" tree_definition = "x""#.to_vec()),
        ("common/terrain/fixture.txt", b"categories = { fixture = {} }".to_vec()),
        ("common/combat_tactics.txt", Vec::new()),
        ("localisation/english/fixture.yml", "\u{feff}l_english:\n fixture_unlisted_modifier:0 \"A £$fixture_icon_name$\"\n fixture_late_key:0 \"£fixture_late\"\n fixture_icon_name:0 \"fixture_substituted\"\n SHORTCUT:0 \"Shortcut: $KEY$\"\n".as_bytes().to_vec()),
        ("gfx/icon.tga", tga(4, 4)),
        ("gfx/late.tga", tga(3, 7)),
        ("gfx/leaders/fixture.tga", tga(8, 12)),
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
    let overlay = root.join("mod");
    let mut null_tactic = vec![0_u8; 128 + 6 * 4 * 4];
    null_tactic[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [(4, 124_u32), (12, 4), (16, 6), (28, 1), (76, 32), (88, 32)] {
        null_tactic[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (path, bytes) in [
        ("gfx/leaders/fixture.tga", tga(14, 20)),
        ("gfx/interface/landcombat/tactics/.dds", null_tactic),
    ] {
        let file = overlay.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let shared =
        Arc::new(ResolvedLayers::from_folders(&[(&root, &[][..]), (&overlay, &[][..])]).unwrap());
    let windows = NativeWindows::load_at(&shared, [640, 480]).unwrap();
    let registry = Registry::load(&shared).unwrap();
    let interface = Interface {
        sprites: content_reader_gfx::gfx::SpriteSet::new(&registry.files),
        registry: Arc::new(registry),
        decode: Decode::default(),
    };
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
    for name in [
        "GFX_navalcombat_air_icon_fixture_bomber",
        "GFX_navalcombat_air_icon_fixture_fighter",
    ] {
        assert_eq!(combat.sprite_size(name), Some([2.0, 4.0]), "{name}");
        assert!(combat.gui.sheet(name.as_bytes()).is_some(), "{name}");
    }
    let localisation = content_reader_localisation::localisation::read(&shared, "english").unwrap();
    let (extra, _) = combat.localisation_assets(&localisation);
    let warnings = combat.defer_window_art(&shared, &localisation, &extra, &lazy);
    assert_eq!(warnings.iter().filter(|note| note.contains("GFX_FIXTURE_MISSING")).count(), 1);
    assert!(!combat.gui.loading_window_art());
    let misses = combat.gui.window_art_misses();
    std::thread::scope(|threads| {
        threads.spawn(|| assert!(combat.gui.sheet(b"GFX_fixture_late").is_none())).join().unwrap();
    });
    assert_eq!(combat.gui.window_art_misses(), misses, "a background request cannot invalidate this draw");
    assert!(combat.gui.loading_window_art());
    assert!(combat.gui.sprite(b"GFX_FIXTURE_TEXTURE").is_none());
    assert!(lazy.settle());
    assert!(!combat.gui.loading_window_art());
    assert_eq!(combat.gui.sheet(b"GFX_fixture_late").unwrap().size, [3, 7]);
    assert!(combat.gui.sprite(b"GFX_FIXTURE_TEXTURE").unwrap().missing.is_some());
    assert_eq!(combat.gui.window_warnings().iter().filter(|note| note.contains("gfx/missing.tga")).count(), 1);
    let map_assets = viewer_counters::counters::gui_assets(
        &windows,
        &interface.registry,
        &localisation,
        extra,
    );
    let mut map_gui = GuiResources::new(
        Arc::clone(&interface.registry),
        b"l_english",
        [10, 10],
        Decode::default(),
    );
    assert!(std::ptr::eq(
        combat.gui.registry().sprite(b"GFX_fixture_late").unwrap(),
        interface.registry.sprite(b"GFX_fixture_late").unwrap(),
    ));
    map_gui.load(&shared, &native_gui::Assets::default(), &mut images).unwrap();
    map_gui.defer_window_art(&shared, &map_assets, &lazy);
    assert!(map_gui.loaded_sheet(b"GFX_ESCAPE").is_none());
    let map_resources = GuiAssets {
        gui: &map_gui,
        windows: &windows,
        localisation: &localisation,
        flags: None,
        physical_viewport: None,
        portrait_textures: None,
    };
    let label = windows.gui_type(b"victory_point_mapicon").unwrap();
    let make_label =
        || native_gui::lifecycle::Instance::construct(label, [640, 480], &map_resources);
    assert!(
        matches!(make_label(), Err(why) if why.contains("loading")),
        "a pending label sprite or font must not be retained as a failed control"
    );
    assert!(lazy.settle());
    assert!(map_gui.loaded_sheet(b"GFX_fixture_late").is_some());
    assert!(
        map_gui.loaded_font(b"fixture_font").is_some(),
        "the label's font should be ready after its request: {:?}",
        map_gui.window_warnings()
    );
    let mut instance = make_label().unwrap();
    instance.gui_show(label, label).unwrap();
    let label_layout = instance.layout(label, &map_resources, None).unwrap().unwrap();
    assert!(!label_layout.parts.is_empty());
    let _art = viewer_native_gui::native_resources::SynchronousWindowArt::enter();
    let inventory = CombatGuiAssets::new(&combat, &Bindings::default());
    let resources =
        CombatGuiResources::new(&combat, &windows, &localisation, &flags, &flags, &inventory);
    let text = localisation.localise(b"fixture_unlisted_modifier").unwrap();
    let defines = Defines::new().unwrap();
    defines
        .load(
            "fixture",
            b"NDefines = { NInterface = {
        TOOLTIP_SCREEN_LEFT_OFFSET_X = 0, TOOLTIP_SCREEN_TOP_OFFSET_Y = 0,
        TOOLTIP_SCREEN_RIGHT_OFFSET_X = 0, TOOLTIP_SCREEN_BOTTOM_OFFSET_Y = 0
    }, NGraphics = { TOOLTIP_SHOW_DELAY = 0.04, TOOLTIP_DELAYED_DELAY = 0, TOOLTIP_HIDE_DELAY = 0 } }",
        )
        .unwrap();
    let mut batch = Batch::default();
    let refused = tooltip_painter::draw(
        &resources,
        &mut batch,
        &text,
        tooltip_painter::TooltipPosition {
            point: [0.0; 2],
            anchor: None,
        },
        [640.0, 480.0],
        &tooltip_painter::TooltipDefines::read(&defines).unwrap(),
        0.0,
    );
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

    let mut bindings = Bindings::default();
    bind_authored(
        &mut bindings,
        &combat,
        &localisation,
        &windows,
        Selection::Naval(7),
    );
    assert!(bindings.refused().is_empty());
    let mut gui = GuiWindows::default();
    let drawn = gui
        .draw(
            &windows,
            &resources,
            &bindings,
            1,
            Selection::Naval(7),
            [0.0; 2],
            false,
            &mut None,
            [640.0, 480.0],
            true,
            native_gui::lifecycle::Tick { time: 0.0, dt: 0.0 },
            false,
            false,
            None,
        )
        .unwrap();
    assert!(drawn.refused.is_empty(), "{:?}", drawn.refused);
    let defines = tooltip_painter::TooltipDefines::read(&defines).unwrap();
    let mut window = Window::default();
    let start = Instant::now();
    let point = Some([12.0, 12.0]);
    assert!(
        window
            .tooltip(Some(&drawn), Some(&defines), point, None, start)
            .is_none()
    );
    assert_eq!(
        window.tooltip_repaint_after(Some(&defines), start),
        Some(Duration::from_secs_f32(defines.timing[0]) + Duration::from_nanos(1),)
    );
    let (text, position) = window
        .tooltip(
            Some(&drawn),
            Some(&defines),
            point,
            None,
            start + Duration::from_secs(1),
        )
        .expect("the close button shows its shortcut after the normal delay");
    assert_eq!(text, "Shortcut: \u{13}ESCAPE");
    let mut batch = Batch::default();
    let refused = tooltip_painter::draw(
        &map_resources,
        &mut batch,
        text,
        position,
        [640.0, 480.0],
        &defines,
        0.0,
    );
    assert!(refused.is_empty(), "{refused:?}");
    let keycap = map_gui.sheet(b"GFX_ESCAPE").unwrap().texture;
    assert!(
        batch
            .quads
            .iter()
            .filter_map(|quad| quad.exact.as_ref()?.native.as_ref()?.bake.as_ref())
            .flat_map(|bake| &bake.draws)
            .any(|draw| draw.textures.contains(&(0, Some(keycap)))),
        "the shared tooltip painter must draw the close shortcut keycap"
    );

    let mut bindings = Bindings::default();
    bind_sprite(
        &mut bindings.values,
        "attacker_leader",
        "GFX_fixture_portrait".into(),
    );
    let mut inventory = CombatGuiAssets::new(&combat, &bindings);
    inventory.refresh_portraits(&combat, 0.0, lazy.version());
    assert!(
        combat
            .native_sprite(b"GFX_fixture_portrait")
            .unwrap()
            .missing
            .is_some()
    );
    assert!(lazy.settle());
    inventory.refresh_portraits(&combat, 0.0, lazy.version());
    let resources =
        CombatGuiResources::new(&combat, &windows, &localisation, &flags, &flags, &inventory);
    use viewer_native_gui::native_gui::Sheets;
    let portrait = resources.sheet(b"GFX_fixture_portrait").unwrap();
    assert_ne!(portrait.texture, NO_TEXTURE);
    assert_eq!(
        portrait.size,
        [14, 20],
        "the recorded mod overlay supplies the portrait"
    );
    let null_picture = combat.gui.sheet(NULL_PICTURE.as_bytes()).unwrap();
    assert_ne!(null_picture.texture, NO_TEXTURE);
    assert_eq!(null_picture.size, [6, 4]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_combat_window_sorts_above_every_map_icon() {
    use viewer_counter_kit::CounterDefines;
    use viewer_foundation::mode_visibility::{PriorityIcon, PriorityTable};
    use viewer_native_gui::sprites::layer;
    let top_icon = layer(i8::MAX, u32::MAX, CHILD_SLOTS - 1);
    assert!(window_layer(0) > top_icon);
    assert!(window_layer(1) > window_layer(0));
    assert!(window_layer(usize::MAX) >= window_layer(70_000));
    let icon = layer(
        CounterDefines::FIXTURE.priorities.get(
            PriorityTable::Default,
            PriorityIcon::LandCombat,
        ),
        9,
        3,
    );
    let mut keys = vec![window_layer(3), icon, window_layer(0)];
    keys.sort();
    assert_eq!(keys, [icon, window_layer(0), window_layer(3)]);
}
