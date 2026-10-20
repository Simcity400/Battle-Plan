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
        None,
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
