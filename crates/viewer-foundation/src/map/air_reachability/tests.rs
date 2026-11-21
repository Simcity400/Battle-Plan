use super::*;
use replay_data::rows::{
    AIR_MISSION_MAP_FLAGS, AIR_MISSION_ZERO_PLANE_FLAGS, CountryRow, FactionMemberRow,
    NavalCombatShipRow, NavalRegionalMineRow, RelationLinkRow, TableId, TableRows,
    VolunteerHostRow, WarRelationRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

const LAND: RegionFractions = RegionFractions {
    land_raw: 100_000,
    sea_raw: 0,
};
const SEA: RegionFractions = RegionFractions {
    land_raw: 0,
    sea_raw: 100_000,
};
const MIXED: RegionFractions = RegionFractions {
    land_raw: 50_000,
    sea_raw: 50_000,
};

fn empty_frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 100,
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
        metrics: Metrics::default(),
    };
    set::<tables::Countries>(
        &mut frame,
        (1..=20)
            .map(|slot| CountryRow {
                id: slot,
                canonical_slot: slot,
                ..Default::default()
            })
            .collect(),
    );
    frame
}

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
}

fn truncate<T: TableSchema>(frame: &mut DecodedFrame) {
    *frame.tables[T::ID.index()] = TableView::Truncated {
        rows: TableRows::empty(T::ID),
        missing: 1,
    };
}

fn table<T: TableSchema>(frame: &mut DecodedFrame) -> &mut Vec<T::Row> {
    match &mut *frame.tables[T::ID.index()] {
        TableView::Complete(rows) => rows.rows_mut::<T>().unwrap(),
        _ => unreachable!(),
    }
}

fn war(first_country: u32, second_country: u32) -> WarRelationRow {
    WarRelationRow {
        first_country,
        second_country,
        ..Default::default()
    }
}

fn province(id: u32, state: u32, controller: u32) -> ProvinceRow {
    ProvinceRow {
        id,
        state,
        controller,
        ..Default::default()
    }
}

fn admission(
    mask: u32,
    reached_count: i32,
    fractions: Option<RegionFractions>,
    dlc_mask: Option<u32>,
) -> TargetAdmission {
    target_admission(
        mask,
        reached_count,
        fractions,
        dlc_mask,
        TargetPredicates::default(),
    )
}

struct PortFixture {
    wing: AirWingRow,
    base: AirBaseRow,
    air_bases: Vec<AirBaseRow>,
    host_owners: BTreeMap<u64, u32>,
    provinces: Vec<ProvinceRow>,
    naval_bases: Vec<NavalBaseRow>,
    province_units: Vec<ProvinceUnitOrderRow>,
    task_forces: Vec<TaskForceRow>,
    wars: Vec<WarRelationRow>,
    volunteer_hosts: Vec<VolunteerHostRow>,
    links: BTreeMap<u32, u32>,
}

impl PortFixture {
    fn new() -> Self {
        let base = AirBaseRow {
            id: 11,
            state: Some(7),
            province: Some(100),
            ..Default::default()
        };
        Self {
            wing: AirWingRow {
                id: 1,
                owner: 1,
                air_base: base.id,
                mission_mask: 1 << 8,
                ..Default::default()
            },
            air_bases: vec![base.clone()],
            base,
            host_owners: BTreeMap::new(),
            provinces: vec![province(100, 7, 2), province(412, 8, 13)],
            naval_bases: vec![NavalBaseRow {
                province: 412,
                country: 1,
                level: 0,
                ..Default::default()
            }],
            province_units: vec![
                ProvinceUnitOrderRow {
                    province: 100,
                    ..Default::default()
                },
                ProvinceUnitOrderRow {
                    province: 412,
                    naval: vec![42],
                    ..Default::default()
                },
            ],
            task_forces: vec![TaskForceRow {
                id: 42,
                province: 412,
                roster_owner: 1,
                logical_owner: 3,
                ..Default::default()
            }],
            wars: vec![war(1, 3)],
            volunteer_hosts: Vec::new(),
            links: BTreeMap::from([(1, 1), (2, 2), (3, 3), (12, 2), (13, 3)]),
        }
    }

    fn volunteer(&mut self, hosts: &[u32]) {
        self.volunteer_hosts = hosts
            .iter()
            .zip(0..)
            .map(|(&host, position)| VolunteerHostRow {
                country: 1,
                position,
                host,
                ..Default::default()
            })
            .collect();
    }

    fn frame(&self) -> DecodedFrame {
        let mut frame = empty_frame();
        set::<tables::Provinces>(&mut frame, self.provinces.clone());
        set::<tables::NavalBases>(&mut frame, self.naval_bases.clone());
        set::<tables::ProvinceUnitOrder>(&mut frame, self.province_units.clone());
        set::<tables::TaskForces>(&mut frame, self.task_forces.clone());
        set::<tables::WarRelations>(&mut frame, self.wars.clone());
        set::<tables::VolunteerHosts>(&mut frame, self.volunteer_hosts.clone());
        frame
    }

    fn registry(&self) -> AirRegistrySources {
        AirRegistrySources {
            bases: self
                .air_bases
                .iter()
                .map(|base| (base.id, base.clone()))
                .collect(),
            host_owners: self.host_owners.clone(),
            ..Default::default()
        }
    }

    fn resolve_in(&self, frame: &DecodedFrame) -> Option<u32> {
        let registry = self.registry();
        let mission = MissionCountrySources::read(frame, &self.links, &registry).unwrap();
        mission.resolve(&self.wing).map(|country| country.slot)
    }

    fn resolve(&self) -> Option<u32> {
        self.resolve_in(&self.frame())
    }

    fn admit_in(&self, frame: &DecodedFrame, reached: &[u32]) -> Option<bool> {
        let registry = self.registry();
        let mission = MissionCountrySources::read(frame, &self.links, &registry).unwrap();
        let wars = Wars::read(frame, &self.links).unwrap();
        let sources = PortTargetSources::read(frame, &wars).unwrap();
        sources.admit(mission.resolve(&self.wing)?, reached)
    }

    fn admit(&self, reached: &[u32]) -> Option<bool> {
        self.admit_in(&self.frame(), reached)
    }
}

#[test]
fn port_strike_uses_base_hash_controller_and_naval_logical_owner() {
    let mut fixture = PortFixture::new();
    assert_eq!(fixture.admit(&[412]), Some(true));
    fixture.naval_bases.clear();
    assert_eq!(fixture.admit(&[412]), Some(false));
    fixture.naval_bases.push(NavalBaseRow {
        province: 412,
        ..Default::default()
    });
    fixture.provinces[1].controller = 1;
    assert_eq!(fixture.admit(&[412]), Some(false));
    fixture.provinces[1].controller = 3;
    fixture.task_forces[0].logical_owner = 2;
    assert_eq!(fixture.admit(&[412]), Some(false));
    fixture.task_forces[0].logical_owner = 3;
    fixture.province_units[1].naval.clear();
    assert_eq!(fixture.admit(&[412]), Some(false));
}

#[test]
fn tactical_target_needs_a_hostile_primary_country_and_the_attached_group() {
    let fixture = PortFixture::new();
    let mut frame = fixture.frame();
    let army = |id: u64| (id << 32) | 53;
    set::<tables::LandCombats>(
        &mut frame,
        vec![LandCombatRow {
            id: 5,
            province: 412,
            attacker_classes: vec![0],
            attackers: vec![army(1)],
            defenders: vec![army(2)],
            ..Default::default()
        }],
    );
    set::<tables::LandCombatSides>(
        &mut frame,
        vec![LandCombatSideRow {
            combat: 5,
            attacker_primary: vec![2],
            defender_primary: vec![13],
            ..Default::default()
        }],
    );
    set::<tables::Divisions>(
        &mut frame,
        vec![
            DivisionRow {
                id: army(1),
                orders_group: army(9),
                ..Default::default()
            },
            DivisionRow {
                id: army(2),
                orders_group: army(8),
                ..Default::default()
            },
        ],
    );
    let registry = fixture.registry();
    let admit = |frame: &DecodedFrame, wing: &AirWingRow, reached: &[u32]| {
        let mission = MissionCountrySources::read(frame, &fixture.links, &registry).unwrap();
        let country = mission.resolve(wing).unwrap();
        let wars = Wars::read(frame, &fixture.links).unwrap();
        TacticalTargetSources::read(frame, &wars)
            .unwrap()
            .admit(wing, country, reached)
    };
    let mut wing = AirWingRow {
        attached_orders_group: Some(0),
        ..fixture.wing.clone()
    };
    assert_eq!(admit(&frame, &wing, &[412]), Some(true));
    assert_eq!(admit(&frame, &wing, &[100]), Some(false));
    wing.attached_orders_group = Some(army(7));
    assert_eq!(admit(&frame, &wing, &[412]), Some(false));
    wing.attached_orders_group = Some(army(8));
    assert_eq!(admit(&frame, &wing, &[412]), Some(true));
    wing.attached_orders_group = None;
    assert_eq!(admit(&frame, &wing, &[412]), None);
    assert_eq!(admit(&frame, &wing, &[100]), Some(false));
    wing.attached_orders_group = Some(army(8));
    table::<tables::LandCombatSides>(&mut frame)[0] = LandCombatSideRow {
        combat: 5,
        attacker_primary: vec![2],
        attacker_secondary: vec![3],
        ..Default::default()
    };
    assert_eq!(admit(&frame, &wing, &[412]), Some(false));
    wing.attached_orders_group = None;
    assert_eq!(admit(&frame, &wing, &[412]), Some(false));
    wing.attached_orders_group = Some(army(8));
    table::<tables::LandCombatSides>(&mut frame).clear();
    assert_eq!(admit(&frame, &wing, &[412]), None);
    truncate::<tables::LandCombats>(&mut frame);
    assert_eq!(admit(&frame, &wing, &[412]), None);
}

#[test]
fn port_strike_mission_country_follows_volunteer_base_controller() {
    let mut fixture = PortFixture::new();
    fixture.wars = vec![war(2, 3)];
    assert_eq!(fixture.admit(&[412]), Some(false));
    fixture.volunteer(&[12]);
    assert_eq!(fixture.admit(&[412]), Some(true));
    fixture.base.province = None;
    fixture.air_bases[0].province = None;
    assert_eq!(fixture.admit(&[412]), None);
    fixture.volunteer(&[]);
    fixture.wars = vec![war(1, 3)];
    assert_eq!(fixture.admit(&[412]), Some(true));
}

#[test]
fn base_controller_needs_the_province_row_in_the_base_state() {
    let mut fixture = PortFixture::new();
    fixture.volunteer(&[2]);
    assert_eq!(fixture.resolve(), Some(2));
    let mut frame = fixture.frame();
    truncate::<tables::Provinces>(&mut frame);
    assert_eq!(fixture.resolve_in(&frame), None);
    let mut frame = fixture.frame();
    truncate::<tables::VolunteerHosts>(&mut frame);
    assert_eq!(fixture.resolve_in(&frame), None);
    fixture.provinces[0].state = 8;
    assert_eq!(fixture.resolve(), None);
    fixture.provinces.remove(0);
    assert_eq!(fixture.resolve(), None);
    fixture.volunteer(&[]);
    assert_eq!(fixture.resolve(), Some(1));
}

#[test]
fn mission_country_rejects_nonpositive_raw_volunteer_controller() {
    let mut fixture = PortFixture::new();
    fixture.provinces[0].controller = 0x8000_0002;
    fixture.links.insert(0x8000_0002, 2);
    fixture.volunteer(&[2]);
    fixture.wars = vec![war(2, 3)];
    assert_eq!(fixture.admit(&[412]), Some(false));
    fixture.wars = vec![war(1, 3)];
    assert_eq!(fixture.admit(&[412]), Some(true));
}

#[test]
fn viewer_relation_gates_on_the_mission_countrys_own_reference() {
    let fixture = PortFixture::new();
    let mut frame = fixture.frame();
    assert_eq!(fixture.admit_in(&frame, &[412]), Some(true));
    table::<tables::Countries>(&mut frame)[0].id = 0x8000_0001;
    assert_eq!(fixture.admit_in(&frame, &[412]), Some(false));
    truncate::<tables::Countries>(&mut frame);
    assert_eq!(fixture.admit_in(&frame, &[412]), None);
}

#[test]
fn carrier_mission_country_uses_host_task_force_roster_owner() {
    let mut fixture = PortFixture::new();
    fixture.base = AirBaseRow {
        id: 11,
        host: 88,
        ..Default::default()
    };
    fixture.air_bases[0] = fixture.base.clone();
    fixture.host_owners.insert(11, 2);
    fixture.volunteer(&[2]);
    fixture.wars = vec![war(2, 3)];
    assert_eq!(fixture.admit(&[412]), Some(true));
    fixture.host_owners.insert(11, 1);
    assert_eq!(fixture.admit(&[412]), Some(false));
    fixture.host_owners.clear();
    fixture.volunteer(&[0, 2]);
    assert_eq!(fixture.resolve(), Some(1));
    assert_eq!(fixture.admit(&[412]), Some(false));
}

#[test]
fn a_null_host_task_force_refuses_only_a_wing_that_needs_its_controller() {
    let mut fixture = PortFixture::new();
    fixture.base = AirBaseRow {
        id: 11,
        host: 88,
        ..Default::default()
    };
    fixture.air_bases[0] = fixture.base.clone();
    let registry = |fixture: &PortFixture| AirRegistrySources {
        hosts_without_task_force: BTreeSet::from([11]),
        ..fixture.registry()
    };
    let resolve = |fixture: &PortFixture| {
        let frame = fixture.frame();
        let registry = registry(fixture);
        let mission = MissionCountrySources::read(&frame, &fixture.links, &registry).unwrap();
        mission.resolve(&fixture.wing).map(|country| country.slot)
    };
    assert_eq!(resolve(&fixture), Some(1));
    fixture.volunteer(&[2]);
    assert_eq!(resolve(&fixture), None);
}

#[test]
fn mission_country_tries_transfer_base_after_ineligible_pool_base() {
    let mut fixture = PortFixture::new();
    fixture.wing.base_override = 22;
    fixture.provinces.push(province(101, 7, 4));
    fixture.provinces.sort_by_key(|row| row.id);
    fixture.air_bases.push(AirBaseRow {
        id: 22,
        state: Some(7),
        province: Some(101),
        ..Default::default()
    });
    fixture.links.insert(4, 4);
    fixture.volunteer(&[4]);
    assert_eq!(fixture.resolve(), Some(4));
    fixture.provinces[0].controller = 0x8000_0002;
    fixture.links.insert(0x8000_0002, 2);
    fixture.volunteer(&[2, 4]);
    assert_eq!(fixture.resolve(), Some(1));
    fixture.provinces[0].controller = 0;
    fixture.volunteer(&[0, 4]);
    assert_eq!(fixture.resolve(), Some(1));
}

#[test]
fn mission_country_reader_does_not_require_port_or_war_tables() {
    let fixture = PortFixture::new();
    let mut frame = DecodedFrame {
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "synthetic gap".into(),
            }
            .into()
        }),
        ..empty_frame()
    };
    set::<tables::Provinces>(&mut frame, fixture.provinces.clone());
    set::<tables::VolunteerHosts>(&mut frame, Vec::new());
    for id in [
        TableId::NavalBases,
        TableId::TaskForces,
        TableId::WarRelations,
        TableId::RelationTuples,
    ] {
        *frame.tables[id.index()] = TableView::Complete(TableRows::empty(TableId::AirBases));
    }
    assert_eq!(fixture.resolve_in(&frame), Some(1));
}

#[test]
fn incomplete_port_sources_or_unjoined_occupants_stay_unknown() {
    let mut fixture = PortFixture::new();
    let mut frame = fixture.frame();
    truncate::<tables::NavalBases>(&mut frame);
    assert_eq!(fixture.admit_in(&frame, &[412]), None);
    let mut frame = fixture.frame();
    truncate::<tables::WarRelations>(&mut frame);
    assert_eq!(fixture.admit_in(&frame, &[412]), None);
    fixture.province_units[1].naval = vec![41, 42];
    assert_eq!(fixture.admit(&[412]), Some(true));
    fixture.province_units[1].naval = vec![41];
    assert_eq!(fixture.admit(&[412]), None);
}

#[test]
fn port_wars_join_aliases_and_a_complete_war_census_proves_peace() {
    let mut fixture = PortFixture::new();
    fixture.wars = vec![war(2, 13)];
    fixture.volunteer(&[12]);
    let mut frame = fixture.frame();
    assert_eq!(fixture.admit_in(&frame, &[412]), Some(true));
    *frame.tables[TableId::RelationTuples.index()] = TableView::Unavailable {
        reason: "tuple census absent".into(),
    };
    assert_eq!(fixture.admit_in(&frame, &[412]), Some(true));
    truncate::<tables::ProvinceUnitOrder>(&mut frame);
    assert_eq!(fixture.admit_in(&frame, &[412]), None);
    set::<tables::WarRelations>(&mut frame, Vec::new());
    assert_eq!(fixture.admit_in(&frame, &[412]), Some(false));
    truncate::<tables::WarRelations>(&mut frame);
    assert_eq!(fixture.admit_in(&frame, &[412]), None);
}

#[test]
fn port_scan_without_a_reached_naval_base_needs_no_war_census() {
    let fixture = PortFixture::new();
    let mut frame = fixture.frame();
    truncate::<tables::WarRelations>(&mut frame);
    truncate::<tables::ProvinceUnitOrder>(&mut frame);
    assert_eq!(fixture.admit_in(&frame, &[100]), Some(false));
    assert_eq!(fixture.admit_in(&frame, &[100, 412]), None);
}

#[test]
fn port_result_respects_static_gate_and_other_f66100_branches() {
    let port = |port| TargetPredicates {
        port,
        ..Default::default()
    };
    assert_eq!(
        target_admission(
            (1 << 1) | (1 << 8),
            1,
            Some(LAND),
            Some(0),
            port(Some(true))
        ),
        TargetAdmission::Allowed
    );
    assert_eq!(
        target_admission(1 << 8, 1, Some(SEA), Some(0), port(Some(true))),
        TargetAdmission::Blocked
    );
    assert_eq!(
        target_admission(1 << 8, 1, Some(LAND), Some(0), port(Some(false))),
        TargetAdmission::Blocked
    );
    assert!(matches!(
        target_admission(1 << 8, 1, None, Some(0), port(Some(true))),
        TargetAdmission::Unknown(_)
    ));
    assert_eq!(
        admission((1 << 8) | (1 << 10), 1, Some(LAND), Some(0)),
        TargetAdmission::Allowed
    );
}

#[test]
fn tactical_static_gate_keeps_later_native_or_branches() {
    let tactical = TargetPredicates {
        tactical: Some(true),
        ..Default::default()
    };
    assert_eq!(
        target_admission(1 << 1, 1, Some(LAND), Some(0), tactical),
        TargetAdmission::Allowed
    );
    assert_eq!(
        target_admission(1 << 1, 1, Some(SEA), Some(0), tactical),
        TargetAdmission::Blocked
    );
    let TargetAdmission::Unknown(causes) = target_admission(1 << 1, 1, None, Some(0), tactical)
    else {
        panic!("undecided tactical branch");
    };
    assert!(causes.iter().eq([Cause::Tactical]));
    assert_eq!(
        admission((1 << 1) | (1 << 9), 1, None, Some(0)),
        TargetAdmission::Allowed
    );
}

fn scout_frame(controller: u32) -> DecodedFrame {
    let mut frame = empty_frame();
    set::<tables::Provinces>(&mut frame, vec![province(100, 4, 0), province(101, 4, 0)]);
    set::<tables::States>(
        &mut frame,
        vec![StateRow {
            id: 4,
            controller,
            ..Default::default()
        }],
    );
    frame
}

#[test]
fn land_scout_uses_state_controller_and_directional_attache() {
    let wing = AirWingRow {
        owner: 1,
        ..Default::default()
    };
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (12, 2)]);
    let regions = BTreeMap::from([(7, vec![100, 101])]);
    let admit = |frame: &DecodedFrame| {
        let wars = Wars::read(frame, &links).unwrap();
        let families = fog_families(frame, &links);
        let states = RegionStates::read(frame, &regions).unwrap();
        let sources =
            ScoutTargetSources::read(frame, &links, &wars, families.as_ref(), &states).unwrap();
        sources.admit(&wing, 7, 0)
    };
    let mut frame = scout_frame(12);
    set::<tables::WarRelations>(&mut frame, vec![war(1, 2)]);
    assert_eq!(admit(&frame), Some(true));
    assert_eq!(
        target_admission(
            1 << 14,
            1,
            Some(LAND),
            Some(0),
            TargetPredicates {
                scout: admit(&frame),
                ..Default::default()
            },
        ),
        TargetAdmission::Allowed
    );
    let attache = |country, target_index| RelationLinkRow {
        country,
        target_index,
        links: RelationLinkRow::ATTACHE,
        transport_subject: Default::default(),
    };
    set::<tables::RelationLinks>(&mut frame, vec![attache(1, 2)]);
    assert_eq!(admit(&frame), Some(false));
    set::<tables::RelationLinks>(&mut frame, vec![attache(2, 1)]);
    assert_eq!(admit(&frame), Some(true));
}

#[test]
fn land_scout_air_role_accepts_foreign_neutral_but_not_same_state() {
    let wing = AirWingRow {
        owner: 1,
        ..Default::default()
    };
    let links = BTreeMap::from([(1, 1), (2, 2)]);
    let regions = BTreeMap::from([(7, vec![100])]);
    let admit = |frame: &DecodedFrame, flags| {
        let wars = Wars::read(frame, &links).unwrap();
        let families = fog_families(frame, &links);
        let states = RegionStates::read(frame, &regions).unwrap();
        let sources =
            ScoutTargetSources::read(frame, &links, &wars, families.as_ref(), &states).unwrap();
        sources.admit(&wing, 7, flags)
    };
    let frame = scout_frame(2);
    assert_eq!(admit(&frame, 0), Some(false));
    assert_eq!(admit(&frame, 1 << 18), Some(true));
    let frame = scout_frame(1);
    assert_eq!(admit(&frame, 1 << 18), Some(false));
    let mut frame = scout_frame(2);
    truncate::<tables::RelationTuples>(&mut frame);
    assert_eq!(admit(&frame, 0), Some(false));
    truncate::<tables::WarRelations>(&mut frame);
    set::<tables::RelationLinks>(&mut frame, Vec::new());
    assert_eq!(admit(&frame, 0), None);
}

#[test]
fn land_scout_missing_relations_and_native_zero_coverage_remain_distinct() {
    let wing = AirWingRow {
        owner: 1,
        ..Default::default()
    };
    let links = BTreeMap::from([(1, 1), (2, 2)]);
    let mut frame = scout_frame(2);
    truncate::<tables::FactionMembers>(&mut frame);
    let wars = Wars::read(&frame, &links).unwrap();
    let families = fog_families(&frame, &links);
    let regions = RegionStates::read(&frame, &BTreeMap::from([(7, vec![100])])).unwrap();
    let sources =
        ScoutTargetSources::read(&frame, &links, &wars, families.as_ref(), &regions).unwrap();
    assert_eq!(sources.admit(&wing, 7, 1 << 18), None);
    let empty = RegionStates::read(&frame, &BTreeMap::from([(7, Vec::new())])).unwrap();
    let sources =
        ScoutTargetSources::read(&frame, &links, &wars, families.as_ref(), &empty).unwrap();
    assert_eq!(sources.admit(&wing, 7, 1 << 18), Some(false));
}

fn fog_families(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Option<Families> {
    Families::read(frame, links)
        .and_then(|families| families.with_attaches(frame, links))
        .ok()
}

fn test_wing() -> AirWingRow {
    AirWingRow {
        id: 1,
        owner: 1,
        air_base: 11,
        ..Default::default()
    }
}

fn test_base() -> AirBaseRow {
    AirBaseRow {
        id: 11,
        state: Some(4),
        province: Some(100),
        ..Default::default()
    }
}

fn test_registry() -> AirRegistrySources {
    AirRegistrySources {
        bases: BTreeMap::from([(11, test_base())]),
        ..Default::default()
    }
}

fn naval_combat_frame() -> DecodedFrame {
    let mut frame = scout_frame(2);
    table::<tables::Provinces>(&mut frame)[0].controller = 2;
    set::<tables::WarRelations>(&mut frame, vec![war(1, 3)]);
    set::<tables::NavalCombats>(
        &mut frame,
        vec![NavalCombatRow {
            id: 55,
            province: 900,
            ..Default::default()
        }],
    );
    let mut detail = NavalCombatDetailsRow {
        combat: 55,
        province: 900,
        ..Default::default()
    };
    detail.sides[0].summary.countries = vec![3];
    detail.sides[1].summary.countries = vec![1];
    set::<tables::NavalCombatDetails>(&mut frame, vec![detail]);
    set::<tables::NavalCombatShips>(
        &mut frame,
        vec![NavalCombatShipRow {
            combat: 55,
            side: 1,
            member_source: 0,
            group_index: 0,
            state: 1,
            detection_state: -1,
            ..Default::default()
        }],
    );
    frame
}

fn naval_member(frame: &mut DecodedFrame) -> &mut NavalCombatShipRow {
    &mut table::<tables::NavalCombatShips>(frame)[0]
}

fn naval_side(frame: &mut DecodedFrame) -> &mut Vec<u32> {
    &mut table::<tables::NavalCombatDetails>(frame)[0].sides[1]
        .summary
        .countries
}

#[test]
fn naval_combat_target_needs_war_and_self_disclosed_active_detected_member() {
    let mut frame = naval_combat_frame();
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
    let (wing, registry) = (test_wing(), test_registry());
    let admit = |frame: &DecodedFrame, reached: &[u32]| {
        let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
        let wars = Wars::read(frame, &links).unwrap();
        let families = Families::read(frame, &links).ok();
        let sources =
            NavalCombatTargetSources::read(frame, &links, &wars, families.as_ref()).unwrap();
        sources.admit_combat(mission.resolve(&wing)?, reached)
    };
    let naval = |naval_combat| TargetPredicates {
        naval_combat,
        ..Default::default()
    };
    assert_eq!(admit(&frame, &[900]), Some(true));
    assert_eq!(
        target_admission(1 << 4, 1, Some(SEA), Some(0), naval(admit(&frame, &[900]))),
        TargetAdmission::Allowed
    );
    assert_eq!(admit(&frame, &[901]), Some(false));
    naval_member(&mut frame).state = 4;
    assert_eq!(admit(&frame, &[900]), Some(false));
    assert!(matches!(
        target_admission(1 << 4, 1, Some(SEA), Some(0), naval(admit(&frame, &[900]))),
        TargetAdmission::Unknown(_)
    ));
    naval_member(&mut frame).state = 1;
    naval_member(&mut frame).detection_state = 0;
    assert_eq!(admit(&frame, &[900]), Some(false));
    table::<tables::NavalCombatDetails>(&mut frame)[0].sides[0]
        .summary
        .countries = vec![99, 3];
    assert_eq!(admit(&frame, &[900]), Some(false));
    table::<tables::NavalCombatDetails>(&mut frame)[0].sides[0]
        .summary
        .countries = vec![99];
    assert_eq!(admit(&frame, &[900]), None);
    table::<tables::NavalCombatDetails>(&mut frame)[0].sides[0]
        .summary
        .countries = vec![3];
    naval_member(&mut frame).detection_state = -1;
    naval_member(&mut frame).member_source = 1;
    assert_eq!(admit(&frame, &[900]), Some(false));
    naval_member(&mut frame).member_source = 0;
    naval_member(&mut frame).group_index = u32::MAX;
    assert_eq!(admit(&frame, &[900]), Some(false));
    naval_member(&mut frame).group_index = 0;
    assert_eq!(admit(&frame, &[900]), Some(true));
    assert_eq!(
        target_admission(1 << 4, 1, Some(LAND), Some(0), naval(admit(&frame, &[900]))),
        TargetAdmission::Blocked
    );
}

#[test]
fn naval_combat_primary_countries_and_volunteer_mission_country_are_distinct() {
    let mut frame = naval_combat_frame();
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (12, 2), (13, 3)]);
    let (wing, registry) = (test_wing(), test_registry());
    let admit = |frame: &DecodedFrame| {
        let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
        let wars = Wars::read(frame, &links).unwrap();
        let families = Families::read(frame, &links).ok();
        let sources =
            NavalCombatTargetSources::read(frame, &links, &wars, families.as_ref()).unwrap();
        sources.admit_combat(mission.resolve(&wing)?, &[900])
    };
    naval_side(&mut frame).clear();
    assert_eq!(admit(&frame), Some(false));

    *naval_side(&mut frame) = vec![2];
    assert_eq!(admit(&frame), Some(false));
    let member = |country| FactionMemberRow {
        country,
        faction: 5,
        ordinal: country,
    };
    set::<tables::FactionMembers>(&mut frame, vec![member(1), member(2)]);
    assert_eq!(admit(&frame), Some(true));
    table::<tables::WarRelations>(&mut frame).push(war(1, 2));
    assert_eq!(admit(&frame), Some(false));
    set::<tables::FactionMembers>(&mut frame, Vec::new());
    set::<tables::WarRelations>(&mut frame, vec![war(1, 3)]);

    *naval_side(&mut frame) = vec![12];
    set::<tables::WarRelations>(&mut frame, vec![war(2, 13)]);
    set::<tables::VolunteerHosts>(
        &mut frame,
        vec![VolunteerHostRow {
            country: 1,
            host: 12,
            ..Default::default()
        }],
    );
    assert_eq!(admit(&frame), Some(true));
    set::<tables::WarRelations>(&mut frame, Vec::new());
    assert_eq!(admit(&frame), Some(false));
    truncate::<tables::NavalCombatShips>(&mut frame);
    assert_eq!(admit(&frame), None);
}

#[test]
fn mine_laying_uses_the_complete_country_vector_and_witnessed_omitted_zeros() {
    let mut frame = empty_frame();
    let links = BTreeMap::from([(1, 1), (2, 2), (12, 2)]);
    let mut wing = AirWingRow {
        owner: 1,
        ..Default::default()
    };
    let admit = |frame: &DecodedFrame, wing: &AirWingRow, region| {
        MineTargetSources::read(frame, &links)
            .unwrap()
            .admit(wing, region, 100_000_000)
    };
    assert_eq!(admit(&frame, &wing, 7), None);
    let witness = |country| NavalRegionalMineRow {
        country,
        kind: 0,
        region: 9,
        amount: 0,
    };
    set::<tables::NavalRegionalMines>(
        &mut frame,
        vec![
            witness(1),
            witness(2),
            NavalRegionalMineRow {
                country: 2,
                kind: 1,
                region: 7,
                amount: 100_000_000,
            },
        ],
    );
    assert_eq!(admit(&frame, &wing, 7), Some(true));
    wing.country_override = 12;
    assert_eq!(admit(&frame, &wing, 7), Some(false));
    assert_eq!(admit(&frame, &wing, 8), Some(true));
    assert_eq!(admit(&frame, &wing, 9), None);
    table::<tables::NavalRegionalMines>(&mut frame)[2].amount -= 1;
    assert_eq!(admit(&frame, &wing, 7), Some(true));
    table::<tables::NavalRegionalMines>(&mut frame).remove(1);
    assert_eq!(admit(&frame, &wing, 8), None);
    truncate::<tables::NavalRegionalMines>(&mut frame);
    assert_eq!(admit(&frame, &wing, 7), None);
}

#[test]
fn mine_laying_obeys_sea_dlc_and_or_branch_priority() {
    let admitted = |mask, fraction, dlc, mine| {
        target_admission(
            mask,
            1,
            fraction,
            dlc,
            TargetPredicates {
                mine,
                ..Default::default()
            },
        )
    };
    assert_eq!(
        admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0x10), Some(true)),
        TargetAdmission::Allowed
    );
    assert_eq!(
        admitted(AIR_MISSION_LAY_MINES, Some(LAND), Some(0x10), Some(true)),
        TargetAdmission::Blocked
    );
    assert_eq!(
        admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0), Some(true)),
        TargetAdmission::Blocked
    );
    assert_eq!(
        admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0x10), Some(false)),
        TargetAdmission::Blocked
    );
    assert!(matches!(
        admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0x10), None),
        TargetAdmission::Unknown(_)
    ));
    assert_eq!(
        admitted(
            (1 << 1) | AIR_MISSION_LAY_MINES,
            Some(MIXED),
            Some(0x10),
            Some(true)
        ),
        TargetAdmission::Allowed
    );
}

fn embarked_frame() -> DecodedFrame {
    let mut frame = naval_combat_frame();
    set::<tables::ProvinceUnitOrder>(
        &mut frame,
        vec![ProvinceUnitOrderRow {
            province: 900,
            mixed: vec![40],
            ..Default::default()
        }],
    );
    set::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: 40,
            logical_owner: 3,
            province: 900,
            ..Default::default()
        }],
    );
    set::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: 40,
            naval_transfer: 1,
            ..Default::default()
        }],
    );
    frame
}

fn embarked_movement(frame: &mut DecodedFrame) -> &mut DivisionMovementRow {
    &mut table::<tables::DivisionMovement>(frame)[0]
}

#[test]
fn embarked_target_uses_current_sea_location_transfer_combat_and_war() {
    let mut frame = embarked_frame();
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
    let (wing, registry) = (test_wing(), test_registry());
    let kinds = BTreeMap::from([(100, ProvinceKind::Land), (900, ProvinceKind::Sea)]);
    let admit = |frame: &DecodedFrame, kinds: &BTreeMap<u32, ProvinceKind>| {
        let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
        let wars = Wars::read(frame, &links).unwrap();
        let sources = NavalOccupantSources::read(frame, &wars, kinds).unwrap();
        sources.admit(mission.resolve(&wing).unwrap(), &[900])
    };
    let embarked = |frame: &DecodedFrame, kinds| admit(frame, kinds).embarked;
    assert_eq!(embarked(&frame, &kinds), Some(true));
    assert_eq!(
        target_admission(
            1 << 4,
            1,
            Some(SEA),
            Some(0),
            TargetPredicates {
                naval_combat: Some(false),
                naval_occupants: admit(&frame, &kinds),
                ..Default::default()
            },
        ),
        TargetAdmission::Allowed
    );
    let land = BTreeMap::from([(900, ProvinceKind::Land)]);
    assert_eq!(embarked(&frame, &land), Some(false));
    embarked_movement(&mut frame).naval_transfer = 0;
    assert_eq!(embarked(&frame, &kinds), Some(false));
    embarked_movement(&mut frame).naval_transfer = 1;
    embarked_movement(&mut frame).combat_count = 1;
    assert_eq!(embarked(&frame, &kinds), Some(false));
    embarked_movement(&mut frame).combat_count = 0;
    table::<tables::Divisions>(&mut frame)[0].logical_owner = 2;
    assert_eq!(embarked(&frame, &kinds), Some(false));
    table::<tables::Divisions>(&mut frame)[0].logical_owner = 3;
    assert_eq!(embarked(&frame, &kinds), Some(true));
}

#[test]
fn task_force_target_needs_sea_location_no_combat_and_war() {
    let mut frame = embarked_frame();
    table::<tables::ProvinceUnitOrder>(&mut frame)[0] = ProvinceUnitOrderRow {
        province: 900,
        mixed: vec![50],
        naval: vec![50],
    };
    set::<tables::TaskForces>(
        &mut frame,
        vec![TaskForceRow {
            id: 50,
            province: 900,
            logical_owner: 3,
            ..Default::default()
        }],
    );
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
    let (wing, registry) = (test_wing(), test_registry());
    let kinds = BTreeMap::from([(900, ProvinceKind::Sea)]);
    let admit = |frame: &DecodedFrame| {
        let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
        let wars = Wars::read(frame, &links).unwrap();
        let sources = NavalOccupantSources::read(frame, &wars, &kinds).unwrap();
        sources.admit(mission.resolve(&wing).unwrap(), &[900])
    };
    assert_eq!(
        admit(&frame),
        NavalOccupants {
            task_force: Some(true),
            embarked: Some(false),
            railway_gun: Some(false),
        }
    );
    table::<tables::TaskForces>(&mut frame)[0].combat_references = 1;
    assert_eq!(admit(&frame).task_force, Some(false));
    let occupants = admit(&frame);
    assert_eq!(
        target_admission(
            1 << 7,
            1,
            Some(SEA),
            Some(0),
            TargetPredicates {
                naval_combat: Some(false),
                naval_occupants: occupants,
                ..Default::default()
            },
        ),
        TargetAdmission::Blocked
    );
    table::<tables::TaskForces>(&mut frame)[0].combat_references = 0;
    table::<tables::TaskForces>(&mut frame)[0].logical_owner = 2;
    assert_eq!(admit(&frame).task_force, Some(false));
    truncate::<tables::WarRelations>(&mut frame);
    assert_eq!(admit(&frame).task_force, None);
    truncate::<tables::TaskForces>(&mut frame);
    let occupants = admit(&frame);
    assert_eq!((occupants.task_force, occupants.railway_gun), (None, None));
}

#[test]
fn embarked_target_preserves_unknown_and_counts_railway_guns() {
    let mut frame = embarked_frame();
    let links = BTreeMap::from([(1, 1), (3, 3)]);
    let (wing, registry) = (test_wing(), test_registry());
    let kinds = BTreeMap::from([(900, ProvinceKind::Sea)]);
    let admit = |frame: &DecodedFrame, kinds: &BTreeMap<u32, ProvinceKind>| {
        let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
        let wars = Wars::read(frame, &links).unwrap();
        let sources = NavalOccupantSources::read(frame, &wars, kinds).unwrap();
        sources.admit(mission.resolve(&wing).unwrap(), &[900])
    };
    assert_eq!(admit(&frame, &kinds).embarked, Some(true));
    table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![77];
    let occupants = admit(&frame, &kinds);
    assert_eq!(
        occupants,
        NavalOccupants {
            task_force: Some(false),
            embarked: Some(false),
            railway_gun: None,
        }
    );
    assert_eq!(
        target_admission(
            1 << 4,
            1,
            Some(SEA),
            Some(0),
            TargetPredicates {
                naval_combat: Some(false),
                naval_occupants: occupants,
                ..Default::default()
            },
        ),
        TargetAdmission::Unknown(Causes(1 << Cause::NavalRailwayGun as u16))
    );
    table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![40];
    assert_eq!(admit(&frame, &BTreeMap::new()), NavalOccupants::default());
    truncate::<tables::DivisionMovement>(&mut frame);
    assert_eq!(admit(&frame, &kinds).embarked, None);
    let TargetAdmission::Unknown(causes) =
        target_admission(1 << 4, 1, Some(SEA), Some(0), TargetPredicates::default())
    else {
        panic!("undecided naval arms");
    };
    assert!(!causes.railway_gun_only());
    assert!(causes.iter().eq([Cause::Naval, Cause::NavalRailwayGun]));
    let TargetAdmission::Unknown(causes) = target_admission(
        1 << 4,
        1,
        None,
        Some(0),
        TargetPredicates {
            naval_combat: Some(true),
            ..Default::default()
        },
    ) else {
        panic!("undecided sea fraction");
    };
    assert!(causes.iter().eq([Cause::Naval]));
}

fn strategic_frame() -> DecodedFrame {
    let mut frame = empty_frame();
    set::<tables::Provinces>(
        &mut frame,
        vec![
            province(100, 4, 1),
            province(400, 7, 2),
            province(401, 7, 3),
            province(900, 8, 3),
        ],
    );
    let state = |id, controller| StateRow {
        id,
        controller,
        ..Default::default()
    };
    set::<tables::States>(&mut frame, vec![state(7, 2), state(8, 3)]);
    set::<tables::WarRelations>(&mut frame, vec![war(1, 2), war(1, 3)]);
    set::<tables::ProvinceBuildings>(
        &mut frame,
        vec![BuildingRow {
            location: 401,
            level: 1,
            active: true,
            ..Default::default()
        }],
    );
    set::<tables::ProvinceUnitOrder>(
        &mut frame,
        vec![ProvinceUnitOrderRow {
            province: 401,
            mixed: vec![40],
            ..Default::default()
        }],
    );
    set::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: 40,
            province: 401,
            ..Default::default()
        }],
    );
    frame
}

#[test]
fn strategic_witness_uses_full_region_states_and_active_building_vectors() {
    let mut frame = strategic_frame();
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
    let regions = BTreeMap::from([(7, vec![400])]);
    let (wing, registry) = (test_wing(), test_registry());
    let full = |frame: &DecodedFrame, reached: &[u32]| {
        let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
        let wars = Wars::read(frame, &links).unwrap();
        let states = RegionStates::read(frame, &regions).unwrap();
        let sources = StrategicTargetSources::read(frame, &wars, &states).unwrap();
        sources.admit(mission.resolve(&wing).unwrap(), 7, reached)
    };
    let admit = |frame: &DecodedFrame, reached: &[u32]| full(frame, reached).found;
    assert_eq!(admit(&frame, &[401]), Some(true));
    assert_eq!(admit(&frame, &[900]), Some(false));
    table::<tables::States>(&mut frame)[0].controller = 99;
    assert_eq!(admit(&frame, &[401]), None);
    table::<tables::Provinces>(&mut frame)[2].controller = 1;
    assert_eq!(admit(&frame, &[401]), Some(false));
    table::<tables::Provinces>(&mut frame)[2].controller = 3;
    table::<tables::States>(&mut frame)[0].controller = 2;
    for bit in [3, 16] {
        assert_eq!(
            target_admission(
                (1 << 1) | (1 << bit),
                1,
                Some(LAND),
                Some(0),
                TargetPredicates {
                    strategic: admit(&frame, &[401]),
                    ..Default::default()
                },
            ),
            TargetAdmission::Allowed
        );
    }
    table::<tables::ProvinceBuildings>(&mut frame)[0].active = false;
    assert_eq!(admit(&frame, &[401]), Some(false));
    const RAIL_WAY: u32 = cbuilding_template::RAIL_WAY_TOKEN;
    const OTHER: u32 = RAIL_WAY + 1;
    table::<tables::ProvinceBuildings>(&mut frame)[0].level = -1;
    table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = RAIL_WAY;
    table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![77];
    assert_eq!(admit(&frame, &[401]), Some(false));
    table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![40];
    table::<tables::ProvinceBuildings>(&mut frame)[0].active = true;
    assert_eq!(admit(&frame, &[401]), Some(false));
    table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![77];
    assert_eq!(
        full(&frame, &[401]),
        Strategic {
            found: None,
            railway_gun: true
        }
    );
    table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = OTHER;
    assert_eq!(admit(&frame, &[401]), Some(false));
    for bit in [3, 16] {
        assert_eq!(
            target_admission(
                1 << bit,
                1,
                Some(LAND),
                Some(0),
                TargetPredicates {
                    strategic: admit(&frame, &[401]),
                    ..Default::default()
                },
            ),
            TargetAdmission::Blocked
        );
    }
    table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = RAIL_WAY;
    let railway = |bit: u32| {
        target_admission(
            1 << bit,
            1,
            Some(LAND),
            Some(0),
            TargetPredicates {
                strategic: None,
                strategic_railway_gun: true,
                ..Default::default()
            },
        )
    };
    for bit in [3, 16] {
        assert!(
            matches!(railway(bit), TargetAdmission::Unknown(causes) if causes.railway_gun_only())
        );
    }
    table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![40];
    assert_eq!(admit(&frame, &[401]), Some(false));
    truncate::<tables::Divisions>(&mut frame);
    assert!(!full(&frame, &[401]).railway_gun);
    assert_eq!(admit(&frame, &[401]), None);
    let building = BuildingRow {
        location: 7,
        level: 2,
        active: true,
        ..Default::default()
    };
    set::<tables::StateBuildings>(&mut frame, vec![building]);
    assert_eq!(admit(&frame, &[401]), Some(true));
    assert_eq!(admit(&frame, &[900]), Some(false));
    table::<tables::StateBuildings>(&mut frame)[0].level = 0;
    table::<tables::StateBuildings>(&mut frame)[0].building_type = RAIL_WAY;
    assert_eq!(
        full(&frame, &[401]),
        Strategic {
            found: None,
            railway_gun: false
        }
    );
    table::<tables::StateBuildings>(&mut frame)[0].building_type = OTHER;
    table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = OTHER;
    assert_eq!(admit(&frame, &[401]), Some(false));
    truncate::<tables::StateBuildings>(&mut frame);
    assert_eq!(admit(&frame, &[401]), None);
}

#[test]
fn railway_gun_omissions_need_every_undecided_branch_to_be_a_railway_arm() {
    let targets = TargetPredicates {
        strategic_railway_gun: true,
        naval_combat: Some(false),
        naval_occupants: NavalOccupants {
            task_force: Some(false),
            embarked: Some(false),
            railway_gun: None,
        },
        ..Default::default()
    };
    let causes = |mask| match target_admission(mask, 1, Some(MIXED), Some(0x10), targets) {
        TargetAdmission::Unknown(causes) => causes,
        other => panic!("{other:?}"),
    };
    let both = causes((1 << 3) | (1 << 4));
    assert!(both.railway_gun_only());
    assert!(
        both.iter()
            .eq([Cause::StrategicRailwayGun, Cause::NavalRailwayGun])
    );
    let mixed = causes((1 << 1) | (1 << 3));
    assert!(!mixed.railway_gun_only());
    assert_eq!(mixed.iter().count(), 2);
    assert!(!Causes::default().railway_gun_only());
}

#[test]
fn bypasses_end_the_scans_f66100_reads() {
    assert_eq!(scanned_bits((1 << 6) | (1 << 8)), 0);
    let transport = (1 << 10) | (1 << 3) | (1 << 8) | 0x1_f000;
    assert_eq!(scanned_bits(transport), (1 << 10) | (1 << 3) | (1 << 8));
    assert_eq!(scanned_bits((1 << 9) | (1 << 16)), 1 << 9);
    assert_eq!(scanned_bits(0x1_f000), 0x1_f000);
}

#[test]
fn category_priority_matches_native_target_branches() {
    assert_eq!(
        category((1 << 10) | (1 << 14) | (1 << 17), 1, 0),
        Some(Category::Transport)
    );
    assert_eq!(category((1 << 14) | 0x85, 1, 0), Some(Category::Scout));
    assert_eq!(category(0x2, 1, 0), Some(Category::BomberFlight));
    assert_eq!(
        category(0x2, 1, 0x6_0001_0000),
        Some(Category::BaseEquipment)
    );
    assert_eq!(
        category((1 << 10) | (1 << 14) | (1 << 17), 0, 0),
        Some(Category::BaseEquipment)
    );
    assert_eq!(category(0x80 | 0x2, 1, 0), Some(Category::Fighter));
    assert_eq!(category(1 << 11, 1, 0), None);
}

#[test]
fn projected_air_definition_bits_preserve_native_map_predicates() {
    let mask = AIR_MISSION_MAP_FLAGS;
    assert_eq!(mask, 0x1f_0025_fc00, "the map's flags and the air menu's");
    for raw in [0, u64::MAX, mask, !mask]
        .into_iter()
        .chain((0..64).map(|bit| 1_u64 << bit))
    {
        let projected = raw & mask;
        assert_eq!(
            raw & AIR_MISSION_ZERO_PLANE_FLAGS != 0,
            projected & AIR_MISSION_ZERO_PLANE_FLAGS != 0
        );
        assert_eq!(category(0x2, 7, raw), category(0x2, 7, projected));
        assert_eq!(
            raw & AIR_MISSION_FOREIGN_TARGET_FLAG != 0,
            projected & AIR_MISSION_FOREIGN_TARGET_FLAG != 0
        );
    }
}

#[test]
fn reached_count_gates_every_mask_and_stale_membership_is_used_as_is() {
    let none = TargetPredicates::default();
    assert_eq!(
        target_admission(1 << 10, 1, Some(LAND), Some(0), none),
        TargetAdmission::Allowed
    );
    assert_eq!(category(1 << 10, 913, 0), Some(Category::Transport));
    assert_eq!(
        admission(1 << 10, 0, Some(LAND), Some(0)),
        TargetAdmission::Blocked
    );
    assert_eq!(
        admission(1 << 6, 0, Some(LAND), Some(0)),
        TargetAdmission::Blocked
    );
    assert_eq!(
        admission(1 << 2, 1, Some(LAND), Some(0)),
        TargetAdmission::Allowed
    );
    assert_eq!(
        target_admission(1 << 2, 0, Some(LAND), Some(0), none),
        TargetAdmission::Blocked
    );
}

#[test]
fn later_proved_true_bypass_resolves_earlier_unknown() {
    assert!(matches!(
        admission(1 << 1, 1, Some(LAND), Some(0)),
        TargetAdmission::Unknown(_)
    ));
    assert_eq!(
        admission((1 << 1) | (1 << 10), 1, Some(LAND), Some(0)),
        TargetAdmission::Allowed
    );
    assert_eq!(
        admission((1 << 8) | (1 << 6), 1, Some(LAND), Some(0)),
        TargetAdmission::Allowed
    );
}

#[test]
fn unknown_target_predicates_do_not_become_false() {
    for bit in [1, 3, 4, 7, 8, 16] {
        assert!(matches!(
            admission(1 << bit, 1, Some(MIXED), Some(0x10)),
            TargetAdmission::Unknown(_)
        ));
    }
    assert!(matches!(
        admission(1 << 14, 1, Some(LAND), Some(0x10)),
        TargetAdmission::Unknown(_)
    ));
}

#[test]
fn zero_region_fraction_blocks_only_the_impossible_target_branches() {
    for bit in [1, 3, 8, 16] {
        assert_eq!(
            admission(1 << bit, 1, Some(SEA), Some(0x10)),
            TargetAdmission::Blocked
        );
    }
    for bit in [4, 7] {
        assert_eq!(
            admission(1 << bit, 1, Some(LAND), Some(0x10)),
            TargetAdmission::Blocked
        );
    }
    assert_eq!(
        admission((1 << 1) | (1 << 15), 1, Some(SEA), Some(0x10)),
        TargetAdmission::Allowed
    );
    assert!(matches!(
        admission((1 << 3) | (1 << 4), 1, Some(LAND), Some(0x10)),
        TargetAdmission::Unknown(_)
    ));
    assert!(matches!(
        admission(1 << 4, 1, None, Some(0x10)),
        TargetAdmission::Unknown(_)
    ));
}

#[test]
fn proved_sea_branches_can_resolve_earlier_unknown() {
    assert_eq!(region_fractions(2, 1, 1), MIXED);
    assert_eq!(region_fractions(100_001, 100_000, 1).sea_raw, 0);
    assert_eq!(region_fractions(0, 0, 0), RegionFractions::default());
    assert_eq!(
        admission((1 << 1) | (1 << 13), 1, Some(MIXED), Some(0x10)),
        TargetAdmission::Allowed
    );
    assert_eq!(
        admission(1 << 13, 1, Some(MIXED), Some(0)),
        TargetAdmission::Blocked
    );
    assert_eq!(
        admission(1 << 14, 1, Some(MIXED), Some(0)),
        TargetAdmission::Allowed
    );
    assert_eq!(
        admission(1 << 15, 1, Some(MIXED), None),
        TargetAdmission::Allowed
    );
    assert!(matches!(
        admission(1 << 13, 1, None, Some(0x10)),
        TargetAdmission::Unknown(_)
    ));
}
