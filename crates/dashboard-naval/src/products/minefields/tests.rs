use super::*;

fn amount(country: u32, region: u32, amount: i64) -> NavalRegionalMineRow {
    NavalRegionalMineRow { country, kind: 1, region, amount }
}

fn witness(country: u32, length: u32) -> NavalRegionalMineRow {
    NavalRegionalMineRow { country, kind: 0, region: length, amount: 0 }
}

fn rows() -> Vec<NavalRegionalMineRow> {
    let mut rows = vec![witness(1, 10), amount(1, 3, 500_000), amount(2, 3, 900_000), amount(2, 12, 100_000), witness(3, 20)];
    rows.sort_by_key(|row| (row.country, row.kind, row.region));
    rows
}

fn name(raw: u32) -> Option<&'static str> {
    match raw {
        1 => Some("GER"),
        2 => Some("ENG"),
        _ => None,
    }
}

#[test]
fn a_witness_proves_regions_below_it_empty_and_everything_else_stays_unknown() {
    let sea = BTreeSet::from([3, 5, 12, 15]);
    let fold = fold(&rows(), name, |_| true, false, &sea);
    let three = &fold.mined[0];
    assert_eq!(three.region, 3);
    assert_eq!(three.owners.iter().map(|owner| owner.tag.as_str()).collect::<Vec<_>>(), ["ENG", "GER"], "largest first");
    let key = |raw: u32, region: u32| u128::from(raw) << 33 | 1 << 32 | u128::from(region);
    assert_eq!(fold.cells, [(3, "ENG".to_owned(), key(2, 3)), (3, "GER".to_owned(), key(1, 3)), (12, "ENG".to_owned(), key(2, 12))]);
    assert_eq!(three.unknown_owners, 1, "only the owner no tag names");
    let twelve = &fold.mined[1];
    assert_eq!(twelve.unknown_owners, 2, "past GER's witness, and the unnamed owner");
    assert_eq!(fold.unknown, [UnknownRegion { region: 5, unknown_owners: 2 }, UnknownRegion { region: 15, unknown_owners: 3 }]);
}

#[test]
fn owners_not_asked_are_not_shown_and_lost_rows_leave_every_region_unknown() {
    let sea = BTreeSet::from([3]);
    let fold = fold(&rows(), name, |tag| tag == "GER", true, &sea);
    assert_eq!(fold.mined[0].owners.len(), 1);
    assert_eq!(fold.mined[0].unknown_owners, 2, "the unnamed owner and the rows the hour lost");
    let empty = super::fold(&[witness(1, 10)], name, |_| true, false, &BTreeSet::from([3]));
    assert_eq!(empty, Fold::default(), "a complete witness with no rows is no mines and nothing unknown");
}

#[test]
fn history_is_the_amount_row_then_zero_under_a_live_witness_and_unknown_elsewhere() {
    let amount: lanes::Runs = vec![(100, 109, None), (110, 119, Some(500_000)), (120, 140, None)];
    let witness: lanes::Runs = vec![(100, 129, Some(0)), (130, 140, None)];
    let runs = combine(&amount, &[(10, &witness)], 3, 100, 140);
    assert_eq!(runs, [(100, 109, Some(0)), (110, 119, Some(500_000)), (120, 129, Some(0)), (130, 140, None)]);
    let past = combine(&amount, &[(3, &witness)], 3, 100, 140);
    assert_eq!(past[0], (100, 109, None), "a witness not reaching the region proves nothing there");
}

#[test]
fn a_history_of_every_owner_of_a_region_is_asked_whatever_their_count() {
    let tags: Vec<Country> = (0..1_500).map(|at| format!("{}{}{}", (b'A' + (at / 676) as u8) as char, (b'A' + (at / 26 % 26) as u8) as char, (b'A' + (at % 26) as u8) as char).into()).collect();
    let params = MinefieldHistoryParams { branch_id: "1".into(), region: 7, countries: tags, first_hour: 0, last_hour: 100 };
    assert_eq!(<MinefieldHistory as Product>::validate(&params), Ok(()));
}

#[test]
fn truncated_minefields_keep_witnessed_owners_live_and_refuse_region_total_with_truncation_reason() {
    use dashboard_follow::live::{CellValue, Cells as LiveCells, GroupState};
    use replay_data::rows::TableRows;
    use replay_frame::frame::Metrics;

    let row_trunc = "Naval regional mines read only in part at this hour (1 rows not read)";
    let table_trunc = "Naval regional mines read only in part at this hour (1 rows not read), and which rows were read is not held at the head";

    let sea = BTreeSet::from([3]);
    let fold10 = fold(&[amount(1, 3, 40), amount(2, 3, 60)], name, |_| true, false, &sea);
    let cells10 = LiveCells::default();
    register_live(&fold10, &cells10).unwrap();
    let live10 = cells10.finish();

    let mut frame11 = DecodedFrame { branch: 1, hour: 11, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Metrics::default() };
    frame11.tables[TableId::NavalRegionalMines.index()] = TableView::Truncated { rows: TableRows::NavalRegionalMines(vec![amount(1, 3, 40)]), missing: 1 }.into();

    let vals11 = live10.values(&frame11);
    assert_eq!(vals11.len(), 2, "owner cells in Row group and region total in Table group");
    assert_eq!(
        vals11[0],
        vec![CellValue::Number(40), CellValue::Missing(row_trunc.into())],
        "Row group keeps witnessed GER live and refuses unwitnessed ENG"
    );
    assert_eq!(
        vals11[1],
        vec![CellValue::Missing(table_trunc.into()), CellValue::Missing(table_trunc.into())],
        "Table group refuses region total operands with main's truncation reason"
    );

    let fold11 = fold(&[amount(1, 3, 40)], name, |_| true, true, &sea);
    assert_eq!(fold11.mined.len(), 1);
    assert_eq!(fold11.mined[0].owners, vec![MineOwner { tag: "GER".into() }]);
    let cells11 = LiveCells::default();
    register_live(&fold11, &cells11).unwrap();
    let live11 = cells11.finish();
    let live11_vals = live11.values(&frame11);
    assert_eq!(live11_vals[0], vec![CellValue::Number(40)], "cold truncated hour keeps witnessed owner cell live");
    assert_eq!(live11_vals[1], vec![CellValue::Missing(table_trunc.into())], "cold truncated hour refuses region total with truncation reason");
    let states11: Vec<GroupState> = live11_vals.into_iter().map(GroupState::Values).collect();
    let json11 = live11.json(&states11);
    let kinds: Vec<&str> = json11["kinds"].as_array().unwrap().iter().map(|k| k["id"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"GER"), "witnessed owner cell remains registered");
    assert!(kinds.contains(&"total"), "region total remains registered under Table scope so UI shows the truncation reason");
}
