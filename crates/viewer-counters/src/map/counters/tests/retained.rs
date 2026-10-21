//! Fresh-versus-retained semantic preparation over the same presented history.

use super::*;

#[test]
fn retained_semantic_plans_match_fresh_through_camera_thresholds_and_admission() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
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
    with_counter_inputs_fixture(
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
            content.bounds = points
                .into_iter()
                .map(|(id, [x, _, z])| (id, [x - 1., z - 1., x + 1., z + 1.]))
                .collect();
            content.region_of = Arc::new([(1, 1), (2, 1), (3, 2)].into());
        },
        |mut inputs| {
            use replay_data::rows::{TableId, TableRows};
            use replay_frame::frame::TableView;
            use crate::map::counters::preparation::plan_probe;
            let frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let mut tables = synthetic_hour(&inputs, &frame);
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
            let heights = [
                80.,
                80.,
                89.999,
                90.,
                90.,
                179.999,
                180.,
                180.,
                349.999,
                350.,
                350.,
                inputs.content.defines.units_cutoff - 0.001,
                inputs.content.defines.units_cutoff,
                80.,
                80.,
            ];
            for (at, height) in heights.into_iter().enumerate() {
                inputs.camera.height = height;
                inputs.camera.center = if at == 4 || at == 8 {
                    [100., 100.]
                } else {
                    [2. + at as f32 * 0.125, 2.]
                };
                inputs.selected = if (5..10).contains(&at) {
                    &selected
                } else {
                    &empty
                };
                let (mut cached, counts) = plan_probe::collect(|| {
                    prepare_retained(&inputs, &frame, &tables, 0, &mut retained).unwrap()
                });
                let mut fresh = prepare_cached(&inputs, &frame, &tables).unwrap();
                hits += counts.semantic_plan_hits;
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

/// Structural equality plus all floating payloads by bits, including recursive native bakes.
/// This is deliberately independent of SpritePass's retained-upload equality.
fn assert_quad_bits(a: &Batch, b: &Batch) {
    assert_eq!(a.quads, b.quads);
    for (a, b) in a.quads.iter().zip(&b.quads) {
        assert_eq!(a.rect.map(f32::to_bits), b.rect.map(f32::to_bits));
        assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
        assert_eq!(a.color.map(f32::to_bits), b.color.map(f32::to_bits));
        assert_eq!(a.rotation.to_bits(), b.rotation.to_bits());
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
    a: &viewer_foundation::map::native_shader::draw::Quad,
    b: &viewer_foundation::map::native_shader::draw::Quad,
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

fn synthetic_hour(inputs: &CounterInputs<'_>, frame: &DecodedFrame) -> HourTables {
    synthetic_hour_with_history(inputs, frame, None)
}

fn synthetic_hour_with_history(
    inputs: &CounterInputs<'_>, frame: &DecodedFrame, history: Option<&History>,
) -> HourTables {
    let moves = viewer_foundation::map::movement_reads::test_support::unnamed_assets();
    hour_tables(
        frame,
        inputs.map,
        Arc::new(Vision::sharing(1, [])),
        inputs.content,
        inputs.assets,
        inputs.equipment,
        inputs.localisation,
        &Default::default(),
        None,
        history,
        &type_tint::Memo::new(type_tint::TABLES, 1),
        &MissionMemo::new(1),
        moves.movement_reads(),
        Err("unreached synthetic naval topology".into()),
    )
    .unwrap()
}

#[test]
fn retained_semantic_plan_key_invalidates_every_non_camera_input_it_reuses() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    with_counter_inputs_fixture(
        action_gui(false),
        |_| {},
        |_| {},
        |inputs| {
            use replay_data::rows::{TableId, TableRows};
            use replay_frame::frame::TableView;
            use crate::map::counters::preparation::plan_probe;
            let frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let mut tables = synthetic_hour(&inputs, &frame);
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
                    // A pan by itself must not invalidate a pure semantic plan.
                    inputs.camera.center[0] += 0.125;
                    let (cached, counts) = plan_probe::collect(|| {
                        prepare_retained(&inputs, &frame, &tables, resources, &mut memo).unwrap()
                    });
                    let fresh = prepare_cached(&inputs, &frame, &tables).unwrap();
                    assert_eq!(
                        cached.fixed.report, fresh.fixed.report,
                        "change {change} repeat {repeat}"
                    );
                    assert_eq!(counts.semantic_plans, 1); // only the base lookup: this hour has no sources
                    assert_eq!(counts.semantic_plan_hits, repeat, "change {change}");
                }
            }
            // Cloning keeps one immutable hour; rebuilding it gets a new held epoch.
            tables = tables.clone();
            assert_eq!(
                plan_probe::collect(|| prepare_retained(
                    &inputs, &frame, &tables, resources, &mut memo
                )
                .unwrap())
                .1
                .semantic_plan_hits,
                1
            );
            tables.epoch = Arc::new(());
            assert_eq!(
                plan_probe::collect(|| prepare_retained(
                    &inputs, &frame, &tables, resources, &mut memo
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
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
    with_counter_inputs_fixture(
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
            content.bounds = points
                .into_iter()
                .map(|(id, [x, _, z])| (id, [x - 1., z - 1., x + 1., z + 1.]))
                .collect();
            content.region_of = Arc::new([(1, 1), (2, 1), (3, 2)].into());
        },
        |mut inputs| {
            use replay_data::rows::{CountryLinkRow, CountryRow, DivisionRow,
                DivisionMovementRow, LandCombatRow, LandCombatSideRow, NavalCombatRow,
                TableId, TableRows};
            use replay_frame::frame::TableView;
            use viewer_foundation::engagement_history::{Covered, NAVAL, NAVAL_ICON};
            use viewer_foundation::icon_frames::Kept;
            let mut frame = DecodedFrame {
                branch: 1, hour: 11, metrics: Default::default(),
                tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                [1, 5].map(|id| CountryRow { id, canonical_slot: id,
                    tag: format!("T{id}").into_bytes(), ..Default::default() }).to_vec()));
            *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
                [1, 5].map(|id| CountryLinkRow { id, canonical_slot: id }).to_vec()));
            *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
                vec![DivisionRow { id: 10, province: 1, organisation: 10, ..Default::default() }]));
            *frame.tables[TableId::DivisionMovement.index()] = TableView::Complete(TableRows::DivisionMovement(
                vec![DivisionMovementRow { division: 10, ..Default::default() }]));
            *frame.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(
                vec![LandCombatRow { id: 61, province: 1, attackers: vec![10], defenders: vec![10],
                    ..Default::default() }]));
            *frame.tables[TableId::LandCombatSides.index()] = TableView::Complete(TableRows::LandCombatSides(
                vec![LandCombatSideRow { combat: 61, attacker_primary: vec![1], defender_primary: vec![5],
                    ..Default::default() }]));
            *frame.tables[TableId::NavalCombats.index()] = TableView::Complete(TableRows::NavalCombats(
                vec![NavalCombatRow { id: 81, province: 2, ..Default::default() }]));
            let populate = |tables: &mut HourTables| {
                tables.divisions = [((1, STANDSTILL), vec![land_unit(10, 1, 10, 10, true),
                    land_unit(11, 1, 10, 10, true)])].into();
                tables.base_icons.headquarters = vec![(1, [2., 9.5, 2.])];
                tables.countries = Arc::new(countries([(1, Allegiance::Ours)]));
            };
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            let selected = BTreeSet::from([Unit::Division(10)]);
            let empty = BTreeSet::new();
            let mut total_drawn = 0;
            let mut actual_markers = 0;
            // Each history publishes once, before that frame's refinement.
            // Include coverage without an answer and recording-player precedence.
            for case in 0..6 {
                let history = History::default();
                let mut tables = synthetic_hour_with_history(&inputs, &frame, Some(&history));
                populate(&mut tables);
                assert!(tables.is_for_frame(&frame.clone()));
                let original_epoch = Arc::clone(&tables.epoch);
                let original_base = Arc::clone(&tables.base);
                let mut memo = Preparation::default();
                let mut windows = [naval_windows::Windows::default(), naval_windows::Windows::default()];
                let mut pings = [PoolView::default(), PoolView::default()];
                let mut bases = [BaseView::default(), BaseView::default()];
                let mut combats = [land_windows::View::default(), land_windows::View::default()];
                let schedules: [Shared; 2] = std::array::from_fn(|_| Shared::default());
                let mut inputs = CounterInputs { selected: &empty, schedule: &schedules[0], ..inputs };
                for at in 0..10 {
                    if at == 3 {
                        let mut covered = Covered::new(frame.hour);
                        if case != 0 { covered.viewers.push((frame.hour, if case == 1 || case == 5 { 5 } else { 1 })); }
                        let flag = match case { 2 => Engaged::Unknown, 3 | 5 => Engaged::Yes, _ => Engaged::No };
                        for (kind, id) in [(LAND, 61), (NAVAL, 81)] {
                            covered.points[kind].insert((id, 0), vec![(frame.hour, flag)]);
                            covered.points[kind].insert((id, 1), vec![(frame.hour, flag)]);
                            covered.recording[kind].insert((id, 0), vec![(frame.hour, Engaged::Yes)]);
                        }
                        for (kind, id) in [(LAND_ICON, 61), (NAVAL_ICON, 81)] {
                            covered.icons[kind].insert(id, vec![(frame.hour, Some(Kept::Frame(7)))]);
                            covered.recording_icons[kind].insert(id, vec![(frame.hour, Some(Kept::Frame(2)))]);
                        }
                        if case == 5 { covered.players.push((frame.hour, Some(1))); }
                        history.branches.lock().unwrap().insert(frame.branch, covered);
                        assert!(history.covers(frame.branch, frame.hour));
                        tables = tables.refine_engagement(Some(&history)).unwrap();
                        assert!(Arc::ptr_eq(&original_base, &tables.base));
                        assert!(!Arc::ptr_eq(&original_epoch, &tables.epoch));
                    }
                    inputs.camera.height = [80., 90., 180., 80., 350., 80., 80., 180., 80., 80.][at];
                    inputs.camera.center = [2. + at as f32 * 0.125, 2.];
                    inputs.selected = if (4..8).contains(&at) { &selected } else { &empty };
                    inputs.hovered = (at == 5).then_some(Unit::Division(10));
                    inputs.visibility.unit_counters = at != 6;
                    inputs.schedule = &schedules[0];
                    // Fixed fixture answers, not another history read or full hour rebuild.
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
                                viewer_foundation::icon_frames::CARRIED.to_owned(),
                                [61].into(),
                            );
                        }
                    }
                    assert_eq!(tables.engaged.land, flags.land, "case {case} frame {at}");
                    assert_eq!(tables.engaged.naval, flags.naval, "case {case} frame {at}");
                    assert_eq!(tables.engaged.land_provenance, provenance);
                    assert_eq!(tables.engaged.naval_provenance, provenance);
                    assert_eq!(tables.land_combats.markers, land_combats.markers);
                    assert_eq!(tables.land_combats.gaps, land_combats.gaps);
                    assert_eq!(tables.land_combats.limits, land_combats.limits);
                    // Project the naval fixture using those fixed answers too. This bypasses
                    // history lookup/refinement while retaining its ordinary refusal coverage.
                    let kept = if at < 3 || case < 2 {
                        Kept::Undecided
                    } else {
                        Kept::Frame(if case == 5 { 2 } else { 7 })
                    };
                    let naval_markers = NavalMarkerTables::read(
                        &frame,
                        Viewer {
                            vision: &original_base.vision,
                            owner: &|raw| original_base.links.get(&raw).copied(),
                        },
                        &flags,
                        &[(81, kept)].into(),
                        &|province| inputs.content.kinds.get(&province).copied(),
                        &|province| stack_anchor(
                            &inputs.content.anchors,
                            &inputs.content.default_anchors,
                            province,
                            STANDSTILL,
                        ),
                    ).unwrap();
                    let expected_tables = HourTables {
                        epoch: Arc::new(()),
                        base: Arc::clone(&original_base),
                        land_combats: Arc::new(land_combats),
                        naval_markers: Arc::new(naval_markers),
                        engaged: Arc::new(flags),
                    };
                    actual_markers += tables.land_combats.markers.len();
                    let mut expected = prepare_retained(&inputs, &frame, &expected_tables, 0, &mut Preparation::default()).unwrap();
                    let mut refined = prepare_retained(&inputs, &frame, &tables, 0, &mut memo).unwrap();
                    assert_eq!(expected.fixed.report, refined.fixed.report, "case {case} frame {at}");
                    for (side, prepared) in [&mut expected, &mut refined].into_iter().enumerate() {
                        inputs.schedule = &schedules[side];
                        prepared.draw(&inputs, at as f32 / 240., &mut windows[side], &mut pings[side],
                            &mut bases[side], &mut combats[side]).unwrap();
                    }
                    assert_eq!(expected.drawn.report, refined.drawn.report, "case {case} frame {at}");
                    for (a, b) in [(&expected.fixed.pings, &refined.fixed.pings),
                        (&expected.fixed.bases, &refined.fixed.bases), (&expected.fixed.combats, &refined.fixed.combats),
                        (&expected.drawn.batch, &refined.drawn.batch), (&expected.drawn.combats, &refined.drawn.combats)] {
                        assert_quad_bits(a, b);
                    }
                    total_drawn += expected.drawn.batch.quads.len();
                }
                assert_eq!(tables.engaged.land_provenance, match case {
                    0 => Provenance::Unread, 1 => Provenance::OtherViewer, _ => Provenance::Indexed,
                });
                if case == 4 {
                    assert_eq!(tables.land_combats.markers[0].frame, Some(7));
                    assert!(!tables.land_combats.markers[0].seen);
                }
                if case == 5 { assert_eq!(tables.engaged.land[&61], Engaged::Yes); }
                let mut changed = frame.clone(); changed.branch += 1;
                assert!(!tables.is_for_frame(&changed));
                changed = frame.clone(); changed.hour += 1;
                assert!(!tables.is_for_frame(&changed));
                for table in TableId::ALL {
                    changed = frame.clone();
                    changed.tables[table.index()] = (*changed.tables[table.index()]).clone().into();
                    assert!(!tables.is_for_frame(&changed), "{table:?}");
                    changed = frame.clone();
                    changed.tables[table.index()] = changed.tables[table.index()].clone().with_sampled_at(Some(frame.hour));
                    assert!(!tables.is_for_frame(&changed), "supplying hour for {table:?}");
                }
            }
            assert!(total_drawn > 0, "must compare real emitted counter quads");
            assert!(actual_markers > 0, "must exercise nonempty history-dependent marker products");
        },
    );
}
