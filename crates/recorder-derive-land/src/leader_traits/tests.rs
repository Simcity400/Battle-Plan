use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::UnitLeaderRow;

const TEXTS: &[&[u8]] = &[b"not read", b"", b"t101", b"t102", b"t103", b"t104"];
const NOT_READ: u32 = 0;

const L1: u64 = 11 << 32 | 5;
const L2: u64 = 12 << 32 | 5;
const A: i32 = 101;
const B: i32 = 102;
const C: i32 = 103;
const D: i32 = 104;

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(LeaderTraits)]);
    writers.start_run();
    writers
}

fn leader_row(handle: u64) -> UnitLeaderRow {
    UnitLeaderRow { id: (handle >> 32) as u32, reference_type: handle as u32, ..Default::default() }
}

fn listed(hour: i32) -> Vec<(u64, Vec<i32>)> {
    match hour {
        0 => vec![(L1, vec![A, B])],
        1 => vec![(L1, vec![A, B, C]), (L2, vec![A])],
        2 => vec![(L1, vec![B, C]), (L2, vec![A])],
        3 | 4 => vec![(L1, vec![B, C, D]), (L2, vec![A])],
        6 => vec![(L1, vec![B, C, D, A])],
        _ => vec![(L1, vec![B, C, D, A]), (L2, vec![A])],
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    let traits = if hour == 3 { Status::Unavailable { reason: NOT_READ } } else { Status::Complete };
    store.status(TableId::UnitLeaderTraits.index(), traits, true, Some(hour), true).unwrap();
    store.status(TableId::UnitLeaders.index(), Status::Complete, true, Some(hour), true).unwrap();
    let listing = listed(hour);
    for (leader, _) in &listing {
        testing::put(store, TableId::UnitLeaders, &leader_row(*leader), TEXTS).unwrap();
    }
    for (key, _) in testing::rows(store, TableId::UnitLeaders) {
        if !listing.iter().any(|(leader, _)| u128::from(*leader) == key) {
            testing::remove(store, TableId::UnitLeaders, key).unwrap();
        }
    }
    let held: Vec<u128> = testing::rows(store, TableId::UnitLeaderTraits).into_iter().map(|(key, _)| key).collect();
    for key in held {
        let kept = listing.iter().any(|(leader, tokens)| *leader == (key >> 32) as u64 && (key as u32 as usize) < tokens.len());
        if !kept {
            testing::remove(store, TableId::UnitLeaderTraits, key).unwrap();
        }
    }
    for (leader, tokens) in listing {
        for (ordinal, token_id) in tokens.into_iter().enumerate() {
            let row = UnitLeaderTraitRow { leader_reference: leader, ordinal: ordinal as u32, token_id, definition_ordinal: token_id, token: format!("t{token_id}").into_bytes() };
            testing::put(store, TableId::UnitLeaderTraits, &row, TEXTS).unwrap();
        }
    }
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn gains(store: &Store) -> Vec<(u64, i32, i64, i64)> {
    let view = store.view(TableId::LeaderTraitGains.index());
    let mut rows: Vec<(u64, i32, i64, i64)> = testing::rows(store, TableId::LeaderTraitGains)
        .into_iter()
        .map(|(key, _)| {
            let slot = view.slot(key).unwrap();
            let token = key as u32 as i32;
            assert_eq!(view.text(1, slot).and_then(|id| view.string(id)), Some(format!("t{token}").as_bytes()), "the trait's spelling");
            ((key >> 64) as u64, token, i64::from((key >> 32) as u32 as i32), view.number(0, slot).unwrap())
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn folded(logged: &[Vec<(u64, i32, i64, i64)>]) -> Vec<Vec<(u64, i32, i64, i64)>> {
    let mut held: std::collections::BTreeMap<(u64, i32), (i64, i64)> = std::collections::BTreeMap::new();
    let mut out = Vec::new();
    for hour in logged {
        for &(leader, token, at, class) in hour {
            if class == i64::from(LeaderTraitGainRow::FIRST_OBSERVED) && held.contains_key(&(leader, token)) {
                continue;
            }
            held.insert((leader, token), (at, class));
        }
        out.push(held.iter().map(|(&(leader, token), &(at, class))| (leader, token, at, class)).collect());
    }
    out
}

#[test]
fn held_gained_and_first_observed_traits_are_dated_and_a_trait_moving_up_the_list_is_none() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    let mut listed = Vec::new();
    for hour in 0..=5 {
        run(&mut store, &mut writers, hour);
        seen.push(gains(&store));
        listed.push(testing::listed(&store, TableId::LeaderTraitGains));
    }
    let [held, gained, first] = [LeaderTraitGainRow::HELD_BEFORE, LeaderTraitGainRow::GAINED, LeaderTraitGainRow::FIRST_OBSERVED].map(i64::from);
    let after_gap = vec![(L1, B, 4, first), (L1, C, 4, first), (L1, D, 4, first), (L2, A, 4, first)];
    assert_eq!(
        seen,
        [vec![(L1, A, 0, held), (L1, B, 0, held)], vec![(L1, C, 1, gained), (L2, A, 1, first)], vec![], vec![], after_gap, vec![(L1, A, 5, gained)]],
        "each hour holds its gains alone; the hour after one not read observes every trait held"
    );
    assert_eq!(listed, [vec![0, 1], vec![2, 3], vec![], vec![], vec![4, 5, 6, 7], vec![8]], "each row is appended once and forgotten at the writer's next hour");
    let start = vec![(L1, A, 0, held), (L1, B, 0, held)];
    let mut one = [start.clone(), vec![(L1, C, 1, gained), (L2, A, 1, first)]].concat();
    one.sort_unstable();
    let mut four = [one.clone(), vec![(L1, D, 4, first)]].concat();
    four.sort_unstable();
    let mut five = four.clone();
    five[0] = (L1, A, 5, gained);
    assert_eq!(folded(&seen), [start, one.clone(), one.clone(), one, four, five], "the reply's fold is the table it replaced");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for hour in 0..=5 {
        run(&mut unbroken, &mut writing, hour);
        expected.push((testing::rows(&unbroken, TableId::LeaderTraitGains), testing::listed(&unbroken, TableId::LeaderTraitGains), testing::rows(&unbroken, TableId::LeaderTraitDates)));
    }
    for stop in [1, 3] {
        let (mut stopped, mut before) = (testing::store(), writers());
        for hour in 0..=stop {
            run(&mut stopped, &mut before, hour);
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for hour in stop + 1..=5 {
            run(&mut resumed, &mut after, hour);
            let seen = (testing::rows(&resumed, TableId::LeaderTraitGains), testing::listed(&resumed, TableId::LeaderTraitGains), testing::rows(&resumed, TableId::LeaderTraitDates));
            assert_eq!(seen, expected[hour as usize], "stopped after hour {stop}, hour {hour}");
        }
    }
}

fn dates(store: &Store) -> Vec<(u64, i32, i64, i64)> {
    let view = store.view(TableId::LeaderTraitDates.index());
    testing::rows(store, TableId::LeaderTraitDates)
        .into_iter()
        .map(|(key, _)| {
            let slot = view.slot(key).unwrap();
            assert_eq!(view.text(3, slot).and_then(|id| view.string(id)), Some(format!("t{}", key as u32).as_bytes()), "the trait's spelling");
            let date = match view.number(2, slot) {
                Some(gained) => (gained, i64::from(LeaderTraitGainRow::GAINED)),
                None => (view.number(0, slot).unwrap(), view.number(1, slot).unwrap()),
            };
            ((key >> 32) as u64, key as u32 as i32, date.0, date.1)
        })
        .collect()
}

#[test]
fn trait_dates_are_the_fold_of_the_run_s_gains_through_the_hour() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut held: std::collections::BTreeMap<(u64, i32), (i64, i64)> = std::collections::BTreeMap::new();
    for hour in 0..=6 {
        run(&mut store, &mut writers, hour);
        if store.view(TableId::LeaderTraitGains.index()).status() == Status::Complete {
            for (leader, token, at, class) in gains(&store) {
                if class == i64::from(LeaderTraitGainRow::GAINED) || !held.contains_key(&(leader, token)) {
                    held.insert((leader, token), (at, class));
                }
            }
        }
        if hour == 6 {
            held.retain(|(leader, _), _| *leader != L2);
        }
        let expected: Vec<(u64, i32, i64, i64)> = held.iter().map(|(&(leader, token), &(at, class))| (leader, token, at, class)).collect();
        assert_eq!(dates(&store), expected, "hour {hour}");
    }
    assert!(dates(&store).iter().all(|date| date.0 == L1), "the deleted leader's dates are gone");
}
