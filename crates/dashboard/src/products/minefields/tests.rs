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
