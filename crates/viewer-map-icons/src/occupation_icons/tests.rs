use super::*;
use content_reader_politics::occupation_modifiers::{Definition, Definitions, Kind};
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::OccupationMap, rows::StateResistanceRow,
    rows::StateRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_gpu::camera::{MapDefines, MapSize, Pose};

fn content() -> Content {
    let definitions = [
        ("state_second", 4_000_000, Kind::StateResistance),
        ("state_first", 2_000_000, Kind::StateResistance),
        ("core_equal", 2_000_000, Kind::CoreResistance),
    ]
    .into_iter()
    .map(|(token, threshold, kind)| {
        (
            token.into(),
            Definition {
                token: token.into(),
                threshold,
                kind,
                sprite: "GFX_even".into(),
                frame: 2,
                ..Default::default()
            },
        )
    })
    .collect();
    Content {
        definitions: Definitions {
            definitions,
            ..Default::default()
        },
        defines: IconDefines {
            cutoff: 900.,
            badge_cutoff: 700.,
            reenable: 2_000_000,
            priority: 30,
            resistance_colors: ColorList(vec![0., 1., 0., 0., 0., 100., 0., 1., 0., 0.75]),
            compliance_colors: ColorList(vec![0., 0., 0., 1., 0., 100., 1., 1., 1., 1.]),
        },
    }
}
fn row() -> StateResistanceRow {
    StateResistanceRow {
        state: 1,
        country: 20,
        level: 3_199_999,
        speed: -5,
        compliance: 1_050_000,
        active: 0,
        map: Recorded::Value(Some(OccupationMap {
            stored_target: None,
            core_country: Some(20),
            resistance: ["state_second", "state_first", "state_first", "core_equal"]
                .map(|s| s.as_bytes().to_vec())
                .to_vec(),
            compliance: vec![],
        })),
        ..Default::default()
    }
}
fn frame(row: StateResistanceRow) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed".into(),
    });
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow {
            id: 10,
            canonical_slot: 2,
        },
        CountryLinkRow {
            id: 11,
            canonical_slot: 2,
        },
        CountryLinkRow {
            id: 20,
            canonical_slot: 3,
        },
    ]));
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 10,
            canonical_slot: 2,
            ..Default::default()
        },
        CountryRow {
            id: 20,
            canonical_slot: 3,
            ..Default::default()
        },
    ]));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 1,
        controller: 11,
        ..Default::default()
    }]));
    tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(vec![row]));
    DecodedFrame {
        branch: 1,
        hour: 24,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}
fn prepare_at(
    mode: Option<Mode>,
    height: f32,
    row: StateResistanceRow,
    country: u32,
) -> PreparedOccupationIcons {
    let content = content();
    let camera = Camera {
        center: [3000., 1500.],
        height,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    prepare(
        &Inputs {
            content: &content,
            mode,
            country,
            state_provinces: &BTreeMap::from([(1, vec![10])]),
            centroid: &|_| Some([3000., 10., 1500.]),
            camera,
            viewport: [2048, 1152],
            ui_scale: 1.,
        },
        &frame(row),
    )
}

#[test]
fn occupation_content_preserves_fractional_distance_cutoffs_and_native_priority_byte() {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "fixture",
            br#"NDefines = {
        NGraphics = { RESISTANCE_MAP_ICON_DISTANCE_CUTOFF=123.75,
            RESISTANCE_MAP_ICON_MODIFIERS_DISTANCE_CUTOFF=77.125,
            OCCUPATION_RESISTANCE_MAP_MODE_COLORS={0,0,0,0,0.2,100,1,1,1,0.8},
            OCCUPATION_COMPLIANCE_MAP_MODE_COLORS={0,0,0,1,0.3,100,1,1,0,0.9}},
        NResistance = { RESISTANCE_TARGET_TO_REENABLE_RESISTANCE=1.25 },
        NMapIcons = { TOP_MAP_ICON=258 }
    }"#,
        )
        .unwrap();
    let loaded = IconDefines::load(&defines).unwrap();
    assert_eq!((loaded.cutoff, loaded.badge_cutoff), (123.75, 77.125));
    assert_eq!((loaded.reenable, loaded.priority), (125_000, 2));
    assert_eq!(loaded.resistance_colors.color(0.), [0., 0., 0., 0.2]);
}

#[test]
fn occupation_markers_use_retained_occurrences_canonical_control_and_strict_cutoffs() {
    let prepared = prepare_at(Some(Mode::Resistance), 500., row(), 2);
    assert!(
        prepared.report.refused.is_empty(),
        "{:?}",
        prepared.report.refused
    );
    let [icon] = prepared.placed.as_slice() else {
        panic!("one admitted state marker expected")
    };
    assert_eq!(icon.text, format!("31%{ICON}{INCREASING} "));
    assert_eq!(
        icon.badges
            .iter()
            .map(|b| b.token.as_str())
            .collect::<Vec<_>>(),
        ["state_first", "state_first", "core_equal", "state_second"]
    );
    let [red, green, blue, _] = content()
        .defines
        .resistance_colors
        .color(3_199_999f32 / 100_000.);
    assert_eq!(icon.color, [red, green, blue, 1.]);
    assert!(
        prepare_at(Some(Mode::Resistance), 700., row(), 2).placed[0]
            .badges
            .is_empty()
    );
    assert!(
        prepare_at(Some(Mode::Resistance), 900., row(), 2)
            .placed
            .is_empty()
    );
    assert!(prepare_at(None, 500., row(), 2).placed.is_empty());
    assert!(
        prepare_at(Some(Mode::Resistance), 500., row(), 3)
            .placed
            .is_empty()
    );
    let compliance = prepare_at(Some(Mode::Compliance), 500., row(), 2);
    assert_eq!(compliance.placed[0].text, "10%");
}

#[test]
fn occupation_controller_zero_requires_actual_viewed_zero_not_a_positive_slot_zero_country() {
    let mut frame = frame(row());
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            controller: 0,
            ..Default::default()
        }]));
    let content = content();
    let members = BTreeMap::from([(1, vec![10])]);
    let inputs = Inputs {
        content: &content,
        mode: Some(Mode::Resistance),
        country: 0,
        state_provinces: &members,
        centroid: &|_| Some([3000., 10., 1500.]),
        camera: Camera {
            center: [3000., 1500.],
            height: 500.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        },
        viewport: [2048, 1152],
        ui_scale: 1.,
    };
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 10,
            canonical_slot: 0,
            ..Default::default()
        }]));
    assert!(prepare(&inputs, &frame).placed.is_empty());
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 0,
            canonical_slot: 0,
            ..Default::default()
        }]));
    assert_eq!(prepare(&inputs, &frame).placed.len(), 1);
}

#[test]
fn occupation_zero_level_uses_stored_target_equality_without_an_active_gate() {
    let mut row = row();
    row.level = 0;
    row.compliance = 0;
    for (target, shown) in [(1_999_999, false), (2_000_000, true)] {
        row.map
            .require_mut("map")
            .unwrap()
            .as_mut()
            .unwrap()
            .stored_target = Some(target);
        assert_eq!(
            !prepare_at(Some(Mode::Resistance), 500., row.clone(), 2)
                .placed
                .is_empty(),
            shown
        );
    }
    row.map
        .require_mut("map")
        .unwrap()
        .as_mut()
        .unwrap()
        .stored_target = None;
    let refused = prepare_at(Some(Mode::Resistance), 500., row.clone(), 2);
    assert!(refused.placed.is_empty());
    assert!(
        refused
            .report
            .refused
            .keys()
            .any(|why| why.contains("cached target"))
    );
    row.compliance = 1;
    assert_eq!(
        prepare_at(Some(Mode::Compliance), 500., row, 2)
            .placed
            .len(),
        1
    );
    assert_eq!(percentage(-50_000, 0, Mode::Resistance), "-0%");
    assert_eq!(percentage(1_099_999, 0, Mode::Compliance), "10%");
}

#[test]
fn occupation_free_view_uses_its_existing_anchor_distance_and_world_family() {
    use viewer_gpu::camera::{Free, Lens};
    let content = content();
    let camera = Camera {
        center: [3000., 1450.],
        height: 1000.,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Free(Free {
            yaw: 0.,
            pitch: 0.,
            roll: 0.,
            lens: Lens::MAP,
        }),
    };
    let out = prepare(
        &Inputs {
            content: &content,
            mode: Some(Mode::Resistance),
            country: 2,
            state_provinces: &BTreeMap::from([(1, vec![10])]),
            centroid: &|_| Some([3000., 950., 1500.]),
            camera,
            viewport: [2048, 1152],
            ui_scale: 1.,
        },
        &frame(row()),
    );
    assert_eq!(out.placed.len(), 1);
    assert_eq!(out.placed[0].anchor.family, IconFamily::Occupation);
    assert_eq!(out.placed[0].anchor.world, [3000., 950., 1500.]);
    assert!(
        prepare_at(Some(Mode::Resistance), f32::NAN, row(), 2)
            .placed
            .is_empty()
    );
}

#[test]
fn occupation_badges_do_not_guess_missing_identity_or_unknown_content() {
    let mut row = row();
    row.map = Recorded::NotRecorded;
    let out = prepare_at(Some(Mode::Resistance), 500., row.clone(), 2);
    assert_eq!(out.placed.len(), 1);
    assert!(out.placed[0].badges.is_empty());
    assert!(
        out.report
            .refused
            .keys()
            .any(|why| why.contains("active definitions"))
    );
    row.map = Recorded::Value(Some(OccupationMap {
        resistance: vec![b"unknown_definition".to_vec(); 2],
        ..Default::default()
    }));
    let out = prepare_at(Some(Mode::Resistance), 500., row, 2);
    assert!(
        out.report
            .refused
            .iter()
            .any(|(why, count)| why.contains("unknown_definition") && *count == 2)
    );
}

const GUI: &[u8] = br#"guiTypes = {
    containerWindowType = { name = resistance_compliance_map_icon size = { width = 120 height = 40 }
        gridBoxType = { name = modifier_grid_list position = { x = 7 y = -9 } size = { width = 91 height = 19 }
            slotsize = { width = 23 height = 17 } max_slots_horizontal = 2 max_slots_vertical = 1 }
        instantTextBoxType = { name = text font = Main_14 maxWidth = 120 maxHeight = 20 text = value format = center }
    }
    containerWindowType = { name = resistance_compliance_map_icon_modifier_entry size = { width = 19 height = 17 }
        iconType = { name = current_mission_bg spriteType = GFX_even position = { x = -2 y = -3 } }
        iconType = { name = icon spriteType = GFX_even position = { x = 0 y = 0 } }
    }
}"#;

#[test]
fn occupation_percentages_draw_opaque_even_with_transparent_map_fill() {
    content_reader_script::test_game::enter();
    use viewer_native_gui::test_support::gui;
    let resources = gui::fixture();
    let windows = gui::windows(GUI);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [400, 400],
        time: Some(0.),
    };
    for (mode, raw, text) in [
        (Mode::Resistance, 0, "0%"),
        (Mode::Resistance, 2_750_000, "27%"),
        (Mode::Compliance, 0, "0%"),
        (Mode::Compliance, 4_025_000, "40%"),
    ] {
        let mut row = row();
        row.level = raw;
        row.compliance = raw;
        row.speed = 0;
        row.map = Recorded::Value(Some(OccupationMap {
            stored_target: Some(2_000_000),
            ..Default::default()
        }));
        let mut prepared = prepare_at(Some(mode), 500., row, 2);
        assert_eq!(prepared.placed.len(), 1, "{mode:?}: {raw}");
        assert_eq!(prepared.placed[0].text, text);
        prepared.placed[0].screen = [100., 100.];
        let mut batch = Batch::default();
        let report = prepared.draw(
            &context,
            &mut OccupationIconView::default(),
            &mut batch,
            &mut Vec::new(),
        );
        assert!(report.refused.is_empty(), "{:?}", report.refused);
        assert_eq!((report.icons, report.badges), (1, 0));
        assert_eq!(batch.quads.len(), text.chars().count());
        assert!(batch.quads.iter().all(|quad| quad.color[3] == 1.));
    }
}

#[test]
fn occupation_grid_centres_admitted_badges_and_reuses_retained_windows() {
    content_reader_script::test_game::enter();
    use viewer_native_gui::test_support::gui;
    let mut resources = gui::fixture();
    resources.physical = Some([400, 400]);
    let windows = gui::windows(GUI);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [400, 400],
        time: Some(0.),
    };
    let mut prepared = PreparedOccupationIcons {
        priority: 30,
        placed: vec![Placed {
            state: 1,
            screen: [100., 100.],
            depth: 0.5,
            anchor: IconAnchor {
                world: [10., 0., 10.],
                cutoff: 900.,
                family: IconFamily::Occupation,
                moves: None,
            },
            text: "31%".into(),
            color: [1.; 4],
            badges: (0..4)
                .map(|index| Badge {
                    token: index.to_string(),
                    sprite: "GFX_even".into(),
                    frame: 1,
                })
                .collect(),
        }],
        ..Default::default()
    };
    let mut view = OccupationIconView::default();
    let mut batch = Batch::default();
    let mut hits = Vec::new();
    let report = prepared.draw(&context, &mut view, &mut batch, &mut hits);
    assert_eq!(
        (report.icons, report.badges),
        (1, 2),
        "{:?}",
        report.refused
    );
    assert!(
        hits.iter()
            .any(|hit| hit.hover == Hover::Resistance { state: 1 })
    );
    assert_eq!(
        hits.iter()
            .filter(|hit| matches!(hit.hover, Hover::Modifier { .. }))
            .count(),
        2
    );
    let slot = view.pool.as_ref().unwrap().slot_of(&1).unwrap();
    assert_eq!(view.entries[&slot].len(), 4);
    let controls = controls(&windows).unwrap();
    let root = windows.gui_type(WINDOW.as_bytes()).unwrap();
    assert_eq!(
        view.pool
            .as_mut()
            .unwrap()
            .get(slot)
            .unwrap()
            .control(root, controls.grid)
            .unwrap()
            .position,
        Some([-16, -9])
    );
    prepared.placed[0].badges.truncate(1);
    let report = prepared.draw(&context, &mut view, &mut batch, &mut hits);
    assert_eq!(
        (report.icons, report.badges),
        (1, 1),
        "{:?}",
        report.refused
    );
    assert_eq!(view.entries[&slot].len(), 4);
    assert_eq!(
        view.entries[&slot]
            .iter()
            .map(|entry| entry.instance.drawn())
            .collect::<Vec<_>>(),
        [true, false, false, false]
    );
}

fn lifecycle_windows(parent: &str, entry: &str) -> NativeWindows {
    let gui = std::str::from_utf8(GUI)
        .unwrap()
        .replace(
            "name = resistance_compliance_map_icon size",
            &format!("name = resistance_compliance_map_icon {parent} size"),
        )
        .replace(
            "name = resistance_compliance_map_icon_modifier_entry size",
            &format!("name = resistance_compliance_map_icon_modifier_entry {entry} size"),
        );
    viewer_native_gui::test_support::gui::windows(gui.as_bytes())
}

fn lifecycle_prepared(state: u32) -> PreparedOccupationIcons {
    PreparedOccupationIcons {
        priority: 30,
        placed: vec![Placed {
            state,
            screen: [100., 100.],
            depth: 0.5,
            anchor: IconAnchor {
                world: [10., 0., 10.],
                cutoff: 900.,
                family: IconFamily::Occupation,
                moves: None,
            },
            text: "31%".into(),
            color: [1.; 4],
            badges: vec![Badge {
                token: "retained".into(),
                sprite: "GFX_animated".into(),
                frame: 1,
            }],
        }],
        ..Default::default()
    }
}

fn lifecycle_draw(
    prepared: &PreparedOccupationIcons,
    view: &mut OccupationIconView,
    resources: &viewer_native_gui::test_support::gui::Fixture,
    windows: &NativeWindows,
    time: f32,
) -> Report {
    let context = Context {
        resources,
        windows,
        viewport: [400, 400],
        time: Some(time),
    };
    let report = prepared.draw(&context, view, &mut Batch::default(), &mut Vec::new());
    assert!(report.refused.is_empty(), "{:?}", report.refused);
    report
}

fn reference_reach(parent: &mut Instance, entry: &mut Instance, controls: &Controls<'_>) {
    for held in parent
        .take_held()
        .into_iter()
        .filter(|held| held.path == controls.grid.path)
    {
        if held.shown {
            entry.gui_show(controls.item, controls.item).unwrap();
        } else {
            entry.hide(controls.item, controls.item).unwrap();
        }
    }
}

fn reference_tick(
    parent: &mut Instance,
    entry: &mut Instance,
    root: &native::Element,
    controls: &Controls<'_>,
    resources: &dyn native_gui::Resources,
    time: f32,
    dt: f32,
) {
    let tick = viewer_native_gui::native_gui::lifecycle::Tick { time, dt };
    parent.tick_clocks(root, resources, tick).unwrap();
    entry.tick_clocks(controls.item, resources, tick).unwrap();
    assert!(entry.registration() > parent.registration());
    entry.tick_transitions(controls.item).unwrap();
    parent.tick_transitions(root).unwrap();
    reference_reach(parent, entry, controls);
    parent.tick_draws(root, resources).unwrap();
    entry.tick_draws(controls.item, resources).unwrap();
}

fn assert_same_entry(
    actual: &Instance,
    expected: &Instance,
    controls: &Controls<'_>,
    resources: &dyn native_gui::Resources,
) {
    assert_eq!(actual.drawn(), expected.drawn());
    assert_eq!(actual.position(), expected.position());
    assert!((actual.opacity() - expected.opacity()).abs() < 1e-6);
    let mut actual = actual.bindings(controls.item, resources).unwrap();
    let mut expected = expected.bindings(controls.item, resources).unwrap();
    assert_eq!(actual.bind(controls.badge), expected.bind(controls.badge));
}

#[test]
fn occupation_erased_and_outside_parents_hide_and_tick_retained_badge_rows() {
    content_reader_script::test_game::enter();
    let mut resources = viewer_native_gui::test_support::gui::fixture();
    resources.physical = Some([400, 400]);
    let windows = lifecycle_windows(
        "",
        "hide_position = { x = -20 y = 0 } hide_animation_type = linear hide_animation_time = 100",
    );
    let controls = controls(&windows).unwrap();
    let root = windows.gui_type(WINDOW.as_bytes()).unwrap();
    for outside in [false, true] {
        let mut prepared = lifecycle_prepared(1);
        let mut view = OccupationIconView::default();
        lifecycle_draw(&prepared, &mut view, &resources, &windows, 0.);
        let slot = view.pool.as_ref().unwrap().slot_of(&1).unwrap();
        let mut parent = view.pool.as_ref().unwrap().instance(slot).unwrap().clone();
        let mut entry = view.entries[&slot][0].instance.clone();
        parent.hide(root, root).unwrap();
        reference_reach(&mut parent, &mut entry, &controls);
        if outside {
            prepared.placed[0].screen = [-1000., -1000.];
        } else {
            prepared.placed.clear();
        }
        let mut last = 0.;
        for step in 1..=5 {
            let time = step as f32 * 0.1;
            reference_tick(
                &mut parent,
                &mut entry,
                root,
                &controls,
                &resources,
                time,
                (time - last).min(icon_windows::GUI_DELTA_CLAMP),
            );
            let report = lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
            assert_eq!((report.icons, report.badges), (0, 0));
            assert_same_entry(
                &view.entries[&slot][0].instance,
                &entry,
                &controls,
                &resources,
            );
            last = time;
        }
        assert!(!view.entries[&slot][0].instance.drawn());
        assert!(!view.take_running());
    }
}

#[test]
fn occupation_parent_fade_preserves_badge_clocks_through_the_delayed_hide() {
    content_reader_script::test_game::enter();
    let mut resources = viewer_native_gui::test_support::gui::fixture();
    resources.physical = Some([400, 400]);
    let windows = lifecycle_windows(
        "fade_type = linear fade_time = 200",
        "fade_type = linear fade_time = 200 hide_position = { x = -20 y = 0 } hide_animation_type = linear hide_animation_time = 100",
    );
    let controls = controls(&windows).unwrap();
    let root = windows.gui_type(WINDOW.as_bytes()).unwrap();
    let mut prepared = lifecycle_prepared(1);
    let mut view = OccupationIconView::default();
    for time in [0., 0.1, 0.2] {
        lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
    }
    let slot = view.pool.as_ref().unwrap().slot_of(&1).unwrap();
    let mut parent = view.pool.as_ref().unwrap().instance(slot).unwrap().clone();
    let mut entry = view.entries[&slot][0].instance.clone();
    parent.hide(root, root).unwrap();
    reference_reach(&mut parent, &mut entry, &controls);
    prepared.placed.clear();
    let mut last = 0.2;
    for step in 3..=10 {
        let time = step as f32 * 0.1;
        reference_tick(
            &mut parent,
            &mut entry,
            root,
            &controls,
            &resources,
            time,
            (time - last).min(icon_windows::GUI_DELTA_CLAMP),
        );
        lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
        assert_same_entry(
            &view.entries[&slot][0].instance,
            &entry,
            &controls,
            &resources,
        );
        last = time;
    }
    assert!(!parent.drawn());
    assert!(!view.entries[&slot][0].instance.drawn());
    assert!(!view.take_running());
}

#[test]
fn occupation_empty_badges_settle_before_the_same_parent_slot_is_reused() {
    content_reader_script::test_game::enter();
    let mut resources = viewer_native_gui::test_support::gui::fixture();
    resources.physical = Some([400, 400]);
    let windows = lifecycle_windows(
        "",
        "fade_type = linear fade_time = 200 hide_position = { x = -20 y = 0 } hide_animation_type = linear hide_animation_time = 100",
    );
    let controls = controls(&windows).unwrap();
    let mut prepared = lifecycle_prepared(1);
    let mut view = OccupationIconView::default();
    for time in [0., 0.1, 0.2] {
        lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
    }
    let slot = view.pool.as_ref().unwrap().slot_of(&1).unwrap();
    let registration = view.entries[&slot][0].instance.registration();
    prepared.placed[0].badges.clear();
    for time in [0.3, 0.4, 0.5, 0.6] {
        let report = lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
        assert_eq!((report.icons, report.badges), (1, 0));
    }
    assert!(!view.entries[&slot][0].instance.drawn());
    prepared = lifecycle_prepared(1);
    for time in [0.7, 0.8, 0.9] {
        lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
    }
    prepared.placed.clear();
    for time in [1., 1.1, 1.2, 1.3, 1.4] {
        lifecycle_draw(&prepared, &mut view, &resources, &windows, time);
    }
    assert!(
        !view.entries[&slot][0].instance.drawn(),
        "the detached row settles before slot reuse"
    );
    let mut expected = view.entries[&slot][0].instance.clone();
    prepared.placed = (2..=MapIconPool::Occupation.initial() as u32 + 1)
        .map(|state| lifecycle_prepared(state).placed.remove(0))
        .collect();
    lifecycle_draw(&prepared, &mut view, &resources, &windows, 1.5);
    let reused = view
        .pool
        .as_ref()
        .unwrap()
        .slot_of(&(MapIconPool::Occupation.initial() as u32 + 1))
        .unwrap();
    assert_eq!(
        reused, slot,
        "fill the round-robin pool to exercise the retained slot"
    );
    assert_eq!(view.entries[&slot][0].instance.registration(), registration);
    expected.gui_show(controls.item, controls.item).unwrap();
    let actual = &view.entries[&slot][0].instance;
    expected
        .set_position(controls.item, controls.item, actual.position())
        .unwrap();
    expected
        .set_opacity(controls.item, controls.item, 1., true)
        .unwrap();
    expected
        .tick(
            controls.item,
            &resources,
            viewer_native_gui::native_gui::lifecycle::Tick {
                time: 1.5,
                dt: (1.5_f32 - 1.4).min(icon_windows::GUI_DELTA_CLAMP),
            },
        )
        .unwrap();
    assert_same_entry(actual, &expected, &controls, &resources);
}
