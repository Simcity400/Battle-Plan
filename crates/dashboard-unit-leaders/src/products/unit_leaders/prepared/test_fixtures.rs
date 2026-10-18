//! Leader and battle frames shared by this product's tests and the dashboard's
//! pipeline tests.
use dashboard_core::test_fixtures::scale_frame;
use replay_data::{
    rows::CharacterPortraitRow, rows::CharacterRow, rows::LandCombatDetailsRow,
    rows::LandCombatResultsRow, rows::LandCombatSideDetails, rows::LandResultSide,
    rows::NavalMissionTaskForceRow, rows::OrdersGroupRow, rows::Recorded, rows::TableId,
    rows::TableRows, rows::TaskForceRow, rows::UnitLeaderRow,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

pub fn frame(hour: i32, countries: u32) -> DecodedFrame {
    branch_frame(1, hour, countries)
}

/// [`frame`] on `branch`.
pub fn branch_frame(branch: u64, hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = scale_frame(branch, hour, countries);
    for rows in [
        TableRows::UnitLeaders(
            (0..countries)
                .map(|index| UnitLeaderRow {
                    id: index + 1,
                    reference_type: 4713,
                    owner: 100_000 + index,
                    kind: 1,
                    category: 1,
                    character_id: index + 1,
                    skill_status: 2,
                    skill_level: 3,
                    components: [1, 2, 3, 4],
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::Characters(
            (0..countries)
                .map(|index| CharacterRow {
                    id: index + 1,
                    token: format!("Synthetic commander {index}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::CharacterPortraits(
            (0..countries)
                .map(|index| CharacterPortraitRow {
                    character: index + 1,
                    portraits: Vec::new(),
                })
                .collect(),
        ),
        TableRows::UnitLeaderTraits(Vec::new()),
        TableRows::LandCombatDetails(Vec::new()),
        TableRows::NavalCombatDetails(Vec::new()),
        TableRows::LandCombatResults(Vec::new()),
        TableRows::NavalResultSources(Vec::new()),
        TableRows::NavalResults(Vec::new()),
        TableRows::NavalCombatResultDetails(Vec::new()),
        TableRows::OrdersGroups(Vec::new()),
        TableRows::Fleets(Vec::new()),
    ] {
        set(&mut frame, rows);
    }
    frame
}
/// An hour of `branch` with `countries` (at least six) leaders at war, every
/// one a land leader of its own country. Every hour moves what leaders do not
/// read: the countries' balances, a task force's position and its mission's
/// fuel, and each combat side's committed and reserve operands. Combat 91
/// runs from 100 to 150, commanded by leaders 0 and 1, until leader 2 takes
/// side 0 at 120; its result is read at 151. Combat 92 runs from 130 to 160,
/// between leaders 1 and 3; its result is read at 161. Leader 0 commands
/// army 1 until leader 4 does from 140; leader 4's skill rises at 170; country
/// 2 is renamed at 175.
pub fn war_frame(branch: u64, hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = branch_frame(branch, hour, countries);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for country in rows.iter_mut() {
            country.political_power_raw = Some(i64::from(hour) * 1000);
        }
        if hour >= 175 {
            rows[2].tag = b"RNM".to_vec();
        }
    }
    if hour >= 170
        && let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
    {
        rows[4].skill_level = 4;
    }
    let operands = |hour: i32, salt: u64| LandCombatSideDetails {
        committed: vec![u64::try_from(hour).unwrap_or_default() * salt],
        reserves: vec![salt],
        ..Default::default()
    };
    let mut combats = Vec::new();
    if (100..=150).contains(&hour) {
        combats.push(LandCombatDetailsRow {
            combat: 91,
            sides: [
                LandCombatSideDetails {
                    commander: if hour < 120 {
                        reference(0)
                    } else {
                        reference(2)
                    },
                    ..operands(hour, 3)
                },
                LandCombatSideDetails {
                    commander: reference(1),
                    ..operands(hour, 5)
                },
            ],
            ..Default::default()
        });
    }
    if (130..=160).contains(&hour) {
        combats.push(LandCombatDetailsRow {
            combat: 92,
            sides: [
                LandCombatSideDetails {
                    commander: reference(1),
                    ..operands(hour, 7)
                },
                LandCombatSideDetails {
                    commander: reference(3),
                    ..operands(hour, 11)
                },
            ],
            ..Default::default()
        });
    }
    set(&mut frame, TableRows::LandCombatDetails(combats));
    let result = |id: u64, combat: u64, winners: u8, commanders: [u64; 2]| LandCombatResultsRow {
        id,
        combat,
        end_hour: hour,
        winners,
        sides: commanders.map(|commander| LandResultSide {
            commander,
            ..Default::default()
        }),
        ..Default::default()
    };
    set(
        &mut frame,
        TableRows::LandCombatResults(match hour {
            151 => vec![result(1, 91, 1, [reference(2), reference(1)])],
            161 => vec![result(2, 92, 2, [reference(1), reference(3)])],
            _ => Vec::new(),
        }),
    );
    set(
        &mut frame,
        TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: 1,
            leader: if hour < 140 {
                reference(0)
            } else {
                reference(4)
            },
            name: Recorded::Value(Some(b"First Army".to_vec())),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 7,
            province: u32::try_from(hour).unwrap_or_default(),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::NavalMissionTaskForces(vec![NavalMissionTaskForceRow {
            task_force: 7,
            fuel: i64::from(hour),
            ..Default::default()
        }]),
    );
    frame
}

pub fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}
pub fn reference(index: u32) -> u64 {
    (u64::from(index + 1) << 32) | 4713
}

pub fn outcome_frames(conflict: bool) -> Vec<DecodedFrame> {
    let mut first = frame(100, 2);
    let mut changed = frame(101, 2);
    let mut ended = frame(102, 2);
    for (frame, commander) in [(&mut first, reference(0)), (&mut changed, reference(1))] {
        set(
            frame,
            TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                combat: 91,
                sides: [
                    LandCombatSideDetails {
                        commander,
                        ..Default::default()
                    },
                    Default::default(),
                ],
                ..Default::default()
            }]),
        );
    }
    set(
        &mut ended,
        TableRows::LandCombatResults(vec![LandCombatResultsRow {
            id: 1,
            combat: 91,
            end_hour: 102,
            winners: 1,
            sides: [
                LandResultSide {
                    commander: reference(1),
                    ..Default::default()
                },
                Default::default(),
            ],
            ..Default::default()
        }]),
    );
    let mut repeated = ended.clone();
    repeated.hour = 103;
    if conflict
        && let TableView::Complete(TableRows::LandCombatResults(rows)) =
            &mut *repeated.tables[TableId::LandCombatResults.index()]
    {
        rows[0].winners = 2;
    }
    vec![first, changed, ended, repeated]
}
