use content_reader_mesh::mesh::MeshProducts as _;
use replay_data::rows::VariantDefinitionRow;
use std::sync::Arc;
use viewer_foundation::movement_timing::Etas;
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
    let (track, heights) = viewer_gpu::unit_pose::test_support::test_track();
    let hour = track.place(&heights, 0.0).unwrap();
    let mut prepared = PreparedModels {
        staged_field: Mutex::new(None),
        naval: true,
        targets: None,
        placed: vec![(
            target(5, 1, Kind::Land),
            counter_move::Position::still([0.0; 3]),
        )],
        ships: NavalSources {
            candidates: vec![viewer_unit_history::unit_history::naval::Candidate {
                province: 6,
                identity: 2,
                ordinal: 0,
                slot: 0,
                priority: 3,
                ours: true,
                class: viewer_runtime::unit_class::CounterClass::TaskForce,
                land_eligible: false,
                sample_count: 0,
                next_arc: 0.0,
                route: Vec::new(),
                speed: 12.0,
                min_arc: 0.0,
                arrival_allowed: true,
                day: 0,
            }],
            formations: vec![viewer_unit_history::unit_history::naval::Formation {
                province: 6,
                identity: 2,
                slot: 0,
                roster: vec![viewer_unit_history::unit_history::naval::Hull {
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
        convoys: None,
        owned: BTreeSet::new(),
        report: ModelReport::default(),
    };
    let unstaged = prepared.field(&assets);
    prepared.stage_field(&assets);
    let staged_instances = prepared
        .staged_field
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .field
        .models
        .instances
        .as_ptr();
    let field = prepared.field(&assets);
    assert_eq!(
        field.models.instances.as_ptr(),
        staged_instances,
        "first view takes ownership"
    );
    assert!(prepared.staged_field.lock().unwrap().is_none());
    let repeated = prepared.field(&assets);
    assert_eq!(field.models, unstaged.models);
    assert_eq!(field.models, repeated.models);
    assert_eq!(field.targets, repeated.targets);
    assert_eq!(field.raids, repeated.raids);
    assert_eq!(
        field.naval.settled_targets(assets.gates),
        repeated.naval.settled_targets(assets.gates)
    );
    assert_eq!(field.targets.as_ptr(), repeated.targets.as_ptr());
    prepared.stage_field(&assets);
    prepared.add_raid_units(&assets, &[]);
    assert!(
        prepared.staged_field.lock().unwrap().is_none(),
        "adding raids clears staging"
    );
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
fn convoys_sail_only_where_they_show_and_name_entities_that_do_not_bake() {
    use crate::trade_routes::{ConvoyDefines, ConvoyLine};
    let assets = UnitModelAssets {
        disabled: None,
        ..UnitModelAssets::off(String::new())
    };
    let mut prepared = PreparedModels {
        staged_field: Mutex::new(None),
        naval: true,
        targets: None,
        ships: NavalSources::default(),
        raid: Vec::new(),
        convoys: None,
        placed: Vec::new(),
        owned: BTreeSet::new(),
        report: ModelReport::default(),
    };
    let defines = ConvoyDefines {
        scale_factor: 0.3,
        max: 4,
        speed: 0.6,
        sleep: 3.0,
        route_offset: 0.5,
        cutoff: 120.0,
    };
    prepared.add_convoys(Vec::new(), defines);
    assert!(!prepared.has_convoys());
    prepared.add_convoys(
        vec![ConvoyLine {
            entity: "westerngfx_traderoute_convoy".into(),
            map_width: 100.,
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            count: 2,
            seed: 0,
        }],
        defines,
    );
    assert!(prepared.has_convoys());
    let (lines, defines, fleet) = prepared.convoys.as_mut().unwrap();
    *fleet.get_mut().unwrap() = Some(crate::trade_routes::ConvoyFleet::new(
        lines,
        defines,
        &|| 7,
        &|_| None,
        &|_, _| 0.,
    ));
    let mut next = PreparedModels {
        staged_field: Mutex::new(None),
        naval: true,
        targets: None,
        ships: NavalSources::default(),
        raid: Vec::new(),
        convoys: None,
        placed: Vec::new(),
        owned: BTreeSet::new(),
        report: ModelReport::default(),
    };
    next.add_convoys(lines.clone(), *defines);
    next.inherit_convoys(&prepared);
    assert!(next.convoys.as_ref().unwrap().2.lock().unwrap().is_some());
    assert!(
        prepared
            .convoys
            .as_ref()
            .unwrap()
            .2
            .lock()
            .unwrap()
            .is_none()
    );
    let mut refused = BTreeMap::new();
    let hidden = prepared.convoy_units(&assets, 1.0, &|_, _| 0.0, |_| false, &mut refused);
    assert!(hidden.is_empty());
    assert!(refused.is_empty());
    let shown = prepared.convoy_units(&assets, 1.0, &|_, _| 0.0, |_| true, &mut refused);
    assert!(shown.is_empty());
    assert_eq!(
        refused.get(
            "convoy entity westerngfx_traderoute_convoy: creation-time bounding sphere unavailable"
        ),
        Some(&2)
    );
    assert!(
        refused
            .keys()
            .any(|reason| reason.contains("native counter seed"))
    );
}

#[test]
fn hourly_mesh_upload_preserves_the_inherited_convoy_clock() {
    use crate::trade_routes::{ConvoyDefines, ConvoyFleet, ConvoyLine};
    let assets = UnitModelAssets {
        disabled: None,
        ..UnitModelAssets::off(String::new())
    };
    let lines = vec![ConvoyLine {
        entity: "synthetic_convoy".into(),
        map_width: 20_000.0,
        points: vec![[0.0, 0.0], [10_000.0, 0.0]],
        count: 1,
        seed: 0,
    }];
    let defines = ConvoyDefines {
        scale_factor: 1.0,
        max: 1,
        speed: 1.0,
        sleep: 3.0,
        route_offset: 0.0,
        cutoff: 120.0,
    };
    let height = |_, _| 0.0;
    let make = || {
        let mut prepared = PreparedModels {
            staged_field: Mutex::new(None),
            naval: true,
            targets: None,
            ships: NavalSources::default(),
            raid: Vec::new(),
            convoys: None,
            placed: Vec::new(),
            owned: BTreeSet::new(),
            report: ModelReport::default(),
        };
        prepared.add_convoys(lines.clone(), defines);
        prepared.stage_field(&assets);
        assert!(
            prepared
                .convoys
                .as_ref()
                .unwrap()
                .2
                .lock()
                .unwrap()
                .is_none()
        );
        prepared
    };
    let fleet = || ConvoyFleet::new(&lines, &defines, &|| 7, &|_| Some(1.0), &height);
    let mut reverse = fleet();
    let negative = ConvoyDefines { speed: -2., ..defines };
    let camera = viewer_gpu::camera::Placement::map([1500., 900.], 40.)
        .on(viewer_gpu::camera::MapSize::TEST, viewer_gpu::camera::MapDefines::test());
    let view = viewer_models::motion_view::MotionView::new(&camera, [800, 600]);
    let wake = |fleet: &ConvoyFleet| fleet.next_arrival(&lines, &assets, &negative, &view, &height);
    assert_eq!(wake(&reverse), 0.);
    reverse.advance(&defines, 0., &height);
    assert!(reverse.advance(&defines, 20_000., &height).is_empty());
    let launch_in = wake(&reverse);
    assert!(launch_in > 0. && launch_in <= defines.sleep);
    assert_eq!(wake(&ConvoyFleet::default()), f32::INFINITY);
    let mut reference = fleet();
    let initial = reference.advance(&defines, 120.0, &height);
    let expected = reference.advance(&defines, 121.0, &height);
    assert_eq!(initial.len(), 1);
    assert_eq!(expected.len(), 1);
    assert!((expected[0].position[0] - initial[0].position[0] - 1.0).abs() < 0.001);

    let mut previous = make();
    let mut sailing = fleet();
    assert_eq!(sailing.advance(&defines, 120.0, &height), initial);
    *previous.convoys.as_mut().unwrap().2.get_mut().unwrap() = Some(sailing);
    let next = Arc::new(make());
    next.inherit_convoys(&previous);
    next.field(&assets);
    next.convoy_units(&assets, 121.0, &height, |_| true, &mut BTreeMap::new());
    let actual = next
        .convoys
        .as_ref()
        .unwrap()
        .2
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .advance(&defines, 121.0, &height);
    assert_eq!(actual, expected, "hour upload must not replay elapsed time");
    let reused = Arc::clone(&next);
    assert!(Arc::ptr_eq(&next, &reused));
    reused.reset_convoys();
    assert!(next.convoys.as_ref().unwrap().2.lock().unwrap().is_none());
    let mut fresh = make();
    fresh.field(&assets);
    assert!(fresh.convoys.as_ref().unwrap().2.lock().unwrap().is_none());
    fresh.stage_field(&assets);
    fresh.add_convoys(Vec::new(), defines);
    assert!(fresh.staged_field.lock().unwrap().is_none());
}

#[test]
fn recorded_stack_and_combat_roles_select_native_states() {
    use viewer_models::entity_animation::*;
    let mut route = Route::default();
    assert_eq!(animation_state(Some(&route), None, Some(false)), Some(IDLE));
    assert_eq!(
        animation_state(Some(&route), None, Some(true)),
        Some(TRAINING)
    );
    route.stack_kind = 19;
    assert_eq!(animation_state(Some(&route), None, Some(true)), Some(IDLE));
    route.stack_kind = 10;
    assert_eq!(
        animation_state(Some(&route), None, Some(false)),
        Some(DEFEND)
    );
    route.stack_kind = 1;
    route.path = vec![42];
    assert_eq!(animation_state(Some(&route), None, Some(false)), Some(MOVE));
    route.retreat = true;
    assert_eq!(
        animation_state(Some(&route), None, Some(false)),
        Some(RETREAT)
    );
    route.combat_role = 1;
    assert_eq!(
        animation_state(Some(&route), None, Some(false)),
        Some(ATTACK)
    );
    route.path.clear();
    assert_eq!(
        animation_state(Some(&route), None, Some(false)),
        Some(SUPPORT)
    );
    assert_eq!(
        animation_state(None, Some(DEFEND), Some(false)),
        Some(DEFEND)
    );
    assert_eq!(animation_state(Some(&route), None, None), Some(SUPPORT));
    assert_eq!(animation_state(None, Some(DEFEND), None), Some(DEFEND));
    assert_eq!(animation_state(Some(&Route::default()), None, None), None);
    assert_eq!(animation_state(None, None, None), None);
}

#[test]
fn animation_selection_prioritizes_combat_and_retreat_over_movement() {
    use viewer_models::entity_animation::*;
    let moving = Route {
        stack_kind: 11,
        path: vec![2],
        ..Route::default()
    };
    assert_eq!(
        animation_state(Some(&moving), None, Some(false)),
        Some(MOVE)
    );
    assert_eq!(
        animation_state(Some(&moving), Some(ATTACK), Some(false)),
        Some(ATTACK)
    );
    assert_eq!(
        animation_state(
            Some(&Route {
                path: vec![],
                ..moving
            }),
            Some(ATTACK),
            Some(false)
        ),
        Some(SUPPORT)
    );
    assert_eq!(
        animation_state(
            Some(&Route {
                retreat: true,
                ..moving.clone()
            }),
            None,
            Some(false)
        ),
        Some(RETREAT)
    );
    assert_eq!(
        animation_state(
            Some(&Route {
                stack_kind: 10,
                ..moving
            }),
            Some(ATTACK),
            Some(false)
        ),
        Some(DEFEND)
    );
    assert_eq!(
        animation_state(None, Some(DEFEND), Some(false)),
        Some(DEFEND)
    );
    assert_eq!(
        animation_state(Some(&Route::default()), None, Some(true)),
        Some(TRAINING)
    );
}

fn along(anchors: &[[f32; 3]]) -> Option<([f32; 3], f32)> {
    counter_move::Motion::new(5632.0, anchors).map(|motion| motion.position_heading())
}

fn names(candidates: Vec<(Level, String)>) -> Vec<String> {
    candidates.into_iter().map(|(_, name)| name).collect()
}

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
fn the_ladder_is_key_outermost_and_prefix_innermost() {
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
    let row = |division| replay_data::rows::DivisionEquipmentRow {
        division,
        ..Default::default()
    };
    let (daily, new) = ([row(1)], [row(2)]);
    let equipment = |new_complete| Equipment::join(&daily, &new, new_complete);
    assert_eq!(equipment(true).of(2).unwrap().len(), 1);
    assert!(equipment(true).of(3).unwrap().is_empty());
    assert!(equipment(false).of(3).is_err());
}

fn division(id: u64, province: u32, template: u64) -> replay_data::rows::DivisionRow {
    replay_data::rows::DivisionRow {
        id,
        roster_owner: 100,
        logical_owner: 100,
        province,
        template,
        ..Default::default()
    }
}

fn status(division: u64, readable: bool) -> replay_data::rows::DivisionStatusRow {
    replay_data::rows::DivisionStatusRow {
        division,
        camouflage: replay_data::rows::Recorded::Value(
            readable.then_some(replay_data::rows::DivisionCamouflage { category: Some(1) }),
        ),
        ..Default::default()
    }
}

fn hour_frame(
    divisions: Vec<replay_data::rows::DivisionRow>,
    statuses: Vec<replay_data::rows::DivisionStatusRow>,
    provinces: u32,
) -> DecodedFrame {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::ProvinceRow,
        rows::TableId, rows::TableRows, rows::WeatherProvinceRow, rows::WeatherRegionRow,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
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

fn prepare_frame() -> DecodedFrame {
    hour_frame(
        vec![division(1, 9, 50), division(2, 8, 51), division(3, 7, 50)],
        vec![status(1, true), status(3, false)],
        9,
    )
}

mod aircraft_designer;

mod model_witness;

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

#[test]
fn unit_models_use_native_camera_admission_at_the_edges() {
    use viewer_buildings::map_buildings::BuildingHistory;
    use viewer_unit_history::unit_history::{History, test_support::{GATES, target}};
    use viewer_gpu::{camera::{MapDefines, MapSize, Placement}, province_admission::edge_boxes};
    let bounds = edge_boxes();
    assert_eq!(bounds[&5], [3044.0, 1000.0, 3046.0, 1002.0]);
    let mut assets = UnitModelAssets::off(String::new());
    assets.bounds = Arc::new(bounds);
    let targets = (1..=6)
        .map(|id| target(id, if id == 2 { Kind::Ship } else { Kind::Land }))
        .collect();
    let mut history = History::default();
    history.set(targets, Arc::clone(&assets.bounds), true);
    let camera = Placement {
        center: [3000.0, 1000.0],
        height: 50.0,
        pose: viewer_gpu::camera::Pose::Map,
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
    let created: Vec<_> = history
        .drawn(&frame.units)
        .iter()
        .map(|(_, instance)| instance.animation_seed)
        .collect();
    assert_eq!(created, [1, 2, 5, 6]);
}

fn prepare(assets: &UnitModelAssets, frame: &DecodedFrame) -> ModelReport {
    equipment_graphics::test_support::with_world(frame, &[], |limits| {
        assets
            .prepare(PrepareInputs {
                frame,
                limits,
                vision: &Vision::default(),
                etas: &Etas::default(),
                anchor: &|_, _| Some([0.0; 3]),
                world: None,
                terrain: &BTreeMap::new(), routes: None,
                ships: &ShipPlaces {
                    motions: &viewer_unit_reads::task_force_visual::TaskForceMotions::default(
                    ),
                    movement: None,
                    land: &|_| true,
                    missions: &Default::default(),
                    region: &|_| None,
                    scale: &|_, _| 1.0,
                },
                localisation: &content_reader_localisation::localisation::Localisation::default(),
            })
            .unwrap()
            .report
    })
}

#[test]
fn an_hour_selects_the_designer_model_and_its_snow_camouflage_per_stack() {
    let report = prepare(&camouflaged_assets(), &prepare_frame());
    assert_eq!((report.stacks, report.untemplated), (3, 1));
    assert_eq!(
        report.selection_errors,
        BTreeMap::from([
            ("division camouflage is unavailable this hour".to_owned(), 1),
            (
                "registered entity GER_custom_entity_snow did not bake".to_owned(),
                1
            ),
        ])
    );
}

#[test]
fn a_stack_chosen_alone_takes_what_the_whole_hour_takes() {
    let divisions = vec![
        division(1, 9, 50),
        division(2, 8, 51),
        division(3, 7, 50),
        division(4, 6, 50),
    ];
    let frame = hour_frame(
        divisions.clone(),
        vec![status(1, true), status(3, false)],
        9,
    );
    let whole = prepare(&camouflaged_assets(), &frame);
    let mut alone = ModelReport::default();
    let mut entities = BTreeMap::new();
    for division in &divisions {
        let assets = camouflaged_assets();
        let key = SelectionKey {
            transport: false,
            template: division.template,
            oldest: division.id,
            roster_owner: division.roster_owner,
            expeditionary_owner: division.expeditionary_owner,
        };
        let model = equipment_graphics::test_support::with_world(&frame, &[], |limits| {
            assets
                .land_models(
                    SelectInputs {
                        frame: &frame,
                        limits,
                        ours: Vision::default().ours(),
                        terrain: &BTreeMap::new(),
                        localisation: &content_reader_localisation::localisation::Localisation::default(),
                        divisions: None,
                        lap: None,
                    },
                    &KeptChoices::default(),
                    &Told::default(),
                    |models| models.choose(division.province, key),
                )
                .unwrap()
        });
        match model {
            StackModel::Chosen(model) => {
                assets.ensure(std::slice::from_ref(&model.entity));
                if let Err(reason) = assets.chosen_slot(&assets.library(), &model.entity) {
                    alone.error(reason);
                }
                entities.insert(division.id, (model.level, model.entity));
            }
            StackModel::UnknownIdentity => alone.unknown_identity += 1,
            StackModel::Untemplated => alone.untemplated += 1,
            StackModel::Withheld(reason) => alone.error(reason),
        }
    }
    assert_eq!(
        entities,
        BTreeMap::from([(1, (Level::Template, "GER_custom_entity_snow".to_owned()))])
    );
    assert_eq!(
        (alone.selection_errors, alone.untemplated, alone.unknown_identity),
        (whole.selection_errors, whole.untemplated, whole.unknown_identity)
    );
}

struct Session {
    made: Made,
    stood: Vec<bool>,
    chosen: Vec<Result<String, String>>,
}

fn sessions(
    assets: &UnitModelAssets,
    frames: &[DecodedFrame],
    stacks: &[(u32, u64, u64)],
) -> Vec<Session> {
    let told: Vec<_> = frames
        .iter()
        .map(|frame| (frame.clone(), Told::default()))
        .collect();
    told_sessions(assets, &told, stacks)
}

fn told_sessions(
    assets: &UnitModelAssets,
    frames: &[(DecodedFrame, Told)],
    stacks: &[(u32, u64, u64)],
) -> Vec<Session> {
    let kept = KeptChoices::default();
    frames
        .iter()
        .map(|(frame, told)| {
            equipment_graphics::test_support::with_world(frame, &[], |limits| {
                assets
                    .land_models(
                        SelectInputs {
                            frame,
                            limits,
                            ours: Vision::default().ours(),
                            terrain: &BTreeMap::new(),
                            localisation:
                                &content_reader_localisation::localisation::Localisation::default(),
                            divisions: None,
                        lap: None,
                        },
                        &kept,
                        told,
                        |models| {
                            let keys: Vec<_> = stacks
                                .iter()
                                .map(|&(province, oldest, template)| {
                                    let key = SelectionKey {
                                        transport: false,
                                        template,
                                        oldest,
                                        roster_owner: 100,
                                        expeditionary_owner: 0,
                                    };
                                    (province, key)
                                })
                                .collect();
                            let stood = keys
                                .iter()
                                .map(|&(province, key)| models.stands(province, key))
                                .collect();
                            let chosen = keys
                                .iter()
                                .map(|&(province, key)| match models.choose(province, key) {
                                    StackModel::Chosen(model) => Ok(model.entity),
                                    other => Err(format!("{other:?}")),
                                })
                                .collect();
                            Session {
                                made: models.made(),
                                stood,
                                chosen,
                            }
                        },
                    )
                    .unwrap()
            })
        })
        .collect()
}

#[test]
fn a_row_a_kept_selection_read_drops_that_selection_and_no_other() {
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::TableView;
    let templates = |second: &[u8]| {
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 50,
                model_entity: b"GER_custom_entity".to_vec(),
                ..Default::default()
            },
            DivisionTemplateIconRow {
                template: 52,
                model_entity: second.to_vec(),
                ..Default::default()
            },
        ]))
        .into()
    };
    let mut first = hour_frame(
        vec![division(1, 1, 50), division(3, 4, 52)],
        vec![status(1, true), status(3, true)],
        9,
    );
    first.tables[TableId::DivisionTemplateIcons.index()] = templates(b"GER_other_entity");
    let mut second = first.clone();
    second.tables[TableId::DivisionTemplateIcons.index()] = templates(b"GER_custom_entity");
    let third = second.clone();
    let mut assets = camouflaged_assets();
    assets.registered.insert("GER_other_entity".to_owned());
    let stacks = [(1, 1, 50), (4, 3, 52)];
    let hours = sessions(&assets, &[first, second, third], &stacks);
    let made: Vec<_> = hours.iter().map(|session| session.made).collect();
    assert_eq!(
        made,
        [
            Made {
                selections: 2,
                camouflages: 2,
                contexts: 1
            },
            Made {
                selections: 1,
                ..Made::default()
            },
            Made::default(),
        ]
    );
    let stood: Vec<_> = hours.iter().map(|session| session.stood.clone()).collect();
    assert_eq!(stood, [[false, false], [true, false], [true, true]]);
    let snow = Ok("GER_custom_entity_snow".to_owned());
    assert_eq!(
        hours[0].chosen,
        [snow.clone(), Ok("GER_other_entity".to_owned())]
    );
    assert_eq!(hours[1].chosen, [snow.clone(), snow.clone()]);
    assert_eq!(hours[2].chosen, [snow.clone(), snow]);
}

#[test]
fn a_weather_or_status_change_makes_again_the_camouflage_that_read_it_and_no_other() {
    use replay_data::rows::{ProvinceRow, TableId, TableRows, WeatherProvinceRow};
    use replay_frame::frame::TableView;
    let weather = |dry: &[u32]| {
        TableView::Complete(TableRows::WeatherProvinces(
            (0..=9)
                .map(|id| WeatherProvinceRow {
                    id,
                    snow: i64::from(!dry.contains(&id)),
                    ..Default::default()
                })
                .collect(),
        ))
        .into()
    };
    let mut first = hour_frame(
        vec![division(1, 1, 50), division(3, 4, 50), division(5, 7, 50)],
        vec![status(1, true), status(3, true), status(5, true)],
        9,
    );
    first.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=9)
            .map(|id| ProvinceRow {
                id,
                state: [1, 1, 1, 2, 2, 2, 0, 0, 0][id as usize - 1],
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    first.tables[TableId::WeatherProvinces.index()] = weather(&[4]);
    let mut second = first.clone();
    second.tables[TableId::WeatherProvinces.index()] = weather(&[4, 6]);
    let mut third = second.clone();
    third.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![
            status(1, true),
            status(3, true),
            status(5, false),
        ]))
        .into();
    let stacks = [(1, 1, 50), (4, 3, 50), (7, 5, 50)];
    let frames = [first, second, third];
    let kept = sessions(&camouflaged_assets(), &frames, &stacks);
    let made: Vec<_> = kept.iter().map(|session| session.made).collect();
    assert_eq!(
        made,
        [
            Made {
                selections: 3,
                camouflages: 3,
                contexts: 1
            },
            Made {
                camouflages: 1,
                ..Made::default()
            },
            Made {
                camouflages: 1,
                ..Made::default()
            },
        ]
    );
    let snow = Ok("GER_custom_entity_snow".to_owned());
    assert_eq!(kept[0].chosen, [snow.clone(), snow.clone(), snow.clone()]);
    assert_eq!(
        kept[1].chosen,
        [snow.clone(), Ok("GER_custom_entity".to_owned()), snow]
    );
    for (hour, frame) in frames.iter().enumerate() {
        let alone = sessions(&camouflaged_assets(), std::slice::from_ref(frame), &stacks);
        assert_eq!(kept[hour].chosen, alone[0].chosen, "hour {hour}");
    }
    assert!(
        kept[2].chosen[2]
            .as_ref()
            .is_err_and(|why| why.contains("division camouflage is unavailable this hour")),
        "{:?}",
        kept[2].chosen
    );
}

#[test]
fn a_session_told_the_rows_made_anew_takes_them_and_compares_no_table() {
    use replay_data::rows::{CountryRow, TableId, TableRows};
    use replay_frame::frame::{SharedView, TableView};
    let templates = |second: &[u8]| -> SharedView {
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 50,
                model_entity: b"GER_custom_entity".to_vec(),
                ..Default::default()
            },
            DivisionTemplateIconRow {
                template: 52,
                model_entity: second.to_vec(),
                ..Default::default()
            },
        ]))
        .into()
    };
    let at = TableId::DivisionTemplateIcons.index();
    let mut first = hour_frame(
        vec![division(1, 1, 50), division(3, 4, 52)],
        vec![status(1, true), status(3, true)],
        9,
    );
    first.tables[at] = templates(b"GER_other_entity");
    let mut second = first.clone();
    second.tables[at] = templates(b"GER_other_entity");
    let told_second = Told::new(vec![(
        TableId::DivisionTemplateIcons,
        first.tables[at].clone(),
        vec![(50, u64::MAX)],
    )]);
    let mut third = second.clone();
    third.tables[at] = templates(b"GER_custom_entity");
    let told_third = Told::new(vec![(
        TableId::DivisionTemplateIcons,
        first.tables[at].clone(),
        Vec::new(),
    )]);
    let mut fourth = third.clone();
    fourth.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..Default::default()
        }]))
        .into();
    let hours = [
        (first, Told::default()),
        (second, told_second),
        (third, told_third),
        (fourth, Told::default()),
    ];
    let mut assets = camouflaged_assets();
    assets.registered.insert("GER_other_entity".to_owned());
    let stacks = [(1, 1, 50), (4, 3, 52)];
    let kept = told_sessions(&assets, &hours, &stacks);
    let made: Vec<_> = kept.iter().map(|session| session.made).collect();
    assert_eq!(
        made,
        [
            Made {
                selections: 2,
                camouflages: 2,
                contexts: 1
            },
            Made {
                selections: 1,
                ..Made::default()
            },
            Made {
                selections: 1,
                ..Made::default()
            },
            Made {
                contexts: 1,
                ..Made::default()
            },
        ]
    );
    let stood: Vec<_> = kept.iter().map(|session| session.stood.clone()).collect();
    assert_eq!(
        stood,
        [[false, false], [false, true], [true, false], [true, true]]
    );
    let snow = Ok("GER_custom_entity_snow".to_owned());
    assert_eq!(kept[2].chosen, [snow.clone(), snow]);
    for (hour, (frame, _)) in hours.iter().enumerate() {
        let alone = sessions(&assets, std::slice::from_ref(frame), &stacks);
        assert_eq!(kept[hour].chosen, alone[0].chosen, "hour {hour}");
    }
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
fn unit_graphics_defines_a_mod_breaks_load_with_the_recorded_ones() {
    let defines = |cutoff: &str| {
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        defines
            .load(
                "common/defines/00_defines.lua",
                format!(
                    "NDefines = {{ NGraphics = {{ UNITS_DISTANCE_CUTOFF = {cutoff},                      SHIPS_DISTANCE_CUTOFF = 1, UNIT_ARROW_DISTANCE_CUTOFF = 1,                      SHIP_IN_PORT_SCALE = 1, MESH_POPUP_SCALE_UP_SPEED = 1,                      MESH_POPUP_SCALE_DOWN_SPEED = 1, SHIP_POPUP_SCALE_DOWN_SPEED = 1,                      NAVAL_UNIT_MOVEMENT_SPEED = 1, LAND_UNIT_MOVEMENT_SPEED = 1 }} }}"
                )
                .as_bytes(),
            )
            .unwrap();
        defines
    };
    let (shown, recorded) = (defines("\"far\""), defines("200"));
    let load = |_: LoadStack,
                defines: &content_reader_defines::defines::Defines,
                _: &mut Vec<content_reader_parser::Diagnostic>| {
        Ok(UnitModelAssets {
            gates: Gates::read(defines)?,
            disabled: None,
            ..UnitModelAssets::off(String::new())
        })
    };
    let mut warnings = Vec::new();
    let mut refusal = None;
    let layers = ResolvedLayers::default();
    let assets = UnitModelAssets::load_with(
        &layers,
        &shown,
        Some(&recorded),
        false,
        &mut warnings,
        |why| refusal = Some(why),
        load,
    );
    assert!(assets.disabled.is_none(), "{warnings:?}");
    assert!(assets.gates.units(150.0) && !assets.gates.units(250.0));
    let refusal = refusal.unwrap();
    assert!(
        refusal.starts_with("graphics defines for the unit models (")
            && refusal.contains("UNITS_DISTANCE_CUTOFF"),
        "{refusal}"
    );
    let assets =
        UnitModelAssets::load_with(&layers, &shown, None, false, &mut warnings, |_| {}, load);
    assert!(assets.disabled.is_some());
    assert!(layers.refused().contains(LAYER_OFF));
}

#[test]
fn a_unit_model_layer_the_mods_break_loads_from_the_recording() {
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    let mut stacks = Vec::new();
    let load = |stack: LoadStack,
                _: &content_reader_defines::defines::Defines,
                _: &mut Vec<content_reader_parser::Diagnostic>| {
        stacks.push(stack);
        match stack {
            LoadStack::Shown => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gfx/naval_combat.txt: no formation",
            )),
            LoadStack::Recorded => Ok(UnitModelAssets {
                disabled: None,
                ..UnitModelAssets::off(String::new())
            }),
        }
    };
    let layers = ResolvedLayers::default();
    let mut warnings = Vec::new();
    let mut refusal = None;
    let assets = UnitModelAssets::load_with(
        &layers,
        &defines,
        Some(&defines),
        true,
        &mut warnings,
        |why| refusal = Some(why),
        load,
    );
    assert!(assets.disabled.is_none(), "{warnings:?}");
    assert_eq!(
        stacks,
        [LoadStack::Shown, LoadStack::Shown, LoadStack::Recorded]
    );
    let refusal = refusal.unwrap();
    assert!(
        refusal.starts_with("unit models (") && refusal.contains("naval_combat.txt"),
        "{refusal}"
    );
    assert!(layers.refused().is_empty());
}

#[test]
fn an_unreadable_template_leaves_its_stacks_unmodelled_and_counted() {
    use replay_data::{rows::DivisionTemplateIconRow, rows::TableId, rows::TableRows};
    let mut frame = prepare_frame();
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        replay_frame::frame::TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 50,
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

#[test]
fn a_division_in_both_equipment_tables_takes_the_hourly_rows_once() {
    let row = |division, ordinal, amount_raw| replay_data::rows::DivisionEquipmentRow {
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
fn a_keyed_lookup_takes_what_the_whole_join_took() {
    use replay_data::rows::{DivisionEquipmentRow, DivisionModelInputRow, TableId, TableRows};
    let equipment = |division, ordinal, amount_raw| DivisionEquipmentRow {
        division,
        ordinal,
        amount_raw,
        ..Default::default()
    };
    let daily = [
        equipment(1, 0, 10),
        equipment(1, 1, 11),
        equipment(2, 0, 20),
        equipment(4, 0, 40),
        equipment(4, 1, 41),
    ];
    let new = [equipment(1, 0, 99), equipment(3, 0, 30), equipment(3, 1, 31)];
    let mut joined: BTreeMap<u64, Vec<&DivisionEquipmentRow>> = BTreeMap::new();
    for row in &new {
        joined.entry(row.division).or_default().push(row);
    }
    let hourly: BTreeSet<u64> = joined.keys().copied().collect();
    for row in daily.iter().filter(|row| !hourly.contains(&row.division)) {
        joined.entry(row.division).or_default().push(row);
    }
    let amounts = |rows: &[DivisionEquipmentRow]| -> Vec<i64> { rows.iter().map(|row| row.amount_raw).collect() };
    for new_complete in [true, false] {
        let lookup = Equipment::join(&daily, &new, new_complete);
        for division in 0..=5 {
            let was: Result<Vec<i64>, String> = match joined.get(&division) {
                Some(rows) => Ok(rows.iter().map(|row| row.amount_raw).collect()),
                None if new_complete => Ok(Vec::new()),
                None => Err("division equipment unknown until the next daily refresh".to_owned()),
            };
            assert_eq!(lookup.of(division).map(amounts), was, "division {division}");
        }
    }

    let statuses = [status(2, true), status(5, false), status(9, true)];
    let witnesses = [2, 7].map(|division| DivisionModelInputRow {
        division,
        model_number: Some(division as i32),
        ..Default::default()
    });
    for (lookup, joined) in [
        (
            Statuses::Status(&statuses),
            statuses
                .iter()
                .map(|row| (row.division, ModelStatus::Status(row)))
                .collect::<BTreeMap<_, _>>(),
        ),
        (
            Statuses::Witness(&witnesses),
            witnesses
                .iter()
                .map(|row| (row.division, ModelStatus::Witness(row)))
                .collect(),
        ),
    ] {
        for division in 0..=10 {
            let read = |status: ModelStatus<'_>| (status.division(), status.model_number());
            assert_eq!(
                lookup.get(division).map(read),
                joined.get(&division).copied().map(read),
                "division {division}"
            );
        }
    }

    let variants: Vec<_> = [3, 10, 11, 40]
        .map(|id| VariantDefinitionRow {
            id,
            equipment_token: id as u32 + 100,
            ..Default::default()
        })
        .into();
    let joined: BTreeMap<_, _> = variants.iter().map(|row| (row.id, row)).collect();
    let lookup = model_selection::Variants::new(&variants);
    for id in 0..=41 {
        assert_eq!(lookup.get(id), joined.get(&id).copied(), "variant {id}");
    }

    let mut frame = prepare_frame();
    let templates: Vec<_> = [10, 50, 70]
        .map(|template| DivisionTemplateIconRow {
            template,
            priority_unit: template as u32,
            ..Default::default()
        })
        .into();
    let joined: BTreeMap<_, _> = templates.iter().map(|row| (row.template, row)).collect();
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        replay_frame::frame::TableView::Complete(TableRows::DivisionTemplateIcons(
            templates.clone(),
        ));
    let assets = camouflaged_assets();
    let terrain = BTreeMap::new();
    let localisation = content_reader_localisation::localisation::Localisation::default();
    equipment_graphics::test_support::with_world(&frame, &[], |limits| {
        let hour = Hour::read(&assets, &frame, &terrain, limits, &localisation).unwrap();
        for template in 0..=80 {
            assert_eq!(
                hour.template(template),
                joined.get(&template).copied(),
                "template {template}"
            );
        }
    });
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
    assert_eq!(along(&[[0.0, 0.0, 0.0]]), None);
}

#[test]
fn the_offset_points_the_mesh_s_own_forward_axis_along_the_route() {
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

fn appearance_frame(cosmetic: &[u8], culture: &[u8]) -> DecodedFrame {
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::TableId,
        rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for this hour".into(),
    });
    for (id, rows) in [
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
            TableId::CountryAppearance,
            TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 100,
                cosmetic_tag: cosmetic.to_vec(),
                graphical_culture: culture.to_vec(),
                ..Default::default()
            }]),
        ),
    ] {
        views[id.index()] = TableView::Complete(rows);
    }
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn walker_assets(registered: &[&str], sub_units: Option<&[u8]>) -> UnitModelAssets {
    UnitModelAssets {
        sub_units: sub_units.map_or_else(model_selection::Content::default, |text| {
            model_selection::Content::from_test_documents(&[text])
        }),
        registered: registered.iter().map(|name| (*name).to_owned()).collect(),
        disabled: None,
        ..UnitModelAssets::off(String::new())
    }
}

#[test]
fn the_walker_is_the_viewing_country_s_infantry_by_the_land_ladder() {
    let frame = appearance_frame(b"", b"western_european_gfx");
    let entity = |registered: &[&str]| walker_assets(registered, None).infantry_entity(&frame, 1);
    let every = [
        "GER_infantry_entity",
        "western_european_gfx_infantry_entity",
        "infantry_entity",
        PLACEHOLDER_ENTITY,
    ];
    for (skip, level) in [
        (0, Level::Tag),
        (1, Level::Culture),
        (2, Level::Bare),
        (3, Level::Placeholder),
    ] {
        assert_eq!(entity(&every[skip..]), Ok((level, every[skip].to_owned())));
    }
    let missing = entity(&[]).unwrap_err();
    assert!(missing.contains(PLACEHOLDER_ENTITY), "{missing}");
    let cosmetic = appearance_frame(b"GER_kaiser", b"western_european_gfx");
    assert_eq!(
        walker_assets(&["GER_kaiser_infantry_entity", "GER_infantry_entity"], None)
            .infantry_entity(&cosmetic, 1),
        Ok((Level::Cosmetic, "GER_kaiser_infantry_entity".to_owned()))
    );
    let unreadable = appearance_frame(b"", &[0xff]);
    assert_eq!(
        walker_assets(&["GER_infantry_entity"], None).infantry_entity(&unreadable, 1),
        Ok((Level::Tag, "GER_infantry_entity".to_owned()))
    );
    let refused = walker_assets(&["infantry_entity"], None)
        .infantry_entity(&unreadable, 1)
        .unwrap_err();
    assert!(refused.contains("graphical culture"), "{refused}");
    assert!(
        walker_assets(&every, None)
            .infantry_entity(&frame, 2)
            .is_err()
    );
    assert_eq!(
        UnitModelAssets::off("no descriptors".into()).infantry_entity(&frame, 1),
        Err("no descriptors".to_owned())
    );
}

#[test]
fn the_walker_s_sprite_key_follows_its_own() {
    let frame = appearance_frame(b"", b"western_european_gfx");
    let rifleman: Option<&[u8]> = Some(br"sub_units = { infantry = { sprite = rifleman } }");
    let assets = walker_assets(
        &["western_european_gfx_rifleman_entity", "infantry_entity"],
        rifleman,
    );
    assert_eq!(
        assets.infantry_entity(&frame, 1),
        Ok((Level::Bare, "infantry_entity".to_owned()))
    );
    let assets = walker_assets(
        &["western_european_gfx_rifleman_entity", PLACEHOLDER_ENTITY],
        rifleman,
    );
    assert_eq!(
        assets.infantry_entity(&frame, 1),
        Ok((
            Level::Culture,
            "western_european_gfx_rifleman_entity".to_owned()
        ))
    );
}

#[test]
fn an_entity_a_mods_unreadable_descriptor_leaves_undecided_is_drawn_from_the_recording() {
    use viewer_models::test_support::{MESHES_GFX, PLANE_ASSET, triangle_mesh, visual_stack};
    let (root, layers) = visual_stack(
        "undecided",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            (
                "art/gfx/entities/broken.asset",
                br#"entity = { name = "plane" zeppelin } }{ = ="#,
            ),
        ],
    );
    let presentation = layers.presentation().clone();
    let mut assets = UnitModelAssets::off(String::new());
    assets.roster = presentation.descriptor_roster().unwrap();
    assets.layers = presentation.clone();
    assets.registered.insert("plane".into());
    assert!(entity_refusal(&assets.roster, "plane").is_some());
    assert_eq!(assets.refusal("plane"), None);
    let slot = assets.bake_slot("plane").unwrap();
    assert!(!assets.library().meshes[slot].visual);
    let refusals = presentation.read_warnings().overlay_refusals().unwrap();
    assert!(refusals.contains("model plane ("), "{refusals}");
    assert!(assets.refusal("zeppelin").is_some());
    assert!(presentation.refused().contains("model entity zeppelin"));
    assert!(!presentation.refused().contains("model entity plane"));
    let _ = std::fs::remove_dir_all(&root);
}

fn lazy_bake_assets(name: &str) -> (std::path::PathBuf, UnitModelAssets) {
    use viewer_models::test_support::{MESHES_GFX, triangle_mesh, visual_stack};
    let (root, layers) = visual_stack(
        name,
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            (
                "base/gfx/entities/planes.asset",
                br#"entity = { name = first pdxmesh = body }
                    entity = { name = second pdxmesh = body }
                    entity = { name = missing pdxmesh = hull }"#,
            ),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.0)),
        ],
    );
    let mut assets = UnitModelAssets::off(String::new());
    assets.roster = layers.descriptor_roster().unwrap();
    assets.registered = assets.roster.descriptors.entities.keys().cloned().collect();
    assets.layers = layers;
    (root, assets)
}

#[test]
fn warm_unit_bakes_do_not_lock_the_cold_registry_or_other_names() {
    let (root, assets) = lazy_bake_assets("warm_bakes");
    assets.ensure(&["first".into()]);
    assert!(assets.library().slot("first").is_some());
    {
        let _registry = assets.bakes.lock().unwrap();
        assets.ensure(&["first".into()]);
    }
    let cold = Arc::new(Mutex::new(false));
    assets
        .bakes
        .lock()
        .unwrap()
        .insert("missing".into(), Arc::clone(&cold));
    let _loading = cold.lock().unwrap();
    assets.ensure(&["second".into(), "first".into()]);
    assert!(assets.library().slot("second").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlapping_unit_bake_batches_load_each_name_once() {
    let (root, assets) = lazy_bake_assets("overlapping_bakes");
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for names in [
            ["second", "first", "second"],
            ["first", "second", "first"],
            ["second", "second", "first"],
            ["first", "first", "second"],
        ] {
            let (assets, start) = (&assets, &start);
            scope.spawn(move || {
                start.wait();
                assets.ensure(&names.map(str::to_owned));
            });
        }
    });
    let library = assets.library();
    assert_eq!(library.meshes.len(), 2);
    let first = library.slot("first").unwrap();
    let second = library.slot("second").unwrap();
    assert_ne!(first, second);
    assert_eq!(library.meshes[first].name, "first");
    assert_eq!(library.meshes[second].name, "second");
    assert_ne!(library.meshes[first].asset, library.meshes[second].asset);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsuccessful_unit_bakes_are_not_retried() {
    let (root, mut assets) = lazy_bake_assets("failed_bakes");
    assets.ensure(&["missing".into()]);
    assert!(assets.library().slot("missing").is_none());
    assert!(*assets.bakes.lock().unwrap()["missing"].lock().unwrap());
    let readable = assets.roster.descriptors.entities["first"].clone();
    Arc::make_mut(&mut assets.roster)
        .descriptors
        .entities
        .insert("missing".into(), readable);
    assets.ensure(&["missing".into()]);
    assert!(assets.library().slot("missing").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_bakes_keep_named_attachment_refusals_for_the_layer_report() {
    use viewer_models::test_support::{MESHES_GFX, triangle_mesh, visual_stack};
    let (root, layers) = visual_stack(
        "bake_warnings",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            (
                "base/gfx/entities/armed.asset",
                br#"entity = { name = armed pdxmesh = body attach = { mount = absent_child } }"#,
            ),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.0)),
        ],
    );
    let mut assets = UnitModelAssets::off(String::new());
    assets.roster = layers.descriptor_roster().unwrap();
    assets.registered = assets.roster.descriptors.entities.keys().cloned().collect();
    assets.layers = layers;
    assets.ensure(&["armed".into()]);
    assets.ensure(&["armed".into()]);
    assert!(assets.library().slot("armed").is_some());
    let warnings = assets.bake_warnings();
    assert_eq!(
        warnings
            .iter()
            .filter(|line| line.contains("armed -> absent_child"))
            .count(),
        1,
        "{warnings:?}"
    );
    let report = ModelReport {
        bake_warnings: warnings,
        ..ModelReport::default()
    };
    assert!(
        report
            .warnings()
            .iter()
            .any(|line| line.starts_with("Unit models: Attached entities refused")
                && line.contains("armed -> absent_child"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_demand_schedules_keep_stable_groups_events_and_sound_creation_order() {
    use super::*;
    let descriptors = mesh::parse_descriptors(
        br#"
        entity = { name = first state = { name = idle event = { sound = { soundeffect = hum } } } }
        entity = { name = second state = { name = idle event = { sound = { soundeffect = hum } } } }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let mut reference = None;
    for first in ["first", "second"] {
        let mut assets = UnitModelAssets::off(String::new());
        assets.registered = ["first".into(), "second".into()].into();
        assets.roster = Arc::new(mesh::Roster {
            descriptors: descriptors.clone(),
            ..Default::default()
        });
        assets.ensure(&[first.into()]);
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for names in [
                ["first", "second"],
                ["second", "first"],
                ["first", "first"],
                ["second", "second"],
            ] {
                let (assets, barrier) = (&assets, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    assets.ensure(&names.map(str::to_owned));
                });
            }
        });
        let library = assets.library();
        assert_eq!(
            library.slot(first),
            Some(0),
            "the schedules really differ in layout"
        );
        let mut pass = viewer_models::ModelPass::new(&library, "unit models");
        pass.update_placed(
            ["second", "first"]
                .into_iter()
                .enumerate()
                .map(|(at, name)| {
                    (
                        library.slot(name).unwrap(),
                        ModelInstance {
                            scale: 1.,
                            animation: viewer_models::entity_animation::IDLE,
                            animation_seed: at as u32 + 17,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            256.,
        );
        pass.animate(1., None, &[0.]);
        assert_eq!(pass.events.len(), 2);
        let mut sounds = Vec::new();
        for event in &mut pass.events {
            event.key.pass = 0;
        }
        viewer_models::sound_events::collect(
            &pass.events,
            &mut Default::default(),
            &mut sounds,
            |_| false,
        );
        let observed = (
            pass.groups_for_tests()
                .iter()
                .map(|group| (group.asset.clone(), group.bounds, group.centroid))
                .collect::<Vec<_>>(),
            pass.events
                .iter()
                .map(|event| (event.key.clone(), event.seed))
                .collect::<Vec<_>>(),
            sounds,
        );
        if let Some(expected) = &reference {
            assert_eq!(&observed, expected);
        } else {
            reference = Some(observed);
        }
    }
}

fn prepared(assets: &UnitModelAssets, frame: &DecodedFrame, naval: bool) -> PreparedModels {
    equipment_graphics::test_support::with_world(frame, &[], |limits| {
        let places = ShipPlaces {
            motions: &viewer_unit_reads::task_force_visual::TaskForceMotions::default(),
            movement: None,
            land: &|_| true,
            missions: &Default::default(),
            region: &|_| None,
            scale: &|_, _| 1.0,
        };
        let inputs = PrepareInputs {
            frame,
            limits,
            vision: &Vision::default(),
            etas: &Etas::default(),
            anchor: &|_, _| Some([0.0; 3]),
            world: None,
            terrain: &BTreeMap::new(),
            routes: None,
            ships: &places,
            localisation: &content_reader_localisation::localisation::Localisation::default(),
        };
        assets.prepare_naval_or_not(inputs, naval, &mut |_| {}, &mut BuilderSpans::new(false)).unwrap()
    })
}

#[test]
fn models_made_without_ships_say_so_and_take_the_ships_layers_beside_their_own_land() {
    let (assets, frame) = (camouflaged_assets(), prepare_frame());
    let with = prepared(&assets, &frame, true);
    let without = prepared(&assets, &frame, false);
    assert!(with.naval(), "made with ships, its ships are the hour's own");
    assert!(!without.naval(), "made without ships, it says so");
    assert!(!PreparedModels::drawn_by_layers().naval());
    assert_eq!(without.report.stacks, with.report.stacks, "the land is the same");
    assert_eq!(without.placed.len(), with.placed.len());
    let ship = |identity: u64| viewer_unit_history::unit_history::naval::Candidate {
        province: 6,
        identity,
        ordinal: 0,
        slot: 0,
        priority: 3,
        ours: true,
        class: viewer_runtime::unit_class::CounterClass::TaskForce,
        land_eligible: false,
        sample_count: 0,
        next_arc: 0.0,
        route: Vec::new(),
        speed: 12.0,
        min_arc: 0.0,
        arrival_allowed: true,
        day: 0,
    };
    let layer = NavalSources {
        candidates: vec![ship(2), ship(3)],
        positions: vec![viewer_unit_history::unit_history::naval::FormationPlacement { force: 2, position: counter_move::Position::still([1.0; 3]), rotation: 0.5, scale: 1.0 }],
        ..NavalSources::default()
    };
    let mut report = ModelReport { ship_hulls: 4, ..ModelReport::default() };
    report.error("No ship is drawn: TaskForces is not wholly recorded this hour");
    let joined = without.with_ships(layer, &report);
    assert!(joined.naval(), "joined, it holds the layer's ships");
    assert_eq!(joined.ships().candidates.iter().map(|candidate| candidate.identity).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(joined.placed.len(), without.placed.len(), "its own land stays");
    assert_eq!(joined.report.stacks, without.report.stacks);
    assert_eq!(joined.report.ship_hulls, without.report.ship_hulls + 4);
    assert!(joined.report.warnings().iter().any(|line| line.contains("TaskForces is not wholly recorded")), "the layer's refusal is named: {:?}", joined.report.warnings());
    let ships: Vec<_> = joined.movement_positions().filter(|(unit, ..)| matches!(unit, Unit::TaskForce(_))).map(|(unit, ..)| unit).collect();
    assert_eq!(ships, [Unit::TaskForce(2)], "the ships' animators move the layer's formations");
    assert!(without.ships().candidates.is_empty() && !without.naval(), "the models it was joined from are unchanged");
}

#[test]
fn equipment_of_a_division_not_asked_is_refused_by_name_never_read_as_none() {
    let rows = [replay_data::rows::DivisionEquipmentRow { division: 5, ordinal: 0, ..Default::default() }];
    let asked: BTreeSet<u64> = [5].into();
    let mut equipment = Equipment::join(&rows, &[], true);
    equipment.asked = Some(&asked);
    assert_eq!(equipment.of(5).map(<[_]>::len), Ok(1));
    assert!(equipment.of(6).is_err_and(|why| why.contains("not asked")), "a division not asked is refused");
    equipment.asked = None;
    assert_eq!(equipment.of(6).map(<[_]>::len), Ok(0), "over whole tables a division in neither holds none");
}
