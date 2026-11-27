use super::super::spending::{Facts, LineFacts, SpentStep, unread_spans};
use super::super::SpendingGap;
use super::index;
use dashboard_base::products::breakdown::{BreakdownAmount, BreakdownState};
use replay_data::rows::TagHolderRow;

const NONE: u32 = TagHolderRow::NONE;

fn steps(spent: &[(i32, Option<i64>)]) -> Vec<SpentStep> {
    spent.iter().map(|&(known_at, total)| SpentStep { known_at, total_raw: total.map(|value| value.to_string()) }).collect()
}

fn facts() -> Facts {
    Facts {
        from: 0,
        to: 100,
        spent: [
            ((false, false, 7, 1, NONE), steps(&[(10, Some(500_000)), (30, Some(900_000))])),
            ((false, false, 8, 1, NONE), steps(&[(20, Some(200_000))])),
            ((true, false, 7, 1, NONE), steps(&[(15, Some(100_000))])),
        ]
        .into(),
        names: [(7, "Workshop".to_owned()), (8, "type_b".to_owned())].into(),
        ..Facts::default()
    }
}

#[test]
fn spending_at_an_hour_is_the_running_total_then() {
    let at = index(&facts()).at(25);
    let parts = &at.breakdown.parts;
    assert_eq!(at.breakdown.total.amount, Some(BreakdownAmount { numerator: "8".into(), denominator: "1".into() }), "the step read at hour 30 is not counted at hour 25");
    assert_eq!(parts.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["New buildings", "Repairs"]);
    assert_eq!(parts[0].children.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Workshop", "type_b"]);
    assert_eq!(at.through, 29);
    let new_type = |hour: i32| index(&facts()).at(hour).breakdown.parts[0].children.iter().find(|part| part.name == "type_b").map(|part| part.ordinal);
    assert_eq!(new_type(25), new_type(90), "a type keeps the ordinal of its first appearance at every hour");
}

#[test]
fn hours_with_neither_queue_read_are_not_an_exact_zero() {
    let mut held = facts();
    held.spent.clear();
    held.unrecorded.push((0, 4, "The construction queues were not read, so output in those hours is not counted".into()));
    let early = index(&held).at(3);
    assert_eq!((early.breakdown.total.state, early.breakdown.total.amount.clone(), early.breakdown.unrecorded_hours), (BreakdownState::NotRecorded, None, 4), "no hour witnessed is no figure at all");
    assert_eq!(early.through, 3, "the count of unread hours grows each hour");
    let later = index(&held).at(50);
    assert_eq!((later.breakdown.total.state, later.breakdown.total.amount), (BreakdownState::Partial, Some(BreakdownAmount { numerator: "0".into(), denominator: "1".into() })), "an unread stretch keeps every later zero partial");
}

#[test]
fn an_empty_queue_read_in_full_certifies_only_itself() {
    let mut gaps = Vec::new();
    unread_spans(0, 100, vec![(0, 100)], "buildings", &mut gaps);
    unread_spans(0, 100, vec![(0, 39), (60, 100)], "railways", &mut gaps);
    assert_eq!(gaps, [(40, 59, "railways".to_owned())], "a railway queue unread at 40 to 59 is unread then, whatever the building queue shows");
    let mut held = facts();
    held.unrecorded = gaps;
    let at = index(&held).at(50);
    assert_eq!((at.breakdown.total.state, at.breakdown.unrecorded_hours), (BreakdownState::Partial, 11), "no witness of the railway queue is never a recorded zero");
    assert!(at.breakdown.total.reason.as_deref().is_some_and(|reason| reason.contains("railways")));
    let mut both = facts();
    both.unrecorded = vec![(40, 59, "buildings".to_owned()), (45, 59, "railways".to_owned())];
    let at = index(&both).at(59);
    assert_eq!(at.breakdown.unrecorded_hours, 20, "an hour neither queue was read is one unread hour, not two");
    let reason = at.breakdown.total.reason.unwrap_or_default();
    assert!(reason.contains("buildings") && reason.contains("railways"), "each queue names its own gap: {reason}");
}

#[test]
fn an_output_gap_is_dated_at_the_read_that_shows_it_and_stays() {
    let mut held = facts();
    held.lines.push(LineFacts { leaf: (false, 7, 1, NONE), building: Some(7), new_from: Some(5), repair_from: None, gaps: vec![(12, SpendingGap::InexactOutput), (40, SpendingGap::FinalOutputMissing)], before_first_read: Some(5) });
    held.lines.push(LineFacts { leaf: (false, 9, 2, NONE), building: Some(9), new_from: Some(60), repair_from: Some(70), gaps: vec![(61, SpendingGap::LinkedOutput), (80, SpendingGap::InexactOutput)], before_first_read: None });
    held.unattributed.push(35);
    let index = index(&held);
    let state_of = |hour: i32| index.at(hour).breakdown.parts[0].children.iter().find(|part| part.name == "Workshop").map(|part| part.state);
    assert_eq!(state_of(11), Some(BreakdownState::Known), "before the read showing the gap the output was exact");
    assert_eq!(state_of(12), Some(BreakdownState::Partial));
    assert_eq!(state_of(90), Some(BreakdownState::Partial), "a missing output qualifies the running total after its project closed");
    assert_eq!(index.at(11).through, 11, "the next change, a gap read at 12, ends the answer");
    let before = index.at(34);
    assert_eq!((before.unattributed, before.before_first_read), (0, 1));
    let after = index.at(36);
    assert_eq!(after.unattributed, 1);
    assert!(after.breakdown.total.reason.as_deref().is_some_and(|reason| reason.contains("not traced to a country")), "output no country can be given leaves the total partial");
    assert!(index.at(65).breakdown.parts.iter().all(|part| part.key != "repair" || part.children.iter().all(|child| child.name != "Building name unavailable")));
    let ninth = |hour: i32, kind: &str| {
        index.at(hour).breakdown.parts.iter().find(|part| part.key == kind).and_then(|part| part.children.iter().find(|child| child.key == "type:9")).map(|child| child.state)
    };
    assert_eq!(ninth(65, "new"), Some(BreakdownState::Partial), "the gap read at 61 belongs to the new work witnessed then");
    assert_eq!(ninth(75, "repair"), None, "a new-build gap at 61 never qualifies repairs first seen at 70");
    assert_eq!(ninth(85, "repair"), Some(BreakdownState::Partial), "a gap read during the repair is the repair's");
}

#[test]
fn spending_no_longer_read_makes_the_part_a_lower_bound_never_zero() {
    let mut held = facts();
    held.spent.insert((false, false, 7, 1, NONE), steps(&[(10, Some(500_000)), (20, None)]));
    let at = index(&held).at(25);
    let part = at.breakdown.parts[0].children.iter().find(|part| part.name == "Workshop").expect("workshop");
    assert_eq!((part.state, part.amount.clone()), (BreakdownState::AtLeast, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() })));
    assert_eq!(at.breakdown.total.state, BreakdownState::AtLeast);
}

#[test]
fn repeated_breakdown_answers_reuse_the_held_index_without_rereading_storage() {
    use super::{ConstructionBreakdown, ConstructionBreakdownParams};
    use dashboard_base::{
        Source, SourceKey,
        products::{Country, LoadContext, Product},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_columnar::unit::reader::{DecodeCounts, count_decodes};
    use replay_data::rows::{BuildingConstructionRow, ConstructionLineOutputRow, Recorded, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let frames = (100..=110)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let spent = match hour {
                100..=103 => 100_000,
                104..=107 => 300_000,
                _ => 600_000,
            };
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                id: 1,
                country: 9001,
                building_type: 7,
                target_state: 12,
                cost: 500_000,
                created_hour: 90,
                ..Default::default()
            }]))
            .into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(vec![ConstructionLineOutputRow {
                line: 1,
                since_hour: 100,
                construction_raw: spent,
                country: Recorded::Value(Some(9001)),
                building_type: Recorded::Value(Some(7)),
                target_state: Recorded::Value(Some(12)),
                exact: true,
                ..Default::default()
            }]))
            .into();
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "construction-breakdown".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    let load = LoadContext { content: None };
    let ask = |hour: i32| {
        count_decodes(|| {
            ConstructionBreakdown::load(
                &source,
                &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour },
                &load,
                &cancel,
            )
            .unwrap()
        })
    };

    let (cold, cold_counts) = ask(105);
    assert_ne!(cold_counts, DecodeCounts::default(), "the first breakdown answer reads columnar storage to build the index");
    assert_eq!(cold.breakdown.total.state, BreakdownState::Known);
    assert_eq!(cold.breakdown.total.amount, Some(BreakdownAmount { numerator: "3".into(), denominator: "1".into() }));
    assert_eq!(cold.through, 107);

    let (same, same_counts) = ask(105);
    assert_eq!(same_counts, DecodeCounts::default(), "repeating the same hour reuses the held index without reading storage");
    assert_eq!(same, cold);

    let (earlier, earlier_counts) = ask(102);
    assert_eq!(earlier_counts, DecodeCounts::default(), "an earlier hour reuses the held index without reading storage");
    assert_eq!(earlier.breakdown.total.state, BreakdownState::Known);
    assert_eq!(earlier.breakdown.total.amount, Some(BreakdownAmount { numerator: "1".into(), denominator: "1".into() }));
    assert_eq!(earlier.through, 103);

    let (later, later_counts) = ask(109);
    assert_eq!(later_counts, DecodeCounts::default(), "a later hour reuses the held index without reading storage");
    assert_eq!(later.breakdown.total.state, BreakdownState::Known);
    assert_eq!(later.breakdown.total.amount, Some(BreakdownAmount { numerator: "6".into(), denominator: "1".into() }));
    assert_eq!(later.through, 110);
}
