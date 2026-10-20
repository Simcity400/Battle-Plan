use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn history(division: u64, list: u8, ordinal: u32, row: DivisionHistoryRow) -> SharedDivisionHistoryRow {
    Arc::new(DivisionHistoryRow { division, list, ordinal, ..row })
}

/// GER (raw 11) fields divisions 1 and 2, one intelligence copy (3) and one
/// division of an unlinked country (4).
fn frame() -> DecodedFrame {
    let mut f = dashboard_base::test_fixtures::source(1, 60_000_001, None);
    *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow { id: 1, roster_owner: 11, template: 5, ..Default::default() },
        DivisionRow { id: 2, roster_owner: 11, template: 5, ..Default::default() },
        DivisionRow { id: 3, roster_owner: 11, template: 5, reveal_skip: 1, ..Default::default() },
        DivisionRow { id: 4, roster_owner: 77, template: 6, ..Default::default() },
    ]));
    *f.tables[TableId::DivisionStatus.index()] = TableView::Complete(TableRows::DivisionStatus(vec![
        DivisionStatusRow { division: 1, career_counter: Recorded::Value(Some(1_234)), experience: Recorded::Value(Some(600_000)), ..Default::default() },
        DivisionStatusRow { division: 2, career_counter: Recorded::Value(None), ..Default::default() },
    ]));
    *f.tables[TableId::DivisionLandInputs.index()] = TableView::Complete(TableRows::DivisionLandInputs(vec![
        DivisionLandInputsRow { division: 1, manpower: vec![DivisionManpowerContributor { amount: 10, ..Default::default() }], ..Default::default() },
    ]));
    // Division 1: a summary of 3 medals (not the sum of its two award entries),
    // a disabled award slot, a career entry. Division 2's history was refused.
    *f.tables[TableId::DivisionHistory.index()] = TableView::Complete(TableRows::DivisionHistory(vec![
        history(1, DivisionHistoryRow::SUMMARY, 0, DivisionHistoryRow { available: 1, amount: 3, ..Default::default() }),
        history(1, DivisionHistoryRow::CAREER, 0, DivisionHistoryRow { kind: 1, hour: 59_000_000, provinces: vec![7, 9], ..Default::default() }),
        history(1, DivisionHistoryRow::AWARDS, 0, DivisionHistoryRow { medal_enabled: 1, medal_token: Some(31), medal_count: 1, ..Default::default() }),
        history(1, DivisionHistoryRow::AWARDS, 1, DivisionHistoryRow { medal_enabled: 0, ..Default::default() }),
        history(1, DivisionHistoryRow::AWARDS, 2, DivisionHistoryRow { medal_enabled: 1, medal_token: Some(31), medal_count: 2, ..Default::default() }),
        history(2, DivisionHistoryRow::SUMMARY, 0, DivisionHistoryRow { available: 0, reason: b"history read refused".to_vec(), ..Default::default() }),
    ]));
    *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        LexicalTokenRow { token: 31, literal: b"mod_medal".to_vec() },
    ]));
    f
}

fn naming() -> Naming<'static> {
    Naming { cat: None, names: None, icon: &|_| Err("no artwork".into()) }
}

#[test]
fn rankings_use_the_recorded_medal_total_and_leave_out_intelligence_copies() {
    let mut refusals = Refusals::default();
    let r = careers(&frame(), None, &naming(), &mut refusals, &AtomicBool::new(false)).unwrap();
    assert_eq!(r.divisions.iter().map(|d| d.division.as_str()).collect::<Vec<_>>(), ["1", "2", "4"]);
    let first = &r.divisions[0].facts;
    assert_eq!(first.medals, Some(3), "the summary's total, not the count of award entries");
    assert_eq!(first.awards.len(), 1);
    assert_eq!((first.awards[0].count, r.medals[first.awards[0].medal].key.as_str()), (2, "mod_medal"), "repeated awards count, disabled slots do not");
    assert_eq!(first.kills, Some(1_234));
    assert_eq!(first.experience_raw.as_deref(), Some("60000"), "experience normalised by the division's manpower");
    let refused = &r.divisions[1].facts;
    assert!(refused.medals.is_none() && refused.history_missing.as_deref() == Some("history read refused"), "a refused history is unknown, never zero medals");
    assert!(refused.kills.is_none());
    assert!(r.divisions[2].facts.history_missing.is_some(), "a division without a history row");
    assert_eq!(r.templates.len(), 2);
    let only = careers(&frame(), Some(&["GER".to_string()]), &naming(), &mut refusals, &AtomicBool::new(false)).unwrap();
    assert_eq!(only.divisions.len(), 2, "a country selection keeps its own divisions");
}

#[test]
fn a_record_keeps_native_entry_order_and_names_what_it_cannot_describe() {
    let mut refusals = Refusals::default();
    let places = |provinces: &[u32]| provinces.iter().map(|p| Ok(format!("Province {p}"))).collect();
    let r = record(&frame(), 1, &naming(), &places, &mut refusals).unwrap();
    assert!(!r.absent);
    assert_eq!(r.tag.as_deref(), Some("GER"));
    assert_eq!(r.entries.len(), 3, "the disabled award slot is not an entry");
    assert!(!r.entries[0].award && r.entries[1].award && r.entries[2].award);
    assert_eq!(r.entries[0].provinces, [7, 9]);
    assert_eq!(r.entries[0].hour, 59_000_000);
    assert!(r.entries.iter().all(|entry| entry.description.is_none() && entry.description_missing.is_some()), "descriptions need the content stack");
    assert_eq!(r.entries[1].medal, r.entries[2].medal);
    assert!(record(&frame(), 99, &naming(), &places, &mut refusals).unwrap().absent);
}

#[test]
fn five_thousand_divisions_rank_without_a_cap() {
    let mut f = frame();
    let divisions: Vec<_> = (10..5_010u64).map(|id| DivisionRow { id, roster_owner: 11, template: 5, ..Default::default() }).collect();
    let history: Vec<_> = (10..5_010u64)
        .map(|id| history(id, DivisionHistoryRow::SUMMARY, 0, DivisionHistoryRow { available: 1, amount: (id % 7) as i32, ..Default::default() }))
        .collect();
    *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
    *f.tables[TableId::DivisionHistory.index()] = TableView::Complete(TableRows::DivisionHistory(history));
    let mut refusals = Refusals::default();
    let r = careers(&f, None, &naming(), &mut refusals, &AtomicBool::new(false)).unwrap();
    assert_eq!(r.divisions.len(), 5_000);
    assert_eq!(r.divisions.iter().filter_map(|d| d.facts.medals).map(i64::from).sum::<i64>(), (10..5_010i64).map(|id| id % 7).sum::<i64>());
}
