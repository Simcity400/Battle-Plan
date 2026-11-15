use super::{
    catalogue::{Drawn, Group, Kind, Member, Piece, Record, drawn, groups, logistics_lines, stock_lines, summary, variant_line},
    daily_units_raw,
    history::{Logistics, Replay, Stock, carried, split_logistics},
};
use crate::span::HourSpan;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan { first_hour, last_hour }
}

fn member(token: &str, archetype: Option<&str>, domain: Option<super::Domain>) -> Member {
    Member { token: token.into(), archetype: archetype.map(str::to_owned), domain, names: Vec::new() }
}

/// A read holds for a day at most, up to the next read and the holder's last hour; a missed read
/// leaves the hours after the carry unknown, never the last value carried on.
#[test]
fn a_read_is_carried_a_day_to_the_next_read_and_no_further() {
    assert_eq!(carried(&[10, 34, 58], 100), vec![span(10, 81)]);
    assert_eq!(carried(&[10, 82], 90), vec![span(10, 33), span(82, 90)]);
    assert_eq!(carried(&[10], 20), vec![span(10, 20)]);
    // Rows shown at the first read, none at the second (an AI-controlled read), shown again.
    let held = [(5, true), (40, false), (70, true)];
    let (with, without) = split_logistics(&[10, 34, 58, 82], &held, 100);
    assert_eq!(with, vec![span(10, 57), span(82, 100)]);
    assert_eq!(without, vec![span(58, 81)]);
}

/// A type sums its designs exactly at every hour; a design of unknown type holding anything leaves
/// every type's total unknown until it holds none, and a design's own line is its stock.
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
    // Excluding the type keeps its designs out of every group but the excluded designs' own rows.
    let included = groups(&variants, &BTreeMap::new(), &BTreeMap::new(), &[":mod_gun".to_owned()]);
    assert!(included.iter().all(|group| group.variants == vec![3]));
}

/// A type's need is its write's available operand less its balance; without the operand an
/// ordinary type derives it from its stock and a type with a note leaves it unknown; a resolved
/// arm is unknown while an unresolved type holds anything.
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
    // Listed at zero, the unresolved type leaves the arm's need unknown; holding, its whole line.
    assert_eq!(values(&lines[2]), vec![(0, Some((-50, None))), (30, None)]);
}

/// A line drawn over a view: an hourly view lists its runs, a long view folds them into the grid's
/// bins with each bin's first, last, least and greatest; a break follows an unknown stretch.
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
    // A view starting inside a piece draws from its own first hour, its runs and bins cut to it.
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
