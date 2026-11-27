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
        names: [(7, vec![(0, "Workshop".to_owned())]), (8, vec![(0, "type_b".to_owned())])].into(),
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
    held.lines.push(LineFacts { leaf: (false, 7, 1, NONE), building: Some(7), new_from: Some(5), repair_from: None, gaps: vec![(12, i32::MAX, SpendingGap::InexactOutput), (40, i32::MAX, SpendingGap::FinalOutputMissing)], before_first_read: Some(5) });
    held.lines.push(LineFacts { leaf: (false, 9, 2, NONE), building: Some(9), new_from: Some(60), repair_from: Some(70), gaps: vec![(61, 69, SpendingGap::LinkedOutput), (80, i32::MAX, SpendingGap::InexactOutput)], before_first_read: None });
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

#[test]
fn new_work_flag_is_not_backdated_to_earlier_repair_charges() {
    use super::super::spending::gap_assignment;
    use super::{ConstructionBreakdown, ConstructionBreakdownParams};
    use dashboard_base::{
        Source, SourceKey,
        products::{Country, LoadContext, Product},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{BuildingConstructionRow, ConstructionLineOutputRow, LexicalTokenRow, Recorded, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    assert_eq!(gap_assignment(Some(120), Some(115), 105, i32::MAX), Some((true, 115)));
    assert_eq!(gap_assignment(Some(120), Some(100), 122, i32::MAX), Some((false, 122)));
    assert_eq!(gap_assignment(Some(60), Some(70), 61, 69), Some((false, 61)));
    assert_eq!(gap_assignment(Some(60), Some(70), 80, i32::MAX), Some((true, 80)));
    assert_eq!(gap_assignment(Some(100), Some(100), 105, i32::MAX), None);
    assert_eq!(gap_assignment(Some(120), None, 105, 109), None, "a closed gap does not qualify work first witnessed after it ended");

    let deferred = Facts {
        from: 100,
        to: 125,
        spent: [((false, false, 9, 14, NONE), steps(&[(120, Some(100_000))]))].into(),
        lines: vec![
            LineFacts {
                leaf: (false, 7, 12, NONE),
                building: Some(7),
                new_from: Some(120),
                repair_from: Some(115),
                gaps: vec![(105, i32::MAX, SpendingGap::LinkedOutput)],
                before_first_read: None,
            },
            LineFacts {
                leaf: (false, 8, 13, NONE),
                building: Some(8),
                new_from: None,
                repair_from: None,
                gaps: vec![(118, i32::MAX, SpendingGap::InexactOutput)],
                before_first_read: None,
            },
            LineFacts {
                leaf: (false, 9, 14, NONE),
                building: Some(9),
                new_from: Some(120),
                repair_from: None,
                gaps: vec![(105, 109, SpendingGap::InexactOutput)],
                before_first_read: None,
            },
        ],
        ..Facts::default()
    };
    let deferred_index = index(&deferred);
    let at_105 = deferred_index.at(105);
    assert_eq!(at_105.breakdown.total.state, BreakdownState::Partial, "an output gap at 105 qualifies the total immediately even when child purpose waits");
    assert_eq!(at_105.through, 114, "validity ends before the child attribution appears at 115");
    let at_120 = deferred_index.at(120);
    let new_part = at_120.breakdown.parts.iter().find(|part| part.key == "new").expect("exact spent part for building 9");
    assert_eq!(new_part.children.len(), 1, "unknown-purpose gap qualifies total without fabricating a child");
    assert_eq!(new_part.children[0].key, "type:9");
    assert_eq!(new_part.children[0].state, BreakdownState::Known, "a closed gap at 105..109 does not qualify new work first witnessed at 120");

    let frames = (100..=125)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                id: 1,
                country: 9001,
                building_type: 7,
                target_state: 12,
                cost: 500_000,
                created_hour: 90,
                amount: if hour >= 120 { 2 } else { 0 },
                repair_levels: 1,
                ..Default::default()
            }]))
            .into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            let outputs = if hour >= 105 {
                vec![ConstructionLineOutputRow {
                    line: 1,
                    since_hour: 100,
                    linked: true,
                    exact: false,
                    repair_ticks: if hour >= 115 { 1 } else { 0 },
                    country: Recorded::Value(Some(9001)),
                    building_type: Recorded::Value(Some(7)),
                    target_state: Recorded::Value(Some(12)),
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
            frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 7,
                literal: b"alpha_mill".to_vec(),
            }]))
            .into();
            for table in [
                TableId::BuildingConstruction,
                TableId::RailwayConstruction,
                TableId::ConstructionLineOutput,
                TableId::LexicalTokens,
            ] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let loaded = Source::from_replay(
        SourceKey { source_id: "construction-new-work".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    let reply = ConstructionBreakdown::load(
        &loaded,
        &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 108 },
        &LoadContext { content: None },
        &cancel,
    )
    .unwrap();
    assert!(reply.breakdown.parts.iter().all(|part| part.key != "new"), "new work first seen at 120 must not receive the gap at 105");
    let repair = reply.breakdown.parts.iter().find(|part| part.key == "repair").expect("repair part carries the gap");
    assert_eq!(repair.state, BreakdownState::Partial);

    let signed_frames = (100..=102)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(Vec::new())).into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(vec![ConstructionLineOutputRow {
                line: 5, since_hour: 100, construction_raw: -25, exact: hour < 102,
                country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
            }]))
            .into();
            for table in [TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::ConstructionLineOutput] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let signed_replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, signed_frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let signed_source = Source::from_replay(
        SourceKey { source_id: "construction-signed-output".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(signed_replay),
        &cancel,
    )
    .unwrap();
    let signed_reply = ConstructionBreakdown::load(
        &signed_source,
        &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 102 },
        &LoadContext { content: None },
        &cancel,
    )
    .unwrap();
    let signed_new = signed_reply.breakdown.parts.iter().find(|part| part.key == "new").expect("negative output establishes new-building purpose");
    assert_eq!(signed_new.state, BreakdownState::Partial);
    assert_eq!(signed_new.children.iter().find(|child| child.key == "type:7").map(|child| child.state), Some(BreakdownState::Partial));
}

#[test]
fn construction_spellings_and_warnings_are_dated_to_their_charge_hours() {
    use super::{ConstructionBreakdown, ConstructionBreakdownParams};
    use dashboard_base::{
        catalogue::tests::folder_catalogue,
        products::{Country, Product},
        test_fixtures::{source, source_with_content},
    };
    use replay_data::rows::{BuildingConstructionRow, ConstructionLineOutputRow, LexicalTokenRow, Recorded, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::atomic::AtomicBool;

    let (_, dir) = folder_catalogue(&[
        ("common/buildings/00_test.txt", "buildings = { alpha_mill = { icon_frame = 1 } beta_mill = { icon_frame = 1 } }"),
        ("localisation/languages.yml", "\u{feff}l_english:\n l_english: \"English\"\n"),
        ("localisation/english/test_l_english.yml", "\u{feff}l_english:\n alpha_mill:0 \"Alpha Mill\"\n beta_mill:0 \"Beta Mill\"\n"),
    ]);
    let frames = (100..=135)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let mut lines = vec![
                BuildingConstructionRow { id: 1, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 90, amount: 1, ..Default::default() },
                BuildingConstructionRow { id: 2, country: 9001, building_type: 8, target_state: 13, cost: 500_000, created_hour: 90, amount: 1, ..Default::default() },
            ];
            if hour >= 130 {
                lines.push(BuildingConstructionRow { id: 3, country: 9001, building_type: 99, target_state: 14, cost: 500_000, created_hour: 130, amount: 1, ..Default::default() });
            }
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(lines)).into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            let mut outputs = Vec::new();
            if hour >= 105 {
                outputs.push(ConstructionLineOutputRow {
                    line: 1, since_hour: 100, construction_raw: 100_000, exact: true,
                    country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
                });
                outputs.push(ConstructionLineOutputRow {
                    line: 2, since_hour: 100, construction_raw: 200_000, exact: true,
                    country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(8)), target_state: Recorded::Value(Some(13)), ..Default::default()
                });
            }
            if hour >= 130 {
                outputs.push(ConstructionLineOutputRow {
                    line: 3, since_hour: 130, construction_raw: 100_000, exact: true,
                    country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(99)), target_state: Recorded::Value(Some(14)), ..Default::default()
                });
            }
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
            frame.tables[TableId::LexicalTokens.index()] = if hour < 120 {
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 7, literal: b"alpha_mill".to_vec() }])).into()
            } else if hour < 128 {
                TableView::Complete(TableRows::LexicalTokens(vec![
                    LexicalTokenRow { token: 7, literal: b"beta_mill".to_vec() },
                    LexicalTokenRow { token: 8, literal: b"alpha_mill".to_vec() },
                ]))
                .into()
            } else if hour < 130 {
                TableView::Unavailable { reason: "lexical unread mid-stretch".into() }.into()
            } else if hour <= 132 {
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 7, literal: b"beta_mill".to_vec() }])).into()
            } else {
                TableView::Unavailable { reason: "lexical unread at end".into() }.into()
            };
            for table in [TableId::CountryLinks, TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::ConstructionLineOutput] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            if hour < 128 || (130..=132).contains(&hour) {
                frame.metrics.table_ns[TableId::LexicalTokens.index()] = 1;
            }
            frame
        })
        .collect();
    let (source, load) = source_with_content(frames, &dir.0);
    let cancel = AtomicBool::new(false);
    let ask = |hour: i32| {
        ConstructionBreakdown::load(
            &source,
            &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour },
            &load,
            &cancel,
        )
        .unwrap()
    };
    fn child_name<'a>(reply: &'a super::ConstructionBreakdownReply, key: &str) -> Option<&'a str> {
        reply.breakdown.parts.iter().find(|part| part.key == "new").and_then(|part| part.children.iter().find(|child| child.key == key)).map(|child| child.name.as_str())
    }

    let before_spend = ask(100);
    assert!(before_spend.warnings.is_empty(), "naming warnings do not precede the first charged building: {:?}", before_spend.warnings);
    assert_eq!(before_spend.through, 104);

    let early = ask(105);
    assert_eq!(child_name(&early, "type:7"), Some("Alpha Mill"), "spelling at charge hour 105 is alpha_mill");
    assert_eq!(child_name(&early, "type:8"), Some("Building name unavailable"), "token 8 is not yet spelled at 105");
    assert_eq!(early.through, 119, "validity ends before token spellings change at 120");
    assert!(early.warnings.iter().any(|w| w.contains("lexical dictionary") && w.contains('8')), "{:?}", early.warnings);
    assert!(early.warnings.iter().all(|w| !w.contains("99")), "warning for building 99 charged at 130 must not appear at 105: {:?}", early.warnings);

    let mid = ask(125);
    assert_eq!(child_name(&mid, "type:7"), Some("Beta Mill"), "token 7 updated at 120");
    assert_eq!(child_name(&mid, "type:8"), Some("Alpha Mill"), "token 8 spelled at 120");
    assert_eq!(mid.through, 129, "validity ends before token 8 drops and building 99 is charged at 130");
    assert!(mid.warnings.iter().all(|w| !w.contains("lexical dictionary")), "both 7 and 8 are spelled by 125 and 99 is not charged yet: {:?}", mid.warnings);

    let dropped = ask(131);
    assert_eq!(child_name(&dropped, "type:7"), Some("Beta Mill"));
    assert_eq!(child_name(&dropped, "type:8"), Some("Building name unavailable"), "a complete dictionary omitting token 8 clears its spelling");
    assert_eq!(child_name(&dropped, "type:99"), Some("Building name unavailable"));
    assert!(dropped.warnings.iter().any(|w| w.contains("lexical dictionary") && w.contains('8') && w.contains("99")), "{:?}", dropped.warnings);

    let late = ask(135);
    assert_eq!(child_name(&late, "type:7"), Some("Beta Mill"), "earlier spellings survive an unavailable stretch end");
    assert_eq!(child_name(&late, "type:8"), Some("Building name unavailable"));
    assert_eq!(child_name(&late, "type:99"), Some("Building name unavailable"));
    assert!(late.warnings.iter().any(|w| w.contains("lexical dictionary") && w.contains("99")), "{:?}", late.warnings);
}

#[test]
fn owner_unknown_before_census_qualifies_both_eventual_owner_and_other_countries() {
    use super::{ConstructionBreakdown, ConstructionBreakdownParams};
    use dashboard_base::{
        Source, SourceKey,
        products::{Country, LoadContext, Product},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{BuildingConstructionRow, ConstructionLineOutputRow, Recorded, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let frames = (100..=115)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            if hour < 110 {
                frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(Vec::new())).into();
            }
            let buildings = if hour >= 110 {
                vec![BuildingConstructionRow {
                    id: 5, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 95, amount: 1, ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(buildings)).into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            frame.tables[TableId::ConstructionLineOutput.index()] = if (105..110).contains(&hour) {
                TableView::Unavailable { reason: "output unread before census".into() }.into()
            } else {
                TableView::Complete(TableRows::ConstructionLineOutput(vec![
                    ConstructionLineOutputRow {
                        line: 5,
                        since_hour: 100,
                        construction_raw: 100_000,
                        repair_ticks: 1,
                        exact: true,
                        country: Recorded::Value((hour >= 110).then_some(9001)),
                        building_type: Recorded::Value(Some(7)),
                        target_state: Recorded::Value(Some(12)),
                        ..Default::default()
                    },
                ]))
                .into()
            };
            for table in [TableId::CountryLinks, TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::ConstructionLineOutput] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let loaded = Source::from_replay(
        SourceKey { source_id: "construction-owner-before-census".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    for slot in [7, 8] {
        for hour in [100, 105] {
            let reply = ConstructionBreakdown::load(
                &loaded,
                &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(slot), hour },
                &LoadContext { content: None },
                &cancel,
            )
            .unwrap();
            assert_eq!(reply.unattributed, 1, "slot {slot} at {hour} must count the owner-unknown gap");
            assert_eq!(reply.before_first_read, 0, "slot {slot} at {hour} must not reveal eventual owner via before_first_read");
            assert!(reply.breakdown.parts.is_empty(), "slot {slot} at {hour} must not reveal eventual owner via child gap during unread output");
            assert_eq!(reply.breakdown.total.state, BreakdownState::Partial);
        }
        let at_110 = ConstructionBreakdown::load(
            &loaded,
            &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(slot), hour: 110 },
            &LoadContext { content: None },
            &cancel,
        )
        .unwrap();
        assert_eq!(at_110.unattributed, 1);
        assert_eq!(at_110.before_first_read, u32::from(slot == 7), "link recorded at 110 attributes before_first_read once ownership is known");
    }

    let recovered_frames = (100..=112)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let buildings = if hour >= 110 {
                vec![BuildingConstructionRow {
                    id: 5, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 95, amount: 1, ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(buildings)).into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(vec![
                ConstructionLineOutputRow {
                    line: 5,
                    since_hour: 100,
                    construction_raw: 100_000,
                    exact: true,
                    country: Recorded::Value((hour >= 105).then_some(9001)),
                    building_type: Recorded::Value(Some(7)),
                    target_state: Recorded::Value(Some(12)),
                    ..Default::default()
                },
            ]))
            .into();
            for table in [TableId::CountryLinks, TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::ConstructionLineOutput] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let recovered_replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, recovered_frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let recovered_source = Source::from_replay(
        SourceKey { source_id: "construction-owner-recovered-output".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(recovered_replay),
        &cancel,
    )
    .unwrap();
    let counts: Vec<u32> = [100, 105, 110]
        .into_iter()
        .map(|hour| {
            ConstructionBreakdown::load(
                &recovered_source,
                &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour },
                &LoadContext { content: None },
                &cancel,
            )
            .unwrap()
            .before_first_read
        })
        .collect();
    assert_eq!(counts, [0, 1, 1], "ownership recovered by output at 105 counts before_first_read immediately at 105");
}

#[test]
fn closed_gap_in_earlier_stretch_does_not_reopen_after_clipped_second_stretch() {
    use super::{ConstructionBreakdown, ConstructionBreakdownParams};
    use dashboard_base::{
        Source, SourceKey,
        products::{Country, LoadContext, Product},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_capture_spool::spool::Record;
    use replay_data::rows::{BuildingConstructionRow, ConstructionLineOutputRow, Recorded, TableId, TableRows};
    use replay_frame::{
        branches::{BranchReason, BranchStart, Parentage},
        frame::TableView,
    };
    use std::sync::{Arc, atomic::AtomicBool};

    let build_frame = |branch: u64, hour: i32, has_line: bool, amount: i32, exact: bool, spent: i64| {
        let mut frame = source(branch, hour, Some(1));
        let buildings = if has_line {
            vec![BuildingConstructionRow {
                id: 1, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 95, amount, ..Default::default()
            }]
        } else {
            Vec::new()
        };
        let outputs = if has_line && hour >= 105 {
            vec![ConstructionLineOutputRow {
                line: 1, since_hour: 100, construction_raw: spent, exact,
                country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
            }]
        } else {
            Vec::new()
        };
        frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(buildings)).into();
        frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
        frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
        frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
        for table in [TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::ConstructionLineOutput] {
            frame.metrics.table_ns[table.index()] = 1;
        }
        frame
    };

    let run1 = records(1, (100..=109).map(|hour| build_frame(1, hour, true, 0, false, 0)).collect());
    let mut run2 = records(2, (110..=130).map(|hour| build_frame(2, hour, true, i32::from(hour >= 120), true, if hour >= 120 { 100_000 } else { 0 })).collect());
    run2[1] = Record::BranchStart(BranchStart {
        id: 2, session: 2, loaded_hour: Some(109), first_hour: 110,
        parent: Parentage::Probable { branch: 1, hour: 109 }, reason: BranchReason::LoadSignal,
    });
    let mut run3 = records(3, (126..=128).map(|hour| build_frame(3, hour, false, 0, true, 0)).collect());
    run3[1] = Record::BranchStart(BranchStart {
        id: 3, session: 3, loaded_hour: Some(125), first_hour: 126,
        parent: Parentage::Probable { branch: 2, hour: 125 }, reason: BranchReason::LoadSignal,
    });
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[run1, run2, run3])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let branch_id = replay.timelines().expect("timelines").list.iter().find(|t| t.last_hour() == 128).expect("clipped branch").id;
    let cancel = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "construction-clipped-second-stretch".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    let at_122 = ConstructionBreakdown::load(
        &source,
        &ConstructionBreakdownParams { branch_id: branch_id.to_string(), country: Country::Slot(7), hour: 122 },
        &LoadContext { content: None },
        &cancel,
    )
    .unwrap();
    assert_eq!(at_122.breakdown.total.state, BreakdownState::Partial, "the closed gap from run 1 keeps the total partial");
    let new_part = at_122.breakdown.parts.iter().find(|part| part.key == "new").expect("new spending at 120");
    assert_eq!(new_part.children[0].state, BreakdownState::Known, "run 1's end at 109 must not reopen the closed gap after run 2 clipped at 125");
}

#[test]
fn gap_attribution_separates_witnessed_work_from_inferred_queue_state_and_building_changes() {
    use super::{ConstructionBreakdown, ConstructionBreakdownParams};
    use dashboard_base::{
        Source, SourceKey,
        products::{Country, LoadContext, Product},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{BuildingConstructionRow, BuildingRow, ConstructionLineOutputRow, Recorded, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let cancel = AtomicBool::new(false);
    let load_at = |id: &str, frames: Vec<replay_frame::frame::DecodedFrame>| -> (Arc<Replay>, Source) {
        let replay = Arc::new(
            Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
                .unwrap()
                .into_timelines(&Default::default())
                .unwrap(),
        );
        let src = Source::from_replay(
            SourceKey { source_id: id.into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::clone(&replay),
            &cancel,
        )
        .unwrap();
        (replay, src)
    };
    let ask = |src: &Source, hour: i32| {
        ConstructionBreakdown::load(
            src,
            &ConstructionBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour },
            &LoadContext { content: None },
            &cancel,
        )
        .unwrap()
    };
    let part_state = |reply: &super::ConstructionBreakdownReply, key: &str| {
        reply.breakdown.parts.iter().find(|part| part.key == key).map(|part| part.state)
    };
    let child_state = |reply: &super::ConstructionBreakdownReply, part_key: &str, child_key: &str| {
        reply
            .breakdown
            .parts
            .iter()
            .find(|part| part.key == part_key)
            .and_then(|part| part.children.iter().find(|child| child.key == child_key))
            .map(|child| child.state)
    };

    let unresolved_tick_frames = (100..=118)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                id: 1, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 95,
                amount: match hour { ..=114 => 1, 115 => 2, _ => 3 }, repair_levels: 0, ..Default::default()
            }]))
            .into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(Vec::new())).into();
            frame.tables[TableId::ProvinceBuildings.index()] = TableView::Complete(TableRows::ProvinceBuildings(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            let outputs = if hour >= 115 {
                vec![ConstructionLineOutputRow {
                    line: 1, since_hour: 100, exact: hour == 115, repair_ticks: 1,
                    country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
            for table in [
                TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::StateBuildings, TableId::ProvinceBuildings, TableId::ConstructionLineOutput,
            ] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let (_, unresolved_src) = load_at("gap-row4-unresolved-tick", unresolved_tick_frames);
    let unresolved_116 = ask(&unresolved_src, 116);
    assert_eq!(unresolved_116.breakdown.total.state, BreakdownState::Partial);
    assert!(unresolved_116.breakdown.parts.is_empty(), "an inexact gap after an uncompleted zero-repair-level tick must qualify only total, never guess New or Repair");

    let tick_gap_frames = (100..=118)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                id: 2, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 95,
                amount: match hour { ..=114 => 1, 115 => 2, _ => 3 }, repair_levels: 0, ..Default::default()
            }]))
            .into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(Vec::new())).into();
            frame.tables[TableId::ProvinceBuildings.index()] = TableView::Complete(TableRows::ProvinceBuildings(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            let outputs = if hour >= 115 {
                vec![ConstructionLineOutputRow {
                    line: 2, since_hour: 100, exact: false, repair_ticks: 1,
                    country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
            for table in [
                TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::StateBuildings, TableId::ProvinceBuildings, TableId::ConstructionLineOutput,
            ] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let (_, tick_gap_src) = load_at("gap-row4-tick-gap", tick_gap_frames);
    let tick_116 = ask(&tick_gap_src, 116);
    assert_eq!(part_state(&tick_116, "repair"), Some(BreakdownState::Partial));
    assert_eq!(part_state(&tick_116, "new"), None, "queue addition at 115/116 after an unresolved repair tick must not guess New");

    let possible_frames = (113..=116)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                id: 3, country: 9001, building_type: 7, target_state: 12, target_location: 12, cost: 500_000, created_hour: 95,
                amount: 2, repair_levels: 0, ..Default::default()
            }]))
            .into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
                location: 12, building_type: 7, country: 9001, level: if hour >= 115 { 3 } else { 2 }, healthy_level: if hour >= 115 { 3 } else { 2 }, active: true, ..Default::default()
            }]))
            .into();
            frame.tables[TableId::ProvinceBuildings.index()] = TableView::Complete(TableRows::ProvinceBuildings(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            let outputs = if hour >= 115 {
                vec![ConstructionLineOutputRow {
                    line: 3, since_hour: 113, exact: false, repair_ticks: 1,
                    country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
            for table in [
                TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::StateBuildings, TableId::ProvinceBuildings, TableId::ConstructionLineOutput,
            ] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let (possible_replay, possible_src) = load_at("gap-row5-possible", possible_frames);
    let poss_hist = replay_lanes::lanes::history(
        possible_replay.recording(), 0, TableId::ConstructionProjects, &[super::super::col::POSSIBLE, super::super::col::BUILT], replay_lanes::lanes::Keys::These(&[3]), 113, 116,
    )
    .unwrap();
    assert!(
        poss_hist.keys[0].1.iter().any(|step| step.hour == 115
            && matches!(step.cells.as_deref(), Some([replay_lanes::lanes::Cell::Number(1), replay_lanes::lanes::Cell::Number(0)]))),
        "fixture must record POSSIBLE=1 and BUILT=0 at hour 115"
    );
    let possible_115 = ask(&possible_src, 115);
    assert_eq!(part_state(&possible_115, "repair"), Some(BreakdownState::Partial), "POSSIBLE is not witnessed new work and must not suppress a repair tick");
    assert_eq!(part_state(&possible_115, "new"), None);

    for (case_id, initial_repair, continuous) in [
        ("gap-row6-1to0-after", true, false),
        ("gap-row6-1to0-continuous", true, true),
        ("gap-row6-0to0-after", false, false),
    ] {
        let frames = (118..=123)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                    id: 4, country: 9001, building_type: 7, target_state: 12, target_location: 12, cost: 500_000, created_hour: 95,
                    amount: 1, repair_levels: i32::from(initial_repair && hour < 120), ..Default::default()
                }]))
                .into();
                frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
                frame.tables[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
                    location: 12, building_type: 7, country: 9001, level: 2, healthy_level: if hour >= 120 { 2 } else { 1 }, active: true, ..Default::default()
                }]))
                .into();
                frame.tables[TableId::ProvinceBuildings.index()] = TableView::Complete(TableRows::ProvinceBuildings(Vec::new())).into();
                frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
                let outputs = if continuous || hour >= 121 {
                    vec![ConstructionLineOutputRow {
                        line: 4, since_hour: 118, exact: false,
                        country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
                    }]
                } else {
                    Vec::new()
                };
                frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(outputs)).into();
                for table in [
                    TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::StateBuildings, TableId::ProvinceBuildings, TableId::ConstructionLineOutput,
                ] {
                    frame.metrics.table_ns[table.index()] = 1;
                }
                frame
            })
            .collect();
        let (rep, src) = load_at(case_id, frames);
        let hist = replay_lanes::lanes::history(rep.recording(), 0, TableId::ConstructionProjects, &[super::super::col::REPAIRED], replay_lanes::lanes::Keys::These(&[4]), 118, 120).unwrap();
        assert!(
            hist.keys[0].1.iter().any(|step| step.hour == 120 && matches!(step.cells.as_deref(), Some([replay_lanes::lanes::Cell::Number(1)]))),
            "{case_id} must record REPAIRED=1 at 120"
        );
        if continuous {
            let at_120 = ask(&src, 120);
            assert_eq!(part_state(&at_120, "repair"), Some(BreakdownState::Partial), "{case_id} at 120");
            assert_eq!(part_state(&at_120, "new"), None, "{case_id} stays repair through 120");
            let at_121 = ask(&src, 121);
            assert_eq!(part_state(&at_121, "repair"), Some(BreakdownState::Partial), "{case_id} keeps earlier repair partial at 121");
            assert_eq!(part_state(&at_121, "new"), Some(BreakdownState::Partial), "{case_id} switches to new at 121");
        } else {
            let at_122 = ask(&src, 122);
            assert_eq!(part_state(&at_122, "repair"), None, "{case_id} gap starting at 121 must not mark repair");
            assert_eq!(part_state(&at_122, "new"), Some(BreakdownState::Partial), "{case_id} gap starting at 121 belongs to new");
        }
    }

    let both_frames = (118..=121)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            frame.tables[TableId::BuildingConstruction.index()] = if hour == 119 {
                TableView::Unavailable { reason: "queue skipped at 119".into() }.into()
            } else {
                TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow {
                    id: 5, country: 9001, building_type: 7, target_state: 12, target_location: 12, cost: 500_000, created_hour: 95,
                    amount: if hour >= 120 { 1 } else { 2 }, repair_levels: i32::from(hour < 120), ..Default::default()
                }]))
                .into()
            };
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
                location: 12, building_type: 7, country: 9001,
                level: if hour >= 120 { 3 } else { 2 },
                healthy_level: match hour { 118 => 1, 119 => 2, _ => 3 },
                active: true, ..Default::default()
            }]))
            .into();
            frame.tables[TableId::ProvinceBuildings.index()] = TableView::Complete(TableRows::ProvinceBuildings(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(vec![ConstructionLineOutputRow {
                line: 5, since_hour: 118, exact: false,
                country: Recorded::Value(Some(9001)), building_type: Recorded::Value(Some(7)), target_state: Recorded::Value(Some(12)), ..Default::default()
            }]))
            .into();
            for table in [TableId::RailwayConstruction, TableId::StateBuildings, TableId::ProvinceBuildings, TableId::ConstructionLineOutput] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            if hour != 119 {
                frame.metrics.table_ns[TableId::BuildingConstruction.index()] = 1;
            }
            frame
        })
        .collect();
    let (both_replay, both_src) = load_at("gap-row7-both", both_frames);
    let both_hist = replay_lanes::lanes::history(
        both_replay.recording(), 0, TableId::ConstructionProjects, &[super::super::col::BUILT, super::super::col::REPAIRED], replay_lanes::lanes::Keys::These(&[5]), 118, 121,
    )
    .unwrap();
    assert!(
        both_hist.keys[0].1.iter().any(|step| step.hour == 120
            && matches!(step.cells.as_deref(), Some([replay_lanes::lanes::Cell::Number(1), replay_lanes::lanes::Cell::Number(1)]))),
        "fixture must record both BUILT=1 and REPAIRED=1 on the same census at 120"
    );
    let both_120 = ask(&both_src, 120);
    assert_eq!(part_state(&both_120, "repair"), Some(BreakdownState::Partial));
    assert_eq!(part_state(&both_120, "new"), Some(BreakdownState::Partial), "witnessing both REPAIRED and BUILT at 120 marks both Repair and New partial");

    let building_change_frames = (105..=112)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let buildings = if hour >= 110 {
                vec![BuildingConstructionRow {
                    id: 6, country: 9001, building_type: 7, target_state: 12, cost: 500_000, created_hour: 95, amount: 0, repair_levels: 1, ..Default::default()
                }]
            } else {
                Vec::new()
            };
            frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(buildings)).into();
            frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
            frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new())).into();
            frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(TableRows::ConstructionLineOutput(vec![ConstructionLineOutputRow {
                line: 6, since_hour: 105, exact: false, repair_ticks: 1,
                country: Recorded::Value(Some(9001)), building_type: Recorded::Value(None), target_state: Recorded::Value(Some(12)), ..Default::default()
            }]))
            .into();
            for table in [TableId::BuildingConstruction, TableId::RailwayConstruction, TableId::ConstructionLineOutput] {
                frame.metrics.table_ns[table.index()] = 1;
            }
            frame
        })
        .collect();
    let (_, bchange_src) = load_at("gap-row9-building-change", building_change_frames);
    let at_108 = ask(&bchange_src, 108);
    assert_eq!(child_state(&at_108, "repair", "type:none"), Some(BreakdownState::Partial));
    assert_eq!(child_state(&at_108, "repair", "type:7"), None, "building type first seen at 110 must not be backdated to 108");
    let at_110 = ask(&bchange_src, 110);
    assert_eq!(child_state(&at_110, "repair", "type:none"), Some(BreakdownState::Partial));
    assert_eq!(child_state(&at_110, "repair", "type:7"), Some(BreakdownState::Partial), "gap splits at 110 when building type 7 is recorded");
}
