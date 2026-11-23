use super::*;
use recorder_derive_framework::{
    forces::testing::{put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use std::collections::HashMap;

const FIRST: i32 = 20;
const LAST: i32 = 100;

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

fn world(hour: i32) -> Vec<RaidAirScoreRow> {
    let mut rows = vec![score(1, 5, i64::from((hour / 7) % 4) * 100, 10)];
    if hour.div_euclid(DAY) % 2 == 0 {
        rows.push(score(2, 5, -50, 30));
    }
    if hour >= 30 {
        rows.push(score(2, 6, 400, 0));
    }
    if hour <= 60 {
        rows.push(score(3, 6, 7, 3));
    }
    rows.sort_by_key(|row| (row.country, row.region));
    rows
}

const KEYS: [(u32, u32); 4] = [(1, 5), (2, 5), (2, 6), (3, 6)];

fn scores(hour: i32, (country, region): (u32, u32)) -> (i128, i128) {
    world(hour).iter().find(|row| (row.country, row.region) == (country, region)).map_or((0, 0), |row| (i128::from(row.friendly.max(0)), i128::from(row.enemy.max(0))))
}

fn feed(store: &mut Store, hour: i32) {
    store.begin_hour(hour, if hour == FIRST { &[b"x"] } else { &[] }).unwrap();
    put(store, hour, TableId::RaidAirScores, &world(hour), Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirOwnerDays::default())]);
    writers.start_run();
    writers
}

fn owner(store: &Store, (country, region): (u32, u32)) -> Option<OwnerDay> {
    let view = store.view(TableId::AirOwnerDays.index());
    view.slot(u128::from(country) << 32 | u128::from(region)).and_then(|slot| OwnerDay::read(&view, slot))
}

#[test]
fn an_owners_totals_in_a_region_are_its_scores_at_each_earlier_days_end() {
    let mut store = store();
    let mut writers = writers();
    let mut ended: HashMap<(u32, u32), (i128, i128)> = HashMap::new();
    let mut previous: Option<i32> = None;
    for hour in FIRST..=LAST {
        let day = hour.div_euclid(DAY);
        if let Some(before) = previous.filter(|before| before.div_euclid(DAY) != day) {
            for key in KEYS {
                let (friendly, enemy) = scores(before, key);
                let total = ended.entry(key).or_default();
                total.0 += friendly;
                total.1 += enemy;
            }
        }
        feed(&mut store, hour);
        writers.hour(1, hour, hour == FIRST, &mut store).unwrap();
        for key in KEYS {
            let before = ended.get(&key).copied().unwrap_or_default();
            let now = scores(hour, key);
            let held = owner(&store, key);
            let got = held.map_or(((0, 0), (0, 0)), |row| (row.before(day), row.through(day)));
            assert_eq!(got, (before, (before.0 + now.0, before.1 + now.1)), "owner {key:?} at hour {hour}");
        }
        previous = Some(hour);
    }
    assert!(owner(&store, (3, 6)).is_some(), "an owner's row keeps its totals after its entry goes");
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<i32>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for hour in FIRST..=LAST {
            feed(&mut store, hour);
            writers.hour(1, hour, hour == FIRST, &mut store).unwrap();
            seen.push(rows(&store, TableId::AirOwnerDays));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(AirOwnerDays::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in [FIRST, 23, 24, 47, 61] {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {resume}");
    }
}
