use super::*;
use crate::map::{
    native_gui::lifecycle::{Instance, Tick},
    test_support::{Fixture, SYNTHETIC_NAVAL_GUI, SYNTHETIC_NAVY_GUI, VIEWPORT, part},
};
const STILL: Tick = Tick { time: 0.0, dt: 0.0 };

/// Inline icons appearing through localised button, text-box or nested-window text
/// are acquired with window assets. Lookup tries GFX_ plus the name, then the name.
/// Missing localisation keys show themselves.
#[test]
fn inline_icons_of_localised_authored_texts_are_acquired() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = SYNTHETIC_NAVAL_GUI.replacen(
        "buttonType = { name = \"bg_btn\"",
        "instantTextboxType = { name = \"note\" font = \"fixture_mono\" text = \"NOTE_KEY\" }\n\
         containerWindowType = { name = \"inner\" buttonType = { name = \"b\" \
         spriteType = \"GFX_x\" buttonText = \"BUTTON_KEY\" buttonFont = \"fixture_mono\" } }\n\
         buttonType = { name = \"bg_btn\"",
        1,
    );
    let windows = NativeWindows::read(
        format!("{SYNTHETIC_NAVY_GUI}\n{source}").as_bytes(),
        VIEWPORT,
    )
    .unwrap();
    let localisation: installed_content_localisation::localisation::Localisation = [
        ("NOTE_KEY".to_owned(), "a £note_icon b".to_owned()),
        ("BUTTON_KEY".to_owned(), "£button_icon".to_owned()),
        ("UNRELATED_KEY".to_owned(), "£unrelated_inline".to_owned()),
    ]
    .into_iter()
    .collect();
    let assets = windows.assets(&localisation);
    for name in [
        "GFX_note_icon",
        "note_icon",
        "GFX_button_icon",
        "button_icon",
    ] {
        assert!(assets.sprites.contains(name.as_bytes()), "{name}");
    }
    assert!(!assets.sprites.contains(b"GFX_unrelated_inline".as_slice()));
    // Without the entries nothing but the keys is shown, and no icon is acquired.
    let bare = windows.assets(&Default::default());
    assert!(!bare.sprites.contains(b"GFX_note_icon".as_slice()));
}

#[test]
fn raid_tooltip_regression_inventory_loads_runtime_selected_localisation() {
    let loc: installed_content_localisation::localisation::Localisation = [
        ("arbitrary_scripted_choice", "\u{a3}scripted|2"),
        ("ABCDE_project_narrative", "\u{a3}project"),
        ("cosmetic_country_name", "\u{a3}country"),
        ("arbitrary_substitution", "$inline_value$"),
        ("inline_value", "\u{a3}substituted"),
        ("unrelated_to_any_tooltip_key", "\u{a3}unrelated"),
    ]
    .map(|(key, value)| (key.into(), value.into()))
    .into_iter()
    .collect();
    let inventory = native_gui::Assets::from_localisation(&loc);
    for stem in ["scripted", "project", "country", "substituted", "unrelated"] {
        for name in [format!("GFX_{stem}"), stem.into()] {
            assert!(inventory.sprites.contains(name.as_bytes()), "{name}");
        }
    }
}

/// `unit_counter_navy` holding a `windowType` whose width is `max_resolution` and whose
/// background comes from its `parent`, as a mod may declare it.
const MAX_RESOLUTION_NAVY: &str = r#"guiTypes = {
    windowType = { name = "base" size = { x = 40 y = 20 } background = "GFX_base_bg" }
    containerWindowType = { name = "unit_counter_navy" size = { width = 66 height = 33 }
        buttonType = { name = "count_bg" quadTextureSprite = "GFX_fixture_navy_plate" frame = 1 }
        windowType = { name = "wide" parent = "base" position = { x = 1 y = 1 }
            size = { x = max_resolution y = 5 } } }
    containerWindowType = { name = "naval_combat_mapicon" size = { width = 33 height = 43 } } }"#;

/// The load-time read decides only the asset set, which no GUI viewport changes: the synthetic
/// windows and a `max_resolution` fixture read at two viewports acquire the same sprites,
/// fonts and textures.
#[test]
fn the_asset_set_does_not_depend_on_the_gui_viewport() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let localisation = installed_content_localisation::localisation::Localisation::default();
    for source in [
        format!(
            "{SYNTHETIC_NAVY_GUI}
{SYNTHETIC_NAVAL_GUI}"
        ),
        MAX_RESOLUTION_NAVY.to_owned(),
    ] {
        let at = |viewport| {
            let assets = NativeWindows::read(source.as_bytes(), viewport)
                .unwrap()
                .assets(&localisation);
            (assets.sprites, assets.fonts, assets.textures)
        };
        let wide = at([1920, 1080]);
        assert_eq!(wide, at([1, 614]));
        assert_eq!(wide, at([1092, 614]));
    }
}

/// A `max_resolution` window lays out under the viewer's GUI viewport: read, constructed and
/// laid out in `gui_viewport(size, ui_scale)` (1366 x 768 at 1.25 is 1092 x 614).
#[test]
fn a_max_resolution_window_lays_out_under_the_viewers_viewport() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let viewport = crate::map::native_shader::draw::gui_viewport([1366, 768], 1.25);
    assert_eq!(viewport, [1092, 614]);
    let viewport = viewport.map(|value| value as i32);
    let windows = NativeWindows::read(MAX_RESOLUTION_NAVY.as_bytes(), viewport).unwrap();
    let navy = windows.navy();
    let fixture = Fixture::new(
        &[
            ("GFX_fixture_navy_plate", 3, [246, 45]),
            ("GFX_base_bg", 1, [8, 8]),
        ],
        &[],
    );
    let mut instance = Instance::construct(navy, viewport, &fixture).unwrap();
    instance.gui_show(navy, navy).unwrap();
    instance.tick(navy, &fixture, STILL).unwrap();
    let layout = instance.layout(navy, &fixture, None).unwrap().unwrap();
    assert_eq!(layout.size, [66, 33]);
    assert!(part(&layout, "count_bg").is_some());
}

/// Intel, resource and adjacency windows load their authored sprites with the other
/// map-icon windows, so their backgrounds and glyphs draw.
#[test]
fn intel_and_resource_icon_windows_load_their_authored_sprites() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut source = format!("{SYNTHETIC_NAVY_GUI}\n{SYNTHETIC_NAVAL_GUI}");
    for (window, sprite) in [
        (INTEL_CONTAINER_WINDOW, "GFX_intel_container"),
        (INTEL_SHIELD_WINDOW, "GFX_intel_shield"),
        (INTEL_OPERATIVE_WINDOW, "GFX_intel_operative"),
        (INTEL_OPERATION_WINDOW, "GFX_intel_operation"),
        (INTEL_MORE_WINDOW, "GFX_intel_more"),
        (RESOURCE_WINDOW, "GFX_resource"),
        (RESOURCE_ITEM_WINDOW, "GFX_resource_item"),
        (ADJACENCY_RULE_WINDOW, "GFX_adjacency"),
    ] {
        source.push_str(&format!(
            "\nguiTypes = {{ containerWindowType = {{ name = \"{window}\" \
             iconType = {{ name = \"icon\" spriteType = \"{sprite}\" }} }} }}"
        ));
    }
    let windows = NativeWindows::read(source.as_bytes(), VIEWPORT).unwrap();
    let assets = windows.assets(&Default::default());
    for sprite in [
        "GFX_intel_container",
        "GFX_intel_shield",
        "GFX_intel_operative",
        "GFX_intel_operation",
        "GFX_intel_more",
        "GFX_resource",
        "GFX_resource_item",
        "GFX_adjacency",
    ] {
        assert!(assets.sprites.contains(sprite.as_bytes()), "{sprite}");
    }
}
