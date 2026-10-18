use super::*;
use replay_data::{
    rows::CountryRow, rows::DivisionRow, rows::NavalBaseRow, rows::NavalCombatDetailsRow,
    rows::NavalCombatRow, rows::NavalCombatShipRow, rows::NavalMineLevelRow, rows::ProvinceRow,
    rows::ProvinceUnitOrderRow, rows::StateRow, rows::TableId, rows::TableRows, rows::TableSchema,
    rows::TaskForceRow, rows::WarRelationRow,
};
use replay_frame::{frame::Metrics, frame::TableView};
use std::time::Instant;

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

/// Complete empty tables, with `countries` slots owning their numbers.
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

/// A hostile port at 412 and a region-7 mine level for country 1.
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
    set::<tables::NavalMineLevels>(
        &mut frame,
        vec![NavalMineLevelRow {
            country: 1,
            region: 7,
            raw_mines: 0,
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
    // the game's count gate and the bit-6 bypass read no scan.
    let empty = scans.predicates(&wing(1 << 8), COUNTRY, 7, &inputs(Vec::new()));
    assert_eq!(empty.port, None);
    let bypass = scans.predicates(&wing((1 << 8) | (1 << 6)), COUNTRY, 7, &reached);
    assert_eq!(bypass.port, None);
    // Past the bit 9/10 bypass, admission reads no bit 12..16 scan, but
    // still the earlier ones.
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

/// A synthetic hour at campaign density: 13,000 provinces (a third
/// sea), 170 regions, 1,500 active countries with 60 wars per 100, 200
/// naval combats of 40 ships, 2,000 naval bases, two task forces and
/// 20 divisions per active country, and 5,000 registered missions of
/// 16 reached provinces. 3,500 more countries without units, wars or
/// provinces must not change a single war test or admission. Prints
/// the scan build and per-hour predicate time; run with
/// `--release --nocapture` to measure.
#[test]
fn target_scans_scale_with_missions_not_countries() {
    let run = |countries: u32| {
        let (frame, links, regions, wings) = dense_hour(countries);
        let kinds: BTreeMap<u32, ProvinceKind> = (1..=13_000)
            .map(|id| {
                (
                    id,
                    if id % 3 == 0 {
                        ProvinceKind::Sea
                    } else {
                        ProvinceKind::Land
                    },
                )
            })
            .collect();
        let fractions: BTreeMap<u32, RegionFractions> = regions
            .keys()
            .map(|&region| {
                (
                    region,
                    RegionFractions {
                        land_raw: 66_666,
                        sea_raw: 33_333,
                    },
                )
            })
            .collect();
        let begin = Instant::now();
        let wars = Wars::read(&frame, &links).unwrap();
        let families = Families::read(&frame, &links).ok();
        let fog = families
            .clone()
            .and_then(|families| families.with_attaches(&frame, &links).ok());
        let scans = Scans {
            tactical: None,
            port: Some(PortTargetSources::read(&frame, &wars).unwrap()),
            strategic: Some(StrategicTargetSources::read(&frame, &wars, &regions).unwrap()),
            scout: Some(
                ScoutTargetSources::read(&frame, &links, &wars, fog.as_ref(), &regions).unwrap(),
            ),
            naval_combat: Some(
                NavalCombatTargetSources::read(&frame, &links, &wars, families.as_ref()).unwrap(),
            ),
            occupants: Some(NavalOccupantSources::read(&frame, &wars, &kinds).unwrap()),
            mine: Some(MineTargetSources::read(&frame, &links).unwrap()),
            fractions: &fractions,
            dlc_mask: Some(0x10),
            mine_maximum_raw: 100_000,
        };
        let built = begin.elapsed();
        let begin = Instant::now();
        let admissions: Vec<_> = wings
            .iter()
            .map(|(wing, input)| {
                let country = MissionCountry { slot: wing.owner };
                scans.admission(wing, country, wing.mission_region.unwrap(), input)
            })
            .collect();
        let scanned = begin.elapsed();
        let allowed = admissions
            .iter()
            .filter(|&&admission| admission == TargetAdmission::Allowed)
            .count();
        println!(
            "{countries} countries, {} missions: build {built:?}, admission {scanned:?}, {allowed} allowed",
            wings.len()
        );
        (wars.tests.get(), admissions)
    };
    let (tests, admissions) = run(1_500);
    assert!(tests > 0);
    assert!(admissions.contains(&TargetAdmission::Allowed));
    assert_eq!(run(5_000), (tests, admissions));
}

type Hour = (
    DecodedFrame,
    BTreeMap<u32, u32>,
    BTreeMap<u32, Vec<u32>>,
    Vec<(AirWingRow, AirMissionInputRow)>,
);

fn dense_hour(countries: u32) -> Hour {
    const PROVINCES: u32 = 13_000;
    const ACTIVE: u32 = 1_500;
    let mut frame = frame(countries);
    let links = (1..=countries).map(|slot| (slot, slot)).collect();
    let owner = |id: u32| id % ACTIVE + 1;
    set::<tables::Provinces>(
        &mut frame,
        (1..=PROVINCES)
            .map(|id| ProvinceRow {
                id,
                state: id / 10 + 1,
                controller: owner(id / 10),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::States>(
        &mut frame,
        (1..=PROVINCES / 10 + 1)
            .map(|id| StateRow {
                id,
                controller: owner(id - 1),
                ..Default::default()
            })
            .collect(),
    );
    let mut wars: Vec<_> = (1..=ACTIVE)
        .step_by(5)
        .flat_map(|first| (1..=3).map(move |step| war(first, (first + step * 7) % ACTIVE + 1)))
        .collect();
    wars.sort_unstable_by_key(|row| (row.first_country, row.second_country));
    wars.dedup_by_key(|row| (row.first_country, row.second_country));
    set::<tables::WarRelations>(&mut frame, wars);
    let sea = |index: u32| (index * 3) % PROVINCES + 3;
    let mut combats: Vec<_> = (0..200_u32)
        .map(|index| NavalCombatRow {
            id: u64::from(index + 1),
            province: sea(index * 17),
            ..Default::default()
        })
        .collect();
    combats.sort_unstable_by_key(|row| row.id);
    let details = combats
        .iter()
        .map(|combat| {
            let mut detail = NavalCombatDetailsRow {
                combat: combat.id,
                province: combat.province,
                ..Default::default()
            };
            detail.sides[0].summary.countries = vec![owner(combat.id as u32)];
            detail.sides[1].summary.countries = vec![owner(combat.id as u32 * 7)];
            detail
        })
        .collect();
    let mut ships: Vec<_> = combats
        .iter()
        .flat_map(|combat| {
            (0..40_u32).map(move |slot| NavalCombatShipRow {
                combat: combat.id,
                side: (slot % 2) as u8,
                battle_slot: slot,
                state: 1,
                detection_state: i32::from(slot % 3 == 0),
                ..Default::default()
            })
        })
        .collect();
    ships.sort_unstable_by_key(|row| (row.combat, row.side, row.battle_slot));
    set::<tables::NavalCombats>(&mut frame, combats);
    set::<tables::NavalCombatDetails>(&mut frame, details);
    set::<tables::NavalCombatShips>(&mut frame, ships);
    let mut bases: Vec<_> = (0..2_000_u32)
        .map(|index| NavalBaseRow {
            province: index * 6 + 1,
            ..Default::default()
        })
        .collect();
    bases.sort_unstable_by_key(|row| row.province);
    set::<tables::NavalBases>(&mut frame, bases);
    let task_forces: Vec<_> = (0..ACTIVE * 2)
        .map(|index| TaskForceRow {
            id: u64::from(index) + 1,
            province: sea(index),
            logical_owner: owner(index),
            ..Default::default()
        })
        .collect();
    let divisions: Vec<_> = (0..ACTIVE * 20)
        .map(|index| DivisionRow {
            id: 1_000_000 + u64::from(index),
            province: index % PROVINCES + 1,
            logical_owner: owner(index),
            ..Default::default()
        })
        .collect();
    let mut orders = BTreeMap::<u32, ProvinceUnitOrderRow>::new();
    for task_force in &task_forces {
        let order = orders.entry(task_force.province).or_default();
        order.province = task_force.province;
        order.naval.push(task_force.id);
        order.mixed.push(task_force.id);
    }
    for division in &divisions {
        let order = orders.entry(division.province).or_default();
        order.province = division.province;
        order.mixed.push(division.id);
    }
    set::<tables::ProvinceUnitOrder>(&mut frame, orders.into_values().collect());
    set::<tables::TaskForces>(&mut frame, task_forces);
    set::<tables::Divisions>(&mut frame, divisions);
    let regions: BTreeMap<u32, Vec<u32>> = (1..=170)
        .map(|region| {
            (
                region,
                (0..PROVINCES / 170)
                    .map(|index| index * 170 + region)
                    .collect(),
            )
        })
        .collect();
    let masks = [
        1 << 4,
        1 << 8,
        1 << 3,
        1 << 14,
        AIR_MISSION_LAY_MINES,
        1 << 2,
    ];
    let wings = (0..5_000_u32)
        .map(|index| {
            let region = index % 170 + 1;
            let wing = AirWingRow {
                id: u64::from(index) + 1,
                owner: owner(index),
                mission_mask: masks[index as usize % masks.len()],
                mission_region: Some(region),
                ..Default::default()
            };
            let reached = (0..16)
                .map(|step| (index * 31 + step * 13) % PROVINCES + 1)
                .collect();
            (wing, inputs(reached))
        })
        .collect();
    (frame, links, regions, wings)
}
