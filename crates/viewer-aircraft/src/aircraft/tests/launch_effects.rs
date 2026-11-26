use super::*;
use crate::aircraft::launches::{AIR_BASE, GUN_EMPLACEMENT, Launch, PASS_SECONDS, ROCKET_SITE};
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

#[test]
fn launch_records_come_on_the_passes_after_their_entity_finishes() {
    assert_eq!(PASS_SECONDS, 2.0);
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
    for record in 0..20 {
        let heading = effect.heading(record);
        assert!((0.0..std::f32::consts::TAU).contains(&heading));
    }
    assert_ne!(effect.heading(0), effect.heading(1));
    assert_eq!(effect.finished(2.4, 2.5), Some(0));
    assert_eq!(effect.finished(2.5, 6.4), None);
    assert_eq!(effect.finished(6.4, 6.5), Some(1));
    assert_eq!(effect.finished(0.0, 30.0), Some(6));
    assert_eq!(effect.finished(3.0, 1.0), None);
    for seed in 0..50 {
        let delay = Launch::delay(seed, 3.0, 2.0);
        assert!((0.0..1.5).contains(&delay));
    }
    assert!(launch(0.5, 0.0).instance(0, 0.5).is_none());
}

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
    let definition = |id| content_reader_map::map::ProvinceDefinition {
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
            b"NDefines = { NAirGfx = { RANDOM_SCENARIO_ANIMATION_MAX_DELAY = 2.5 } }",
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

#[test]
fn a_launch_finishing_at_the_opening_instant_explodes_once() {
    assert_eq!(Launch::delay(9, 0.0, 1.0), 0.0);
    let place = Some(([5.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]));
    let mut effect = launch(0.0, 0.0);
    effect.explosions = Some(Arc::new(vec![(5, place)]));
    let records = |impacts: Vec<viewer_models::strike_effects::Impact>| {
        impacts
            .iter()
            .map(|impact| impact.id as u32)
            .collect::<Vec<_>>()
    };
    let mut plan = AircraftPlan {
        launches: vec![effect],
        ..Default::default()
    };
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
    let mut late = AircraftPlan {
        launches: plan.launches.clone(),
        ..Default::default()
    };
    assert_eq!(
        records(late.missile_explosions(0.5, 1.0)),
        Vec::<u32>::new()
    );
    let mut early = AircraftPlan {
        launches: plan.launches.clone(),
        ..Default::default()
    };
    assert_eq!(records(early.missile_explosions(-0.5, 1.0)), [0]);
}
