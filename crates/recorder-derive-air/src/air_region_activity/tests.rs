use super::*;
use recorder_derive_framework::{
    forces::testing::{put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};

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

fn sums(hour: i32) -> HashMap<u32, (i128, i128)> {
    let mut sums: HashMap<u32, (i128, i128)> = HashMap::new();
    for row in world(hour) {
        let sum = sums.entry(row.region).or_default();
        sum.0 += i128::from(row.friendly.max(0));
        sum.1 += i128::from(row.enemy.max(0));
    }
    sums
}

fn feed(store: &mut Store, hour: i32) {
    store.begin_hour(hour, if hour == FIRST { &[b"x"] } else { &[] }).unwrap();
    put(store, hour, TableId::RaidAirScores, &world(hour), Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirRegionActivity::default())]);
    writers.start_run();
    writers
}

fn activity(store: &Store, region: u32) -> Option<Activity> {
    let view = store.view(TableId::AirRegionActivity.index());
    view.slot(u128::from(region)).and_then(|slot| Activity::read(&view, slot))
}

#[test]
fn a_regions_totals_are_its_sums_at_each_earlier_days_end() {
    let mut store = store();
    let mut writers = writers();
    let mut ended: HashMap<u32, (i128, i128)> = HashMap::new();
    let mut previous: Option<(i32, HashMap<u32, (i128, i128)>)> = None;
    for hour in FIRST..=LAST {
        let day = hour.div_euclid(DAY);
        if let Some((_, held)) = previous.take().filter(|(before, _)| before.div_euclid(DAY) != day) {
            for (region, (friendly, enemy)) in held {
                let total = ended.entry(region).or_default();
                total.0 += friendly;
                total.1 += enemy;
            }
        }
        feed(&mut store, hour);
        writers.hour(1, hour, hour == FIRST, &mut store).unwrap();
        let now = sums(hour);
        for region in [5, 6] {
            let row = activity(&store, region).unwrap_or_else(|| panic!("region {region} has a row at hour {hour}"));
            let before = ended.get(&region).copied().unwrap_or_default();
            let (friendly, enemy) = now.get(&region).copied().unwrap_or_default();
            assert_eq!(row.before(day), before, "region {region} before hour {hour}");
            assert_eq!(row.through(day), (before.0 + friendly, before.1 + enemy), "region {region} through hour {hour}");
        }
        previous = Some((hour, now));
    }
    assert_eq!(activity(&store, 7), None, "a region no owner scored in has no row");
}

#[test]
fn a_spans_daily_sums_are_two_reads() {
    let mut store = store();
    let mut writers = writers();
    let mut held: Vec<(i32, HashMap<u32, Activity>)> = Vec::new();
    for hour in FIRST..=LAST {
        feed(&mut store, hour);
        writers.hour(1, hour, hour == FIRST, &mut store).unwrap();
        held.push((hour, [5, 6].into_iter().filter_map(|region| Some((region, activity(&store, region)?))).collect()));
    }
    let day_end = |day: i32| (day * DAY + DAY - 1).min(LAST);
    for &(start, ref at_start) in &held {
        for &(end, ref at_end) in held.iter().filter(|(end, _)| *end >= start) {
            for region in [5u32, 6] {
                let mut expected = (0, 0);
                for day in start.div_euclid(DAY)..=end.div_euclid(DAY) {
                    let read = if day == end.div_euclid(DAY) { end } else { day_end(day) };
                    let (friendly, enemy) = sums(read).get(&region).copied().unwrap_or_default();
                    expected = (expected.0 + friendly, expected.1 + enemy);
                }
                let through = at_end.get(&region).map_or((0, 0), |row| row.through(end.div_euclid(DAY)));
                let before = at_start.get(&region).map_or((0, 0), |row| row.before(start.div_euclid(DAY)));
                assert_eq!((through.0 - before.0, through.1 - before.1), expected, "region {region} from {start} through {end}");
            }
        }
    }
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
            seen.push(rows(&store, TableId::AirRegionActivity));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(AirRegionActivity::default())]);
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
