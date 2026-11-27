use super::with_counter_inputs_and_content;

use super::{Allegiance, Arc, BTreeSet, BaseContext, BaseIcons, BaseView, Batch, BuildingIconContent, CombatFlags, CounterColorMode, CounterInputs, DecodedFrame, Engaged, History, HourTables, LAND, LAND_ICON, LandMarker, LandMarkers, Layouts, MissionMemo, NativeWindows, NavalBaseIcon, NavalMarkerTables, PoolView, Preparation, Provenance, ProvinceKind, STANDSTILL, Shared, Unit, Viewer, Vision, action_gui, building_facts, counter_order, countries, hour_tables, land_unit, land_windows, layout_gui, native, naval_windows, prepare_cached, prepare_retained, stack_anchor, type_tint};

#[test]
fn retained_semantic_plans_match_fresh_through_camera_thresholds_and_admission() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(br#"guiTypes={
        containerWindowType={name=unit_mapicon size={width=81 height=29}
            gridBoxType={name=units_box size={width=81 height=290}
                slotsize={width=81 height=1} max_slots_horizontal=1 format=UPPER_LEFT}
        }
        positionType={name=unit_counter_zoomed_in_offset position={x=0 y=0}}
        containerWindowType={name=unit_counter_navy size={width=20 height=10}}
        containerWindowType={name=naval_headquarter_mapicon size={width=81 height=29}
            buttonType={name=bg_btn spriteType=GFX_synthetic_plate}}
        containerWindowType={name=naval_combat_mapicon size={width=20 height=10}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions position={x=57 y=9} frame=12}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay position={x=3 y=4}}
        }
    }"#);
    with_counter_inputs_and_content(
        guis,
        |assets| {
            assets.layouts = Layouts::read(&assets.windows);
        },
        |content| {
            let points = [
                (1, [2., 9.5, 2.]),
                (2, [20., 9.5, 2.]),
                (3, [100., 9.5, 100.]),
            ];
            content.default_anchors = Arc::new(points.into());
            content.bounds = Arc::new(
                points
                    .into_iter()
                    .map(|(id, [x, _, z])| (id, [x - 1., z - 1., x + 1., z + 1.]))
                    .collect(),
            );
            content.region_of = Arc::new([(1, 1), (2, 1), (3, 2)].into());
        },
        |mut inputs, content| {
            use viewer_counter_kit::plan_probe;
            use replay_data::rows::{TableId, TableRows};
            use replay_frame::frame::TableView;
            let frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let mut tables = synthetic_hour(content, &inputs, &frame);
            tables.divisions = [
                (
                    (1, STANDSTILL),
                    vec![
                        land_unit(10, 1, 10, 10, true),
                        land_unit(11, 1, 10, 10, true),
                    ],
                ),
                ((2, STANDSTILL), vec![land_unit(12, 1, 10, 10, true)]),
                ((3, STANDSTILL), vec![land_unit(13, 1, 20, 20, true)]),
            ]
            .into();
            tables.base_icons.headquarters = vec![(1, [2., 9.5, 2.]), (3, [100., 9.5, 100.])];
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            tables.countries = Arc::new(countries([(1, Allegiance::Ours)]));
            let selected = BTreeSet::from([Unit::Division(10)]);
            let empty = BTreeSet::new();
            let mut retained = Preparation::default();
            let mut windows = [
                naval_windows::Windows::default(),
                naval_windows::Windows::default(),
            ];
            let schedules: [Shared; 2] = std::array::from_fn(|_| Shared::default());
            let mut inputs = CounterInputs {
                selected: &empty,
                schedule: &schedules[0],
                ..inputs
            };
            let mut pings = [PoolView::default(), PoolView::default()];
            let mut bases = [BaseView::default(), BaseView::default()];
            let mut combats = [land_windows::View::default(), land_windows::View::default()];
            let mut hits = 0;
            let mut equal_groups = 0;
            let mut drawn = 0;
            let mut counter_drawn = 0;
            let groups = inputs.content.defines.groups;
            let low = groups.province_distance - 10.0;
            let heights = [
                low,
                low,
                groups.province_distance - 0.001,
                groups.province_distance,
                groups.province_distance,
                groups.state_distance - 0.001,
                groups.state_distance,
                groups.state_distance,
                groups.strategic_distance - 0.001,
                groups.strategic_distance,
                groups.strategic_distance,
                inputs.content.defines.units_cutoff - 0.001,
                inputs.content.defines.units_cutoff,
                low,
                low,
            ];
            use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
            use viewer_gpu::camera::{Free, Lens, Pose};
            let flown = [
                ([-20., 2.], 30., FRAC_PI_2, -0.4),
                ([-18., 2.], 30., FRAC_PI_2, -0.4),
                ([-16., 2.5], 30., FRAC_PI_2 + 0.1, -0.4),
                ([-16., 2.5], 30., FRAC_PI_2 + 0.3, -0.5),
                ([80., 80.], 40., FRAC_PI_4, -0.5),
                ([82., 81.], 40., FRAC_PI_4 + 0.05, -0.5),
            ];
            let frames = heights
                .into_iter()
                .enumerate()
                .map(|(at, height)| {
                    let center = if at == 4 || at == 8 { [100., 100.] } else { [2. + at as f32 * 0.125, 2.] };
                    (center, height, None)
                })
                .chain(flown.map(|(center, height, yaw, pitch)| (center, height, Some(Free::new(yaw, pitch, Lens::MAP)))));
            let (mut free_bases, mut free_columns) = (0, 0);
            for (at, (center, height, free)) in frames.enumerate() {
                inputs.camera.height = height;
                inputs.camera.center = center;
                if let Some(free) = free {
                    inputs.camera.pose = Pose::Free(free);
                }
                inputs.selected = if (5..10).contains(&at) {
                    &selected
                } else {
                    &empty
                };
                let (mut cached, counts) = plan_probe::collect(|| {
                    prepare_retained(&inputs, &tables, 0, &mut retained).unwrap()
                });
                let mut fresh = prepare_cached(&inputs, &tables).unwrap();
                hits += counts.semantic_plan_hits;
                if free.is_some() {
                    free_bases += counts.base_plan_hits;
                    free_columns += counts.column_plan_hits;
                }
                equal_groups += counts.ordered_group_members_equal;
                assert_eq!(
                    cached.fixed.report, fresh.fixed.report,
                    "prepare frame {at}"
                );
                for (side, prepared) in [&mut fresh, &mut cached].into_iter().enumerate() {
                    inputs.schedule = &schedules[side];
                    prepared
                        .draw(
                            &inputs,
                            at as f32 / 240.,
                            &mut windows[side],
                            &mut pings[side],
                            &mut bases[side],
                            &mut combats[side],
                        )
                        .unwrap();
                }
                counter_drawn += fresh.drawn.batch.quads.len();
                assert_eq!(
                    fresh.drawn.report, cached.drawn.report,
                    "draw report frame {at}"
                );
                for (a, b) in [
                    (&fresh.fixed.pings, &cached.fixed.pings),
                    (&fresh.fixed.bases, &cached.fixed.bases),
                    (&fresh.fixed.combats, &cached.fixed.combats),
                    (&fresh.drawn.batch, &cached.drawn.batch),
                    (&fresh.drawn.combats, &cached.drawn.combats),
                ] {
                    assert_quad_bits(a, b);
                    drawn += a.quads.len();
                }
            }
            assert!(
                hits > 0,
                "differential fixture must actually reach semantic hits"
            );
            assert!(free_bases > 0, "a flying free eye keeps its base icons' layout");
            assert!(free_columns > 0, "and a land column's plan");
            assert!(
                equal_groups > 0,
                "must hit a nonempty ordered column, not just empty bases"
            );
            assert!(drawn > 0, "a refusal-only fixture is not a drawing proof");
            assert!(
                counter_drawn > 0,
                "must emit a land column, not just base icons"
            );
        },
    );
}

fn assert_quad_bits(a: &Batch, b: &Batch) {
    assert_eq!(a.quads, b.quads);
    for (a, b) in a.quads.iter().zip(&b.quads) {
        assert_eq!(a.rect.map(f32::to_bits), b.rect.map(f32::to_bits));
        assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
        assert_eq!(a.color.map(f32::to_bits), b.color.map(f32::to_bits));
        assert_eq!(a.rotation.map(f32::to_bits), b.rotation.map(f32::to_bits));
        assert_eq!(a.shade.to_bits(), b.shade.to_bits());
        if let (Some(a), Some(b)) = (a.anchor, b.anchor) {
            assert_eq!(a.world.map(f32::to_bits), b.world.map(f32::to_bits));
            assert_eq!(a.cutoff.to_bits(), b.cutoff.to_bits());
        }
        if let (Some(a), Some(b)) = (a.present, b.present) {
            assert_eq!(a.pivot.map(f32::to_bits), b.pivot.map(f32::to_bits));
            assert_eq!(a.scale.to_bits(), b.scale.to_bits());
            assert_eq!(a.alpha.to_bits(), b.alpha.to_bits());
        }
        if let (Some(a), Some(b)) = (&a.exact, &b.exact) {
            assert_eq!(
                a.corners.map(|v| v.map(f32::to_bits)),
                b.corners.map(|v| v.map(f32::to_bits))
            );
            assert_eq!(
                a.uv.map(|v| v.map(f32::to_bits)),
                b.uv.map(|v| v.map(f32::to_bits))
            );
            for (a, b) in [
                (a.lod_bias.as_slice(), b.lod_bias.as_slice()),
                (&a.offsets, &b.offsets),
                (&a.inputs, &b.inputs),
                (&a.second_color, &b.second_color),
            ] {
                assert_eq!(
                    a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                );
            }
            if let (Some(a), Some(b)) = (&a.native, &b.native) {
                assert_native_bits(a, b);
            }
        }
    }
}

fn assert_native_bits(
    a: &viewer_native_gui::native_shader::draw::Quad,
    b: &viewer_native_gui::native_shader::draw::Quad,
) {
    assert_eq!(a, b);
    assert_eq!(
        a.constants.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.constants.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    for ((_, a, _), (_, b, _)) in a.extra_constants.iter().zip(&b.extra_constants) {
        assert_eq!(
            a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }
    if let (Some(a), Some(b)) = (&a.bake, &b.bake) {
        for (a, b) in a.draws.iter().zip(&b.draws) {
            assert_native_bits(a, b);
        }
    }
}

fn synthetic_hour(content: &super::CounterContent, inputs: &CounterInputs<'_>, frame: &DecodedFrame) -> HourTables {
    synthetic_hour_with_history(content, inputs, frame, None)
}

fn synthetic_hour_with_history(
    content: &super::CounterContent,
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    history: Option<&History>,
) -> HourTables {
    let moves = viewer_unit_reads::movement_reads::test_support::unnamed_assets();
    hour_tables(
        frame,
        inputs.map,
        Arc::new(Vision::sharing(1, [])),
        content,
        inputs.assets,
        inputs.equipment,
        inputs.localisation,
        &Default::default(),
        None,
        history,
        None,
        &type_tint::Memo::new(type_tint::TABLES),
        &MissionMemo::new(),
        moves.movement_reads(),
        Err("unreached synthetic naval topology".into()),
    )
    .unwrap()
}

#[test]
fn retained_semantic_plan_key_invalidates_every_non_camera_input_it_reuses() {
    content_reader_script::test_game::enter();
    with_counter_inputs_and_content(
        action_gui(false),
        |_| {},
        |_| {},
        |inputs, content| {
            use viewer_counter_kit::plan_probe;
            use replay_data::rows::{TableId, TableRows};
            use replay_frame::frame::TableView;
            let frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let mut tables = synthetic_hour(content, &inputs, &frame);
            let mut memo = Preparation::default();
            let selected = BTreeSet::from([Unit::Division(10)]);
            let other_windows = NativeWindows::read(
                br#"guiTypes={
            containerWindowType={name=unit_counter_navy}
            containerWindowType={name=naval_combat_mapicon}
        }"#,
                [400, 400],
            )
            .unwrap();
            let empty = BTreeSet::new();
            let mut inputs = CounterInputs {
                selected: &empty,
                ..inputs
            };
            let mut resources = 0;
            for change in 0..12 {
                match change {
                    0 => {}
                    1 => resources += 1,
                    2 => inputs.viewport[0] += 1,
                    3 => inputs.ui_scale = 1.25,
                    4 => inputs.settings.nato = !inputs.settings.nato,
                    5 => inputs.visibility.province_icons = !inputs.visibility.province_icons,
                    6 => inputs.selected = &selected,
                    7 => inputs.selected_air = &[11],
                    8 => inputs.selected_air_bases = &[12],
                    9 => inputs.selected_fleets = &[13],
                    10 => inputs.hovered = Some(Unit::Division(10)),
                    11 => inputs.windows = &other_windows,
                    _ => unreachable!(),
                }
                for repeat in 0..2 {
                    inputs.camera.center[0] += 0.125;
                    let (cached, counts) = plan_probe::collect(|| {
                        prepare_retained(&inputs, &tables, resources, &mut memo).unwrap()
                    });
                    let fresh = prepare_cached(&inputs, &tables).unwrap();
                    assert_eq!(
                        cached.fixed.report, fresh.fixed.report,
                        "change {change} repeat {repeat}"
                    );
                    assert_eq!(counts.semantic_plans, 1);
                    assert_eq!(counts.semantic_plan_hits, repeat, "change {change}");
                }
            }
            tables = tables.clone();
            assert_eq!(
                plan_probe::collect(|| prepare_retained(
                    &inputs, &tables, resources, &mut memo
                )
                .unwrap())
                .1
                .semantic_plan_hits,
                1
            );
            tables.0.epoch = Arc::new(());
            assert_eq!(
                plan_probe::collect(|| prepare_retained(
                    &inputs, &tables, resources, &mut memo
                )
                .unwrap())
                .1
                .semantic_plan_hits,
                0
            );
        },
    );
}

#[test]
fn arrival_refinement_matches_fixed_expectations_through_publication_and_view_changes() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(br#"guiTypes={
        containerWindowType={name=unit_mapicon size={width=81 height=29}
            gridBoxType={name=units_box size={width=81 height=290}
                slotsize={width=81 height=1} max_slots_horizontal=1 format=UPPER_LEFT}
        }
        positionType={name=unit_counter_zoomed_in_offset position={x=0 y=0}}
        containerWindowType={name=unit_counter_navy size={width=20 height=10}}
        containerWindowType={name=naval_headquarter_mapicon size={width=81 height=29}
            buttonType={name=bg_btn spriteType=GFX_synthetic_plate}}
        containerWindowType={name=naval_combat_mapicon size={width=20 height=10}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions position={x=57 y=9} frame=12}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay position={x=3 y=4}}
        }
    }"#);
    with_counter_inputs_and_content(
        guis,
        |assets| {
            assets.layouts = Layouts::read(&assets.windows);
        },
        |content| {
            let points = [
                (1, [2., 9.5, 2.]),
                (2, [20., 9.5, 2.]),
                (3, [100., 9.5, 100.]),
            ];
            content.default_anchors = Arc::new(points.into());
            content.bounds = Arc::new(
                points
                    .into_iter()
                    .map(|(id, [x, _, z])| (id, [x - 1., z - 1., x + 1., z + 1.]))
                    .collect(),
            );
            content.region_of = Arc::new([(1, 1), (2, 1), (3, 2)].into());
        },
        |mut inputs, content| {
            use replay_data::rows::{
                CountryLinkRow, CountryRow, DivisionMovementRow, DivisionRow, LandCombatRow,
                LandCombatSideRow, NavalCombatRow, TableId, TableRows,
            };
            use replay_frame::frame::TableView;
            use viewer_engagements::engagement_history::{Covered, NAVAL, NAVAL_ICON};
            use viewer_engagements::icon_frames::Kept;
            let mut frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                [1, 5]
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("T{id}").into_bytes(),
                        ..Default::default()
                    })
                    .to_vec(),
            ));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    [1, 5]
                        .map(|id| CountryLinkRow {
                            id,
                            canonical_slot: id,
                        })
                        .to_vec(),
                ));
            *frame.tables[TableId::Divisions.index()] =
                TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                    id: 10,
                    province: 1,
                    organisation: 10,
                    ..Default::default()
                }]));
            *frame.tables[TableId::DivisionMovement.index()] =
                TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
                    division: 10,
                    ..Default::default()
                }]));
            *frame.tables[TableId::LandCombats.index()] =
                TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                    id: 61,
                    province: 1,
                    attacker_classes: vec![0],
                    attackers: vec![10],
                    defenders: vec![10],
                    ..Default::default()
                }]));
            *frame.tables[TableId::LandCombatSides.index()] =
                TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
                    combat: 61,
                    attacker_primary: vec![1],
                    defender_primary: vec![5],
                    ..Default::default()
                }]));
            *frame.tables[TableId::NavalCombats.index()] =
                TableView::Complete(TableRows::NavalCombats(vec![NavalCombatRow {
                    id: 81,
                    province: 2,
                    ..Default::default()
                }]));
            let populate = |tables: &mut HourTables| {
                tables.divisions = [(
                    (1, STANDSTILL),
                    vec![
                        land_unit(10, 1, 10, 10, true),
                        land_unit(11, 1, 10, 10, true),
                    ],
                )]
                .into();
                tables.base_icons.headquarters = vec![(1, [2., 9.5, 2.])];
                tables.countries = Arc::new(countries([(1, Allegiance::Ours)]));
            };
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            let selected = BTreeSet::from([Unit::Division(10)]);
            let empty = BTreeSet::new();
            let mut total_drawn = 0;
            let mut actual_markers = 0;
            for case in 0..6 {
                let history = History::default();
                let mut tables = synthetic_hour_with_history(content, &inputs, &frame, Some(&history));
                populate(&mut tables);
                assert!(tables.is_for_frame(&frame.clone()));
                let original_epoch = Arc::clone(&tables.0.epoch);
                let original_base = Arc::clone(&tables.0.base);
                let mut memo = Preparation::default();
                let mut windows = [
                    naval_windows::Windows::default(),
                    naval_windows::Windows::default(),
                ];
                let mut pings = [PoolView::default(), PoolView::default()];
                let mut bases = [BaseView::default(), BaseView::default()];
                let mut combats = [land_windows::View::default(), land_windows::View::default()];
                let schedules: [Shared; 2] = std::array::from_fn(|_| Shared::default());
                let mut inputs = CounterInputs {
                    selected: &empty,
                    schedule: &schedules[0],
                    ..inputs
                };
                for at in 0..10 {
                    if at == 3 {
                        let mut covered = Covered::new(frame.hour);
                        if case != 0 {
                            covered
                                .viewers
                                .push((frame.hour, if case == 1 || case == 5 { 5 } else { 1 }));
                        }
                        let flag = match case {
                            2 => Engaged::Unknown,
                            3 | 5 => Engaged::Yes,
                            _ => Engaged::No,
                        };
                        for (kind, id) in [(LAND, 61), (NAVAL, 81)] {
                            covered.fold(false).record(
                                kind,
                                frame.hour,
                                &[((id, 0), flag), ((id, 1), flag)],
                                &[],
                            );
                            covered.fold(true).record(
                                kind,
                                frame.hour,
                                &[((id, 0), Engaged::Yes)],
                                &[],
                            );
                        }
                        for (kind, id) in [(LAND_ICON, 61), (NAVAL_ICON, 81)] {
                            covered.fold(false).record_icons(
                                kind,
                                frame.hour,
                                &[(id, Some(Kept::Frame(7)))],
                            );
                            covered.fold(true).record_icons(
                                kind,
                                frame.hour,
                                &[(id, Some(Kept::Frame(2)))],
                            );
                        }
                        if case == 5 {
                            covered.players.push((frame.hour, Some(1)));
                        }
                        history
                            .branches
                            .lock()
                            .unwrap()
                            .insert(frame.branch, covered);
                        assert!(history.covers(frame.branch, frame.hour));
                        tables = tables.refine_engagement(Some(&history)).unwrap();
                        assert!(Arc::ptr_eq(&original_base, &tables.0.base));
                        assert!(!Arc::ptr_eq(&original_epoch, &tables.0.epoch));
                    }
                    let groups = inputs.content.defines.groups;
                    let low = groups.province_distance - 10.0;
                    inputs.camera.height = [
                        low,
                        groups.province_distance,
                        groups.state_distance,
                        low,
                        groups.strategic_distance,
                        low,
                        low,
                        groups.state_distance,
                        low,
                        low,
                    ][at];
                    inputs.camera.center = [2. + at as f32 * 0.125, 2.];
                    inputs.selected = if (4..8).contains(&at) {
                        &selected
                    } else {
                        &empty
                    };
                    inputs.hovered = (at == 5).then_some(Unit::Division(10));
                    inputs.visibility.unit_counters = at != 6;
                    inputs.schedule = &schedules[0];
                    let provenance = if at < 3 {
                        Provenance::Pending
                    } else {
                        match case {
                            0 => Provenance::Unread,
                            1 => Provenance::OtherViewer,
                            _ => Provenance::Indexed,
                        }
                    };
                    let (land, naval) = if at < 3 || case < 2 {
                        (Engaged::Yes, Engaged::Unknown)
                    } else {
                        let flag = match case {
                            2 => Engaged::Unknown,
                            4 => Engaged::No,
                            _ => Engaged::Yes,
                        };
                        (flag, flag)
                    };
                    let flags = CombatFlags {
                        land: [(61, land)].into(),
                        naval: [(81, naval)].into(),
                        land_provenance: provenance,
                        naval_provenance: provenance,
                    };
                    let mut land_combats = LandMarkers::default();
                    if let Some(limit) = provenance.limitation() {
                        land_combats.limits.insert(limit.to_owned(), [61].into());
                    }
                    if land == Engaged::Unknown {
                        land_combats.gaps.insert(
                            "icon omitted: engaged flag unknown (no refresh seen since the branch joined a running game or skipped hours, or an input the test reads is unrecorded)".into(),
                            [61].into(),
                        );
                    } else {
                        let seen = land == Engaged::Yes;
                        land_combats.markers.push(LandMarker {
                            id: 61,
                            province: 1,
                            attacker: None,
                            seen,
                            progress: seen.then_some(5_000_000),
                            tint: (!seen).then_some([1.0; 4]),
                            frame: Some(if seen { 0 } else { 7 }),
                            bombardment: false,
                        });
                        if seen {
                            land_combats.gaps.insert(
                                "plate and wedge omitted (survival estimate): the LandCombatDetails table has no row for the combat".into(),
                                [61].into(),
                            );
                        } else {
                            land_combats.limits.insert(
                                viewer_engagements::icon_frames::CARRIED.to_owned(),
                                [61].into(),
                            );
                        }
                    }
                    assert_eq!(tables.0.engaged.land, flags.land, "case {case} frame {at}");
                    assert_eq!(tables.0.engaged.naval, flags.naval, "case {case} frame {at}");
                    assert_eq!(tables.0.engaged.land_provenance, provenance);
                    assert_eq!(tables.0.engaged.naval_provenance, provenance);
                    assert_eq!(tables.0.land_combats.markers, land_combats.markers);
                    assert_eq!(tables.0.land_combats.gaps, land_combats.gaps);
                    assert_eq!(tables.0.land_combats.limits, land_combats.limits);
                    let kept = if at < 3 || case < 2 {
                        Kept::Undecided
                    } else {
                        Kept::Frame(if case == 5 { 2 } else { 7 })
                    };
                    let naval_markers = NavalMarkerTables::read(
                        &frame,
                        Viewer {
                            vision: &original_base.vision,
                            owner: &|raw| original_base.links().get(&raw).copied(),
                        },
                        &flags,
                        &[(81, kept)].into(),
                        &|province| inputs.content.kinds.get(&province).copied(),
                        &|province| {
                            stack_anchor(
                                &inputs.content.anchors,
                                &inputs.content.default_anchors,
                                province,
                                STANDSTILL,
                            )
                        },
                    )
                    .unwrap();
                    let expected_tables = HourTables(viewer_counter_kit::HourTables {
                        epoch: Arc::new(()),
                        base: Arc::clone(&original_base),
                        land_combats: Arc::new(land_combats),
                        naval_markers: Arc::new(naval_markers),
                        engaged: Arc::new(flags),
                    });
                    actual_markers += tables.0.land_combats.markers.len();
                    let mut expected = prepare_retained(
                        &inputs,
                        &expected_tables,
                        0,
                        &mut Preparation::default(),
                    )
                    .unwrap();
                    let mut refined =
                        prepare_retained(&inputs, &tables, 0, &mut memo).unwrap();
                    assert_eq!(
                        expected.fixed.report, refined.fixed.report,
                        "case {case} frame {at}"
                    );
                    for (side, prepared) in [&mut expected, &mut refined].into_iter().enumerate() {
                        inputs.schedule = &schedules[side];
                        prepared
                            .draw(
                                &inputs,
                                at as f32 / 240.,
                                &mut windows[side],
                                &mut pings[side],
                                &mut bases[side],
                                &mut combats[side],
                            )
                            .unwrap();
                    }
                    assert_eq!(
                        expected.drawn.report, refined.drawn.report,
                        "case {case} frame {at}"
                    );
                    for (a, b) in [
                        (&expected.fixed.pings, &refined.fixed.pings),
                        (&expected.fixed.bases, &refined.fixed.bases),
                        (&expected.fixed.combats, &refined.fixed.combats),
                        (&expected.drawn.batch, &refined.drawn.batch),
                        (&expected.drawn.combats, &refined.drawn.combats),
                    ] {
                        assert_quad_bits(a, b);
                    }
                    total_drawn += expected.drawn.batch.quads.len();
                }
                assert_eq!(
                    tables.0.engaged.land_provenance,
                    match case {
                        0 => Provenance::Unread,
                        1 => Provenance::OtherViewer,
                        _ => Provenance::Indexed,
                    }
                );
                if case == 4 {
                    assert_eq!(tables.0.land_combats.markers[0].frame, Some(7));
                    assert!(!tables.0.land_combats.markers[0].seen);
                }
                if case == 5 {
                    assert_eq!(tables.0.engaged.land[&61], Engaged::Yes);
                }
                let mut changed = frame.clone();
                changed.branch += 1;
                assert!(!tables.is_for_frame(&changed));
                changed = frame.clone();
                changed.hour += 1;
                assert!(!tables.is_for_frame(&changed));
                for table in TableId::ALL {
                    changed = frame.clone();
                    changed.tables[table.index()] = (*changed.tables[table.index()]).clone().into();
                    assert!(!tables.is_for_frame(&changed), "{table:?}");
                    changed = frame.clone();
                    changed.tables[table.index()] = changed.tables[table.index()]
                        .clone()
                        .with_sampled_at(Some(frame.hour));
                    assert!(
                        !tables.is_for_frame(&changed),
                        "supplying hour for {table:?}"
                    );
                }
            }
            assert!(total_drawn > 0, "must compare real emitted counter quads");
            assert!(
                actual_markers > 0,
                "must exercise nonempty history-dependent marker products"
            );
        },
    );
}

#[test]
fn base_icon_sorted_lookups_keep_expected_state_refusals_and_output_across_hours() {
    use replay_data::rows::{
        BuildingRow, CountryLinkRow, CountryRow, FacilityMapFlagRow, LexicalTokenRow, NavalBaseRow,
        ProvinceRow, SimulationFeatureRow, TableId, TableRows,
    };
    use replay_frame::frame::TableView;
    use viewer_gpu::province_map::ProvinceMap;
    content_reader_script::test_game::enter();
    let stack = content_reader_layers::test_support::Stack::new("base-icon-lookups");
    let base = stack.base();
    stack.write(&base, "gfx/FX/defines_hlsl_dx11.fxh", "");
    stack.write(
        &base,
        "gfx/FX/buttonstate.shader",
        r#"
        VertexStruct I { float3 p : POSITION; };
        VertexStruct O { float4 p : SV_POSITION; };
        VertexShader = { MainCode V [[ O main(I v) { O o; o.p=float4(v.p,1); return o; } ]] }
        PixelShader = { MainCode P [[ float4 main(O v) : SV_TARGET { return float4(0,1,0,1); } ]] }
        Effect Up { VertexShader = V PixelShader = P }
    "#,
    );
    stack.write(
        &base,
        "interface/base-icons.gfx",
        r#"spriteTypes={
        spriteType={name=GFX_synthetic_plate textureFile="gfx/plate.tga"}
        spriteType={name=GFX_mapicon_facility_air textureFile="gfx/plate.tga"}
        spriteType={name=GFX_mapicon_facility_naval textureFile="gfx/plate.tga"}
    }"#,
    );
    let mut image = vec![0_u8; 18];
    image[2] = 2;
    image[12..14].copy_from_slice(&81_u16.to_le_bytes());
    image[14..16].copy_from_slice(&29_u16.to_le_bytes());
    image[16] = 32;
    image[17] = 8;
    image.resize(18 + 81 * 29 * 4, 255);
    std::fs::write(base.join("gfx/plate.tga"), image).unwrap();
    let layers = stack.layers(&[]);
    let guis = layout_gui(
        br#"guiTypes={
        containerWindowType={name=unit_mapicon size={width=81 height=29}
            gridBoxType={name=units_box size={width=81 height=290}
                slotsize={width=81 height=1} max_slots_horizontal=1 format=UPPER_LEFT}}
        positionType={name=unit_counter_zoomed_in_offset position={x=0 y=0}}
        containerWindowType={name=unit_counter_navy size={width=20 height=10}}
        containerWindowType={name=naval_combat_mapicon size={width=20 height=10}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg quadTextureSprite=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions frame=12}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
        containerWindowType={name=naval_headquarter_mapicon size={width=81 height=29}
            buttonType={name=bg_btn quadTextureSprite=GFX_synthetic_plate}}
        containerWindowType={name=special_facility_mapicon size={width=81 height=29}
            buttonType={name=bg_btn quadTextureSprite=GFX_synthetic_plate}}
        containerWindowType={name=naval_base_mapicon size={width=81 height=29}
            buttonType={name=bg_btn quadTextureSprite=GFX_synthetic_plate}}
    }"#,
    );
    with_counter_inputs_and_content(
        guis,
        |assets| {
            assets.layouts = Layouts::read(&assets.windows);
            assets.gui = viewer_native_gui::native_resources::GuiResources::new(
                native::resources::Registry::load(&layers).unwrap(),
                b"english",
                [4, 4],
                Default::default(),
            );
            assets
                .gui
                .load(
                    &layers,
                    &viewer_native_gui::native_gui::Assets {
                        sprites: [
                            b"GFX_synthetic_plate".to_vec(),
                            b"GFX_mapicon_facility_air".to_vec(),
                            b"GFX_mapicon_facility_naval".to_vec(),
                        ]
                        .into(),
                        ..Default::default()
                    },
                    &mut Vec::new(),
                )
                .unwrap();
        },
        |content| {
            let definitions = content_reader_key_names::buildings::parse(
                b"buildings={hq_first={naval_headquarter=yes} hq_second={naval_headquarter=yes}
                fac_a={specialization={air}} fac_b={specialization={naval}}
                ambiguous={naval_headquarter=yes specialization={land}}
                ambiguous={naval_headquarter=no specialization={nuclear}}}",
                &content_reader_script::key_names::KeyNames::default(),
            )
            .unwrap();
            content.building_icons = BuildingIconContent::load(
                &definitions,
                &[],
                &ProvinceMap {
                    width: 1,
                    height: 1,
                    ids: &[1],
                },
                |_, _, _| building_facts::SlotAdmission::Admitted,
                [[0.; 3]; 2],
            );
            content.default_anchors = Arc::new([(1, [2., 9.5, 2.]), (3, [3., 9.5, 2.])].into());
            content.ports = [(1, [2., 9.5, 2.]), (3, [3., 9.5, 2.])].into();
            content.kinds = Arc::new([(1, ProvinceKind::Land), (3, ProvinceKind::Land)].into());
        },
        |inputs, content| {
            let links = [(10, 1), (20, 2)].into();
            let ids = [(1, 10), (2, 20)].into();
            let vision = Vision::sharing(1, []);
            let schedules: [Shared; 2] = std::array::from_fn(|_| Shared::default());
            let mut windows = [
                naval_windows::Windows::default(),
                naval_windows::Windows::default(),
            ];
            let mut pings = [PoolView::default(), PoolView::default()];
            let mut base_views = [BaseView::default(), BaseView::default()];
            let mut combats = [land_windows::View::default(), land_windows::View::default()];
            let mut inputs = CounterInputs {
                schedule: &schedules[0],
                ..inputs
            };
            let mut emitted = 0;
            let mut admitted = 0;
            let mut refused = 0;
            let mut errors = 0;
            let mut naval = 0;
            for hour in 0..36 {
                let case = hour % 18;
                let mut frame = DecodedFrame {
                    branch: 1,
                    hour,
                    metrics: Default::default(),
                    tables: TableId::ALL
                        .map(|table| TableView::Complete(TableRows::empty(table)).into()),
                };
                let mut put = |rows: TableRows| {
                    let table = rows.table();
                    *frame.tables[table.index()] = TableView::Complete(rows);
                };
                put(TableRows::Countries(vec![
                    CountryRow {
                        id: 10,
                        canonical_slot: 1,
                        tag: b"AAA".to_vec(),
                        ..Default::default()
                    },
                    CountryRow {
                        id: 20,
                        canonical_slot: 2,
                        tag: b"BBB".to_vec(),
                        ..Default::default()
                    },
                ]));
                put(TableRows::CountryLinks(vec![
                    CountryLinkRow {
                        id: 10,
                        canonical_slot: 1,
                    },
                    CountryLinkRow {
                        id: 20,
                        canonical_slot: 2,
                    },
                ]));
                put(TableRows::Provinces(
                    [1, 3]
                        .map(|id| ProvinceRow {
                            id,
                            state: id,
                            owner: 10,
                            controller: if case == 3 {
                                20
                            } else if case == 4 {
                                0
                            } else if case == 5 {
                                999
                            } else {
                                10
                            },
                            ..Default::default()
                        })
                        .into(),
                ));
                let mut buildings: Vec<_> = [1, 3]
                    .into_iter()
                    .flat_map(|location| {
                        (1..=4).map(move |kind| BuildingRow {
                            location,
                            building_type: kind,
                            ordinal: if case == 2 { 0 } else { 5 - kind },
                            country: if case == 6 {
                                999
                            } else if case == 7 {
                                20
                            } else {
                                0
                            },
                            status_flags: if case == 8 { 0 } else { 1 },
                            level: if case == 1 && kind == 1 || case == 9 {
                                0
                            } else {
                                1
                            },
                            ..Default::default()
                        })
                    })
                    .collect();
                if case == 10 {
                    buildings.retain(|row| row.location != 1 || row.building_type != 3);
                }
                if case == 11 {
                    buildings.push(BuildingRow {
                        location: 3,
                        building_type: 5,
                        level: 1,
                        country: 10,
                        ..Default::default()
                    });
                }
                put(TableRows::ProvinceBuildings(buildings));
                put(TableRows::LexicalTokens(
                    ["hq_first", "hq_second", "fac_a", "fac_b", "ambiguous"]
                        .into_iter()
                        .enumerate()
                        .filter(|(index, _)| case != 12 || *index != 2)
                        .map(|(index, literal)| LexicalTokenRow {
                            token: index as u32 + 1,
                            literal: literal.as_bytes().to_vec(),
                        })
                        .collect(),
                ));
                put(TableRows::FacilityMapFlags(
                    [1, 3]
                        .map(|province| FacilityMapFlagRow {
                            province,
                            flag: u8::from(case != 13),
                        })
                        .into(),
                ));
                put(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                    dlc_mask: if case == 14 { 0 } else { 0x8000 },
                    ..Default::default()
                }]));
                put(TableRows::NavalBases(
                    [1, 3]
                        .map(|province| NavalBaseRow {
                            province,
                            country: if case == 15 { 20 } else { 10 },
                            ..Default::default()
                        })
                        .into(),
                ));
                for table in [TableId::Provinces, TableId::ProvinceBuildings] {
                    let TableView::Complete(rows) = &*frame.tables[table.index()] else {
                        unreachable!()
                    };
                    rows.validate_keys().unwrap();
                }
                if case == 16 {
                    let TableView::Complete(rows) =
                        &*frame.tables[TableId::ProvinceBuildings.index()]
                    else {
                        unreachable!()
                    };
                    let rows = rows.clone();
                    *frame.tables[TableId::ProvinceBuildings.index()] =
                        TableView::Truncated { rows, missing: 1 };
                }
                if case == 17 {
                    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
                        reason: "core fixture refusal".into(),
                    };
                }
                if hour == 15 {
                    *frame.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
                        reason: "feature fixture refusal".into(),
                    };
                }
                let order = counter_order::RecordedOrder::read(&frame).unwrap();
                let context = BaseContext {
                        building_icons: &content.building_icons,
                    content: inputs.content,
                    vision: &vision,
                    links: &links,
                    country_ids: &ids,
                    relations: Err("fixture relations unavailable"),
                    order: order.as_ref(),
                    carry_tokens: false,
                };
                let actual = BaseIcons::read_profiled(
                    &frame,
                    &context,
                    &mut viewer_runtime::profiling::BuilderSpans::new(false),
                );
                if case == 17 {
                    let why = actual.unwrap_err();
                    assert_eq!(why.kind(), std::io::ErrorKind::InvalidData);
                    assert_eq!(
                        why.to_string(),
                        format!(
                            "hour {hour} Provinces is unavailable: core fixture refusal; the core tables \
                             Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn"
                        )
                    );
                    errors += 1;
                    continue;
                }
                let actual = actual.unwrap();
                let mut expected = BaseIcons::default();
                for (province, anchor) in [(1, [2., 9.5, 2.]), (3, [3., 9.5, 2.])] {
                    if case == 15 {
                        *expected.unknown.naval_bases
                            .entry("fixture relations unavailable".into()).or_default() += 1;
                    } else {
                        expected.naval.push(NavalBaseIcon {
                            province,
                            frame: 2,
                            highlighted: false,
                            enabled: true,
                            capacity: None,
                            capacity_refusal: Some(
                                "txt_capacity is withheld: the naval port intel level is unknown"
                            ),
                        });
                    }
                    let hq_refusal = match case {
                        11 if province == 3 => Some(
                            "naval HQ building ambiguous is declared more than once, differently"
                        ),
                        16 => Some("the ProvinceBuildings census is incomplete"),
                        _ if hour == 15 => Some("the SimulationFeatures DLC mask is unavailable"),
                        5 | 6 | 8 => Some("the building's country is unresolved"),
                        _ => None,
                    };
                    if let Some(why) = hq_refusal {
                        *expected.unknown.naval_headquarters.entry(why.into()).or_default() += 1;
                    } else if !matches!(case, 3 | 4 | 7 | 9 | 14) {
                        expected.headquarters.push((province, anchor));
                    }
                    if matches!(case, 4 | 13) {
                        continue;
                    }
                    let facility_refusal = match case {
                        11 if province == 3 => Some(
                            "facility building ambiguous is declared more than once, differently"
                        ),
                        12 => Some("facility building fac_a has no single token this hour"),
                        16 => Some("the ProvinceBuildings census is incomplete"),
                        5 | 6 | 8 => Some("the building's country is unresolved"),
                        _ => None,
                    };
                    if let Some(why) = facility_refusal {
                        *expected.unknown.facilities.entry(why.into()).or_default() += 1;
                    } else if !matches!(case, 3 | 7 | 9) {
                        let sprite = if case == 2 {
                            "GFX_mapicon_facility_air"
                        } else {
                            "GFX_mapicon_facility_naval"
                        };
                        expected.facilities.push((province, anchor, sprite.into()));
                    }
                }
                assert_eq!(expected.naval, actual.naval, "naval state hour {hour}");
                assert_eq!(expected.unknown, actual.unknown, "refusal counts hour {hour}");
                assert_eq!(expected.refused, actual.refused, "named refusals hour {hour}");
                assert_eq!(
                    expected.headquarters
                        .iter()
                        .map(|(p, a)| (*p, a.map(f32::to_bits)))
                        .collect::<Vec<_>>(),
                    actual.headquarters
                        .iter()
                        .map(|(p, a)| (*p, a.map(f32::to_bits)))
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    expected.facilities
                        .iter()
                        .map(|(p, a, s)| (*p, a.map(f32::to_bits), s))
                        .collect::<Vec<_>>(),
                    actual.facilities
                        .iter()
                        .map(|(p, a, s)| (*p, a.map(f32::to_bits), s))
                        .collect::<Vec<_>>()
                );
                admitted += actual.headquarters.len() + actual.facilities.len();
                refused += actual.unknown.totals().into_iter().sum::<usize>();
                naval += actual.naval.len();
                let groups = inputs.content.defines.groups;
                inputs.camera.height = if hour % 4 == 0 {
                    groups.state_distance
                } else {
                    groups.province_distance - 10.0
                };
                inputs.camera.center = [2. + (hour % 3) as f32 * 0.125, 2.];
                let mut tables = synthetic_hour(content, &inputs, &frame);
                let mut prepared = [expected, actual].map(|icons| {
                    tables.base_icons = icons;
                    prepare_cached(&inputs, &tables).unwrap()
                });
                assert_eq!(prepared[0].fixed.report, prepared[1].fixed.report);
                for (side, prepared) in prepared.iter_mut().enumerate() {
                    inputs.schedule = &schedules[side];
                    prepared
                        .draw(
                            &inputs,
                            hour as f32 / 24.,
                            &mut windows[side],
                            &mut pings[side],
                            &mut base_views[side],
                            &mut combats[side],
                        )
                        .unwrap();
                }
                let [expected, actual] = &prepared;
                if hour == 0 {
                    assert!(
                        !expected.fixed.bases.quads.is_empty(),
                        "fixture must emit native base windows: {:?}",
                        expected.drawn.report
                    );
                }
                assert_eq!(
                    expected.drawn.report, actual.drawn.report,
                    "draw reports hour {hour}"
                );
                for (a, b) in [
                    (&expected.fixed.pings, &actual.fixed.pings),
                    (&expected.fixed.bases, &actual.fixed.bases),
                    (&expected.fixed.combats, &actual.fixed.combats),
                    (&expected.drawn.batch, &actual.drawn.batch),
                    (&expected.drawn.combats, &actual.drawn.combats),
                ] {
                    assert_quad_bits(a, b);
                    emitted += a.quads.len();
                }
            }
            assert!(
                errors > 0 && refused > 0 && admitted > 0 && naval > 0 && emitted > 0,
                "must prove decided/refused/error and nonempty drawing branches: {errors}/{refused}/{admitted}/{naval}/{emitted}"
            );
        },
    );
}

#[test]
fn keep_drawn_preserves_motion_continuity_when_land_appears_below_cutoff() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(br#"guiTypes={
        containerWindowType={name=unit_mapicon size={width=81 height=29}
            gridBoxType={name=units_box size={width=81 height=290}
                slotsize={width=81 height=1} max_slots_horizontal=1 format=UPPER_LEFT}
        }
        positionType={name=unit_counter_zoomed_in_offset position={x=0 y=0}}
        containerWindowType={name=unit_counter_navy size={width=20 height=10}}
        containerWindowType={name=naval_headquarter_mapicon size={width=81 height=29}
            buttonType={name=bg_btn spriteType=GFX_synthetic_plate}}
        containerWindowType={name=naval_combat_mapicon size={width=20 height=10}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions position={x=57 y=9} frame=12}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay position={x=3 y=4}}
        }
    }"#);
    with_counter_inputs_and_content(
        guis,
        |assets| {
            assets.layouts = Layouts::read(&assets.windows);
        },
        |content| {
            let points = [(1, [2., 9.5, 2.])];
            content.default_anchors = Arc::new(points.into());
            content.bounds = Arc::new([(1, [1., 1., 3., 3.])].into());
            content.region_of = Arc::new([(1, 1)].into());
        },
        |inputs, content| {
            use replay_data::rows::{TableId, TableRows};
            use replay_frame::frame::TableView;
            let frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let mut tables = synthetic_hour(content, &inputs, &frame);
            tables.divisions = [((1, STANDSTILL), vec![land_unit(10, 1, 10, 10, true)])].into();
            tables.countries = Arc::new(countries([(1, Allegiance::Ours)]));
            let empty = BTreeSet::new();
            let schedule = Shared::default();
            let mut inputs = CounterInputs {
                selected: &empty,
                schedule: &schedule,
                ..inputs
            };
            inputs.camera.center = [2., 2.];
            inputs.camera.height = inputs.content.defines.icons_cutoff + 10.0;
            let mut above = prepare_cached(&inputs, &tables).unwrap();
            assert!(above.motion_layout().is_none(), "above the cutoff no land layout is built");
            above.set_motion_continuity(2);
            assert_eq!(above.motion_continuity(), 2);

            inputs.camera.height = 80.;
            let mut below = prepare_cached(&inputs, &tables).unwrap();
            assert_eq!(below.motion_layout().expect("land layout below cutoff").continuity(), 0);
            below.keep_drawn(&mut above);
            assert_eq!(below.motion_continuity(), 2);
            assert_eq!(
                below.motion_layout().expect("land layout below cutoff").continuity(),
                2,
                "zooming below the cutoff keeps the publication's continuity epoch"
            );

            inputs.camera.height = inputs.content.defines.icons_cutoff + 10.0;
            let mut above_again = prepare_cached(&inputs, &tables).unwrap();
            above_again.keep_drawn(&mut below);
            assert!(above_again.motion_layout().is_none());
            assert_eq!(above_again.motion_continuity(), 2);

            inputs.camera.height = 80.;
            let mut below_again = prepare_cached(&inputs, &tables).unwrap();
            below_again.keep_drawn(&mut above_again);
            assert_eq!(
                below_again.motion_layout().expect("land layout below cutoff").continuity(),
                2
            );
        },
    );
}
