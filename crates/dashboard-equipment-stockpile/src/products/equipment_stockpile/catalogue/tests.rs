//! The catalogue and chart lines over the publication of synthetic walks hold the reads,
//! coverage, variants and counts the recording gives; a group adds up as the widget's own
//! at-hour figures do.

use super::*;
use crate::products::equipment_stockpile::emit::{
    SCHEMA,
    tests::{self as walks, Seen},
};
use dashboard_series::{memory::Memory};
use replay_data::{format::FormatVersion, rows::TableId};
use std::collections::BTreeSet;

fn snapshot<'c>(pages: &Memory, root: u64) -> (Snapshot<'c, Memory>, Directory) {
    let mut snapshot = Snapshot::open(pages.clone(), root, walks::identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    (snapshot, directory)
}

/// The campaign's publication, a segment starting at every walked hour and cut every fifth.
fn campaign() -> (Memory, u64) {
    let runs = walks::campaign_runs();
    let hours = walks::walked(&runs, 1);
    let restarts = (1..hours.len()).collect();
    let mut cuts: Vec<i32> = hours.iter().copied().skip(4).step_by(5).collect();
    if cuts.last() != hours.last() {
        cuts.push(*hours.last().unwrap());
    }
    let mut projected = walks::project(&runs, &restarts);
    walks::build(projected.remove(&1).unwrap(), &cuts, &cuts)
}

fn holder(tag: &str) -> StockpileHolder {
    StockpileHolder {
        tag: tag.into(),
        within: None,
    }
}

/// Consecutive walked hours where `held` holds the same value, as spans.
fn spans_where(hours: &[(i32, Option<u64>)]) -> Vec<HourSpan> {
    let mut spans: Vec<(HourSpan, u64)> = Vec::new();
    for &(hour, held) in hours {
        let Some(held) = held else { continue };
        match spans.last_mut() {
            Some((span, last)) if *last == held && span.last_hour + 1 == hour => span.last_hour = hour,
            _ => spans.push((HourSpan { first_hour: hour, last_hour: hour }, held)),
        }
    }
    spans.into_iter().map(|(span, _)| span).collect()
}

#[test]
fn the_catalogue_holds_every_read_coverage_variant_and_count_the_walks_give() {
    let runs = walks::campaign_runs();
    let expected = walks::expected(&runs, 1);
    let (pages, root) = campaign();
    let (mut snapshot, directory) = snapshot(&pages, root);
    let reading = Reading::bare(None);
    let format = FormatVersion::CURRENT;
    let frames: Vec<_> = runs.iter().filter(|(branch, _)| *branch == 1).flat_map(|(_, frames)| frames).collect();
    let read = |table, hour: i32| frames.iter().any(|frame| frame.hour == hour
        && dashboard_text::metadata::slot_read(format, &frame.metrics, table, hour));
    let tags: BTreeSet<String> = expected.keys().map(|(tag, _)| tag.clone()).collect();
    for tag in tags {
        let params = StockpileCatalogueParams { branch_id: "1".into(), holders: vec![holder(&tag)], included: Vec::new() };
        let reply = query(&mut snapshot, &directory, &params, &reading, &AtomicBool::new(false)).unwrap();
        let states: Vec<(i32, &walks::State)> = expected.iter().filter(|((held, _), _)| *held == tag).map(|((_, hour), state)| (*hour, state)).collect();
        let known = |stream: usize| -> Vec<(i32, Option<u64>)> {
            states.iter().map(|(hour, state)| (*hour, match state.coverage[stream] { Seen::Known(holder) => Some(holder), _ => None })).collect()
        };
        let rows = |held: bool| -> Vec<(i32, Option<u64>)> {
            states.iter().map(|(hour, state)| (*hour, match (state.coverage[2], &state.logistics) {
                (Seen::Known(holder), Some(rows)) if rows.is_empty() != held => Some(holder),
                _ => None,
            })).collect()
        };
        assert_eq!(reply.stock_hours, spans_where(&known(0)), "{tag} stock hours");
        assert_eq!(reply.line_hours, spans_where(&known(1)), "{tag} line hours");
        assert_eq!(reply.balance_hours, spans_where(&rows(true)), "{tag} balance hours");
        assert_eq!(reply.ai_hours, spans_where(&rows(false)), "{tag} AI hours");
        let reads = |stream: usize, table| -> Vec<i32> {
            known(stream).into_iter().filter(|(hour, held)| held.is_some() && read(table, *hour)).map(|(hour, _)| hour).collect()
        };
        assert_eq!(reply.reads, reads(0, TableId::Stockpiles), "{tag} reads");
        assert_eq!(reply.line_reads, reads(1, TableId::Stockpiles), "{tag} line reads");
        let logistics_reads: Vec<i32> = rows(true).into_iter().filter(|(hour, held)| held.is_some() && read(TableId::LogisticsEquipment, *hour))
            .map(|(hour, _)| hour).collect();
        assert_eq!(reply.logistics_reads, logistics_reads, "{tag} logistics reads");
        let variants: BTreeSet<u64> = states.iter().flat_map(|(_, state)| state.stock.iter().flat_map(|stock| stock.keys())
            .chain(state.lines.iter().flat_map(|lines| lines.keys())).copied()).collect();
        assert_eq!(reply.variants.iter().map(|variant| variant.id.parse().unwrap()).collect::<BTreeSet<u64>>(), variants, "{tag} variants");
        // Every read's counts, each part over the reads its coverage holds.
        let mut totals = [0_u64; 11];
        for (hour, state) in &states {
            for (part, table, stream) in [(0..1, TableId::Stockpiles, 0), (1..7, TableId::Stockpiles, 1), (7..11, TableId::LogisticsEquipment, 2)] {
                if matches!(state.coverage[stream], Seen::Known(_)) && read(table, *hour) {
                    for at in part {
                        totals[at] += state.counts[at].unwrap();
                    }
                }
            }
        }
        let said = |text: &str| reply.warnings.iter().find_map(|warning| warning.strip_suffix(text).or_else(|| warning.split_once(text).map(|(count, _)| count))
            .and_then(|count| count.trim().parse::<u64>().ok())).unwrap_or(0);
        assert_eq!(said(" production line reads have no positive cost"), totals[1], "{tag} costless");
        assert_eq!(said(" naval, refit or railway-gun line reads"), totals[2], "{tag} other lines");
        assert_eq!(said(" logistics archetype reads have no matching"), totals[7], "{tag} missing available");
        let overflowed = totals[0] + totals[3] + totals[4] + totals[8] + totals[9] + totals[10];
        assert_eq!(said(" recorded totals do not fit a 64-bit amount"), overflowed, "{tag} overflows");
    }
}

#[test]
fn a_variant_s_line_is_its_stock_at_every_known_hour_and_breaks_where_it_is_not() {
    let runs = walks::campaign_runs();
    let expected = walks::expected(&runs, 1);
    let (pages, root) = campaign();
    let (mut snapshot, directory) = snapshot(&pages, root);
    let params = StockpileChartParams {
        branch_id: "1".into(),
        holders: vec![holder("GER")],
        included: Vec::new(),
        lines: vec![StockpileLine { group: wire(Kind::Variant, "10"), measure: StockpileMeasure::Stock },
            StockpileLine { group: wire(Kind::Variant, "404"), measure: StockpileMeasure::Stock }],
        from: 100.0,
        to: 147.0,
        width: 400,
    };
    let reply = chart(&mut snapshot, &directory, &params, &Reading::bare(None), &AtomicBool::new(false)).unwrap();
    let runs = reply.lines[0].summary.runs.as_ref().expect("an hourly view lists its runs");
    for ((tag, hour), state) in expected.iter().filter(|((tag, _), _)| tag == "GER") {
        let drawn = runs.iter().find(|run| run.first_hour <= *hour && *hour <= run.last_hour).map(|run| run.value);
        let held = state.stock.as_ref().map(|stock| stock.get(&10).copied().unwrap_or(Some(0)));
        assert_eq!(drawn, held.flatten().map(|raw| raw as f64 / 1e5), "{tag} at {hour}");
    }
    // A line starts with a break wherever its coverage starts or follows an unknown stretch.
    assert!(runs[0].break_before);
    assert!(runs.windows(2).all(|pair| pair[1].break_before || pair[0].last_hour + 1 == pair[1].first_hour));
    // A group the holders never had draws nothing and says so.
    assert!(reply.lines[1].summary.buckets.iter().all(Option::is_none));
    assert!(reply.warnings.iter().any(|warning| warning.contains("404")));
}

/// A replay of two resolved land types, an untyped variant and an unresolved logistics type,
/// over hours 0 to 99.
fn fixture() -> (Replay, BTreeMap<u64, Member>, TypeTokens, BTreeMap<String, Domain>) {
    let logistic = |balance, stock, available| Some(Logistics { balance: Some(balance), stock: Some(stock), available });
    let replay = Replay {
        stock: vec![HourSpan { first_hour: 0, last_hour: 99 }],
        lines: Vec::new(),
        balance: vec![HourSpan { first_hour: 0, last_hour: 99 }],
        ai: Vec::new(),
        variants: [
            (1, vec![(0, Stock::Amount(10)), (50, Stock::Amount(20))]),
            (2, vec![(0, Stock::Amount(5)), (70, Stock::Unknown)]),
            (3, vec![(0, Stock::Absent), (60, Stock::Amount(7)), (65, Stock::Absent)]),
        ].into(),
        logistics: [
            (1, vec![(0, logistic(3, 9, Some(9)))]),
            (2, vec![(0, logistic(-2, 4, None))]),
            (4, vec![(0, logistic(0, 0, Some(0))), (40, logistic(1, 1, Some(1)))]),
        ].into(),
        counts: Vec::new(),
    };
    let member = |archetype: Option<&str>| Member {
        token: archetype.map(|key| format!("{key}_1")).unwrap_or_default(),
        archetype: archetype.map(str::to_owned),
        domain: archetype.map(|_| Domain::Land),
        names: Vec::new(),
        designs: Vec::new(),
    };
    let variants = [(1, member(Some("a"))), (2, member(Some("b"))), (3, member(None))].into();
    let logistics = [("a".to_owned(), (vec![1], true)), ("b".to_owned(), (vec![2], true)), (format!("{UNRESOLVED_MARK}x"), (vec![4], true))].into();
    let domains = [("a".to_owned(), Domain::Land), ("b".to_owned(), Domain::Land)].into();
    (replay, variants, logistics, domains)
}

fn pieces<T: Copy>(line: &[Piece<T>]) -> Vec<(i32, i32, Option<T>)> {
    line.iter().map(|piece| (piece.first, piece.last, piece.value)).collect()
}

#[test]
fn a_type_or_an_arm_is_unknown_wherever_a_variant_of_unknown_type_holds_anything() {
    let (replay, variants, logistics, domains) = fixture();
    let groups = groups(&variants, &logistics, &domains, &[]);
    let at = |kind, key: &str| groups.iter().position(|group| group.kind == kind && group.key == key).unwrap();
    let summed: Vec<(usize, &Group)> = groups.iter().enumerate().filter(|(_, group)| group.kind != Kind::Variant).collect();
    let lines = stock_lines(&replay, &summed, &[3], &AtomicBool::new(false)).unwrap();
    let line = |kind, key| &lines[summed.iter().position(|(index, _)| *index == at(kind, key)).unwrap()];
    assert_eq!(pieces(line(Kind::Type, "a")), [(0, 49, Some(10)), (50, 59, Some(20)), (60, 64, None), (65, 99, Some(20))]);
    // A variant of its own whose stock is not computable leaves its type unknown, never partial.
    assert_eq!(pieces(line(Kind::Type, "b")), [(0, 59, Some(5)), (60, 64, None), (65, 69, Some(5)), (70, 99, None)]);
    assert_eq!(pieces(line(Kind::Domain, "land")), [(0, 49, Some(15)), (50, 59, Some(25)), (60, 64, None), (65, 69, Some(25)), (70, 99, None)]);
    // A variant's own line is its stock: none where it holds no row.
    assert_eq!(pieces(&variant_line(&replay, 3)), [(0, 59, Some(0)), (60, 64, Some(7)), (65, 99, Some(0))]);
}

#[test]
fn surplus_and_need_follow_the_same_write_and_an_unresolved_type_decides_a_resolved_arm() {
    let (replay, variants, logistics, domains) = fixture();
    let groups = groups(&variants, &logistics, &domains, &[]);
    let summed: Vec<(usize, &Group)> = groups.iter().enumerate().filter(|(_, group)| group.kind != Kind::Variant).collect();
    let note = "This convoy archetype needs the native available-stock operand at the same balance write.";
    let unresolved = format!("{UNRESOLVED_MARK}x");
    let record = Record {
        logistics: &logistics,
        notes: [("a", None), ("b", Some(note))].into(),
        unresolved: vec![unresolved.as_str()],
    };
    let lines = logistics_lines(&replay, &summed, &record, &AtomicBool::new(false)).unwrap();
    let line = |kind, key: &str| {
        let index = groups.iter().position(|group| group.kind == kind && group.key == key).unwrap();
        pieces(&lines[summed.iter().position(|(at, _)| *at == index).unwrap()])
    };
    // Need is the available operand less the balance of the same write.
    assert_eq!(line(Kind::Type, "a"), [(0, 99, Some((3, Some(6))))]);
    // A type whose need needs the operand has none without it; its surplus still holds.
    assert_eq!(line(Kind::Type, "b"), [(0, 99, Some((-2, None)))]);
    // The arm: an unresolved type listed, even at zero, leaves its need unknown; holding
    // anything, it leaves the whole arm unknown.
    assert_eq!(line(Kind::Domain, "land"), [(0, 39, Some((1, None))), (40, 99, None)]);
    // The unresolved arm is the unresolved type's own sum, its need from the operand it records.
    assert_eq!(line(Kind::Domain, UNRESOLVED_ARM), [(0, 39, Some((0, Some(0)))), (40, 99, Some((1, Some(0))))]);
}

#[test]
fn need_wraps_as_the_native_subtraction_does_and_a_bound_need_is_unknown() {
    let (mut replay, variants, logistics, domains) = fixture();
    replay.logistics.insert(1, vec![(0, Some(Logistics { balance: Some(i64::MIN), stock: Some(0), available: Some(i64::MAX) })),
        (50, Some(Logistics { balance: Some(0), stock: Some(0), available: Some(NEED_BOUND_RAW) }))]);
    let groups = groups(&variants, &logistics, &domains, &["a".to_owned()]);
    let summed: Vec<(usize, &Group)> = groups.iter().enumerate().filter(|(_, group)| group.kind != Kind::Variant).collect();
    let record = Record { logistics: &logistics, notes: [("a", None)].into(), unresolved: Vec::new() };
    let lines = logistics_lines(&replay, &summed, &record, &AtomicBool::new(false)).unwrap();
    let index = groups.iter().position(|group| group.kind == Kind::Type && group.key == "a").unwrap();
    let line = pieces(&lines[summed.iter().position(|(at, _)| *at == index).unwrap()]);
    assert_eq!(line, [(0, 49, Some((i128::from(i64::MIN), Some(-1)))), (50, 99, Some((0, None)))]);
    // Only the included types make rows.
    assert!(groups.iter().all(|group| group.kind != Kind::Type || group.key == "a"));
}

#[test]
fn a_view_s_buckets_hold_each_line_s_hours_and_break_where_they_stop() {
    let lines = vec![
        Drawn { first: 200, last: 299, value: 3.0, absolute: 300_000, break_before: true },
        Drawn { first: 0, last: 99, value: 1.0, absolute: 100_000, break_before: true },
        Drawn { first: 100, last: 149, value: 2.0, absolute: 200_000, break_before: false },
    ];
    let summary = summary(lines, (0, 399), (0.0, 399.0, 4)).unwrap();
    let buckets: Vec<_> = summary.buckets.iter().map(|bucket| bucket.map(|bucket| (bucket.first_hour, bucket.last_hour, bucket.min_value,
        bucket.first_break, bucket.has_break))).collect();
    assert_eq!(buckets, [Some((0, 99, 1.0, true, false)), Some((100, 149, 2.0, false, true)), Some((200, 299, 3.0, true, false)), None]);
    assert_eq!(summary.buckets[0].unwrap().absolute_total, 10_000_000);
    assert!(summary.runs.is_none(), "a wide view lists no runs");
}

#[test]
fn an_archetype_s_row_is_its_greatest_token_holding_one_at_the_hour() {
    let (mut replay, variants, mut logistics, domains) = fixture();
    let row = |balance| Some(Logistics { balance: Some(balance), stock: Some(10), available: Some(10) });
    // Token 1 alone until 50, token 5 from 30: from 30 the later token holds the archetype's row.
    replay.logistics.insert(1, vec![(0, row(3)), (50, None)]);
    replay.logistics.insert(5, vec![(30, row(7))]);
    logistics.get_mut("a").unwrap().0.push(5);
    let groups = groups(&variants, &logistics, &domains, &["a".to_owned()]);
    let summed: Vec<(usize, &Group)> = groups.iter().enumerate().filter(|(_, group)| group.kind != Kind::Variant).collect();
    let record = Record { logistics: &logistics, notes: [("a", None)].into(), unresolved: Vec::new() };
    let lines = logistics_lines(&replay, &summed, &record, &AtomicBool::new(false)).unwrap();
    let index = groups.iter().position(|group| group.kind == Kind::Type && group.key == "a").unwrap();
    let line = pieces(&lines[summed.iter().position(|(at, _)| *at == index).unwrap()]);
    assert_eq!(line, [(0, 29, Some((3, Some(7)))), (30, 99, Some((7, Some(3))))]);
}
