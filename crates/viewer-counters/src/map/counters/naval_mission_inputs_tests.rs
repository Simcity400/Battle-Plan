use super::*;
use replay::frame::{Metrics, TableView};
use viewer_foundation::table_access::TokenKeys;

fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|i| TableView::Complete(TableRows::empty(TableId::ALL[i])))
            .map(Into::into),
    }
}
fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}
fn forces(frame: &DecodedFrame) -> HashMap<u64, &TaskForceRow> {
    data::<tables::TaskForces>(frame)
        .unwrap()
        .iter()
        .map(|row| (row.id, row))
        .collect()
}
fn hour<'a>(
    frame: &'a DecodedFrame,
    anchors: &'a CounterContent,
    native: &'a NativeInputs,
    countries: (&'a BTreeMap<u32, u32>, &'a BTreeMap<u32, u32>),
) -> Hour<'a> {
    Hour::read(
        frame,
        anchors,
        native,
        countries,
        forces(frame),
        (
            Err("unused in fixture".into()),
            Err("unused in fixture".into()),
        ),
        1,
    )
    .unwrap()
}
fn content(terrain: &[u8]) -> Content {
    Content {
        terrain: installed_content::terrain::parse_definitions(terrain).unwrap(),
        naval_terrain: BTreeMap::new(),
        efficiencies: derive::Defines {
            convoy_defense_max_convoy_to_ship_ratio: 1_000_000,
            convoy_defense_max_region_to_taskforce_ratio: 150_000,
            mission_efficiency_pow_factor: 100_000,
            ..Default::default()
        },
        fuel_factor: 0,
        underway_factor: 0,
        distance_factor: 100_000,
        edges: OnceLock::new(),
        distances: OnceLock::new(),
    }
}
fn anchors() -> CounterContent {
    CounterContent {
        terrain: Default::default(),
        mission_content: Err("unused in fixture".into()),
        port_mission_defines: viewer_foundation::map::port_mission_defines::Defines::read(
            &installed_content::defines::Defines::new().unwrap(),
        ),
        anchors: Arc::new(BTreeMap::new()),
        stack_scales: BTreeMap::new(),
        default_anchors: Arc::new(BTreeMap::new()),
        site_icons: Default::default(),
        ports: BTreeMap::new(),
        building_icons: BuildingIconContent::default(),
        kinds: Arc::new(BTreeMap::new()),
        bounds: BTreeMap::new(),
        region_of: Arc::new([(1, 1), (2, 2)].into()),
        region_provinces: [(1, vec![1]), (2, vec![2])].into(),
        region_icons: BTreeMap::new(),
        missing_standstill: Vec::new(),
        map: MapSize {
            width: 10,
            height: 10,
        },
        aggression: Arc::new(Vec::new()),
        fog: Arc::new(Err("unused in fixture".into())),
        defines: CounterDefines::VANILLA,
        supply_grace: Err("unused in fixture".into()),
        dig_in_factor: Err("unused in fixture".into()),
        attrition: Err("unused in fixture".into()),
        ability_comms_scaling: Err("unused in fixture".into()),
        ability_no_hq_scaling: Err("unused in fixture".into()),
        comms_max_distance: Err("unused in fixture".into()),
        subunit_tokens: Ok(TokenKeys::new(&["cruiser".to_owned()])),
        ideologies: Default::default(),
    }
}
fn force_frame() -> DecodedFrame {
    let mut frame = frame();
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 11,
            fleet: 9,
            mission: 2,
            roster_owner: 7,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::NavalMissionTaskForces(vec![NavalMissionTaskForceRow {
            task_force: 11,
            mission_owner: 7,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::NavalMissionCountries(vec![NavalMissionCountryRow {
            country: 100,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::NavalMissionCountries as u32,
            owner: 100,
            sample_hour: 0,
        }]),
    );
    frame
}

#[test]
fn unread_commander_is_named_unknown_and_read_zero_is_explicit() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = force_frame();
    let content = content(b"categories={ocean={}}");
    let mut anchors = anchors();
    anchors.mission_content = Ok(content);
    let links = [(7, 1)].into();
    let ids = [(1, 100)].into();
    let countries = (&links, &ids);
    if let TableView::Complete(TableRows::NavalMissionTaskForces(rows)) =
        &mut *frame.tables[TableId::NavalMissionTaskForces.index()]
    {
        rows[0].leader = 41;
        rows[0].headquarters = vec![51];
    }
    let native = NativeInputs::read(&frame).unwrap();
    let combined = |frame: &DecodedFrame| {
        hour(frame, &anchors, &native, countries).combined(11, keys::NAVAL_COORDINATION)
    };
    assert!(combined(&frame).unwrap_err().contains("slice witness"));
    set(
        &mut frame,
        TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::NavalMissionCommanders as u32,
            owner: 100,
            sample_hour: 0,
        }]),
    );
    set(
        &mut frame,
        TableRows::NavalMissionCommanders(vec![
            NavalMissionCommanderRow {
                country: 100,
                reference: 41,
                source: 0,
                ..Default::default()
            },
            NavalMissionCommanderRow {
                country: 100,
                reference: 51,
                source: 1,
                coordination: None,
                ..Default::default()
            },
        ]),
    );
    assert!(
        combined(&frame)
            .unwrap_err()
            .contains("incomplete source quartet")
    );
    set(
        &mut frame,
        TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            index: keys::NAVAL_COORDINATION,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::NavalMissionCommanders(
            (1..=4)
                .map(|source| NavalMissionCommanderRow {
                    country: 100,
                    reference: 51,
                    source,
                    coordination: Some(i64::from(source)),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    // A missing row remains a named unknown even with a complete slice witness.
    assert!(
        combined(&frame)
            .unwrap_err()
            .contains("commander 0x29 source 0 has no NavalMissionCommanders row")
    );
    if let TableView::Complete(TableRows::NavalMissionCommanders(rows)) =
        &mut *frame.tables[TableId::NavalMissionCommanders.index()]
    {
        rows.insert(
            0,
            NavalMissionCommanderRow {
                country: 100,
                reference: 41,
                source: 0,
                ..Default::default()
            },
        );
    }
    // A successfully read, zero-contributing leader has a row; all four HQ sources add.
    assert_eq!(combined(&frame), Ok(10));
}

#[test]
fn ship_indexes_stay_cold_until_a_mission_chain_needs_ships() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = frame();
    set(
        &mut frame,
        TableRows::Ships(
            (1..=2)
                .map(|id| ShipRow {
                    id,
                    task_force: 11,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let content = content(b"categories={ocean={}}");
    let mut anchors = anchors();
    anchors.mission_content = Ok(content);
    let native = NativeInputs::default();
    let links = BTreeMap::new();
    let ids = BTreeMap::new();
    let countries = (&links, &ids);
    let inputs = hour(&frame, &anchors, &native, countries);
    assert!(inputs.ships.get().is_none());
    assert!(inputs.flags.get().is_none());
    assert_eq!(inputs.ships(11).unwrap().len(), 2);
    assert!(inputs.ships.get().is_some());
    assert!(inputs.flags.get().is_none());
}

#[test]
fn p3_terrain_merges_common_before_clamping_and_keeps_the_first_class_bucket() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = force_frame();
    let content=content(b"categories={ocean={naval_coordination=0.5 cruiser={naval_coordination=0.9} cruiser={naval_coordination=-0.9}}}");
    let mut anchors = anchors();
    anchors.mission_content = Ok(content);
    set(
        &mut frame,
        TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 7,
                literal: b"cruiser".to_vec(),
            },
            LexicalTokenRow {
                token: 19,
                literal: b"naval_coordination".to_vec(),
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            index: keys::NAVAL_COORDINATION,
            token: 19,
            flags: engine::modifier_definition::CLAMP_MAXIMUM,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::Ships(vec![
            ShipRow {
                id: 1,
                task_force: 11,
                class_id: 7,
                ..Default::default()
            },
            ShipRow {
                id: 2,
                task_force: 11,
                class_id: 8,
                ..Default::default()
            },
        ]),
    );
    let native = NativeInputs::default();
    let links = [(7, 1)].into();
    let ids = [(1, 100)].into();
    let countries = (&links, &ids);
    let check = |frame: &DecodedFrame| {
        let inputs = hour(frame, &anchors, &native, countries);
        inputs.terrain_value(inputs.force(11).unwrap(), keys::NAVAL_COORDINATION)
    };
    // The cruiser clamps .5+.9 to1; the unregistered class uses common .5.
    assert_eq!(check(&frame), Ok(75_000));
    if let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
        &mut *frame.tables[TableId::ModifierDefinitions.index()]
    {
        rows[0].feature_selector = 2;
        rows[0].eligibility = 1;
    }
    assert!(check(&frame).unwrap_err().contains("DLC feature mask"));
    set(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 4,
            ..Default::default()
        }]),
    );
    assert_eq!(check(&frame), Ok(0));
    set(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 6,
            ..Default::default()
        }]),
    );
    assert_eq!(check(&frame), Ok(75_000));
}

#[test]
fn p2_join_keeps_the_native_range_boundary_and_null_home_base_hub_guard() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = force_frame();
    let content = content(b"categories={ocean={}}");
    let mut anchors = anchors();
    anchors.mission_content = Ok(content);
    if let TableView::Complete(TableRows::TaskForces(rows)) =
        &mut *frame.tables[TableId::TaskForces.index()]
    {
        rows[0].province = 1;
    }
    if let TableView::Complete(TableRows::NavalMissionTaskForces(rows)) =
        &mut *frame.tables[TableId::NavalMissionTaskForces.index()]
    {
        rows[0].range_base = 600_000;
    }
    set(
        &mut frame,
        TableRows::Fleets(vec![FleetRow {
            id: 9,
            home_base: 3,
            ..Default::default()
        }]),
    );
    let graph = installed_content::province_graph::ProvinceGraph {
        edges: [(1, vec![(2, Some(300_000))]), (2, vec![(1, Some(300_000))])].into(),
        ..Default::default()
    };
    let geometry = installed_content::province_geometry::ProvinceGeometry {
        centroids: [(1, [0, 0]), (2, [3, 0]), (3, [0, 0]), (4, [3, 0])].into(),
        land: [3, 4].into(),
        width: 10,
        ..Default::default()
    };
    let ports: BTreeMap<_, _> = [(3, 1), (4, 2)].into();
    let native = NativeInputs::default();
    let links = [(7, 1)].into();
    let ids = [(1, 100)].into();
    let countries = (&links, &ids);
    let check = |frame: &DecodedFrame, bases: &BTreeSet<u32>| {
        let mut hour = hour(frame, &anchors, &native, countries);
        hour.topology = Ok((&graph, &geometry));
        hour.ports = Ok(ports.clone());
        hour.bases = Ok(bases.clone());
        hour.active(11, 2)
    };
    assert_eq!(check(&frame, &[3].into()), Ok(true));
    let cache = anchors
        .mission_content
        .as_ref()
        .unwrap()
        .distances
        .get()
        .unwrap() as *const _;
    if let TableView::Complete(TableRows::NavalMissionTaskForces(rows)) =
        &mut *frame.tables[TableId::NavalMissionTaskForces.index()]
    {
        rows[0].range_base = 599_998;
    }
    assert_eq!(check(&frame, &[3].into()), Ok(false));
    if let TableView::Complete(TableRows::NavalMissionCountries(rows)) =
        &mut *frame.tables[TableId::NavalMissionCountries.index()]
    {
        rows[0].hubs = vec![4];
    }
    assert_eq!(check(&frame, &[4].into()), Ok(true));
    if let TableView::Complete(TableRows::Fleets(rows)) =
        &mut *frame.tables[TableId::Fleets.index()]
    {
        rows[0].home_base = 0;
    }
    assert_eq!(check(&frame, &[4].into()), Ok(false));
    assert_eq!(
        anchors
            .mission_content
            .as_ref()
            .unwrap()
            .distances
            .get()
            .unwrap() as *const _,
        cache
    );
}

#[test]
fn unlinked_mission_owner_is_named_without_losing_the_other_operands() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = force_frame();
    if let TableView::Complete(TableRows::NavalMissionTaskForces(rows)) =
        &mut *frame.tables[TableId::NavalMissionTaskForces.index()]
    {
        rows[0].mission_owner = 999_999;
    }
    let mut anchors = anchors();
    anchors.mission_content = Ok(content(b"categories={ocean={}}"));
    let native = NativeInputs::default();
    let links = [(7, 1)].into();
    let ids = [(1, 100)].into();
    let mut hour = hour(&frame, &anchors, &native, (&links, &ids));
    let owner = hour.input(11).unwrap().mission_owner;
    assert_eq!(owner, 999_999);
    assert_eq!(
        hour.country_value(owner, keys::CONVOY_ESCORT_EFFICIENCY),
        Err("naval mission country 999999 is not linked".into())
    );
    assert_eq!(hour.combined(11, keys::NAVAL_COORDINATION), Ok(0));
}
