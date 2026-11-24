use super::test_support::{at_sea_flotilla, navy_fixture, port_flotilla, production, production_selected, sprite_frame, windows};
use content_reader_layers::ResolvedLayers;
use crate::{CounterColorMode, CounterSettings, LAND_WINDOW, Layouts, STACK_WINDOW, type_tint};
use std::path::Path;
use super::{
    Allegiance, Arc, BARS, BG_FRAME, BTreeSet, CAPITALS, CARRIER, CARRIER_SPRITE, CARRIER_UNDEFINED,
    COUNT, Call, CombatProgress, CombatState, Engaged, FRAME_BTN, HULL_AMBIGUOUS, HULL_UNDEFINED,
    HULL_UNRECORDED, IDEOLOGY, Instance, Layout, MARKER, NO_INTEL_SPRITE, NativeWindows, ORDERS,
    PLATE, PLATE_FRAME, PRIDE, PROGRESS, Rotation, SUPPLY, TYPE_SPRITE, Update, Writes, apply,
    combat_calls, counter_inputs, counter_order, mission_item_setup, native, native_gui,
    window_assets,
};
use viewer_foundation::allegiance;
use viewer_native_gui::gui_resources::GuiAssets;
use viewer_native_gui::native_gui::PartKind;
use viewer_native_gui::native_gui::lifecycle::Tick;
use viewer_native_gui::native_resources::GuiResources;
use viewer_native_gui::test_support::{
    Fixture, SYNTHETIC_NAVAL_GUI, SYNTHETIC_NAVY_GUI, VIEWPORT, part,
};

const STILL: Tick = Tick { time: 0.0, dt: 0.0 };

fn read_gui(bytes: &[u8]) -> native::Gui {
    native::read_at(bytes, VIEWPORT)
}

#[test]
fn the_native_windows_load_and_lay_out_without_the_legacy_readers() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_native_windows_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let gui = format!("{SYNTHETIC_NAVY_GUI}\n{SYNTHETIC_NAVAL_GUI}");
    let unterminated = &gui[..gui.rfind('}').unwrap()];
    let mut fnt = String::from(
        "info face=\"x\" size=14\ncommon lineHeight=14 base=14 scaleW=8 scaleH=16 pages=1\n\
         page id=0 file=\"main.tga\"\nchars count=10\n",
    );
    for digit in '0'..='9' {
        fnt.push_str(&format!(
            "char id={} x=0 y=0 width=7 height=14 xoffset=0 yoffset=0 xadvance=7 page=0 \
             chnl=15\n",
            u32::from(digit)
        ));
    }
    let mut tga = vec![0u8; 18];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&8u16.to_le_bytes());
    tga[14..16].copy_from_slice(&16u16.to_le_bytes());
    tga[16] = 32;
    tga[17] = 8;
    tga.resize(18 + 8 * 16 * 4, 0);
    for (path, bytes) in [
        ("interface/mapicons.gui", unterminated.as_bytes().to_vec()),
        (
            "interface/fonts.gfx",
            b"bitmapfonts = { bitmapfont = { name = \"fixture_mono\" path = \"gfx/fonts/main\" } }"
                .to_vec(),
        ),
        ("gfx/fonts/main.fnt", fnt.into_bytes()),
        ("gfx/fonts/main.tga", tga),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(Path::new(&root), &[][..])]).unwrap();
    let windows = NativeWindows::load_at(&layers, [1920, 1080]).unwrap();
    let layouts = Layouts::read(&windows);
    assert!(layouts.land.as_ref().unwrap_err().contains(LAND_WINDOW));
    assert!(layouts.columns.as_ref().unwrap_err().contains(STACK_WINDOW));
    let localisation = content_reader_localisation::localisation::Localisation::default();
    let registry = native::resources::Registry::load(&layers).unwrap();
    let mut gui = GuiResources::new(registry, b"l_english", [5632, 2048], Default::default());
    let assets = window_assets(&windows, gui.registry(), &localisation);
    gui.load(&layers, &assets, &mut Vec::new()).unwrap();
    let resources = GuiAssets {
        gui: &gui,
        windows: &windows,
        localisation: &localisation,
        flags: None,
        physical_viewport: Some(VIEWPORT),
        portrait_textures: None,
    };
    let mut instance = Instance::construct(windows.navy(), VIEWPORT, &resources).unwrap();
    instance.gui_show(windows.navy(), windows.navy()).unwrap();
    instance.tick(windows.navy(), &resources, STILL).unwrap();
    let layout = instance.layout(windows.navy(), &resources, None);
    let _ = std::fs::remove_dir_all(&root);
    let layout = layout.unwrap().unwrap();
    assert_eq!(layout.size, [82, 45]);
}

fn drawn(layout: &Layout) -> Vec<&str> {
    layout
        .parts
        .iter()
        .map(|part| part.path.rsplit('.').next().unwrap())
        .collect()
}

fn cell_page(layout: &Layout, name: &str) -> usize {
    match part(layout, name).unwrap().kind {
        PartKind::Cell { texture, .. } => texture,
        _ => panic!("{name} draws no bound texture"),
    }
}

fn instance(
    window: &native::Element,
    setup: &[(String, Call)],
    updates: &[&[(String, Call)]],
    show_after: bool,
    fixture: &Fixture,
) -> Instance {
    let mut instance = Instance::construct(window, VIEWPORT, fixture).unwrap();
    apply(&mut instance, window, setup, fixture).unwrap();
    for (index, calls) in updates.iter().enumerate() {
        if index == 0 && !show_after {
            instance.gui_show(window, window).unwrap();
        }
        apply(&mut instance, window, calls, fixture).unwrap();
        if index == 0 && show_after {
            instance.gui_show(window, window).unwrap();
        }
        instance.tick(window, fixture, STILL).unwrap();
    }
    instance
}

fn made(update: Update) -> Vec<(String, Call)> {
    assert!(update.unknown.is_empty(), "{:?}", update.unknown);
    update.calls
}

fn layout_of(instance: &Instance, window: &native::Element, fixture: &Fixture) -> Layout {
    instance.layout(window, fixture, None).unwrap().unwrap()
}

fn at_sea_layout(window: &native::Element, calls: &[(String, Call)], fixture: &Fixture) -> Layout {
    layout_of(
        &instance(window, &[], &[calls], false, fixture),
        window,
        fixture,
    )
}

fn item_layout(window: &native::Element, calls: &[(String, Call)], fixture: &Fixture) -> Layout {
    layout_of(
        &instance(window, &mission_item_setup(window), &[calls], true, fixture),
        window,
        fixture,
    )
}

fn hull_of(layout: &Layout) -> (Vec<u8>, u32, [f32; 2]) {
    match &part(layout, "type").unwrap().kind {
        PartKind::Sprite {
            sprite,
            uv,
            offsets,
            geometry,
            ..
        } => (
            sprite.clone(),
            (offsets[0] / uv[2]).round() as u32,
            [geometry.rect[2], geometry.rect[3]],
        ),
        other => panic!("type draws {other:?}"),
    }
}

#[test]
fn detailed_coarse_and_port_selection_follows_the_first_task_force() {
    content_reader_script::test_game::enter();
    use viewer_runtime::unit_handle::Unit;
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    for mut flotilla in [
        at_sea_flotilla(false),
        at_sea_flotilla(true),
        port_flotilla(),
    ] {
        flotilla.members = Arc::from([7, 8]);
        for selected_id in [7, 8] {
            let selected = BTreeSet::from([Unit::TaskForce(selected_id)]);
            let (update, _) = production_selected(
                window,
                &flotilla,
                CounterSettings::default(),
                &fixture,
                &selected,
            );
            let layout = if flotilla.port {
                item_layout(window, &update.calls, &fixture)
            } else {
                at_sea_layout(window, &update.calls, &fixture)
            };
            assert_eq!(part(&layout, "count_selected").is_some(), selected_id == 7);
            assert_eq!(update.selectable, Some((true, Arc::from([7, 8]))));
        }
    }
    let mut unseen = at_sea_flotilla(false);
    unseen.visible_member = false;
    let selected = BTreeSet::from([Unit::TaskForce(7)]);
    let (update, _) = production_selected(
        window,
        &unseen,
        CounterSettings::default(),
        &fixture,
        &selected,
    );
    assert!(
        part(
            &at_sea_layout(window, &update.calls, &fixture),
            "count_selected"
        )
        .is_none()
    );
    unseen.coarse = true;
    let (update, _) = production_selected(
        window,
        &unseen,
        CounterSettings::default(),
        &fixture,
        &selected,
    );
    assert!(
        part(
            &at_sea_layout(window, &update.calls, &fixture),
            "count_selected"
        )
        .is_some()
    );
}

#[test]
fn an_at_sea_counter_takes_the_type_icon_relation_for_its_plate_and_hull() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let foreign = crate::flotilla::Flotilla {
        owner: 5,
        logical: Some(5),
        type_tint: Some(type_tint::Side::Friend),
        side: Ok(type_tint::plate(false, Some(type_tint::Side::Friend))),
        ..at_sea_flotilla(false)
    };
    let (calls, _) = production(window, &foreign, CounterSettings::default(), &fixture);
    let layout = at_sea_layout(window, &calls.calls, &fixture);
    assert_eq!(
        part(&layout, "count_bg").unwrap().color,
        allegiance::Tints::VANILLA.allied
    );
    assert_eq!(
        part(&layout, "type").unwrap().color,
        type_tint::Colors::VANILLA.of(type_tint::Side::Friend)
    );
    let unknown = crate::flotilla::Flotilla {
        type_tint: None,
        side: Ok(type_tint::plate(false, None)),
        ..foreign
    };
    let (calls, _) = production(window, &unknown, CounterSettings::default(), &fixture);
    let layout = at_sea_layout(window, &calls.calls, &fixture);
    assert_eq!(
        part(&layout, "count_bg").unwrap().color,
        allegiance::Tints::VANILLA.other
    );
    assert!(part(&layout, "type").is_none());
}

#[test]
fn a_mission_item_takes_its_own_allegiance_for_its_plate() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let enemy = crate::flotilla::Flotilla {
        owner: 5,
        logical: Some(5),
        type_tint: Some(type_tint::Side::Friend),
        side: Ok(Allegiance::Enemy),
        mission: None,
        ..port_flotilla()
    };
    let (calls, _) = production(window, &enemy, CounterSettings::default(), &fixture);
    let layout = item_layout(window, &calls.calls, &fixture);
    assert_eq!(
        part(&layout, "count_bg").unwrap().color,
        allegiance::Tints::VANILLA.enemy
    );
    let unknown = crate::flotilla::Flotilla {
        side: Err("relations: WarRelations".into()),
        ..enemy
    };
    let (update, _) = production(window, &unknown, CounterSettings::default(), &fixture);
    assert_eq!(
        update.unknown,
        [(PLATE, "relations: WarRelations".to_owned())]
    );
    assert!(!update.wrote.contains(&PLATE));
    let whole = calls;
    let colours = |update: &Update| {
        update
            .calls
            .iter()
            .filter(|(path, call)| {
                path.ends_with(".count_bg")
                    && matches!(call, Call::Control(bound) if bound.color.is_some())
            })
            .count()
    };
    assert_eq!((colours(&update), colours(&whole)), (0, 1));
    assert_eq!(update.calls.len() + 1, whole.calls.len());
    let by_country = CounterSettings {
        color_mode: CounterColorMode::Country,
        ..CounterSettings::default()
    };
    let colored = crate::flotilla::Flotilla {
        logical: Some(0),
        ..unknown
    };
    let (calls, _) = production(window, &colored, by_country, &fixture);
    let layout = item_layout(window, &calls.calls, &fixture);
    assert_eq!(
        part(&layout, "count_bg").unwrap().color,
        [0.1, 0.2, 0.3, 1.0]
    );
}

#[test]
fn a_detailed_counter_at_sea_draws_the_native_write_set() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let (calls, report) = production(
        window,
        &at_sea_flotilla(false),
        CounterSettings::default(),
        &fixture,
    );
    let layout = at_sea_layout(window, &calls.calls, &fixture);
    assert_eq!(
        drawn(&layout),
        [
            "count_bg",
            "group_marker",
            "counter_ideology",
            "counter_overlay",
            "type",
            "capital_ship",
            "capital_count_txt",
            "count_txt",
            "mission_type",
            "bar_org",
            "bar_str",
            "flag",
            "low_supply",
            "air_base_icon",
            "invalid_orders",
        ]
    );
    assert_eq!(sprite_frame(&layout, "count_bg"), 0);
    assert_eq!(sprite_frame(&layout, "mission_type"), 0);
    assert_eq!(sprite_frame(&layout, "low_supply"), 2);
    assert_eq!(sprite_frame(&layout, "air_base_icon"), 1);
    assert_eq!(
        part(&layout, "count_bg").unwrap().color,
        allegiance::Tints::VANILLA.ours
    );
    assert_eq!(cell_page(&layout, "flag"), 1_005);
    assert_eq!(
        hull_of(&layout),
        (
            b"GFX_unit_destroyer_icon_medium_white".to_vec(),
            0,
            [34.0, 15.0]
        )
    );
    assert!(matches!(
        &part(&layout, "capital_count_txt").unwrap().kind,
        PartKind::Text { glyphs, .. } if glyphs.len() == 1
    ));
    assert_eq!(sprite_frame(&layout, "counter_ideology"), 1);
    assert!(report.is_empty());
    assert_eq!(
        calls.wrote.iter().filter(|part| **part == IDEOLOGY).count(),
        1
    );
    let peaceful = crate::flotilla::Flotilla {
        ideology: Ok(None),
        ..at_sea_flotilla(false)
    };
    let (calls, _) = production(window, &peaceful, CounterSettings::default(), &fixture);
    let layout = at_sea_layout(window, &calls.calls, &fixture);
    assert!(part(&layout, "counter_ideology").is_none());
    assert!(calls.unknown.is_empty());
    let undecided = crate::flotilla::Flotilla {
        ideology: Err("CountryCivilWars is not recorded".into()),
        ..at_sea_flotilla(false)
    };
    let (calls, _) = production(window, &undecided, CounterSettings::default(), &fixture);
    assert_eq!(
        calls.unknown,
        [(
            IDEOLOGY,
            "counter_ideology is undecided: CountryCivilWars is not recorded".to_owned()
        )]
    );
    let mut active = at_sea_flotilla(false);
    active.status.active = Some(true);
    let (calls, _) = production(window, &active, CounterSettings::default(), &fixture);
    let layout = at_sea_layout(window, &calls.calls, &fixture);
    assert_eq!(sprite_frame(&layout, "count_bg"), 1);
}

const PORTRAIT_NAVY: &str = r#"guiTypes = { containerWindowType = { name = "unit_counter_navy"
    size = { width = 66 height = 33 }
    iconType = { name = "general_portrait" spriteType = "GFX_general_portrait" }
    iconType = { name = "invalid_orders" spriteType = "GFX_fixture_invalid_orders"
        position = { x = -8 y = -3 } } } }"#;

#[test]
fn a_counter_window_keeps_what_its_updates_leave() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let gui = read_gui(PORTRAIT_NAVY.as_bytes());
    let window = gui.template("unit_counter_navy").unwrap();
    let flotilla = |coarse: bool, invalid: bool| {
        let mut flotilla = at_sea_flotilla(coarse);
        flotilla.status.invalid_orders = Some(invalid);
        flotilla
    };
    let mut view = Instance::construct(window, VIEWPORT, &fixture).unwrap();
    view.gui_show(window, window).unwrap();
    let mut other = view.clone();
    let step = |instance: &mut Instance, flotilla: &crate::flotilla::Flotilla| {
        let (calls, _) = production(window, flotilla, CounterSettings::default(), &fixture);
        apply(instance, window, &calls.calls, &fixture).unwrap();
        instance.tick(window, &fixture, STILL).unwrap();
        drawn(&layout_of(instance, window, &fixture))
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let none = Vec::<String>::new();
    assert_eq!(step(&mut view, &flotilla(true, false)), ["invalid_orders"]);
    assert_eq!(step(&mut view, &flotilla(false, false)), none);
    assert_eq!(step(&mut view, &flotilla(true, true)), none);
    assert_eq!(step(&mut view, &flotilla(false, true)), ["invalid_orders"]);
    assert_eq!(step(&mut view, &flotilla(true, false)), ["invalid_orders"]);
    assert_eq!(
        step(&mut other, &flotilla(false, false)),
        ["general_portrait"]
    );
}

#[test]
fn a_pooled_mission_item_keeps_what_its_updater_does_not_write() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let item = port_flotilla();
    let (calls, _) = production(window, &item, CounterSettings::default(), &fixture);
    let calls = calls.calls;
    for (path, call) in &calls {
        let name = path.rsplit('.').next().unwrap();
        if [
            "capital_ship",
            "type",
            "flag",
            "count_bg",
            "count_txt",
            "capital_count_txt",
        ]
        .contains(&name)
        {
            assert!(
                !matches!(call, Call::Show | Call::Hide | Call::SetHideFlag(_)),
                "{name}: {call:?}"
            );
        }
        if name == "type"
            && let Call::Control(bound) = call
        {
            assert_eq!(bound.color, None);
        }
    }
    let mut pooled = instance(
        window,
        &mission_item_setup(window),
        &[&calls],
        true,
        &fixture,
    );
    let layout = layout_of(&pooled, window, &fixture);
    assert!(part(&layout, "counter_overlay").is_none());
    assert_eq!(sprite_frame(&layout, "mission_type"), 2);
    assert_eq!(sprite_frame(&layout, "low_supply"), 2);
    let mut dim = port_flotilla();
    dim.supply = counter_inputs::NavalSupply::Retain;
    dim.bars = crate::flotilla::NavalBars::Hidden;
    let (calls, _) = production(window, &dim, CounterSettings::default(), &fixture);
    let calls = calls.calls;
    assert!(!calls.iter().any(|(path, _)| path.ends_with(".low_supply")));
    apply(&mut pooled, window, &calls, &fixture).unwrap();
    pooled.gui_show(window, window).unwrap();
    pooled.tick(window, &fixture, STILL).unwrap();
    assert_eq!(
        sprite_frame(&layout_of(&pooled, window, &fixture), "low_supply"),
        2
    );
}

#[test]
fn the_updaters_hide_bit_writes_reach_authored_hidden_controls() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let gui = read_gui(
        br#"guiTypes = { containerWindowType = { name = "unit_counter_navy"
            size = { width = 66 height = 33 }
            iconType = { name = "mission_type" spriteType = "GFX_fixture_mission"
                hide = yes }
            iconType = { name = "no_orders" spriteType = "GFX_fixture_no_orders" } } }"#,
    );
    let window = gui.template("unit_counter_navy").unwrap();
    let (calls, _) = production(
        window,
        &port_flotilla(),
        CounterSettings::default(),
        &fixture,
    );
    let mut item = instance(
        window,
        &mission_item_setup(window),
        &[&calls.calls],
        true,
        &fixture,
    );
    let layout = layout_of(&item, window, &fixture);
    assert_eq!(drawn(&layout), ["mission_type"]);
    item.gui_show(window, window).unwrap();
    item.tick(window, &fixture, STILL).unwrap();
    assert_eq!(drawn(&layout_of(&item, window, &fixture)), ["mission_type"]);
    let combat = read_gui(
        br#"guiTypes = { containerWindowType = { name = "naval_combat_mapicon"
            size = { width = 33 height = 43 }
            iconType = { name = "progress" spriteType = "GFX_bar" hide = yes }
            iconType = { name = "progress_small" spriteType = "GFX_bar" } } }"#,
    );
    let window = combat.template("naval_combat_mapicon").unwrap();
    let fixture = Fixture::new(&[], &[("GFX_bar", [49, 9])]);
    let calls = made(combat_calls(
        window,
        &CombatState {
            rotation: Some(Rotation {
                engaged: Some(Engaged {
                    variant: 0,
                    progress: Ok(CombatProgress {
                        small: false,
                        progress: 50_000,
                        flip: false,
                    }),
                }),
            }),
            hq: Ok(false),
        },
        &fixture,
    ));
    let icon = instance(window, &[], &[&calls], false, &fixture);
    assert_eq!(drawn(&layout_of(&icon, window, &fixture)), ["progress"]);
}

#[test]
fn a_scaled_bar_steps_by_its_integer_drawable_extent() {
    content_reader_script::test_game::enter();
    let gui = read_gui(
        br#"guiTypes = { containerWindowType = { name = "unit_counter_navy"
            size = { width = 66 height = 33 }
            iconType = { name = "bar_org" spriteType = "GFX_fixture_org"
                position = { x=13 y=15 } scale = 0.69 rotation = 1.5708 }
            iconType = { name = "bar_str" spriteType = "GFX_undefined_bar"
                position = { x=13 y=19 } rotation = 1.5708 } } }"#,
    );
    let window = gui.template("unit_counter_navy").unwrap();
    let mut sheets = vec![("GFX_fixture_org", 1, [3, 17])];
    sheets.push(("GFX_default_fallback_texture", 1, [4, 9]));
    let fixture = Fixture::new(&sheets, &[]);
    let mut flotilla = at_sea_flotilla(false);
    flotilla.bars = crate::flotilla::NavalBars::Fill(0.9, 0.9);
    let (calls, _) = production(window, &flotilla, CounterSettings::default(), &fixture);
    let calls = calls.calls;
    let offset = |name: &str| {
        calls.iter().find_map(|(path, call)| match call {
            Call::Control(bound) if path.ends_with(&format!(".{name}")) => Some(bound.offset),
            _ => None,
        })
    };
    assert_eq!(offset("bar_org"), Some(Some([4, 0])));
    assert_eq!(offset("bar_str"), Some(Some([4, 0])));
    assert!(part(&at_sea_layout(window, &calls, &fixture), "bar_org").is_some());
}

#[test]
fn an_undecidable_hull_leaves_the_type_sprite_unknown() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    use counter_order::HullIcon;
    let update = |instance: &mut Instance, hull: HullIcon| {
        let mut flotilla = at_sea_flotilla(false);
        flotilla.cells.as_mut().unwrap().hull = hull;
        let (update, report) = production(window, &flotilla, CounterSettings::default(), &fixture);
        apply(instance, window, &update.calls, &fixture).unwrap();
        instance.tick(window, &fixture, STILL).unwrap();
        (
            hull_of(&layout_of(instance, window, &fixture)).0,
            update,
            report,
        )
    };
    let mut counter = Instance::construct(window, VIEWPORT, &fixture).unwrap();
    counter.gui_show(window, window).unwrap();
    let destroyer = b"GFX_unit_destroyer_icon_medium_white".to_vec();
    let (hull, written, _) = update(
        &mut counter,
        HullIcon::Resolved(Some(b"destroyer".to_vec())),
    );
    assert_eq!(hull, destroyer);
    assert!(written.wrote.contains(&TYPE_SPRITE));
    assert!(written.unknown.is_empty(), "{:?}", written.unknown);
    let (hull, kept, report) = update(&mut counter, HullIcon::Resolved(Some(b"cruiser".to_vec())));
    assert_eq!(hull, destroyer);
    assert_eq!(
        report.get(format!("GFX_unit_cruiser_icon_medium_white{HULL_UNDEFINED}").as_str()),
        Some(&1)
    );
    assert!(!kept.wrote.contains(&TYPE_SPRITE));
    assert!(kept.unknown.is_empty(), "{:?}", kept.unknown);
    let (hull, kept, report) = update(&mut counter, HullIcon::Resolved(None));
    assert_eq!(hull, destroyer);
    assert!(!report.keys().any(|key| key.contains("hull")));
    assert!(kept.unknown.is_empty(), "{:?}", kept.unknown);
    for (hull, why) in [
        (HullIcon::Unrecorded, HULL_UNRECORDED),
        (HullIcon::Ambiguous, HULL_AMBIGUOUS),
    ] {
        let (kept, undecided, report) = update(&mut counter, hull);
        assert_eq!(kept, destroyer);
        assert_eq!(undecided.unknown, [(TYPE_SPRITE, why.to_owned())]);
        assert!(!undecided.wrote.contains(&TYPE_SPRITE));
        assert!(!report.keys().any(|key| key.contains("hull")), "{report:?}");
    }
    let mut hidden = at_sea_flotilla(false);
    hidden.hidden = true;
    let (update, _) = production(window, &hidden, CounterSettings::default(), &fixture);
    assert!(update.wrote.contains(&TYPE_SPRITE));
    let setter = update
        .calls
        .into_iter()
        .find_map(|(path, call)| match call {
            Call::SetSprite(_, setter) if path.ends_with(".type") => Some(setter),
            _ => None,
        });
    assert_eq!(setter, Some(native_gui::SpriteSetter::Icon));
}

#[test]
fn an_incomplete_ship_census_leaves_the_counts_unknown() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let unknown = |update: &Update, part: &str| update.unknown.iter().any(|(at, _)| *at == part);
    fn count_text(update: &Update) -> Option<&str> {
        update.calls.iter().find_map(|(path, call)| match call {
            Call::SetText(text) if path.ends_with(".count_txt") => Some(text.as_str()),
            _ => None,
        })
    }
    for coarse in [false, true] {
        let mut sea = at_sea_flotilla(coarse);
        sea.ships = None;
        sea.cells = None;
        let (update, _) = production(window, &sea, CounterSettings::default(), &fixture);
        assert!(unknown(&update, COUNT), "coarse {coarse}");
        assert!(unknown(&update, CAPITALS), "coarse {coarse}");
        assert_eq!(count_text(&update), None, "coarse {coarse}");
        sea.ships = Some(0);
        let (update, _) = production(window, &sea, CounterSettings::default(), &fixture);
        assert!(!unknown(&update, COUNT) && update.wrote.contains(&COUNT));
        assert_eq!(count_text(&update), Some("0"), "coarse {coarse}");
    }
    let mut item = port_flotilla();
    item.cells = None;
    item.item = item.item.map(|intel| crate::flotilla::ItemIntel {
        ships: None,
        ..intel
    });
    let (update, _) = production(window, &item, CounterSettings::default(), &fixture);
    assert!(unknown(&update, COUNT) && unknown(&update, CAPITALS));
    assert_eq!(count_text(&update), None);
    item.item = item
        .item
        .map(|intel| crate::flotilla::ItemIntel { byte: 99, ..intel });
    let (update, _) = production(window, &item, CounterSettings::default(), &fixture);
    assert!(!unknown(&update, COUNT) && update.wrote.contains(&COUNT));
    assert_eq!(count_text(&update), Some("?"));
}

#[test]
fn missing_operands_leave_exactly_their_parts_unknown() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let unknown = |mut flotilla: crate::flotilla::Flotilla| {
        flotilla.status = counter_inputs::NavalStatus::default();
        flotilla.cells = None;
        flotilla.marker = Err("fleet 3's row is not recorded".to_owned());
        flotilla.bars = crate::flotilla::NavalBars::Unknown(crate::flotilla::BARS_UNRECORDED.to_owned());
        flotilla.supply = counter_inputs::NavalSupply::Unavailable;
        flotilla
    };
    let parts = |update: &Update| {
        let mut parts: Vec<&str> = update.unknown.iter().map(|(part, _)| *part).collect();
        parts.sort_unstable();
        parts.dedup();
        parts
    };
    let touched = |update: &Update, names: &[&str]| {
        update
            .calls
            .iter()
            .filter(|(path, call)| {
                names.iter().any(|name| path.ends_with(&format!(".{name}")))
                    && matches!(call, Call::Show | Call::Hide | Call::SetHideFlag(_))
            })
            .count()
    };
    let children = [
        "no_orders",
        "invalid_orders",
        "pride_of_the_fleet_icon",
        "air_base_icon",
        "group_marker",
        "low_supply",
    ];
    let (sea, _) = production(
        window,
        &unknown(at_sea_flotilla(false)),
        CounterSettings::default(),
        &fixture,
    );
    let mut expected = vec![
        ORDERS,
        PRIDE,
        CARRIER,
        CARRIER_SPRITE,
        CAPITALS,
        MARKER,
        BARS,
        SUPPLY,
        PLATE_FRAME,
        TYPE_SPRITE,
    ];
    expected.sort_unstable();
    assert_eq!(parts(&sea), expected);
    assert!(sea.wrote.contains(&PLATE));
    assert_eq!(touched(&sea, &children), 0, "{:?}", sea.calls);
    let (item, _) = production(
        window,
        &unknown(port_flotilla()),
        CounterSettings::default(),
        &fixture,
    );
    expected.retain(|part| *part != PLATE_FRAME);
    assert_eq!(parts(&item), expected);
    assert!(item.wrote.contains(&PLATE));
    assert_eq!(touched(&item, &children), 0, "{:?}", item.calls);
    let (whole, _) = production(
        window,
        &port_flotilla(),
        CounterSettings::default(),
        &fixture,
    );
    assert!(whole.unknown.is_empty(), "{:?}", whole.unknown);
    for part in [
        ORDERS,
        PRIDE,
        CARRIER,
        CARRIER_SPRITE,
        CAPITALS,
        MARKER,
        BARS,
        SUPPLY,
        TYPE_SPRITE,
    ] {
        assert!(whole.wrote.contains(&part), "{part}");
    }
}

#[test]
fn an_undefined_carrier_sprite_is_counted_by_name() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let mut rocket = port_flotilla();
    rocket.cells.as_mut().unwrap().carrier = [Some((counter_order::ROCKET_SITE_ICON, 1))].into();
    let (update, report) = production(window, &rocket, CounterSettings::default(), &fixture);
    assert_eq!(
        report.get(format!("{}{CARRIER_UNDEFINED}", counter_order::ROCKET_SITE_ICON).as_str()),
        Some(&1)
    );
    assert!(update.kept.contains(&CARRIER_SPRITE));
    assert!(!update.wrote.contains(&CARRIER_SPRITE));
    let (update, report) = production(
        window,
        &port_flotilla(),
        CounterSettings::default(),
        &fixture,
    );
    assert!(
        !report.keys().any(|key| key.ends_with(CARRIER_UNDEFINED)),
        "{report:?}"
    );
    assert!(update.wrote.contains(&CARRIER_SPRITE));
}

#[test]
fn a_hull_followed_by_an_undefined_no_intel_sprite_keeps_the_hull() {
    content_reader_script::test_game::enter();
    let mut fixture = navy_fixture();
    fixture
        .sheets
        .remove(NO_INTEL_SPRITE.as_bytes())
        .expect("the navy fixture defines it");
    let windows = windows();
    let window = windows.navy();
    let hull = window.find(native::Kind::Icon, "type").unwrap();
    let mut counter = Instance::construct(window, VIEWPORT, &fixture).unwrap();
    counter.gui_show(window, window).unwrap();
    let mut update = |flotilla: &crate::flotilla::Flotilla| {
        let (calls, _) = production(window, flotilla, CounterSettings::default(), &fixture);
        apply(&mut counter, window, &calls.calls, &fixture).unwrap();
        counter.tick(window, &fixture, STILL).unwrap();
        (
            hull_of(&layout_of(&counter, window, &fixture)),
            counter
                .drawable_extent(window, hull, &fixture)
                .unwrap()
                .unwrap(),
        )
    };
    let destroyer = b"GFX_unit_destroyer_icon_medium_white".to_vec();
    let (seen, extent) = update(&at_sea_flotilla(false));
    assert_eq!(seen, (destroyer.clone(), 0, [34.0, 15.0]));
    assert_eq!(extent, [34, 15]);
    let mut hidden = at_sea_flotilla(false);
    hidden.hidden = true;
    let (unseen, extent) = update(&hidden);
    assert_eq!(unseen.0, destroyer);
    assert_eq!(unseen.2, [34.0, 15.0]);
    assert_eq!(extent, [34, 15]);
}

#[test]
fn the_counter_options_select_the_nato_frame_and_the_country_colour() {
    content_reader_script::test_game::enter();
    let fixture = navy_fixture();
    let windows = windows();
    let window = windows.navy();
    let nato = CounterSettings {
        nato: true,
        ..CounterSettings::default()
    };
    let mut hidden = at_sea_flotilla(false);
    hidden.hidden = true;
    let no_intel = NO_INTEL_SPRITE.as_bytes().to_vec();
    for (settings, frame) in [(CounterSettings::default(), 0), (nato, 1)] {
        let (calls, _) = production(window, &hidden, settings, &fixture);
        let layout = at_sea_layout(window, &calls.calls, &fixture);
        assert_eq!(hull_of(&layout), (no_intel.clone(), frame, [34.0, 15.0]));
        let mut item = port_flotilla();
        item.item = Some(crate::flotilla::ItemIntel {
            byte: 120,
            ..item.item.unwrap()
        });
        let (calls, _) = production(window, &item, settings, &fixture);
        let calls = calls.calls;
        let first = item_layout(window, &calls, &fixture);
        assert_eq!(hull_of(&first).0, no_intel);
        assert_eq!(hull_of(&first).1, 0);
        let again = layout_of(
            &instance(
                window,
                &mission_item_setup(window),
                &[&calls, &calls],
                true,
                &fixture,
            ),
            window,
            &fixture,
        );
        assert_eq!(hull_of(&again), (no_intel.clone(), frame, [34.0, 15.0]));
        let thrice = layout_of(
            &instance(
                window,
                &mission_item_setup(window),
                &[&calls, &calls, &calls],
                true,
                &fixture,
            ),
            window,
            &fixture,
        );
        assert_eq!(hull_of(&thrice), hull_of(&again));
    }
    let (calls, _) = production(window, &at_sea_flotilla(false), nato, &fixture);
    assert_eq!(hull_of(&at_sea_layout(window, &calls.calls, &fixture)).1, 0);
    let country = CounterSettings {
        color_mode: CounterColorMode::Country,
        ..CounterSettings::default()
    };
    for flotilla in [
        at_sea_flotilla(false),
        at_sea_flotilla(true),
        port_flotilla(),
    ] {
        let (calls, _) = production(window, &flotilla, country, &fixture);
        let calls = calls.calls;
        let layout = if flotilla.port {
            item_layout(window, &calls, &fixture)
        } else {
            at_sea_layout(window, &calls, &fixture)
        };
        assert_eq!(
            part(&layout, "count_bg").unwrap().color,
            [0.1, 0.2, 0.3, 1.0]
        );
    }
    let mut foreign = at_sea_flotilla(false);
    foreign.logical = Some(5);
    let (update, _) = production(window, &foreign, country, &fixture);
    let [(part, reason)] = update.unknown.as_slice() else {
        panic!("{:?}", update.unknown);
    };
    assert_eq!(*part, PLATE);
    assert!(reason.contains("country slot 5's UI colour"), "{reason}");
}

#[test]
fn the_combat_progress_value_rounds_as_the_game_does() {
    let value = |progress, flip| {
        CombatProgress {
            small: false,
            progress,
            flip,
        }
        .value()
    };
    assert_eq!(value(0, false), 0);
    assert_eq!(value(100_000, false), 100);
    assert_eq!(value(12_499, false), 12);
    assert_eq!(value(12_500, false), 13);
    assert_eq!(value(12_500, true), 88);
    assert_eq!(value(-12_500, false), -13);
}

#[test]
fn the_combat_state_calls_go_through_the_native_lookups() {
    content_reader_script::test_game::enter();
    let windows = windows();
    let window = windows.naval_combat();
    let fixture = Fixture::new(
        &[
            ("GFX_fixture_combat", 2, [70, 35]),
            ("GFX_fixture_hq", 1, [41, 41]),
        ],
        &[
            ("GFX_fixture_progress", [154, 14]),
            ("GFX_fixture_progress_small", [49, 9]),
        ],
    );
    let progress = |small, flip| CombatProgress {
        small,
        progress: 40_000,
        flip,
    };
    let engaged = |variant, hq, progress| CombatState {
        rotation: Some(Rotation {
            engaged: Some(Engaged { variant, progress }),
        }),
        hq: Ok(hq),
    };
    let undecided = combat_calls(
        window,
        &engaged(1, false, Err("its size is undecided".to_owned())),
        &fixture,
    );
    assert_eq!(
        undecided.unknown,
        [(
            PROGRESS,
            "icon omitted: its progress bar is undecided: its size is undecided".to_owned()
        )]
    );
    assert_eq!(undecided.wrote, [BG_FRAME, FRAME_BTN]);
    let calls = undecided.calls;
    let mut icon = instance(window, &[], &[&calls], false, &fixture);
    let layout = layout_of(&icon, window, &fixture);
    assert_eq!(drawn(&layout), ["bg_btn"]);
    assert_eq!(sprite_frame(&layout, "bg_btn"), 1);
    let unengaged = CombatState {
        rotation: Some(Rotation { engaged: None }),
        hq: Ok(false),
    };
    apply(
        &mut icon,
        window,
        &made(combat_calls(window, &unengaged, &fixture)),
        &fixture,
    )
    .unwrap();
    assert_eq!(
        sprite_frame(&layout_of(&icon, window, &fixture), "bg_btn"),
        1
    );
    let bar = |layout: &Layout, name: &str| match part(layout, name).unwrap().kind {
        PartKind::Progress {
            geometry, state, ..
        } => (geometry, state),
        _ => panic!("{name} is not a progress bar"),
    };
    for (small, [w, h]) in [(false, [154.0, 14.0]), (true, [49.0, 9.0])] {
        let calls = made(combat_calls(
            window,
            &engaged(0, true, Ok(progress(small, false))),
            &fixture,
        ));
        let layout = layout_of(
            &instance(window, &[], &[&calls], false, &fixture),
            window,
            &fixture,
        );
        let shown = if small { "progress_small" } else { "progress" };
        assert_eq!(drawn(&layout), ["bg_btn", "frame_btn", shown]);
        let (unflipped, state) = bar(&layout, shown);
        assert!((state - 0.4).abs() < 1e-6);
        assert_eq!(unflipped.rotation, 0.0);
        let calls = made(combat_calls(
            window,
            &engaged(0, false, Ok(progress(small, true))),
            &fixture,
        ));
        let layout = layout_of(
            &instance(window, &[], &[&calls], false, &fixture),
            window,
            &fixture,
        );
        assert_eq!(drawn(&layout), ["bg_btn", shown]);
        let (flipped, state) = bar(&layout, shown);
        assert!((state - 0.6).abs() < 1e-6);
        assert_eq!(flipped.rotation, std::f32::consts::PI);
        assert_eq!(flipped.rect[0], unflipped.rect[0] + w);
        assert_eq!(flipped.rect[1], unflipped.rect[1] - h);
    }
}

#[test]
fn an_unmeasured_flipped_bar_takes_every_call_but_its_move() {
    content_reader_script::test_game::enter();
    let windows = windows();
    let window = windows.naval_combat();
    let state = CombatState {
        rotation: Some(Rotation {
            engaged: Some(Engaged {
                variant: 1,
                progress: Ok(CombatProgress {
                    small: false,
                    progress: 40_000,
                    flip: true,
                }),
            }),
        }),
        hq: Ok(false),
    };
    let mut unmeasured = Fixture::new(&[("GFX_fixture_combat", 2, [70, 35])], &[]);
    unmeasured.session_sprites = true;
    let partial = combat_calls(window, &state, &unmeasured);
    assert!(
        matches!(
            partial.unknown.as_slice(),
            [(PROGRESS, why)] if why.starts_with(
                "icon omitted: naval_combat_mapicon does not update: \
                 naval_combat_mapicon is not drawn: "
            )
        ),
        "{:?}",
        partial.unknown
    );
    assert_eq!(partial.wrote, [BG_FRAME, FRAME_BTN]);
    let progress = window.find(native::Kind::Icon, "progress").unwrap();
    let bar = partial
        .calls
        .iter()
        .find_map(|(path, call)| match call {
            Call::Control(bound) if *path == progress.path => Some(bound),
            _ => None,
        })
        .unwrap();
    assert_eq!(bar.rotation, Some(std::f32::consts::PI));
    assert_eq!(bar.offset, None);
    let measured = Fixture::new(
        &[("GFX_fixture_combat", 2, [70, 35])],
        &[("GFX_fixture_progress", [154, 14])],
    );
    let whole = combat_calls(window, &state, &measured);
    assert!(whole.unknown.is_empty(), "{:?}", whole.unknown);
    assert_eq!(whole.wrote, [BG_FRAME, PROGRESS, FRAME_BTN]);
    let others = |calls: &[(String, Call)]| {
        calls
            .iter()
            .filter(|(_, call)| !matches!(call, Call::Control(_)))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(others(&partial.calls), others(&whole.calls));
    assert!(others(&whole.calls).len() > 4);
}

#[test]
fn grid_items_are_finalized_and_placed_through_the_module() {
    content_reader_script::test_game::enter();
    let fixture = Fixture::new(
        &[
            ("GFX_fixture_combat", 2, [70, 35]),
            ("GFX_badge", 1, [10, 10]),
            ("GFX_tile", 1, [20, 10]),
        ],
        &[],
    );
    let gui = read_gui(
        br#"guiTypes = {
            containerWindowType = { name = "naval_combat_mapicon" size = { width = 33 height = 43 }
                buttonType = { name = "bg_btn" quadTextureSprite = "GFX_fixture_combat" }
                gridboxtype = { name = "attacker_grid" position = { x = 5 y = 6 }
                    size = { width = 40 height = 40 } slotsize = { width = 20 height = 10 }
                    format = "UPPER_LEFT" max_slots_horizontal = 1 }
                iconType = { name = "badge" spriteType = "GFX_badge" } }
            containerWindowType = { name = "tile" size = { width = 20 height = 10 }
                position = { x = 1 y = 2 }
                iconType = { name = "body" spriteType = "GFX_tile" } } }"#,
    );
    let window = gui.template("naval_combat_mapicon").unwrap();
    let tile = gui.template("tile").unwrap();
    let layout = layout_of(
        &instance(window, &[], &[&[]], false, &fixture),
        window,
        &fixture,
    );
    assert_eq!(drawn(&layout), ["bg_btn", "badge"]);
    let holder = &layout.holders[0];
    let grid = window.find(native::Kind::Grid, "attacker_grid").unwrap();
    let places = holder
        .finalize(grid, VIEWPORT, &[tile, tile, tile])
        .unwrap();
    assert_eq!(places, [[6, 8], [6, 18], [6, 28]]);
    let item = layout_of(
        &instance(tile, &[], &[&[]], false, &fixture),
        tile,
        &fixture,
    );
    let parts = holder.place(places[1], &item);
    assert_eq!(parts.len(), 1);
}

#[test]
fn port_items_are_placed_in_box_ships() {
    content_reader_script::test_game::enter();
    let fixture = Fixture::new(
        &[
            ("GFX_mapicon_naval_base", 1, [24, 26]),
            ("GFX_tile", 1, [58, 32]),
        ],
        &[],
    );
    let gui = read_gui(
        br#"guiTypes = {
            containerWindowType = { name = "naval_base_mapicon" size = { width=32 height=32 }
                buttonType = { name = "bg_btn" quadTextureSprite = "GFX_mapicon_naval_base"
                    position = { x=4 y=3 } }
                gridboxtype = { name = "box_ships" position = { x=-15 y=35 }
                    max_slots_horizontal = 1 max_slots_vertical = 2
                    size = { width=100%% height=100%% } slotsize = { width=58 height=32 }
                    format = "UPPER_LEFT" } }
            containerWindowType = { name = "tile" size = { width = 58 height = 32 }
                iconType = { name = "body" spriteType = "GFX_tile" } } }"#,
    );
    let base = gui.template("naval_base_mapicon").unwrap();
    let tile = gui.template("tile").unwrap();
    let layout = layout_of(
        &instance(base, &[], &[&[]], false, &fixture),
        base,
        &fixture,
    );
    let grid = base.find(native::Kind::Grid, "box_ships").unwrap();
    let holder = layout
        .holders
        .iter()
        .find(|holder| holder.path == grid.path)
        .unwrap();
    let places = holder
        .finalize(grid, VIEWPORT, &[tile, tile, tile])
        .unwrap();
    assert_eq!(places, [[-15, 35], [-15, 67]]);
}
