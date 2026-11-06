//! An hour's reply over the publication of synthetic walks holds what the recording gives its
//! holder at that hour, however the walk is cut into segments, published and resumed.

use super::*;
use crate::products::equipment_stockpile::{
    UNRESOLVED_MARK,
    emit::{
        SCHEMA,
        tests::{self as walks, Seen, State},
    },
};
use dashboard_series::{memory::Memory};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};

/// `branch`'s publication from `runs`, a segment starting at every `restart`th walked hour and
/// a cut after every `every`th, resumed from the publication at each where `resume`.
fn published(
    runs: &[(u64, Vec<DecodedFrame>)],
    branch: u64,
    restart: usize,
    every: usize,
    resume: bool,
) -> (Memory, u64, Vec<i32>) {
    let hours = walks::walked(runs, branch);
    let restarts: BTreeSet<usize> = (1..hours.len()).filter(|at| at % restart == 0).collect();
    let mut cuts: Vec<i32> = hours
        .iter()
        .enumerate()
        .filter(|&(at, _)| at % every == every - 1)
        .map(|(_, hour)| *hour)
        .collect();
    if cuts.last() != hours.last() {
        cuts.push(*hours.last().unwrap());
    }
    let resumed = if resume { cuts.clone() } else { Vec::new() };
    let mut projected = walks::project(runs, &restarts);
    let (pages, root) = walks::build(projected.remove(&branch).unwrap(), &cuts, &resumed);
    (pages, root, hours)
}

/// Each tag's reply at each hour `expected` names.
fn replies(pages: &Memory, root: u64, expected: &walks::States) -> BTreeMap<(String, i32), EquipmentStockpileReply> {
    let mut snapshot = Snapshot::open(pages.clone(), root, walks::identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let reading = Reading::bare(None);
    expected
        .keys()
        .map(|(tag, hour)| {
            let params = StockpileAtParams {
                branch_id: "1".into(),
                tag: tag.clone(),
                hour: *hour,
            };
            let reply = query(
                &mut snapshot,
                &directory,
                &params,
                &reading,
                &AtomicBool::new(false),
            )
            .unwrap()
            .expect("an accepted hour is answered");
            ((tag.clone(), *hour), reply)
        })
        .collect()
}

fn amount(runs: &[AmountRun], hour: i32) -> Option<Option<i64>> {
    let [run] = runs else {
        assert!(runs.is_empty(), "an hour's value is one run");
        return None;
    };
    assert_eq!((run.first_hour, run.last_hour), (hour, hour));
    Some(run.amount_raw.as_ref().map(|raw| raw.parse().unwrap()))
}

/// The logistics token each of the walks' spellings names.
const TOKENS: [(u32, &str); 3] = [(1, "infantry_equipment_1"), (2, "destroyer_1"), (3, "truck_1")];

/// What the reply says of the holder at the hour, in the walks' own terms.
fn state_of(reply: &EquipmentStockpileReply, hour: i32) -> (bool, Option<walks::Stock>, Option<walks::Lines>, Option<walks::Logistic>) {
    let held = |spans: &[HourSpan]| match spans {
        [] => false,
        [span] => (span.first_hour, span.last_hour) == (hour, hour),
        _ => panic!("an hour is one span"),
    };
    let stock = held(&reply.stock_hours).then(|| {
        reply
            .variants
            .iter()
            .filter_map(|variant| Some((variant.id.parse().unwrap(), amount(&variant.stock, hour)?)))
            .collect()
    });
    let lines = held(&reply.line_hours).then(|| {
        reply
            .variants
            .iter()
            .filter_map(|variant| {
                let units = amount(&variant.daily, hour)?;
                let rest = [&variant.daily_ic, &variant.unit_cost_low, &variant.unit_cost_high]
                    .map(|runs| amount(runs, hour).expect("a line holds every value"));
                Some((variant.id.parse().unwrap(), [units, rest[0], rest[1], rest[2]]))
            })
            .collect()
    });
    let ai = held(&reply.ai_hours);
    let logistics = (held(&reply.balance_hours) || ai).then(|| {
        reply
            .logistics
            .iter()
            .map(|item| {
                let token = TOKENS
                    .iter()
                    .find(|(_, spelling)| item.key == format!("{UNRESOLVED_MARK}{spelling}"))
                    .map(|(token, _)| *token)
                    .expect("every token is spelled");
                let values = [&item.balance, &item.stock, &item.available]
                    .map(|runs| amount(runs, hour).expect("a row holds every value"));
                (token, values)
            })
            .collect()
    });
    assert!(!(ai && logistics.as_ref().is_some_and(|rows: &walks::Logistic| !rows.is_empty())));
    (ai, stock, lines, logistics)
}

fn known(state: &State, stream: usize) -> bool {
    matches!(state.coverage[stream], Seen::Known(_))
}

/// The hour each table was last read by `hour` in the walks: a stream's coverage starts at a
/// read and holds until its next.
fn last_reads(runs: &[(u64, Vec<DecodedFrame>)], branch: u64) -> BTreeMap<i32, [Option<i32>; 2]> {
    let format = FormatVersion::CURRENT;
    let mut last = [None; 2];
    let mut reads = BTreeMap::new();
    for frame in runs
        .iter()
        .filter(|(held, _)| *held == branch)
        .flat_map(|(_, frames)| frames)
    {
        for (at, table) in [TableId::Stockpiles, TableId::LogisticsEquipment].into_iter().enumerate() {
            if dashboard_text::metadata::slot_read(format, &frame.metrics, table, frame.hour) {
                last[at] = Some(frame.hour);
            }
        }
        reads.insert(frame.hour, last);
    }
    reads
}

#[test]
fn every_hour_answers_what_the_recording_gives_its_holder_however_it_is_walked() {
    for runs in [walks::campaign_runs(), vec![(1, walks::days())]] {
        for branch in runs.iter().map(|(branch, _)| *branch).collect::<BTreeSet<_>>() {
            let expected = walks::expected(&runs, branch);
            let reads = last_reads(&runs, branch);
            for (restart, every, resume) in [(usize::MAX, usize::MAX, false), (6, 8, false), (1, 5, true), (30, 24, true)] {
                let (pages, root, _) = published(&runs, branch, restart, every, resume);
                let label = format!("branch {branch}, segments every {restart}, cuts every {every}");
                for ((tag, hour), reply) in replies(&pages, root, &expected) {
                    let state = &expected[&(tag.clone(), hour)];
                    let (ai, stock, lines, logistics) = state_of(&reply, hour);
                    assert_eq!(stock, state.stock, "{label}: {tag} stock at {hour}");
                    assert_eq!(lines, state.lines, "{label}: {tag} lines at {hour}");
                    assert_eq!(logistics, state.logistics, "{label}: {tag} logistics at {hour}");
                    assert_eq!(ai, state.logistics.as_ref().is_some_and(BTreeMap::is_empty), "{label}: {tag} AI at {hour}");
                    // Each value is as of the table's latest read, which the reply dates.
                    let [stock_read, logistics_read] = reads[&hour];
                    assert_eq!(reply.reads, known(state, 0).then(|| stock_read.unwrap()).into_iter().collect::<Vec<_>>(), "{label}: {tag} at {hour}");
                    assert_eq!(reply.line_reads, known(state, 1).then(|| stock_read.unwrap()).into_iter().collect::<Vec<_>>(), "{label}: {tag} at {hour}");
                    assert_eq!(reply.logistics_reads, (known(state, 2) && !ai).then(|| logistics_read.unwrap()).into_iter().collect::<Vec<_>>(),
                        "{label}: {tag} at {hour}");
                }
            }
        }
    }
}

#[test]
fn an_hour_warns_of_its_own_read_counts_and_names_its_variants_as_then() {
    let runs = walks::campaign_runs();
    let (pages, root, _) = published(&runs, 1, usize::MAX, usize::MAX, false);
    let expected = walks::expected(&runs, 1);
    let at = |tag: &str, hour: i32| {
        let wanted: walks::States = [((tag.to_owned(), hour), expected[&(tag.to_owned(), hour)].clone())].into();
        replies(&pages, root, &wanted).remove(&(tag.to_owned(), hour)).unwrap()
    };
    let has = |reply: &EquipmentStockpileReply, text: &str| reply.warnings.iter().any(|warning| warning.contains(text));
    // A costless line, a naval line and an overflowing total are counted at their reads.
    assert!(has(&at("GER", 100), "1 production lines at this read have no positive cost"));
    assert!(has(&at("FRA", 100), "1 naval, refit or railway-gun lines at this read"));
    assert!(has(&at("GER", 128), "1 recorded totals at this read do not fit a 64-bit amount"));
    assert!(!has(&at("GER", 104), "do not fit a 64-bit amount"));
    // A missing available operand is counted where the record holds it.
    assert!(has(&at("GER", 126), "1 logistics archetypes at this read have no matching recorded available-stock operand"));
    // Gewehr 43 is renamed at 108 and read as renamed at 110.
    let name = |reply: &EquipmentStockpileReply| {
        reply.variants.iter().find(|variant| variant.id == "20").unwrap().names.clone()
    };
    assert_eq!(name(&at("GER", 104))[0].first_hour, 100);
    assert_eq!(name(&at("GER", 112))[0].first_hour, 110);
    assert!(name(&at("GER", 112))[0].known);
    // Without content no type resolves, so the reply names none and refuses nothing it cannot read.
    assert!(at("GER", 104).types.is_empty());
    assert!(at("GER", 104).variants.iter().all(|variant| variant.archetype.is_none()));
}

#[test]
fn an_hour_past_the_published_hours_is_pending_and_an_unknown_tag_holds_nothing() {
    let runs = walks::campaign_runs();
    let (pages, root, hours) = published(&runs, 1, usize::MAX, usize::MAX, false);
    let mut snapshot = Snapshot::open(pages, root, walks::identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let reading = Reading::bare(None);
    let ask = |snapshot: &mut Snapshot<'_, Memory>, tag: &str, hour: i32| {
        let params = StockpileAtParams { branch_id: "1".into(), tag: tag.into(), hour };
        query(snapshot, &directory, &params, &reading, &AtomicBool::new(false)).unwrap()
    };
    assert!(ask(&mut snapshot, "GER", hours.last().unwrap() + 1).is_none());
    let nobody = ask(&mut snapshot, "XXX", 104).unwrap();
    assert!(nobody.variants.is_empty() && nobody.stock_hours.is_empty() && nobody.balance_hours.is_empty());
    // An hour not walked holds no coverage, so nothing reads as none there.
    let gap = ask(&mut snapshot, "GER", 137).unwrap();
    assert!(gap.stock_hours.is_empty() && gap.line_hours.is_empty() && gap.variants.is_empty());
}
