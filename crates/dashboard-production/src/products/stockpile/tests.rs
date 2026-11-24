use super::{
    catalogue::{Drawn, Group, Kind, Lines, Member, Piece, Record, drawn, groups, logistics_lines, stock_lines, summary, variant_line},
    daily_units_raw,
    history::{Logistics, Replay, Stock, carried, split_logistics},
    lines::{AI_BALANCE, BALANCE_UNREAD, NEED_UNKNOWN, NO_NEED, NOT_HELD, STOCK_UNKNOWN, STOCK_UNREAD, Shown, bucketed, main_types, missing, next_change, value_at},
};
use dashboard_base::span::HourSpan;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan { first_hour, last_hour }
}

fn member(token: &str, archetype: Option<&str>, domain: Option<super::Domain>) -> Member {
    Member { token: token.into(), archetype: archetype.map(str::to_owned), domain, names: Vec::new() }
}

#[test]
fn a_read_is_carried_a_day_to_the_next_read_and_no_further() {
    assert_eq!(carried(&[10, 34, 58], 100), vec![span(10, 81)]);
    assert_eq!(carried(&[10, 82], 90), vec![span(10, 33), span(82, 90)]);
    assert_eq!(carried(&[10], 20), vec![span(10, 20)]);
    let held = [(5, true), (40, false), (70, true)];
    let (with, without) = split_logistics(&[10, 34, 58, 82], &held, 100);
    assert_eq!(with, vec![span(10, 57), span(82, 100)]);
    assert_eq!(without, vec![span(58, 81)]);
}

#[test]
fn a_types_stock_sums_its_designs_and_an_untyped_holding_makes_it_unknown() {
    let mut replay = Replay { stock: vec![span(0, 99)], ..Replay::default() };
    replay.variants.insert(1, vec![(0, Stock::Amount(100)), (50, Stock::Amount(150))]);
    replay.variants.insert(2, vec![(0, Stock::Amount(7)), (60, Stock::Absent)]);
    replay.variants.insert(3, vec![(0, Stock::Amount(0)), (20, Stock::Amount(5)), (30, Stock::Amount(0))]);
    let mut variants = BTreeMap::new();
    variants.insert(1, member("infantry_equipment_1", Some("infantry_equipment"), Some(super::Domain::Land)));
    variants.insert(2, member("infantry_equipment_2", Some("infantry_equipment"), Some(super::Domain::Land)));
    variants.insert(3, member("mod_gun", None, None));
    let all = groups(&variants, &BTreeMap::new(), &BTreeMap::new(), &[]);
    let rifles = all.iter().find(|group| group.kind == Kind::Type && group.key == "infantry_equipment").unwrap();
    let summed = [(0, rifles)];
    let lines = stock_lines(&replay, &summed, &[3], &AtomicBool::new(false)).unwrap();
    let values: Vec<(i32, i32, Option<i128>)> = lines[0].iter().map(|piece| (piece.first, piece.last, piece.value)).collect();
    assert_eq!(values, vec![(0, 19, Some(107)), (20, 29, None), (30, 49, Some(107)), (50, 59, Some(157)), (60, 99, Some(150))]);
    let own: Vec<Option<i128>> = variant_line(&replay, 2).iter().map(|piece| piece.value).collect();
    assert_eq!(own, vec![Some(7), Some(0)]);
    let included = groups(&variants, &BTreeMap::new(), &BTreeMap::new(), &[":mod_gun".to_owned()]);
    assert!(included.iter().all(|group| group.variants == vec![3]));
}

#[test]
fn a_need_comes_from_the_available_operand_and_an_unresolved_type_decides_an_arm() {
    let mut replay = Replay { balance: vec![span(0, 49)], ..Replay::default() };
    let row = |balance, stock, available| Some(Logistics { balance: Some(balance), stock: Some(stock), available });
    replay.logistics.insert(10, vec![(0, row(-50, 100, Some(120))), (20, row(-50, 100, None))]);
    replay.logistics.insert(11, vec![(0, row(30, 60, None))]);
    replay.logistics.insert(12, vec![(0, row(0, 0, None)), (30, row(5, 5, None))]);
    let mut tokens: BTreeMap<String, (Vec<u32>, bool)> = BTreeMap::new();
    tokens.insert("infantry_equipment".into(), (vec![10], true));
    tokens.insert("convoy".into(), (vec![11], true));
    tokens.insert(":mystery".into(), (vec![12], true));
    let notes: BTreeMap<&str, Option<&str>> = [("infantry_equipment", None), ("convoy", Some("needs the operand")), (":mystery", Some("Need unavailable."))].into_iter().collect();
    let record = Record { logistics: &tokens, notes, unresolved: vec![":mystery"] };
    let rifles = Group { wire: String::new(), kind: Kind::Type, key: "infantry_equipment".into(), variants: Vec::new(), types: vec!["infantry_equipment".into()] };
    let convoys = Group { wire: String::new(), kind: Kind::Type, key: "convoy".into(), variants: Vec::new(), types: vec!["convoy".into()] };
    let land = Group { wire: String::new(), kind: Kind::Domain, key: "land".into(), variants: Vec::new(), types: vec!["infantry_equipment".into()] };
    let summed = [(0, &rifles), (1, &convoys), (2, &land)];
    let lines = logistics_lines(&replay, &summed, &record, &AtomicBool::new(false)).unwrap();
    let values = |line: &Vec<Piece<(i128, Option<i128>)>>| line.iter().map(|piece| (piece.first, piece.value)).collect::<Vec<_>>();
    assert_eq!(values(&lines[0]), vec![(0, Some((-50, Some(170)))), (20, Some((-50, Some(150))))]);
    assert_eq!(values(&lines[1]), vec![(0, Some((30, None)))]);
    assert_eq!(values(&lines[2]), vec![(0, Some((-50, None))), (30, None)]);
}

#[test]
fn a_chart_folds_exact_pieces_into_the_views_bins_and_breaks_after_unknown_hours() {
    let pieces = vec![
        Piece { first: 0, last: 9, value: Some(10_i128), starts: true },
        Piece { first: 10, last: 19, value: None, starts: false },
        Piece { first: 20, last: 39, value: Some(30), starts: false },
    ];
    let mut lines: Vec<Drawn> = Vec::new();
    drawn(&pieces, |raw| Some((raw as f64, raw.unsigned_abs())), (0, 39), None, &mut lines);
    assert_eq!(lines.iter().map(|line| (line.first, line.break_before)).collect::<Vec<_>>(), vec![(0, true), (20, true)]);
    let hourly = summary(std::mem::take(&mut lines), 0, 39, 40);
    assert_eq!(hourly.runs.as_ref().map(Vec::len), Some(2));
    assert_eq!(hourly.buckets.iter().filter(|bucket| bucket.is_some()).count(), 30);
    let mut lines: Vec<Drawn> = Vec::new();
    drawn(&pieces, |raw| Some((raw as f64, raw.unsigned_abs())), (0, 39), None, &mut lines);
    let binned = summary(lines, 0, 39, 2);
    assert!(binned.runs.is_none());
    let held: Vec<_> = binned.buckets.iter().flatten().collect();
    assert!(held.iter().any(|bucket| bucket.min_value == 10.0 && bucket.max_value == 30.0 || bucket.first_value == 10.0));
    assert!(held.iter().all(|bucket| bucket.max_value <= 30.0 && bucket.min_value >= 10.0));
    let mut lines: Vec<Drawn> = Vec::new();
    drawn(&pieces, |raw| Some((raw as f64, raw.unsigned_abs())), (5, 39), None, &mut lines);
    let cut = summary(lines, 5, 39, 40);
    let runs: Vec<(i32, i32)> = cut.runs.unwrap().iter().map(|run| (run.first_hour, run.last_hour)).collect();
    assert_eq!(runs, vec![(5, 9), (20, 39)]);
    let mut lines: Vec<Drawn> = Vec::new();
    drawn(&pieces, |raw| Some((raw as f64, raw.unsigned_abs())), (5, 39), None, &mut lines);
    let binned = summary(lines, 5, 39, 2);
    assert_eq!(binned.buckets.iter().flatten().next().map(|bucket| bucket.first_hour), Some(5));
}

#[test]
fn a_lines_daily_units_are_its_own_truncated_rate_and_none_without_a_cost() {
    assert_eq!(daily_units_raw(250_000, 700_000), Some(35_714));
    assert_eq!(daily_units_raw(250_000, 0), None);
    assert_eq!(daily_units_raw(i64::MAX, 1), None);
}

fn piece<T>(first: i32, last: i32, value: Option<T>) -> Piece<T> {
    Piece { first, last, value, starts: false }
}

#[test]
fn a_type_over_time_reads_its_value_or_its_reason_at_the_hour_and_its_next_change() {
    let replay = Replay { stock: vec![span(0, 39)], balance: vec![span(0, 19)], ai: vec![span(30, 39)], ..Replay::default() };
    let lines = Lines {
        stock: vec![piece(0, 9, Some(250_000)), piece(10, 19, None), piece(20, 39, Some(100_000))],
        logistics: vec![piece(0, 4, Some((-50, Some(200)))), piece(5, 9, Some((10, None))), piece(10, 14, Some((10, Some(0)))), piece(15, 19, None)],
    };
    assert_eq!(value_at(&lines, &replay, Shown::Stock, 3), Ok(2.5));
    assert_eq!(value_at(&lines, &replay, Shown::Stock, 12), Err(STOCK_UNKNOWN));
    assert_eq!(value_at(&lines, &replay, Shown::Stock, 40), Err(STOCK_UNREAD));
    assert_eq!(value_at(&lines, &replay, Shown::Share, 0), Ok(-25.0));
    assert_eq!(value_at(&lines, &replay, Shown::Share, 6), Err(NEED_UNKNOWN));
    assert_eq!(value_at(&lines, &replay, Shown::Share, 11), Err(NO_NEED));
    assert_eq!(value_at(&lines, &replay, Shown::Share, 16), Err(NEED_UNKNOWN));
    assert_eq!(value_at(&lines, &replay, Shown::Share, 25), Err(BALANCE_UNREAD));
    assert_eq!(value_at(&lines, &replay, Shown::Share, 31), Err(AI_BALANCE));
    assert_eq!(next_change([&lines], Shown::Stock, None, 3, 39), Some(10));
    assert_eq!(next_change([&lines], Shown::Stock, None, 25, 39), None);
    assert_eq!(next_change([&lines], Shown::Stock, Some(&[span(0, 6)][..]), 3, 39), Some(7));
    let gaps = missing(&lines, &replay, Shown::Stock, Some(&[span(0, 14)][..]), 0, 39);
    assert_eq!(gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, gap.reason.as_str())).collect::<Vec<_>>(), vec![(10, 14, STOCK_UNKNOWN)]);
    let gaps = missing(&lines, &replay, Shown::Share, None, 15, 39);
    assert_eq!(
        gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, gap.reason.as_str())).collect::<Vec<_>>(),
        vec![(15, 19, NEED_UNKNOWN), (20, 29, BALANCE_UNREAD), (30, 39, AI_BALANCE)]
    );
    assert_ne!(NOT_HELD, STOCK_UNREAD);
}

#[test]
fn the_main_types_are_ranked_by_stock_at_the_hour_and_refused_when_one_is_unknown() {
    use super::Domain::{Air, Land, Naval};
    let values = [
        ("rifles", Some(Land), Ok(900.0)),
        ("guns", Some(Land), Ok(300.0)),
        ("tanks", Some(Land), Ok(300.0)),
        ("trucks", Some(Land), Ok(100.0)),
        ("fighters", Some(Air), Ok(50.0)),
        ("bombers", Some(Air), Ok(0.0)),
        ("ships", Some(Naval), Ok(5000.0)),
    ];
    assert_eq!(main_types(&values, 5), Ok(vec!["rifles".to_owned(), "guns".into(), "tanks".into(), "fighters".into()]));
    let naval = [("ships", Some(Naval), Ok(5.0)), ("subs", Some(Naval), Ok(7.0))];
    assert_eq!(main_types(&naval, 5), Ok(vec!["subs".to_owned(), "ships".into()]));
    let unknown = [("rifles", Some(Land), Ok(900.0)), ("guns", Some(Land), Err(STOCK_UNKNOWN))];
    assert!(main_types(&unknown, 5).unwrap_err().contains(STOCK_UNKNOWN));
}

#[test]
fn a_tile_folds_pieces_into_its_bins_and_marks_a_bin_it_only_partly_covers() {
    let lines = vec![
        Drawn { first: 0, last: 5, value: 1.0, absolute: 1, break_before: true },
        Drawn { first: 6, last: 9, value: 3.0, absolute: 3, break_before: false },
        Drawn { first: 14, last: 17, value: 2.0, absolute: 2, break_before: true },
    ];
    let bins = bucketed(&lines, 0, 17, 10);
    assert_eq!(bins.len(), 256);
    let first = bins[0].as_ref().unwrap();
    assert_eq!((first.first_value, first.last_value, first.min_value, first.max_value, first.has_break), (1.0, 3.0, 1.0, 3.0, false));
    let second = bins[1].as_ref().unwrap();
    assert_eq!((second.first_hour, second.last_hour, second.first_break, second.has_break), (14, 17, true, true));
    assert!(bins[2..].iter().all(Option::is_none));
}
