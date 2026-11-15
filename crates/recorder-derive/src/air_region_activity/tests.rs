use super::*;
use crate::{
    forces::testing::{put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};

const FIRST: i32 = 20;
const LAST: i32 = 100;

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

/// The scores at `hour`: a row moving every seven hours, one present on even days only with a
/// negative friendly score, one appearing at hour 30, and one gone after hour 60.
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

/// Each region's sums at `hour`, as the game's rows give them: a negative score as zero.
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

/// At every hour, a region's totals before its day are the sums at the last hour of each earlier
/// day of the run, and through it add the hour's sums: what a reader at any two hours takes the
/// sum of its days from. A negative score counts as zero; a region whose scores end keeps its
/// totals.
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

/// The sum of a region's days from one hour's day through another's is the difference of two
/// reads, whichever hours of those days are read.
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

/// A resume reads the scores whole: every hour after it equals a run that never stopped.
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
