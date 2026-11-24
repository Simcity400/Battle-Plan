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
fn a_region_total_and_its_rank_need_every_owner_there_known() {
    let sea = BTreeSet::from([3, 4]);
    let rows = vec![witness(1, 10), amount(1, 3, 100), amount(1, 4, 900), witness(2, 4)];
    let fold = fold(&rows, name, |_| true, false, &sea);
    assert_eq!(fold.mined.iter().map(|region| (region.region, region.unknown_owners)).collect::<Vec<_>>(), [(3, 0), (4, 1)],
        "a proven total of 100 ranks before a partial sum of 900 beside ENG, whose read does not reach region 4");
    assert_eq!(fold.proven, BTreeSet::from([3]));
    let cells = dashboard_follow::live::Cells::default();
    register_live(&fold, &cells).unwrap();
    let summed: Vec<u128> = cells.finish().groups.into_iter().filter(|group| group.scope == dashboard_query::coverage::CoverageScope::Table).flat_map(|group| group.keys).collect();
    assert_eq!(summed, [u128::from(1u32) << 33 | 1 << 32 | 3], "only the proven region has a total, so no share is drawn beside an unknown owner");
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
    assert_eq!(fold.mined[0].unknown_owners, 1, "the unnamed owner; how many owners the lost rows hid is not known, so never counted");
    assert_eq!(fold.mined[0].missing.as_deref(), Some(UNREAD), "the lost rows are named as such instead");
    assert!(fold.proven.is_empty(), "a region with an unknown owner has no total");
    let empty = super::fold(&[witness(1, 10)], name, |_| true, false, &BTreeSet::from([3]));
    assert!(empty.mined.is_empty() && empty.unknown.is_empty() && empty.cells.is_empty(), "a complete witness with no rows is no mines and nothing unknown");
}

#[test]
fn truncated_minefields_keep_witnessed_owners_live_and_refuse_region_total_with_truncation_reason() {
    use dashboard_follow::live::{CellValue, Cells as LiveCells, GroupState};
    use dashboard_query::coverage::CoverageScope;
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
    let row_group10 = live10.groups.iter().position(|g| g.scope == CoverageScope::Row).expect("Row group");
    let table_group10 = live10.groups.iter().position(|g| g.scope == CoverageScope::Table).expect("Table group");
    assert_eq!(
        vals11[row_group10],
        vec![CellValue::Number(40), CellValue::Missing(row_trunc.into())],
        "Row group keeps witnessed GER live and refuses unwitnessed ENG"
    );
    assert_eq!(
        vals11[table_group10],
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
    let row_group11 = live11.groups.iter().position(|g| g.scope == CoverageScope::Row).expect("Row group");
    let table_group11 = live11.groups.iter().position(|g| g.scope == CoverageScope::Table).expect("Table group");
    assert_eq!(live11_vals[row_group11], vec![CellValue::Number(40)], "cold truncated hour keeps witnessed owner cell live");
    assert_eq!(live11_vals[table_group11], vec![CellValue::Missing(table_trunc.into())], "cold truncated hour refuses region total with truncation reason");
    let states11: Vec<GroupState> = live11_vals.into_iter().map(GroupState::Values).collect();
    let json11 = live11.json(&states11);
    let kinds: Vec<&str> = json11["kinds"].as_array().unwrap().iter().map(|k| k["id"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"GER"), "witnessed owner cell remains registered");
    assert!(kinds.contains(&"total"), "region total remains registered under Table scope so UI shows the truncation reason");
}

#[test]
fn chosen_regions_lead_in_order_with_unknown_and_empty_picks_and_hidden_regions_leave() {
    let sea = BTreeSet::from([3, 5, 12, 15]);
    let mut chosen = fold(&rows(), name, |_| true, false, &sea);
    let rank = choose(&mut chosen, &[7, 5, 12], &[3, 12], false);
    chosen.mined.sort_by_key(|region| rank.get(&region.region).copied().unwrap_or(usize::MAX));
    let shown: Vec<(u32, bool, u32)> = chosen.mined.iter().map(|region| (region.region, region.pinned, region.unknown_owners)).collect();
    assert_eq!(shown, [(7, true, 2), (5, true, 2), (12, true, 2)], "hiding a pick keeps it; the hidden mined region leaves; a pick outside the catalogue is still the unnamed owner and ENG, whose read has no extent");
    assert_eq!(chosen.unknown, [UnknownRegion { region: 15, unknown_owners: 3 }], "a pick is not also listed unknown");
    assert!(chosen.cells.iter().all(|(region, ..)| *region != 3));

    let mut lost = fold(&rows(), name, |_| true, true, &BTreeSet::new());
    choose(&mut lost, &[40], &[], true);
    assert!(lost.mined.iter().find(|region| region.region == 40).is_some_and(|region| region.missing.is_some()), "an unread pick is not shown as no mines");
}

#[test]
fn every_pick_is_weighed_against_the_recorded_extents_even_without_the_sea_catalogue() {
    let rows = vec![witness(1, 10), amount(1, 3, 500), witness(2, 20)];
    let mut chosen = fold(&rows, name, |_| true, false, &BTreeSet::new());
    choose(&mut chosen, &[4, 15, 25], &[], false);
    let unknown: Vec<(u32, u32)> = chosen.mined.iter().filter(|region| region.pinned).map(|region| (region.region, region.unknown_owners)).collect();
    assert_eq!(unknown, [(4, 0), (15, 1), (25, 2)], "below every extent a pick is a known zero; past GER's extent GER is unknown; past both, both are; never 0 mines for an unknown");
}

#[test]
fn an_empty_reply_says_whether_the_user_hid_every_mined_region_the_read_was_partial_or_nothing_is_mined() {
    let sea = BTreeSet::from([3, 4]);
    let rows = vec![witness(1, 10), amount(1, 3, 500), witness(2, 10)];
    let mut hidden = fold(&rows, name, |_| true, false, &sea);
    choose(&mut hidden, &[], &[3], false);
    assert_eq!(no_regions(&hidden, false, true), Some(NoRegions::Filtered { hidden: 1, outside: 0 }), "hiding the only mined region is not \"no minefields\"");
    let mut none = fold(&[witness(1, 10), witness(2, 10)], name, |_| true, false, &sea);
    choose(&mut none, &[], &[], false);
    assert_eq!(no_regions(&none, false, true), Some(NoRegions::None));
    assert!(matches!(no_regions(&none, false, false), Some(NoRegions::Unread { .. })), "without the sea regions, regions past the extents are not known");
    let mut beyond = fold(&[witness(1, 2)], name, |_| true, false, &sea);
    choose(&mut beyond, &[], &[], false);
    assert!(matches!(no_regions(&beyond, false, true), Some(NoRegions::Unread { .. })), "a sea region past a country's extent is unread");
    let mut lost = fold(&[], name, |_| true, true, &sea);
    choose(&mut lost, &[], &[], true);
    assert_eq!(no_regions(&lost, true, true), Some(NoRegions::Unread { reason: UNREAD.into() }));
    assert_eq!(no_regions(&fold(&rows, name, |_| true, false, &sea), false, true), None, "a listed region: nothing to explain");
}
