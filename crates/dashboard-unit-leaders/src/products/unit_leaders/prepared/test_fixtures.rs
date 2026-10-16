//! Leader and battle frames shared by this product's tests and the dashboard's
//! pipeline tests.
use dashboard_core::test_fixtures::scale_frame;
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{
        CharacterPortraitRow, CharacterRow, LandCombatDetailsRow, LandCombatResultsRow,
        LandCombatSideDetails, LandResultSide, TableId, TableRows, UnitLeaderRow,
    },
};

pub fn frame(hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, countries);
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
