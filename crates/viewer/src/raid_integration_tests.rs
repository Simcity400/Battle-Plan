//! Tests of raid draw composition with battle-plan routes and counter GUI inventory.
use installed_content::gui::native;
use raids::test_support::{DRAW_GUI, assets, frame, path_assets, put};
use replay::rows::{RaidRow, RaidScope};
use replay::{frame::DecodedFrame, rows::tables};
use viewer_foundation::map::{
    icon_windows::PoolView,
    sprites::Batch,
    ui_scale::{Raster, to_interface},
};
use viewer_foundation::map::{
    native_gui::{self},
    native_resources::Context,
    test_support::gui,
};
use {viewer_raids::map::raids, viewer_raids::map::raids::*};
#[test]
fn invasion_routes_use_the_recorded_move_world_and_native_unit_fallbacks() {
    use replay::rows::{
        ArmyOrderPlanRow, ArmyOrderRow, CountryCounterInputRow, DivisionRow, DivisionStatusRow,
        InvasionPreparationRow, LandTheatreRow, OrdersGroupMemberRow, OrdersGroupRow,
    };
    use viewer_counter_products::map::orders::invasion::Routes;
    let assets = path_assets();
    let mut frame = frame();
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    for row in &mut provinces {
        if row.id <= 2 {
            row.owner = 0;
            row.controller = 0;
            row.state = 0;
        }
        row.port_sea = match row.id {
            4 => 1,
            3 => 2,
            _ => 0,
        };
    }
    put::<tables::Provinces>(&mut frame, provinces);
    put::<tables::LandTheatres>(
        &mut frame,
        vec![LandTheatreRow {
            id: 9,
            owner: 11,
            ..Default::default()
        }],
    );
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: 77,
            roster_owner: 11,
            logical_owner: 11,
            province: 4,
            ..Default::default()
        }],
    );
    put::<tables::DivisionStatus>(
        &mut frame,
        vec![DivisionStatusRow {
            division: 77,
            ..Default::default()
        }],
    );
    let roster = |frame: &mut DecodedFrame, first_division| {
        put::<tables::CountryCounterInputs>(
            frame,
            vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 11,
                first_division,
                co_belligerents: Some(vec![]).into(),
                zero_war: Some([0, 0]).into(),
                ..Default::default()
            }],
        )
    };
    roster(&mut frame, 0);
    let operands = |frame: &mut DecodedFrame, unit_owner| {
        put::<tables::InvasionPreparations>(
            frame,
            vec![InvasionPreparationRow {
                owner_group: 8,
                instance: 1,
                unit_owner,
                ..Default::default()
            }],
        )
    };
    operands(&mut frame, 11);
    let order = ArmyOrderRow {
        owner_group: 8,
        instance: 1,
        country: 11,
        order_type: 3,
        paradrop_target: 4,
        path: vec![3],
        ..Default::default()
    };
    let group = OrdersGroupRow {
        handle: 8,
        theatre: 9,
        ..Default::default()
    };
    let plan = ArmyOrderPlanRow {
        assigned_divisions: vec![77],
        ..Default::default()
    };
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &plan, &group),
        Ok(vec![2])
    );
    let empty_plan = ArmyOrderPlanRow::default();
    // No assigned or group unit: the country's roster slot 0, or no route.
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &empty_plan, &group),
        Ok(vec![])
    );
    roster(&mut frame, 77);
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &empty_plan, &group),
        Ok(vec![2])
    );
    roster(&mut frame, 0);
    put::<tables::OrdersGroupMembers>(
        &mut frame,
        vec![OrdersGroupMemberRow {
            group: 8,
            ordinal: 0,
            unit: 77,
        }],
    );
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &empty_plan, &group),
        Ok(vec![2])
    );
    // The search reads the recorded owner's cost operands, and only its.
    for (owner, error) in [
        (12, "differs from its recorded path-cost owner"),
        (0, "differs from its recorded path-cost owner"),
    ] {
        operands(&mut frame, owner);
        assert!(
            Routes::new(&frame, Some(assets.movement_reads()))
                .route(&order, &plan, &group)
                .unwrap_err()
                .contains(error)
        );
    }
    put::<tables::InvasionPreparations>(&mut frame, vec![]);
    assert!(
        Routes::new(&frame, Some(assets.movement_reads()))
            .route(&order, &plan, &group)
            .unwrap_err()
            .contains("no recorded path-cost operands")
    );
    // Ports on one sea skip the search and its costs, but still need a unit.
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces.iter_mut().find(|p| p.id == 3).unwrap().port_sea = 1;
    put::<tables::Provinces>(&mut frame, provinces);
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &empty_plan, &group),
        Ok(vec![1])
    );
    put::<tables::OrdersGroupMembers>(&mut frame, vec![]);
    roster(&mut frame, 0);
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &empty_plan, &group),
        Ok(vec![])
    );
    roster(&mut frame, 77);
    assert_eq!(
        Routes::new(&frame, Some(assets.movement_reads())).route(&order, &empty_plan, &group),
        Ok(vec![1])
    );
}

fn hover_tooltip(offer: bool) {
    use installed_content::localisation::Localisation;
    use raids::test_support::{frame, put, registered_command_power};
    use replay::rows::{RaidTargetRow, RaidTooltipOperandRow};

    let mut assets = assets();
    assets
        .tooltip
        .state_names
        .insert(10, Ok("example_place".into()));
    if !offer {
        assets.catalog.types[0].tooltip.target_loc_key = "example_target".into();
    }
    assets.catalog.types[0].tooltip.days_to_prepare = 2;
    assets.catalog.types[0].command_power_raw = 200_000;
    let mut frame = frame();
    let scope = RaidScope {
        state: 10,
        ..Default::default()
    };
    put::<tables::Raids>(
        &mut frame,
        vec![RaidRow {
            id: 7,
            reference_type: 84,
            phase: 2,
            preparation_hours: 24,
            ..Default::default()
        }],
    );
    put::<tables::RaidTargets>(
        &mut frame,
        vec![RaidTargetRow {
            id: 7,
            reference_type: 84,
            raid_type: 100,
            scope: scope.clone(),
            ..Default::default()
        }],
    );
    let raid = 7u64 << 32 | 84;
    put::<tables::RaidTooltipOperands>(
        &mut frame,
        [
            (engine::raid_tooltip_operands::REFERENCES, 0),
            (engine::raid_tooltip_operands::REFERENCES, 1),
            (
                engine::raid_tooltip_operands::NAME,
                engine::raid_tooltip_operands::ARMY_RESOLVED,
            ),
            (
                engine::raid_tooltip_operands::NAME,
                engine::raid_tooltip_operands::WING_RESOLVED,
            ),
        ]
        .map(|(kind, key)| RaidTooltipOperandRow {
            raid,
            kind,
            key,
            ..Default::default()
        })
        .into(),
    );
    put::<tables::CountryCommandPower>(&mut frame, registered_command_power([0, 1]));
    let loc: Localisation = [
        ("example_place", "Example"),
        ("example_target", "\u{a3}unpreloaded deposit $LOCATION$"),
        ("raid_type_oilfield", "Synthetic mission"),
        ("raid_inst_item_desc", "$TYPE$ -> $TARGET$"),
        ("RAID_INST_PHASE_PREPARING", "In progress"),
        ("raid_inst_progress_desc", "$PHASE$ $PROGRESS|%0$"),
        ("tooltip_raid_view_details_prompt", "Inspect"),
        (
            "raid_insufficient_command_power",
            "Missing \u{a3}unpreloaded $COST|0$",
        ),
    ]
    .map(|(key, text)| (key.into(), text.into()))
    .into_iter()
    .collect();
    let key = if offer {
        tooltips::Key::Offer(
            vec![offers::TooltipTarget {
                scope,
                types: vec![0],
            }]
            .into(),
        )
    } else {
        tooltips::Key::Raid {
            id: 7,
            reference_type: 84,
            incoming: false,
        }
    };
    let windows = gui::windows(DRAW_GUI);
    let mut resources = gui::fixture_with(native::read(br#"guiTypes = {
            textBoxType = { name = ToolTip font = Main_14 maxWidth = 400 borderSize = { x = 2 y = 2 } }
        }"#));
    resources.add_sprite("GFX_on_map_raids_icons_frame_strip", [52, 52], 6);
    resources.add_sprite("glyph", [32, 32], 1);
    resources.add_sprite("plus", [32, 32], 1);
    // GUI loading uses this production inventory. A declared sprite omitted
    // from it refuses the native text bake.
    let inventory = viewer_counters::map::counters::gui_assets(
        &windows,
        &native::resources::Registry::default(),
        &loc,
        native_gui::Assets::from_localisation(&loc),
    );
    if inventory.sprites.contains(b"GFX_unpreloaded".as_slice()) {
        resources.add_sprite("GFX_unpreloaded", [12, 12], 1);
    }
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [2048, 1152],
        time: Some(0.),
    };
    let prepared = PreparedRaids::test_icon(
        [100., 200.],
        RaidIcon {
            province: 1,
            frame: if offer { 0 } else { 2 },
            enabled: !offer,
            glyph: Some("glyph".into()),
            plus: false,
            tooltip: Some(key.clone()),
        },
    );
    let mut hits = Vec::new();
    let mut report = RaidReport::default();
    let mut batch = Batch::default();
    assert_eq!(
        prepared.draw(
            &context,
            assets.priority,
            &mut PoolView::default(),
            &mut batch,
            &mut hits,
            &mut report
        ),
        1
    );
    assert!(report.reasons.is_empty(), "{:?}", report.reasons);
    assert_eq!(hits[0].origin, [125., 175.]);
    assert_eq!(hits[0].size, [52., 52.]);
    // Interactive pointer conversion: undo monitor stretch and GUI scale.
    let raster = Raster {
        ui_scale: 1.25,
        stretch: 1.5,
    };
    let cursor = [150., 200.].map(|pixel| pixel * raster.screen_scale());
    let point = to_interface(cursor.map(|pixel| pixel / raster.stretch), raster.ui_scale);
    let hit = hits
        .iter()
        .rev()
        .find(|hit| hit.contains(point))
        .expect("raid owns hover");
    assert_eq!(hit.key, key);
    let tooltip = tooltips::build_from(tooltips::Inputs {
        assets: &assets,
        frame: &frame,
        ours: if offer { 1 } else { 0 },
        nato: false,
        key: &hit.key,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &Err("no scripted substitutions in this fixture".into()),
        flag_exists: &|_| true,
    });
    assert!(!tooltip.title.is_empty(), "{:?}", tooltip.refused);
    assert!(tooltip.title.contains("\u{13}unpreloaded"), "{tooltip:?}");
    let layout = native_gui::text_sprites::tooltip(&resources, &tooltip.title, 1., true)
        .expect("hovered raid produces a native tooltip drawable");
    let bake = layout
        .parts
        .iter()
        .find_map(|part| match &part.kind {
            native_gui::PartKind::TextSprite { bake, .. } => Some(bake),
            _ => None,
        })
        .expect("native tooltip text bake");
    assert!(bake.size.iter().all(|size| *size > 0));
    assert!(!bake.glyphs.is_empty());
    assert_eq!(bake.inline.len(), 1);
}

#[test]
fn raid_tooltip_regression_active_hover_loads_substituted_target_icon() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    hover_tooltip(false);
}

#[test]
fn raid_tooltip_regression_disabled_offer_hover_loads_diagnostic_icon() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    hover_tooltip(true);
}

#[test]
fn a_stage_threshold_is_crossed_at_its_sub_hour_moment() {
    use viewer_foundation::map::unit_pose::{animation_state, test_support::line_track};
    // Progress runs 0.5 to 0.75 over the hour; a stage at 0.625 is
    // reached halfway through it.
    let track = line_track(60_000, 1_500_000);
    let stages = [
        installed_content::raids::RaidAnimationStage {
            state: "cruise".into(),
            intro: None,
            at_progress_raw: 0,
        },
        installed_content::raids::RaidAnimationStage {
            state: "dive".into(),
            intro: Some("dive_intro".into()),
            at_progress_raw: 62_500,
        },
    ];
    let every = |_: &str| true;
    let stage = |tick: f32| animation_state(&stages, track.progress(tick), &every).unwrap();
    assert_eq!(stage(0.0).state, "cruise");
    assert_eq!(stage(0.49).state, "cruise");
    assert_eq!(stage(0.5).state, "dive");
    assert_eq!(stage(0.75).state, "dive");
    // Playing frame by frame on the viewer's clock, the intro starts on the
    // first frame past the threshold and hands over to the state.
    let rig = viewer_world::map::entity_animation::stage_rig(true);
    let mut player = viewer_world::map::entity_animation::Player::new(&rig);
    let mut started = None;
    for frame in 0..=100 {
        let tick = frame as f32 / 100.0;
        let chosen = stage(tick);
        let code = viewer_world::map::entity_animation::named_stage(
            &chosen.state,
            chosen.intro.as_deref(),
        );
        // A tenth of an hour a second: ten seconds of wall clock an hour.
        let time = 1.0 + tick * 10.0;
        let playing =
            viewer_world::map::entity_animation::root_state(&mut player, &rig, code, time);
        if playing == "dive_intro" && started.is_none() {
            started = Some(tick);
        }
    }
    assert_eq!(started, Some(0.5));
    let code = viewer_world::map::entity_animation::named_stage("dive", Some("dive_intro"));
    assert_eq!(
        viewer_world::map::entity_animation::root_state(&mut player, &rig, code, 12.0),
        "dive"
    );
}
