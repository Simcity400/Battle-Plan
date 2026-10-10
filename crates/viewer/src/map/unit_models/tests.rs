#[test]
fn raid_units_bypass_the_unit_history_and_move_by_the_playback_fraction() {
    use super::*;
    let assets = UnitModelAssets {
        disabled: None,
        ..UnitModelAssets::off(String::new())
    };
    let at = |seed: u32| ModelInstance {
        animation_seed: seed,
        scale: 1.0,
        ..ModelInstance::default()
    };
    let target = |province: u32, seed: u32, kind: Kind| Target {
        province,
        identity: u64::from(seed),
        part: 0,
        order: STANDSTILL,
        occupant: u64::from(seed),
        kind,
        mesh: seed as usize - 1,
        instance: at(seed),
        origin: [0.0; 3],
    };
    let (track, heights) = super::super::raids::arrows::test_track();
    let hour = track.place(&heights, 0.0).unwrap();
    let prepared = PreparedModels {
        placed: vec![(
            target(5, 1, Kind::Land),
            counter_move::Position {
                standing: [0.0; 3],
                motion: None,
            },
        )],
        ships: NavalSources {
            candidates: vec![super::super::unit_history::naval::Candidate {
                province: 6,
                identity: 2,
                ordinal: 0,
                slot: 0,
                priority: 3,
                ours: true,
                class: counter_move::CounterClass::TaskForce,
                land_eligible: false,
                sample_count: 0,
                next_arc: 0.0,
                route: Vec::new(),
                speed: 12.0,
                min_arc: 0.0,
                arrival_allowed: true,
                day: 0,
            }],
            formations: vec![super::super::unit_history::naval::Formation {
                province: 6,
                identity: 2,
                slot: 0,
                roster: vec![super::super::unit_history::naval::Hull {
                    flags: 1 << 6,
                    graphics: Vec::new(),
                    targets: [Some(target(6, 2, Kind::Ship)), None, None, None],
                }],
            }],
            ..Default::default()
        },
        raid: vec![RaidModel {
            slot: 2,
            instance: ModelInstance {
                position: hour.position,
                ..at(7)
            },
            track,
            stages: Vec::new(),
        }],
        owned: BTreeSet::new(),
        report: ModelReport::default(),
    };
    // The history's targets are the figure and the hull; the raid unit
    // is placed beside them, at the hour's start, and its mesh uploaded.
    let field = prepared.field(&assets);
    let seeds = |instances: &[ModelInstance]| {
        instances
            .iter()
            .map(|i| i.animation_seed)
            .collect::<BTreeSet<_>>()
    };
    let targets: Vec<_> = field
        .targets
        .iter()
        .map(|t| t.instance)
        .chain(
            field
                .naval
                .settled_targets(assets.gates)
                .iter()
                .map(|t| t.instance),
        )
        .collect();
    assert_eq!(seeds(&targets), [1, 2].into());
    assert_eq!(field.raids.len(), 1);
    assert_eq!(
        (field.raids[0].0, field.raids[0].1.position),
        (2, hour.position)
    );
    assert_eq!(seeds(&field.models.instances), [1, 2, 7].into());
    // Playback places it again by the fraction.
    let moved = prepared.raid_units(&assets, Some((&heights, 0.5)));
    assert_eq!(
        moved[0].1.position,
        prepared.raid[0]
            .track
            .place(&heights, 0.5)
            .unwrap()
            .position
    );
    assert!(moved[0].1.position[0] > hour.position[0]);
}

use super::*;

#[test]
fn recorded_stack_and_combat_roles_select_native_states() {
    use super::super::entity_animation::*;
    let mut route = Route::default();
    assert_eq!(animation_state(Some(&route), None, false), IDLE);
    assert_eq!(animation_state(Some(&route), None, true), TRAINING);
    route.stack_kind = 19;
    assert_eq!(animation_state(Some(&route), None, true), IDLE);
    route.stack_kind = 10;
    assert_eq!(animation_state(Some(&route), None, false), DEFEND);
    route.stack_kind = 1;
    route.path = vec![42];
    assert_eq!(animation_state(Some(&route), None, false), MOVE);
    route.retreat = true;
    assert_eq!(animation_state(Some(&route), None, false), RETREAT);
    route.combat_role = 1;
    assert_eq!(animation_state(Some(&route), None, false), ATTACK);
    route.path.clear();
    assert_eq!(animation_state(Some(&route), None, false), SUPPORT);
    assert_eq!(animation_state(None, Some(DEFEND), false), DEFEND);
}

#[test]
fn animation_selection_prioritizes_combat_and_retreat_over_movement() {
    use super::super::entity_animation::*;
    let moving = Route {
        stack_kind: 11,
        path: vec![2],
        ..Route::default()
    };
    assert_eq!(animation_state(Some(&moving), None, false), MOVE);
    assert_eq!(animation_state(Some(&moving), Some(ATTACK), false), ATTACK);
    assert_eq!(
        animation_state(
            Some(&Route {
                path: vec![],
                ..moving.clone()
            }),
            Some(ATTACK),
            false
        ),
        SUPPORT
    );
    assert_eq!(
        animation_state(
            Some(&Route {
                retreat: true,
                ..moving.clone()
            }),
            None,
            false
        ),
        RETREAT
    );
    assert_eq!(
        animation_state(
            Some(&Route {
                stack_kind: 10,
                ..moving
            }),
            Some(ATTACK),
            false
        ),
        DEFEND
    );
    assert_eq!(animation_state(None, Some(DEFEND), false), DEFEND);
    assert_eq!(
        animation_state(Some(&Route::default()), None, true),
        TRAINING
    );
}

fn along(anchors: &[[f32; 3]]) -> Option<([f32; 3], f32)> {
    counter_move::Motion::new(5632.0, anchors).map(|motion| motion.position_heading())
}

fn names(candidates: Vec<(Level, String)>) -> Vec<String> {
    candidates.into_iter().map(|(_, name)| name).collect()
}

#[test]
fn ship_and_unit_gates_are_strict_below_their_cutoffs() {
    assert!(GATES.units(239.9) || GATES.ships(239.9));
    assert!(!GATES.units(240.0) && !GATES.ships(240.0));
    assert!(!GATES.units(120.0));
}

/// The shipped `00_graphics.lua` values.
const GATES: Gates = Gates {
    units: 120.0,
    ships: 240.0,
    arrows: 500.0,
    transport_scale: 0.25,
    scale_up: 2.0,
    scale_down: 3.0,
    ship_scale_down: 4.0,
    naval_movement_speed: 12.0,
    land_movement_speed: 12.0,
};

#[test]
fn the_gate_matches_the_reference_and_close_zoom_heights() {
    // Neither daylight reference pair is close to it, and neither is the
    // format-17 pair: pairF_03_zoom_probe fits at 133.617.
    for height in [302.960, 310.044, 239.329, 133.617, 120.0] {
        assert!(!GATES.units(height), "{height}");
    }
    // pairD_in5 at 115.293 is 4.7 inside the cutoff; pairD_in10 at
    // 50.178 is well inside.
    assert!(GATES.units(115.293));
    assert!(GATES.units(50.178));
}

#[test]
fn the_ladder_is_key_outermost_and_prefix_innermost() {
    // Each key tries the prefix ladder once, so every prefix of
    // the first key is tried before the second key is built at all.
    assert_eq!(
        names(candidates(
            Some("GER_cosmetic"),
            Some("GER"),
            Some("western_european_gfx"),
            "militia",
            Some("infantry")
        )),
        [
            "GER_cosmetic_militia_entity",
            "GER_militia_entity",
            "western_european_gfx_militia_entity",
            "militia_entity",
            "GER_cosmetic_infantry_entity",
            "GER_infantry_entity",
            "western_european_gfx_infantry_entity",
            "infantry_entity",
        ]
    );
    // An empty cosmetic tag is still prefixed with its underscore.
    assert_eq!(
        names(candidates(
            Some(""),
            Some("SOV"),
            Some("x_gfx"),
            "infantry",
            None
        )),
        [
            "_infantry_entity",
            "SOV_infantry_entity",
            "x_gfx_infantry_entity",
            "infantry_entity",
        ]
    );
}

#[test]
fn a_sub_unit_whose_sprite_is_its_own_name_contributes_one_key() {
    assert_eq!(entity_keys("infantry", Some("infantry")), ["infantry"]);
    // A key with no sub-unit row at all still reaches the bare name.
    assert_eq!(entity_keys("transport", None), ["transport"]);
}

#[test]
fn the_bare_level_is_always_last_and_a_missing_tag_drops_its_level() {
    assert_eq!(entity_prefixes(None, None, None), [""]);
    assert_eq!(entity_prefixes(None, Some("GER"), None), ["GER", ""]);
    assert_eq!(
        entity_prefixes(Some(""), Some("JAP"), Some("asian_gfx")),
        ["JAP", "asian_gfx", ""]
    );
    assert_eq!(entity_name("", "infantry"), "infantry_entity");
    assert_eq!(
        entity_name("asian_gfx", "cavalry"),
        "asian_gfx_cavalry_entity"
    );
}

fn country() -> ModelCountry {
    ModelCountry {
        tag: "GER".into(),
        cosmetic: "".into(),
        graphical_culture: Some("western_european_gfx".into()),
        original_tag: Some("GER".into()),
        original_graphical_culture: Some("western_european_gfx".into()),
        ruling_ideology: Some(None),
    }
}

#[test]
fn new_divisions_join_the_daily_census_and_absence_is_empty_only_when_both_are_complete() {
    let row = |division| replay::rows::DivisionEquipmentRow {
        division,
        ..Default::default()
    };
    let (daily, new) = (row(1), row(2));
    let equipment = |new_complete| Equipment {
        by_division: [(1, vec![&daily]), (2, vec![&new])].into(),
        new_complete,
    };
    assert_eq!(equipment(true).of(2).unwrap().len(), 1);
    assert!(equipment(true).of(3).unwrap().is_empty());
    assert!(equipment(false).of(3).is_err());
}

fn division(id: u64, province: u32, template: u64) -> replay::rows::DivisionRow {
    replay::rows::DivisionRow {
        id,
        roster_owner: 100,
        logical_owner: 100,
        province,
        template,
        ..Default::default()
    }
}

fn status(division: u64, maximum: Option<i64>) -> replay::rows::DivisionStatusRow {
    replay::rows::DivisionStatusRow {
        division,
        acclimatization_maximum: maximum,
        acclimatization: vec![replay::rows::DivisionAcclimatization {
            category: 1,
            weight: 90,
        }],
        ..Default::default()
    }
}

/// One German hour on provinces `1..=provinces`, all snowy, whose
/// template 50 names a designer model.
fn hour_frame(
    divisions: Vec<replay::rows::DivisionRow>,
    statuses: Vec<replay::rows::DivisionStatusRow>,
    provinces: u32,
) -> DecodedFrame {
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            CountryLinkRow, CountryRow, LexicalTokenRow, ProvinceRow, TableId, TableRows,
            WeatherProvinceRow, WeatherRegionRow,
        },
    };
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for this hour".into(),
    });
    let recorded = [
        (
            TableId::CountryLinks,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            }]),
        ),
        (
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: 100,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                ..Default::default()
            }]),
        ),
        (
            TableId::Provinces,
            TableRows::Provinces(
                (1..=provinces)
                    .map(|id| ProvinceRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (TableId::Divisions, TableRows::Divisions(divisions)),
        (
            TableId::DivisionTemplateIcons,
            TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
                template: 50,
                model_entity: b"GER_custom_entity".to_vec(),
                ..Default::default()
            }]),
        ),
        (TableId::DivisionStatus, TableRows::DivisionStatus(statuses)),
        (
            TableId::LexicalTokens,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"cold".to_vec(),
            }]),
        ),
        (
            TableId::WeatherProvinces,
            TableRows::WeatherProvinces(
                (0..=provinces)
                    .map(|id| WeatherProvinceRow {
                        id,
                        snow: 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::WeatherRegions,
            TableRows::WeatherRegions(vec![WeatherRegionRow {
                max_snow: 100,
                ..Default::default()
            }]),
        ),
    ];
    for (id, rows) in recorded {
        views[id.index()] = TableView::Complete(rows);
    }
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    }
}

/// Three stacks: a designer model on snow, a division with no recorded
/// template, and one whose acclimatization is unreadable.
fn prepare_frame() -> DecodedFrame {
    hour_frame(
        vec![division(1, 9, 50), division(2, 8, 51), division(3, 7, 50)],
        vec![status(1, Some(100)), status(3, None)],
        9,
    )
}

/// The raid unit's aircraft model end to end:
/// Designer entity selection (`raids::arrows::designer_entity`)
/// uses `aircraft_variant_model` and the
/// conventional bridge, from a minimal content fixture and hour.
mod aircraft_designer;

fn camouflaged_assets() -> UnitModelAssets {
    UnitModelAssets {
        camouflage: camouflage::Content::parse(
            b"cold = { change_camo_when = { snow } }",
            75_000,
            50_000,
        )
        .unwrap(),
        registered: ["GER_custom_entity", "GER_custom_entity_snow"]
            .map(str::to_owned)
            .into(),
        disabled: None,
        ..UnitModelAssets::off(String::new())
    }
}

/// Unit models hand their painted, even-rounded province boxes to the shared history.
/// Check the frame's actual creations, including a hull and a land figure at the edges.
#[test]
fn unit_models_use_native_camera_admission_at_the_edges() {
    use super::super::{
        camera::{MapDefines, MapSize, Placement},
        map_buildings::BuildingHistory,
        province_admission::edge_boxes,
        unit_history::{
            History,
            tests::{GATES, target},
        },
    };
    let bounds = edge_boxes();
    assert_eq!(bounds[&5], [3044.0, 1000.0, 3046.0, 1002.0]);
    let mut assets = UnitModelAssets::off(String::new());
    assets.bounds = std::sync::Arc::new(bounds);
    let targets = (1..=6)
        .map(|id| target(id, if id == 2 { Kind::Ship } else { Kind::Land }))
        .collect();
    let mut history = History::default();
    history.set(targets, std::sync::Arc::clone(&assets.bounds), true);
    let camera = Placement {
        center: [3000.0, 1000.0],
        height: 50.0,
    }
    .on(MapSize::TEST, MapDefines::test());
    history.advance_schedule(camera, [1024, 768]);
    let frame = history.update_models(
        camera,
        [1024, 768],
        0.0,
        GATES,
        0,
        &mut BuildingHistory::default(),
        false,
    );
    let created: Vec<_> = frame
        .units
        .iter()
        .map(|(_, instance)| instance.animation_seed)
        .collect();
    assert_eq!(created, [1, 2, 5, 6]);
}

fn prepare(assets: &UnitModelAssets, frame: &DecodedFrame) -> ModelReport {
    equipment_graphics::tests::with_world(frame, &[], |limits| {
        assets
            .prepare(PrepareInputs {
                frame,
                limits,
                vision: &Vision::default(),
                etas: &super::super::division_eta::Etas::default(),
                anchor: &|_, _| Some([0.0; 3]),
                world: None,
                terrain: &BTreeMap::new(),
                ships: &ShipPlaces {
                    motions: &crate::map::task_force_visual::TaskForceMotions::default(),
                    movement: None,
                    land: &|_| true,
                    missions: &Default::default(),
                    region: &|_| None,
                    scale: &|_, _| 1.0,
                },
            })
            .unwrap()
            .report
    })
}

#[test]
fn an_hour_selects_the_designer_model_and_its_snow_camouflage_per_stack() {
    let report = prepare(&camouflaged_assets(), &prepare_frame());
    assert_eq!((report.stacks, report.untemplated), (3, 1));
    // The camouflaged designer model is the chosen entity; this test's
    // library bakes nothing, so it is counted by name.
    assert_eq!(
        report.selection_errors,
        BTreeMap::from([
            (
                "division acclimatization is unavailable this hour".to_owned(),
                1
            ),
            (
                "registered entity GER_custom_entity_snow did not bake".to_owned(),
                1
            ),
        ])
    );
}

#[test]
fn a_content_failure_turns_off_only_the_model_layer_and_says_so() {
    let mut warnings = Vec::new();
    let assets = UnitModelAssets::scoped(
        Err(invalid("missing matching common/acclimatation.txt")),
        &mut warnings,
    );
    assert!(warnings[0].contains("acclimatation.txt"));
    assert!(!assets.gates.units(1.0) && !assets.gates.ships(1.0));
    let report = prepare(&assets, &prepare_frame());
    assert_eq!(report.stacks, 0);
    assert!(report.disabled.unwrap().contains("acclimatation.txt"));
}

#[test]
fn an_unreadable_template_leaves_its_stacks_unmodelled_and_counted() {
    use replay::rows::{DivisionTemplateIconRow, TableId, TableRows};
    let mut frame = prepare_frame();
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        replay::frame::TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 50,
                // A registered designer model would otherwise win at level 1.
                model_entity: b"GER_custom_entity".to_vec(),
                model_unreadable: 1,
                ..Default::default()
            },
        ]));
    let report = prepare(&camouflaged_assets(), &frame);
    assert_eq!(
        report
            .selection_errors
            .get("template model inputs were unreadable this hour"),
        Some(&2)
    );
    assert!(report.by_level.is_empty(), "{:?}", report.by_level);
}

/// The hour builder's selection cost at a large campaign's density:
/// 6,000 divisions in 3,000 stacks, every one through camouflage.
/// `cargo test -p viewer --release hour_builder_selection_cost -- --ignored --nocapture`
#[test]
#[ignore = "timing bench; run in release with --ignored --nocapture"]
fn hour_builder_selection_cost() {
    const PROVINCES: u32 = 3_000;
    let divisions = (0..2 * PROVINCES)
        .map(|index| division(u64::from(index) + 1, index % PROVINCES + 1, 50))
        .collect();
    let statuses = (1..=2 * u64::from(PROVINCES))
        .map(|id| status(id, Some(100)))
        .collect();
    let frame = hour_frame(divisions, statuses, PROVINCES);
    let assets = camouflaged_assets();
    prepare(&assets, &frame);
    let started = std::time::Instant::now();
    let report = prepare(&assets, &frame);
    let elapsed = started.elapsed();
    assert_eq!(report.stacks, PROVINCES as usize);
    println!(
        "{} stacks of {} divisions selected in {:.2} ms",
        report.stacks,
        2 * PROVINCES,
        elapsed.as_secs_f64() * 1e3
    );
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn unit_scale_model_selection() {
    use replay::{
        frame::TableView,
        rows::{CountryLinkRow, CountryRow, TableId, TableRows},
    };
    for count in [50_000u32, 100_000] {
        let provinces = 10_000;
        let divisions = (0..count)
            .map(|i| {
                let mut row = division(u64::from(i) + 1, i % provinces + 1, 50);
                row.roster_owner = i % 1_500 + 1;
                row.logical_owner = row.roster_owner;
                row
            })
            .collect();
        let statuses = (1..=u64::from(count))
            .map(|id| status(id, Some(100)))
            .collect();
        let mut frame = hour_frame(divisions, statuses, provinces);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=1_500)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("TEST{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=1_500)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        let assets = camouflaged_assets();
        let mut times = Vec::new();
        for _ in 0..9 {
            let start = std::time::Instant::now();
            let report = prepare(&assets, &frame);
            times.push(start.elapsed().as_secs_f64() * 1e3);
            assert!(report.stacks > 0);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "UNIT_SCALE name=ModelSelection divisions={count} countries=1500 p50_ms={:.6} max_ms={:.6}",
            times[4], times[8]
        );
    }
}

#[test]
fn a_division_in_both_equipment_tables_takes_the_hourly_rows_once() {
    let row = |division, ordinal, amount_raw| replay::rows::DivisionEquipmentRow {
        division,
        ordinal,
        amount_raw,
        ..Default::default()
    };
    let daily = [row(1, 0, 10), row(1, 1, 11), row(2, 0, 20)];
    let new = [row(1, 0, 99)];
    let equipment = Equipment::join(&daily, &new, true);
    let first: Vec<_> = equipment
        .of(1)
        .unwrap()
        .iter()
        .map(|row| row.amount_raw)
        .collect();
    assert_eq!(first, [99]);
    assert_eq!(equipment.of(2).unwrap().len(), 1);
}

#[test]
fn the_signed_model_number_is_tried_first_and_a_miss_continues_the_ladder() {
    let registered = [
        "GER_infantry_-1_entity",
        "infantry_entity",
        PLACEHOLDER_ENTITY,
    ];
    let lookup = |name: &str| registered.contains(&name);
    let numbered = |number| resolve_model_numbered(&country(), "infantry", None, number, lookup);
    assert_eq!(
        numbered(0xff_u8 as i8),
        Some((Level::Tag, "GER_infantry_-1_entity".into()))
    );
    assert_eq!(numbered(2), Some((Level::Bare, "infantry_entity".into())));
    let unknown = ModelCountry {
        graphical_culture: None,
        ..country()
    };
    // Without the culture, only the first key's cosmetic and tag can be tried.
    assert_eq!(
        resolve_model_numbered(&unknown, "infantry", None, 2, lookup),
        None
    );
    assert_eq!(
        resolve_model_numbered(&country(), "cavalry", None, 2, lookup),
        Some((Level::Placeholder, PLACEHOLDER_ENTITY.into()))
    );
}

#[test]
fn transport_uses_the_original_country_and_its_culture() {
    let dynamic = ModelCountry {
        tag: "D01".into(),
        original_tag: Some("GER".into()),
        ..country()
    };
    let registered = ["GER_transport_entity", "D01_transport_entity"];
    assert_eq!(
        resolve_transport(&dynamic, |name| registered.contains(&name)),
        Some((Level::Tag, "GER_transport_entity".into()))
    );
    assert_eq!(model_owner(5, 9, true), 9);
    assert_eq!(model_owner(5, 9, false), 5);
    assert_eq!(model_owner(5, u32::MAX, true), 5);
}

#[test]
fn the_heading_is_the_spline_tangent_and_zero_points_east() {
    // The engine's heading is atan2(tangent.z, tangent.x), so a route
    // running along +x is zero and one running along +z is a quarter turn.
    let east = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]];
    let (at, facing) = along(&east).expect("a two-province route moves");
    assert!((at[0] - counter_move::SETTLED_ARC).abs() < 1e-3, "{at:?}");
    assert!(facing.abs() < 1e-3, "{facing}");
    let south = [[0.0, 0.0, 0.0], [0.0, 0.0, 100.0]];
    let (_, facing) = along(&south).expect("moves");
    assert!(
        (facing - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
        "{facing}"
    );
    // A one-province route leaves the caller on its province.
    assert_eq!(along(&[[0.0, 0.0, 0.0]]), None);
}

#[test]
fn the_offset_points_the_mesh_s_own_forward_axis_along_the_route() {
    // `world_transform` turns local -Z to `(sin r, 0, -cos r)`, and a unit mesh
    // is authored facing local -Z, so the drawn direction of a stack whose
    // rotation is `heading + MESH_FORWARD_OFFSET` has to be the heading's.
    let forward = |rotation: f32| [rotation.sin(), -rotation.cos()];
    for (route, expect) in [
        ([[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]], [1.0, 0.0]),
        ([[0.0, 0.0, 0.0], [0.0, 0.0, 100.0]], [0.0, 1.0]),
        ([[0.0, 0.0, 0.0], [-100.0, 0.0, 0.0]], [-1.0, 0.0]),
        ([[0.0, 0.0, 0.0], [0.0, 0.0, -100.0]], [0.0, -1.0]),
    ] {
        let (_, heading) = along(&route).expect("moves");
        let drawn = forward(heading + MESH_FORWARD_OFFSET);
        assert!(
            (drawn[0] - expect[0]).abs() < 1e-3 && (drawn[1] - expect[1]).abs() < 1e-3,
            "{drawn:?} against {expect:?}"
        );
    }
}
