use super::*;
use replay_data::rows::{
    CountryRow, NavalBaseRow, NavalRegionalMineRow, ProvinceRow, ProvinceUnitOrderRow, TableId,
    TableRows, TableSchema, TaskForceRow, WarRelationRow,
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

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
}

fn frame(countries: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 101,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    };
    set::<tables::Countries>(
        &mut frame,
        (1..=countries)
            .map(|slot| CountryRow {
                id: slot,
                canonical_slot: slot,
                ..Default::default()
            })
            .collect(),
    );
    frame
}

fn war(first_country: u32, second_country: u32) -> WarRelationRow {
    WarRelationRow {
        first_country,
        second_country,
        ..Default::default()
    }
}

fn port_and_mines() -> DecodedFrame {
    let mut frame = frame(3);
    set::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 412,
            controller: 3,
            ..Default::default()
        }],
    );
    set::<tables::NavalBases>(
        &mut frame,
        vec![NavalBaseRow {
            province: 412,
            ..Default::default()
        }],
    );
    set::<tables::ProvinceUnitOrder>(
        &mut frame,
        vec![ProvinceUnitOrderRow {
            province: 412,
            naval: vec![42],
            ..Default::default()
        }],
    );
    set::<tables::TaskForces>(
        &mut frame,
        vec![TaskForceRow {
            id: 42,
            province: 412,
            logical_owner: 3,
            ..Default::default()
        }],
    );
    set::<tables::WarRelations>(&mut frame, vec![war(1, 3)]);
    set::<tables::NavalRegionalMines>(
        &mut frame,
        vec![NavalRegionalMineRow {
            country: 1,
            kind: 0,
            region: 8,
            amount: 0,
        }],
    );
    frame
}

fn inputs(reached: Vec<u32>) -> AirMissionInputRow {
    AirMissionInputRow {
        reached_count: reached.len() as i32,
        reached_provinces: reached,
        ..Default::default()
    }
}

const COUNTRY: MissionCountry = MissionCountry { slot: 1 };

#[test]
fn predicates_reach_only_the_wings_whose_mask_reads_them() {
    let frame = port_and_mines();
    let links = BTreeMap::from([(1, 1), (3, 3)]);
    let wars = Wars::read(&frame, &links).unwrap();
    let fractions = BTreeMap::from([(7, LAND)]);
    let scans = Scans {
        tactical: None,
        port: Some(PortTargetSources::read(&frame, &wars).unwrap()),
        strategic: None,
        scout: None,
        naval_combat: None,
        occupants: None,
        mine: Some(MineTargetSources::read(&frame, &links).unwrap()),
        fractions: &fractions,
        dlc_mask: Some(0x10),
        mine_maximum_raw: 100_000,
    };
    let wing = |mission_mask| AirWingRow {
        owner: 1,
        mission_mask,
        ..Default::default()
    };
    let reached = inputs(vec![412]);
    let port = scans.predicates(&wing(1 << 8), COUNTRY, 7, &reached);
    assert_eq!((port.port, port.mine), (Some(true), None));
    let mines = scans.predicates(&wing(AIR_MISSION_LAY_MINES), COUNTRY, 7, &reached);
    assert_eq!((mines.port, mines.mine), (None, Some(true)));
    let empty = scans.predicates(&wing(1 << 8), COUNTRY, 7, &inputs(Vec::new()));
    assert_eq!(empty.port, None);
    let bypass = scans.predicates(&wing((1 << 8) | (1 << 6)), COUNTRY, 7, &reached);
    assert_eq!(bypass.port, None);
    let transport = (1 << 10) | (1 << 8) | AIR_MISSION_LAY_MINES;
    let transport = scans.predicates(&wing(transport), COUNTRY, 7, &reached);
    assert_eq!((transport.port, transport.mine), (Some(true), None));
}

#[test]
fn admission_takes_the_mission_regions_fractions() {
    let frame = port_and_mines();
    let links = BTreeMap::from([(1, 1), (3, 3)]);
    let wars = Wars::read(&frame, &links).unwrap();
    let fractions = BTreeMap::from([(7, LAND), (8, SEA)]);
    let scans = Scans {
        tactical: None,
        port: Some(PortTargetSources::read(&frame, &wars).unwrap()),
        strategic: None,
        scout: None,
        naval_combat: None,
        occupants: None,
        mine: None,
        fractions: &fractions,
        dlc_mask: Some(0),
        mine_maximum_raw: 100_000,
    };
    let wing = AirWingRow {
        owner: 1,
        mission_mask: 1 << 8,
        ..Default::default()
    };
    let reached = inputs(vec![412]);
    assert_eq!(
        scans.admission(&wing, COUNTRY, 7, &reached),
        TargetAdmission::Allowed
    );
    assert_eq!(
        scans.admission(&wing, COUNTRY, 8, &reached),
        TargetAdmission::Blocked
    );
    let TargetAdmission::Unknown(causes) = scans.admission(&wing, COUNTRY, 9, &reached) else {
        panic!("a region without fractions is undecided");
    };
    assert!(causes.iter().eq([air_reachability::Cause::Port]));
}

#[test]
fn undecided_wings_are_one_diagnostic_per_reason_and_railway_guns_are_counted() {
    let railway = air_reachability::target_admission(
        (1 << 3) | (1 << 4),
        1,
        Some(RegionFractions {
            land_raw: 50_000,
            sea_raw: 50_000,
        }),
        Some(0x10),
        TargetPredicates {
            strategic_railway_gun: true,
            naval_combat: Some(false),
            naval_occupants: air_reachability::NavalOccupants {
                task_force: Some(false),
                embarked: Some(false),
                railway_gun: None,
            },
            ..Default::default()
        },
    );
    let tactical =
        air_reachability::target_admission(1 << 1, 1, Some(LAND), Some(0), Default::default());
    let mut undecided = Undecided::default();
    for admission in [railway, railway, tactical] {
        let TargetAdmission::Unknown(causes) = admission else {
            panic!("{admission:?}");
        };
        undecided.record(causes);
    }
    let lines: Vec<_> = undecided.diagnostics().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().any(|line| line.starts_with("2 wings omitted")
        && line.contains(air_reachability::Cause::NavalRailwayGun.reason())));
    assert!(lines.iter().any(|line| line.starts_with("1 wings omitted")
        && line.contains(air_reachability::Cause::Tactical.reason())));
    assert_eq!(
        lines.last().unwrap(),
        "2 wings omitted only for the unrecorded railway-gun target arms"
    );
}
