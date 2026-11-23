use super::*;
use recorder_derive_framework::{
    forces::testing::{put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};

const FIRST: i32 = 20;
const LAST: i32 = 90;
const GROUP: u32 = 5;
const TAGS: [(&str, u32); 4] = [("ENG", 1), ("FRA", 2), ("GER", 3), ("USA", 4)];

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

fn span(kind: u8, group: u32, value: u32, ordinal: u32) -> GroupSpanRow {
    GroupSpanRow { kind, group, value, epoch: 0, ordinal }
}

fn groups(hour: i32) -> Vec<GroupSpanRow> {
    let mut spans = vec![span(GroupSpanRow::MEMBER, GROUP, 1, 0), span(GroupSpanRow::MEMBER, GROUP, 2, 1), span(GroupSpanRow::WAR, 1, 3, 0), span(GroupSpanRow::WAR, 2, 3, 0)];
    spans.push(span(GroupSpanRow::LEADER, GROUP, if hour < 52 { 1 } else { 2 }, 0));
    if (64..=65).contains(&hour) {
        spans.push(span(GroupSpanRow::INCOMPLETE, GROUP, 0, 0));
    }
    spans
}

fn scores(hour: i32) -> Vec<RaidAirScoreRow> {
    let mut rows = vec![score(2, 3, 5000, 7), score(3, 2, 150, 20)];
    if (30..=50).contains(&hour) {
        rows.push(score(1, 3, 10, 0));
    }
    if hour >= 60 {
        rows.push(score(4, 2, 40, 0));
    }
    rows.sort_by_key(|row| (row.country, row.region));
    rows
}

fn expected(hour: i32, region: u32) -> i64 {
    if (64..=65).contains(&hour) {
        return 0;
    }
    match region {
        3 if hour < 52 => if (30..=50).contains(&hour) { 10 } else { 0 },
        3 => 5000,
        2 => 150 + if hour >= 60 { 40 } else { 0 },
        _ => 0,
    }
}

fn feed(store: &mut Store, hour: i32) {
    store.begin_hour(hour, if hour == FIRST { &[b"x"] } else { &[] }).unwrap();
    let holders: Vec<TagHolderRow> = TAGS.iter().enumerate().map(|(at, (tag, id))| TagHolderRow { number: at as u32, epoch: 0, slot: *id, id: *id, tag: tag.as_bytes().to_vec() }).collect();
    let links: Vec<CountryLinkRow> = TAGS.iter().map(|(_, id)| CountryLinkRow { id: *id, canonical_slot: *id }).collect();
    put(store, hour, TableId::TagHolders, &holders, Status::Complete);
    put(store, hour, TableId::CountryLinks, &links, Status::Complete);
    put(store, hour, TableId::Groups, &[GroupRow { number: GROUP, faction: 7 }], Status::Complete);
    put(store, hour, TableId::GroupSpans, &groups(hour), Status::Complete);
    put(store, hour, TableId::RaidAirScores, &scores(hour), Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirRegionDays::default())]);
    writers.start_run();
    writers
}

fn activity(store: &Store, region: u32) -> Option<Activity> {
    let view = store.view(TableId::AirRegionDays.index());
    view.slot(u128::from(region)).and_then(|slot| Activity::read(&view, slot))
}

#[test]
fn a_regions_days_sum_the_sides_comparison_at_each_days_end() {
    let mut store = store();
    let mut writers = writers();
    let mut ended: HashMap<u32, i128> = HashMap::new();
    let mut previous: Option<i32> = None;
    for hour in FIRST..=LAST {
        let day = hour.div_euclid(DAY);
        if let Some(before) = previous.filter(|before| before.div_euclid(DAY) != day) {
            for region in [2, 3] {
                *ended.entry(region).or_default() += i128::from(expected(before, region));
            }
        }
        feed(&mut store, hour);
        writers.hour(1, hour, hour == FIRST, &mut store).unwrap();
        for region in [2u32, 3] {
            let held = activity(&store, region);
            let before = ended.get(&region).copied().unwrap_or_default();
            let (got_before, got_through) = held.map_or((0, 0), |row| (row.before(day), row.through(day)));
            assert_eq!((got_before, got_through), (before, before + i128::from(expected(hour, region))), "region {region} at hour {hour}");
        }
        previous = Some(hour);
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
            seen.push(rows(&store, TableId::AirRegionDays));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(AirRegionDays::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in [FIRST, 23, 47, 51, 64] {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {resume}");
    }
}
