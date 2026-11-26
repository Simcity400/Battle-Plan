use super::*;
use replay_data::rows::Recorded;

fn month(group: u32, kind: u8, date: i32) -> CombatLogMonthRow {
    CombatLogMonthRow { country: 9, group, kind, date, ..CombatLogMonthRow::default() }
}

fn line(group: u32, kind: u8, date: i32, variant: u32, amount: i64) -> CombatLogEquipmentRow {
    CombatLogEquipmentRow { country: 9, group, kind, month: 0, variant, date, amount_raw: amount }
}

fn dropped_line(group: u32, kind: u8, date: i32, variant: u32, amount: i64) -> CombatLogDroppedRow {
    CombatLogDroppedRow { country: 9, group, kind, month: 0, variant, date: Recorded::Value(date), amount: Recorded::Value(amount) }
}

#[test]
fn a_month_reads_its_listed_rows_then_the_rows_the_game_dropped_then_says_why_it_is_unknown() {
    let listed = [line(1, 0, 300, 7, 500), line(1, 0, 200, 7, 900)];
    let kept_rows = [dropped_line(1, 0, 100, 7, 400), dropped_line(1, 0, 101, 8, 1)];
    let kept: Vec<&CombatLogDroppedRow> = kept_rows.iter().collect();
    let dates = [50, 100, 200, 300];
    assert_eq!(witness(&month(1, 0, 300), Some(400), &listed, &kept, &dates), Witness::Listed(vec![&listed[0]]));
    assert_eq!(witness(&month(1, 0, 100), Some(400), &listed, &kept, &dates), Witness::Dropped(vec![&kept_rows[0]]), "a dropped row of another entry of the month does not witness it");
    assert_eq!(witness(&month(1, 0, 50), Some(400), &listed, &kept, &dates), Witness::Unknown(UNWITNESSED), "an older month nothing kept is unknown, never empty");
    assert_eq!(witness(&month(1, 0, 450), Some(400), &listed, &kept, &[450]), Witness::Unknown(AFTER_READ));
    assert_eq!(witness(&month(1, 0, 300), None, &[], &kept, &dates), Witness::Unknown(NO_READ_YET));
    assert_eq!(witness(&month(2, 0, 300), Some(400), &listed, &kept, &[200, 300]), Witness::Empty, "a month among its array's last two that the read listed nothing of has none");
}

#[test]
fn recording_the_retained_months_keeps_their_values_without_growing_the_replay() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::{Replay, SeekResult};
    use replay_data::rows::TableRows;

    let dates = [1, 2, 3, 4].map(|month| month_hours(month).0);
    let first = dates[3] + 1;
    let record = |keep_older| {
        let frames = (0..3).map(|step| {
            let mut frame = source(1, first + step, None);
            let rows = dates.iter().enumerate()
                .filter(|(i, _)| keep_older || step != 1 || *i >= 2)
                .map(|(i, &date)| CombatLogEquipmentRow {
                    month: month_bucket(date) as u16,
                    ..line(1, 0, date, 7, 125 + i as i64)
                }).collect();
            frame.tables[TableId::CombatLogEquipment.index()] = TableView::Complete(TableRows::CombatLogEquipment(rows)).into();
            frame
        }).collect();
        Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap()
    };
    let previous = record(false);
    let retained = record(true);
    for (replay, count) in [(&previous, 2), (&retained, 4)] {
        let SeekResult::Frame(frame) = replay.seek_selected(1, first + 1, Some(&[TableId::CombatLogEquipment])).unwrap() else {
            panic!("missing ordinary hour");
        };
        let rows = frame.complete::<tables::CombatLogEquipment>().unwrap();
        assert_eq!(rows.len(), count);
        match witness(&month(1, 0, dates[0]), Some(frame.hour), rows, &[], &dates) {
            Witness::Listed(older) if count == 4 => assert_eq!(older[0].amount_raw, 125),
            Witness::Unknown(UNWITNESSED) if count == 2 => (),
            other => panic!("unexpected witness: {other:?}"),
        }
    }
    let sizes = (previous.bytes().len(), retained.bytes().len());
    eprintln!("three-hour equipment fixture: {} bytes with window replacements, {} bytes with retained months", sizes.0, sizes.1);
    assert!(sizes.1 <= sizes.0, "{sizes:?}");
}

#[test]
fn a_months_witness_reads_its_own_arrays_run_of_the_hours_rows() {
    let mut rows = vec![line(1, 0, 300, 7, 500), line(1, 1, 300, 7, 2), line(2, 0, 300, 7, 3), line(1, 0, 200, 8, 900)];
    rows.sort_by_key(|row| (row.country, row.group, row.kind, row.month, row.variant));
    let other = CombatLogEquipmentRow { country: 10, ..line(1, 0, 300, 7, 4) };
    rows.push(other);
    let run = array_rows(&rows, (9, 1, 0));
    assert_eq!(run.iter().map(|row| (row.variant, row.amount_raw)).collect::<Vec<_>>(), [(7, 500), (8, 900)]);
    assert!(array_rows(&rows, (9, 3, 0)).is_empty());
    assert_eq!(witness(&month(1, 0, 300), Some(400), run, &[], &[200, 300]), Witness::Listed(vec![&run[0]]));
    assert_eq!(witness(&month(1, 0, 300), Some(400), array_rows(&rows, (9, 1, 0)), &[], &[200, 300]), witness(&month(1, 0, 300), Some(400), &rows, &[], &[200, 300]), "the array's run witnesses a month as all the hour's rows do");
}

#[test]
fn a_view_is_read_from_the_request_as_manpower_equipment_ledgers_or_one_month() {
    use super::{ArmyLossesParams, LossView};
    let equipment = LossView::Equipment { ledgers: vec![1, 2, 3] };
    let month = LossView::Month { month: 5, ledgers: None };
    let month_eq = LossView::Month { month: 5, ledgers: Some(vec![1, 2, 3]) };
    let asked = |json: serde_json::Value| serde_json::from_value::<ArmyLossesParams>(json).unwrap().view;
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER" })), LossView::Manpower);
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER", "view": { "kind": "equipment", "ledgers": [1, 2, 3] } })), equipment);
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER", "view": { "kind": "month", "month": 5 } })), month);
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER", "view": { "kind": "month", "month": 5, "ledgers": [1, 2, 3] } })), month_eq);
}

#[test]
fn month_detail_sums_selected_ledgers_and_reports_table_unavailability_in_rust() {
    let cells = dashboard_follow::live::Cells::default();
    let mut ledgers = BTreeMap::new();
    ledgers.insert(1, MonthLedgerAcc { kept: true, missing: None, total: 100_000, cells: Vec::new(), designs: Vec::new() });
    ledgers.insert(2, MonthLedgerAcc { kept: true, missing: None, total: 250_000, cells: Vec::new(), designs: Vec::new() });
    ledgers.insert(3, MonthLedgerAcc { kept: true, missing: Some(NO_READ_YET.into()), total: 0, cells: Vec::new(), designs: Vec::new() });
    let (sum_ok, _, miss_ok) = summarize_selected_ledgers("army1", Some(&[1, 2]), None, &ledgers, &cells);
    assert_eq!(sum_ok.as_deref(), Some("350000"));
    assert_eq!(miss_ok, None);
    let (sum_bad, _, miss_bad) = summarize_selected_ledgers("army1", Some(&[1, 2, 3]), None, &ledgers, &cells);
    assert_eq!(sum_bad, None);
    assert_eq!(miss_bad.as_deref(), Some(NO_READ_YET));
    let avail = ArmyLossesAvailability { own_equipment: Some("incomplete read".into()), ..ArmyLossesAvailability::default() };
    assert_eq!(month_detail_missing(&avail, Some(&[0])).as_deref(), Some("incomplete read"));
    assert_eq!(month_detail_missing(&avail, Some(&[ENEMY_LEDGER])), None);
    assert_eq!(month_detail_missing(&avail, None), None);
}

#[test]
fn rollup_ranks_parts_folds_other_preserves_recorded_zero_and_marks_unknown_months() {
    let cells = dashboard_follow::live::Cells::default();
    let mut builder = RollupBuilder::new(EQUIPMENT_SCALE, false, None);
    let m1 = 1941 * 12 + 5;
    let m2 = 1941 * 12 + 6;
    let m3 = 1941 * 12 + 7;
    builder.add(m1, "a".into(), "Infantry".into(), 500_000, None);
    builder.add(m1, "b".into(), "Artillery".into(), 300_000, None);
    builder.add(m1, "c".into(), "Trucks".into(), 200_000, None);
    builder.add(m1, "d".into(), "Tanks".into(), 100_000, None);
    builder.mark_recorded(m2);
    builder.add(m3, "a".into(), "Infantry".into(), 400_000, None);
    builder.mark_unknown(m3, NO_READ_YET.into());
    let rollup = builder.finish(3, &cells);
    assert_eq!(rollup.parts.len(), 3);
    assert_eq!(rollup.parts[0].key, "a");
    assert_eq!(rollup.parts[1].key, "b");
    assert_eq!(rollup.parts[2].key, OTHER);
    assert_eq!(rollup.parts[2].label, "Other (2)");
    assert_eq!(rollup.parts[2].other_count, 2);
    assert_eq!(rollup.months.len(), 3);
    assert_eq!(rollup.months[0].missing, None);
    assert_eq!(rollup.months[0].values, ["500000", "300000", "300000"]);
    assert_eq!(rollup.months[0].total, "1100000");
    assert_eq!(rollup.months[1].missing, None);
    assert_eq!(rollup.months[1].total, "0");
    assert_eq!(rollup.months[1].values, ["0", "0", "0"]);
    assert_eq!(rollup.months[2].missing.as_deref(), Some(NO_READ_YET));
    assert!(rollup.months[2].values.is_empty());
}

#[test]
fn army_losses_advance_keeps_cached_rollup_on_other_country_and_live_value_edits() {
    use dashboard_base::products::at_hour::{Edits, field_mask};
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    use replay_data::rows::{CombatLogManpowerRow, CountryLinkRow, TableRows};
    use replay_frame::frame::{Metrics, not_selected};

    let holders = Holders::new(
        BTreeMap::from([(
            "GER".to_owned(),
            vec![HolderRun { first: 0, last: 200, holder: Holder::Found { slot: 1, id: 1 }, starts_segment: true }],
        )]),
        Vec::new(),
    );
    let ger_row = CombatLogManpowerRow { country: 1, group: 10, date: 100, losses_0: 500, losses_1: 10, losses_2: 0, ..CombatLogManpowerRow::default() };
    let other_row = CombatLogManpowerRow { country: 2, group: 20, date: 100, losses_0: 900, losses_1: 0, losses_2: 0, ..CombatLogManpowerRow::default() };
    let ger_key = ger_row.key();
    let other_key = other_row.key();
    let mut mp_rows = vec![ger_row, other_row];
    mp_rows.sort_by_key(|r| r.key());
    let links = vec![
        CountryLinkRow { id: 1, canonical_slot: 1 },
        CountryLinkRow { id: 2, canonical_slot: 2 },
    ];
    let mut tables = std::array::from_fn(|_| not_selected());
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links)).into();
    tables[TableId::CombatLogManpower.index()] = TableView::Complete(TableRows::CombatLogManpower(mp_rows)).into();
    let frame = DecodedFrame { branch: 1, hour: 100, tables, metrics: Metrics::default() };

    let col_g = dashboard_declare::field(TableId::CombatLogManpower, "losses_0").unwrap();
    let cells = dashboard_follow::live::Cells::default();
    cells.value("r:1", "p0", Cell { table: TableId::CombatLogManpower, column: col_g, key: ger_key }, 1.0);
    let live = cells.finish();

    let params = ArmyLossesParams {
        branch_id: "1".into(),
        country: Country::Tag("GER".into()),
        view: LossView::Manpower,
        stack: Some(LossStack::Channel),
        limit: Some(8),
    };
    let mask = field_mask(TableId::CombatLogManpower, &["losses_0"]);
    let other_edit = Edits {
        tables: BTreeMap::from([(TableId::CombatLogManpower, BTreeMap::from([(other_key, mask)]))]),
        ..Edits::default()
    };
    assert!(advance_keeps(&frame, &params, &holders, 100, &other_edit, &live, Some(3)).unwrap(), "other country edits never rebuild GER rollup");

    let live_edit = Edits {
        tables: BTreeMap::from([(TableId::CombatLogManpower, BTreeMap::from([(ger_key, mask)]))]),
        ..Edits::default()
    };
    assert!(advance_keeps(&frame, &params, &holders, 100, &live_edit, &live, Some(3)).unwrap(), "in-place live channel edit keeps cached rollup");

    let structural_edit = Edits {
        tables: BTreeMap::from([(TableId::CombatLogGroups, BTreeMap::from([(1u128, 1u64)]))]),
        ..Edits::default()
    };
    assert!(!advance_keeps(&frame, &params, &holders, 100, &structural_edit, &live, Some(3)).unwrap(), "structural table edits rebuild");
}
