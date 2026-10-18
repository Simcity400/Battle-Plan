//! Base-equipment launch effects: the record lifecycle, the site bases and
//! the records over a pass.
use super::*;
use crate::map::aircraft::launches::{
    AIR_BASE, GUN_EMPLACEMENT, Launch, PASS_SECONDS, ROCKET_SITE,
};
use replay_data::{
    rows::LexicalTokenRow, rows::TableId, rows::TableRows, rows::VariantDefinitionRow,
    rows::WingEquipmentRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn launch(spawn: f32, length: f32) -> Launch {
    Launch {
        mesh: 0,
        wing: 1,
        position: [1.0, 2.0, 3.0],
        spawn,
        length,
        seed: 9,
        explosions: None,
    }
}

/// A record comes on a pass, spawns its entity after its delay, shows it
/// until its root reads its clip's length, and the next record comes on the
/// first pass after that. Before, launch effects were only counted.
#[test]
fn launch_records_come_on_the_passes_after_their_entity_finishes() {
    assert_eq!(PASS_SECONDS, 2.0);
    // 0.5 + 2 seconds take two passes; finishing on a pass waits for the
    // next one, as the builder runs before the record is removed.
    assert_eq!(launch(0.5, 2.0).period(), 4.0);
    assert_eq!(launch(0.5, 1.5).period(), 4.0);
    assert_eq!(launch(0.2, 1.0).period(), 2.0);
    let effect = launch(0.5, 2.0);
    assert!(effect.instance(3, 0.4).is_none());
    let shown = effect.instance(3, 0.5).unwrap();
    assert_eq!(
        (
            shown.position,
            shown.scale,
            shown.animation_phase,
            shown.animation_epoch
        ),
        ([1.0, 2.0, 3.0], 1.0, 0.0, 0)
    );
    assert!(effect.instance(3, 2.49).is_some());
    assert!(effect.instance(3, 2.5).is_none());
    let next = effect.instance(3, 4.6).unwrap();
    assert_eq!(next.animation_epoch, 1);
    assert!((next.animation_phase - 0.1).abs() < 1e-5);
    // The heading is a whole turn times a fixed-point draw.
    for record in 0..20 {
        let heading = effect.heading(record);
        assert!((0.0..std::f32::consts::TAU).contains(&heading));
    }
    assert_ne!(effect.heading(0), effect.heading(1));
    // Each record's entity finishes at 2.5 + 4n.
    assert_eq!(effect.finished(2.4, 2.5), Some(0));
    assert_eq!(effect.finished(2.5, 6.4), None);
    assert_eq!(effect.finished(6.4, 6.5), Some(1));
    assert_eq!(effect.finished(0.0, 30.0), Some(6));
    assert_eq!(effect.finished(3.0, 1.0), None);
    // The delay is the define times a unit draw, on the aircraft clock.
    for seed in 0..50 {
        let delay = Launch::delay(seed, 3.0, 2.0);
        assert!((0.0..1.5).contains(&delay));
    }
    // A zero-length entity is never shown.
    assert!(launch(0.5, 0.0).instance(0, 0.5).is_none());
}

/// A launch-flagged record explodes in a province of its mission region's
/// list, duplicates included, when that province has a place.
#[test]
fn a_launch_flagged_record_explodes_in_its_mission_region() {
    let place = |province: u32| Some(([province as f32, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]));
    let mut effect = launch(0.5, 2.0);
    effect.explosions = Some(Arc::new(vec![(3, place(3)), (4, None), (3, place(3))]));
    let mut seen = BTreeSet::new();
    for record in 0..200 {
        match effect.explosion(record) {
            Some((province, position, _)) => {
                assert_eq!((province, position[0]), (3, 3.0));
                seen.insert(province);
            }
            None => {
                seen.insert(4);
            }
        }
    }
    assert_eq!(seen, BTreeSet::from([3, 4]));
    effect.explosions = Some(Arc::new(Vec::new()));
    assert_eq!(effect.explosion(0), None);
}

/// Every state base takes off where the game's base position finder puts
/// it ([`viewer_foundation::map::building_facts::SiteSlots::base`]); a kind
/// other than a rocket site or gun emplacement is an ordinary air base.
/// Before, every state base took off from the slot literally named
/// `air_base`.
#[test]
fn state_bases_take_off_where_the_game_s_finder_puts_them() {
    let content = launches::Content::for_tests(
        BTreeMap::from([
            ((1, AIR_BASE), Ok([9.0, 9.0, 9.0])),
            ((1, ROCKET_SITE), Ok([4.0, 10.0, 5.0])),
            ((1, GUN_EMPLACEMENT), Ok([4.0, 10.0, 5.0])),
        ]),
        3.0,
    );
    let registry = AirRegistrySources::default();
    let carriers = CarrierSources {
        ships: None,
        task_forces: None,
        registry: &registry,
        origin: &|_| Err("unused"),
    };
    let base = |state, kind| AirBaseRow {
        state: Some(state),
        kind,
        ..Default::default()
    };
    let at = |base: AirBaseRow| take_off(&content, &carriers, &base);
    assert_eq!(at(base(1, AIR_BASE)), Ok([9.0, 9.0, 9.0]));
    assert_eq!(at(base(1, 7)), Ok([9.0, 9.0, 9.0]));
    assert_eq!(at(base(1, ROCKET_SITE)), Ok([4.0, 10.0, 5.0]));
    assert_eq!(at(base(1, GUN_EMPLACEMENT)), Ok([4.0, 10.0, 5.0]));
    assert_eq!(
        at(base(2, ROCKET_SITE)),
        Err("its state has no admitted rocket-site slot")
    );
    assert_eq!(
        at(base(2, GUN_EMPLACEMENT)),
        Err("its state has no admitted gun-emplacement slot")
    );
    assert_eq!(
        at(base(2, AIR_BASE)),
        Err("its state has no admitted air-base slot of an air-base type")
    );
}

/// The smallest drawable mesh the library loads: one triangle with a
/// material.
fn mesh_bytes() -> Vec<u8> {
    let mut mesh = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 1., 0.]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        mesh.extend([b'!', name.len() as u8]);
        mesh.extend(name.as_bytes());
        mesh.push(b'f');
        mesh.extend((values.len() as u32).to_le_bytes());
        mesh.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    mesh.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    mesh.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    mesh
}

/// Assets whose content declares two launch entities, with a strategic
/// region 5 listing provinces 3, 4 and 3, and their content folder, which
/// lazy bakes read until the caller removes it.
fn launch_assets() -> (AircraftAssets, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "viewer-aircraft-launch-effects-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, bytes) in [
        (
            "gfx/models/launch.gfx",
            br#"objectTypes = { pdxmesh = { name = "launch_mesh" file = "gfx/models/launch.mesh" } }"#
                .to_vec(),
        ),
        (
            "gfx/entities/launch.asset",
            br#"
            entity = { name = "sam_missile_equipment_1_entity" pdxmesh = "launch_mesh" }
            entity = { name = "explosive_ammo_equipment_1_entity" pdxmesh = "launch_mesh" }
            "#
            .to_vec(),
        ),
        ("gfx/models/launch.mesh", mesh_bytes()),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let definition = |id| installed_content_core::map::ProvinceDefinition {
        id,
        rgb: [0, 0, 0],
        kind: ProvinceKind::Land,
        continent: 0,
    };
    let map = MapData {
        width: 1,
        height: 1,
        province_ids: vec![3],
        definitions: [3, 4].map(|id| (id, definition(id))).into(),
        country_colors: Default::default(),
    };
    let defines = Defines::new().unwrap();
    defines
        .load(
            "fixture",
            b"NDefines = { NAirGfx = { RANDOM_SCENARIO_ANIMATION_MAX_DELAY = 3.0 } }",
        )
        .unwrap();
    let regions = BTreeMap::from([(
        5,
        StrategicRegion {
            provinces: vec![3, 4, 3],
            ..Default::default()
        },
    )]);
    let mut warnings = Vec::new();
    let mut assets = AircraftAssets::load(
        &layers,
        Decode::default(),
        &map,
        (&defines, &defines),
        &regions,
        &mut warnings,
    )
    .unwrap();
    assets.launch_content = launches::Content::for_tests(
        BTreeMap::from([
            ((1, ROCKET_SITE), Ok([10.0, 1.0, 20.0])),
            ((2, GUN_EMPLACEMENT), Ok([30.0, 1.0, 40.0])),
        ]),
        3.0,
    );
    (assets, root)
}

/// A frame whose wings 1..=5 fly variants 11..=15, whose equipment tokens
/// spell the SAM, the gun ammunition (wings 3 and 5) and an equipment no
/// entity is declared for (wing 4).
fn launch_frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 42,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::WingEquipment.index()] = TableView::Complete(TableRows::WingEquipment(
        (1..=5)
            .map(|wing| WingEquipmentRow {
                wing,
                ordinal: 0,
                variant: 10 + wing,
                amount_raw: 100_000,
            })
            .collect(),
    ));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            [(11, 101), (12, 101), (13, 102), (14, 103), (15, 102)]
                .map(|(id, equipment_token)| VariantDefinitionRow {
                    id,
                    equipment_token,
                    ..Default::default()
                })
                .into(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (101, &b"sam_missile_equipment_1"[..]),
            (102, b"explosive_ammo_equipment_1"),
            (103, b"undeclared_equipment"),
        ]
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        })
        .into(),
    ));
    frame
}

/// Over one pass a base's province holds one record: a wing whose entity no
/// content declares starts none and leaves the base free; a launch-flagged
/// record explodes in its mission region; a bit-17 record does not.
#[test]
fn a_pass_starts_one_record_per_base_and_names_what_it_cannot_start() {
    let (assets, root) = launch_assets();
    let frame = launch_frame();
    let registry = AirRegistrySources::default();
    let carriers = CarrierSources {
        ships: None,
        task_forces: None,
        registry: &registry,
        origin: &|_| Err("unused"),
    };
    let wings: Vec<AirWingRow> = (1..=5)
        .map(|id| AirWingRow {
            id,
            mission_region: Some(5),
            ..Default::default()
        })
        .collect();
    let base = |province, state, kind| AirBaseRow {
        province: Some(province),
        state: Some(state),
        kind,
        ..Default::default()
    };
    let bases = [
        base(40, 1, ROCKET_SITE),
        base(40, 1, ROCKET_SITE),
        base(41, 2, GUN_EMPLACEMENT),
        base(42, 2, GUN_EMPLACEMENT),
        base(42, 2, GUN_EMPLACEMENT),
    ];
    let inputs = AirMissionInputRow::default();
    let request = |index: usize, kind| scenarios::LaunchRequest {
        wing: ScenarioWing {
            wing: &wings[index],
            inputs: &inputs,
            base: &bases[index],
            mission_country: MissionCountry { slot: 1 },
            country: 1,
            side: 0,
        },
        kind,
    };
    let sources = LaunchSources {
        frame: &frame,
        assets: &assets,
        carriers: &carriers,
        impact_place: &|province| {
            (province == 3).then_some(([3.0, 0.0, 3.0], [0.0, 0.0, 1.0, 1.0]))
        },
        clock_speed: 2.0,
    };
    let mut records = LaunchRecords::default();
    let started: Vec<_> = [
        request(0, scenarios::LaunchKind::Mission),
        request(1, scenarios::LaunchKind::Definition),
        request(2, scenarios::LaunchKind::Definition),
        request(3, scenarios::LaunchKind::Definition),
        request(4, scenarios::LaunchKind::Definition),
    ]
    .into_iter()
    .filter_map(|request| records.start(request, &sources))
    .collect();
    let diagnostics: Vec<String> = records.diagnostics().collect();
    let library = assets.library();
    assert_eq!(
        started
            .iter()
            .map(|launch| (
                launch.wing,
                library.meshes[launch.mesh].name.as_str(),
                launch.position,
                launch.explosions.as_ref().map(|places| places.len()),
            ))
            .collect::<Vec<_>>(),
        [
            (1, "sam_missile_equipment_1_entity", [10.0, 1.0, 20.0], None),
            (
                3,
                "explosive_ammo_equipment_1_entity",
                [30.0, 1.0, 40.0],
                Some(3)
            ),
            (
                5,
                "explosive_ammo_equipment_1_entity",
                [30.0, 1.0, 40.0],
                Some(3)
            ),
        ],
        "{diagnostics:?}"
    );
    // The entity plays nothing here, so it finishes as it spawns; the delay
    // is a draw under the define over the clock speed.
    for launch in &started {
        assert_eq!(launch.length, 0.0);
        assert!((0.0..1.5).contains(&launch.spawn));
    }
    assert_eq!(
        diagnostics,
        [
            "1 base-equipment launch effects start no record: the content declares no undeclared_equipment_entity, so the game spawns nothing",
            "region 5 lists 1 provinces with no standstill anchor or painted box; a launch effect's explosion drawn there does not play",
        ]
    );
    drop(library);
    drop(assets);
    let _ = std::fs::remove_dir_all(&root);
}

/// A launch with no delay (a zero maximum) whose entity has no root clip
/// spawns and finishes at the clock's opening instant. The first drain
/// reaching that instant takes its explosion once, a swapped-in plan does
/// not take it again, and the later records come on the passes. Before, no
/// `(from, to]` covered the opening instant and the first explosion was lost.
#[test]
fn a_launch_finishing_at_the_opening_instant_explodes_once() {
    assert_eq!(Launch::delay(9, 0.0, 1.0), 0.0);
    let place = Some(([5.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]));
    let mut effect = launch(0.0, 0.0);
    effect.explosions = Some(Arc::new(vec![(5, place)]));
    let records = |impacts: Vec<crate::map::strike_effects::Impact>| {
        impacts
            .iter()
            .map(|impact| impact.id as u32)
            .collect::<Vec<_>>()
    };
    let mut plan = AircraftPlan {
        launches: vec![effect],
        ..Default::default()
    };
    // The scene's first frame drains an empty interval at zero.
    assert_eq!(records(plan.missile_explosions(0.0, 0.0)), [0]);
    assert_eq!(
        records(plan.missile_explosions(0.0, 0.016)),
        Vec::<u32>::new()
    );
    let mut swapped = AircraftPlan {
        launches: plan.launches.clone(),
        ..Default::default()
    };
    swapped.carry_opening(&plan);
    assert_eq!(
        records(swapped.missile_explosions(0.016, 0.5)),
        Vec::<u32>::new()
    );
    assert_eq!(records(swapped.missile_explosions(0.5, 2.0)), [1]);
    // A plan whose first drain starts after the opening instant never takes it.
    let mut late = AircraftPlan {
        launches: plan.launches.clone(),
        ..Default::default()
    };
    assert_eq!(
        records(late.missile_explosions(0.5, 1.0)),
        Vec::<u32>::new()
    );
    // A drain from before the instant takes record 0 once, not twice.
    let mut early = AircraftPlan {
        launches: plan.launches.clone(),
        ..Default::default()
    };
    assert_eq!(records(early.missile_explosions(-0.5, 1.0)), [0]);
}
