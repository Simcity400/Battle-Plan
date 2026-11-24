use super::*;
use crate::products::air_superiority::{
    assignment::{FactionHistory, Groups, Identity, Run, WarRun},
    index,
    timeline::{DayRead, Days, RegionRows},
};
use dashboard_query::{identity::HolderRun, timeline::Join};
use replay_data::rows::{CountryLinkRow, RaidAirScoreRow};
use std::sync::{Arc, atomic::AtomicBool};

const TAGS: [(&str, u32); 4] = [("ENG", 1), ("FRA", 2), ("GER", 3), ("USA", 4)];
const DAYS: usize = 12;
const LAST: i32 = DAYS as i32 * DAY - 1;
const HANDOVER: i32 = 5 * DAY + 12;
const REGIONS: [u32; 4] = [1, 2, 3, 4];

fn run(tag: &str, first_hour: i32, last_hour: i32) -> Run {
    Run { tag: tag.into(), first_hour, last_hour }
}

fn index() -> AirIndex {
    let groups = Groups {
        factions: [(7, FactionHistory { first_hour: 0, members: vec![run("ENG", 0, LAST), run("FRA", 0, LAST)], leaders: vec![run("ENG", 0, HANDOVER - 1), run("FRA", HANDOVER, LAST)], incomplete: vec![] })].into(),
        wars: ["ENG", "FRA"].map(|tag| WarRun { first: tag.into(), second: "GER".into(), first_hour: 0, last_hour: LAST }).into(),
        unobserved_factions: vec![],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: LAST, slot: *id, country_id: *id }])).collect(),
    };
    index::build(groups, 0, LAST)
}

fn holders() -> Holders {
    Holders::new(
        TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last: LAST, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        vec![],
    )
}

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

fn world() -> Vec<Vec<RaidAirScoreRow>> {
    (0..DAYS as i32)
        .map(|day| {
            let mut rows = vec![score(1, 9, 0, 0), score(3, 9, 0, 0), score(4, 9, 0, 0), score(2, 3, 5000, 7)];
            if day == 3 {
                rows.push(score(1, 1, 1000, 40));
            }
            if day <= 9 {
                rows.push(score(3, 2, 150, 20));
            }
            if day == 0 {
                rows.push(score(1, 3, 10, 0));
            }
            if day == 11 {
                rows.push(score(4, 4, 1100, 0));
            }
            rows
        })
        .collect()
}

fn member(day: usize) -> HashSet<u32> {
    if day <= 5 { [1, 2, 3].into() } else { [4].into() }
}

struct Held(Vec<Vec<RaidAirScoreRow>>);

impl RegionRows for Held {
    fn rows(&self, day: usize, region: u32, out: &mut Vec<RaidAirScoreRow>) {
        out.extend(self.0[day].iter().filter(|row| row.region == region).cloned());
    }
}

fn reads(world: &[Vec<RaidAirScoreRow>]) -> Vec<Option<DayRead>> {
    let links = Arc::new(TAGS.iter().map(|(_, id)| CountryLinkRow { id: *id, canonical_slot: *id }).collect::<Vec<_>>());
    world
        .iter()
        .enumerate()
        .map(|(day, rows)| Some(DayRead { hour: day as i32 * DAY + DAY - 1, witnesses: rows.iter().map(|row| (row.country, row.region, row.friendly, row.enemy)).collect(), complete: true, links: Some(Arc::clone(&links)) }))
        .collect()
}

struct Tables {
    world: Vec<Vec<RaidAirScoreRow>>,
    activity: Vec<HashMap<u32, i64>>,
}

impl Ledger for Tables {
    fn read(&self, _table: TableId, _run: u32, _first: i32, _last: i32) -> bool {
        true
    }

    fn owner(&self, _run: u32, hour: i32, raw: u32) -> Option<HashMap<u32, OwnerDay>> {
        let day = hour.div_euclid(DAY) as usize;
        let value = |at: usize, region: u32| self.world[at].iter().find(|row| (row.country, row.region) == (raw, region)).map_or((0, 0), |row| (row.friendly.max(0), row.enemy.max(0)));
        let regions: BTreeSet<u32> = self.world.iter().flatten().filter(|row| row.country == raw).map(|row| row.region).collect();
        Some(
            regions
                .into_iter()
                .map(|region| {
                    let (friendly, enemy) = value(day, region);
                    let (friendly_total, enemy_total) = (0..day).map(|at| value(at, region)).fold((0, 0), |(f, e), (a, b)| (f + a, e + b));
                    (region, OwnerDay { day: day as i32, friendly, enemy, friendly_total, enemy_total })
                })
                .collect(),
        )
    }

    fn regions(&self, _run: u32, hour: i32) -> Option<HashMap<u32, DayActivity>> {
        let day = hour.div_euclid(DAY) as usize;
        let regions: BTreeSet<u32> = self.activity.iter().flat_map(|day| day.keys().copied()).collect();
        let at = |at: usize, region: u32| self.activity[at].get(&region).copied().unwrap_or(0);
        Some(regions.into_iter().map(|region| (region, DayActivity { day: day as i32, activity: at(day, region), total: (0..day).map(|before| at(before, region)).sum() })).collect())
    }

    fn theatres(&self, _run: u32, first: i32, last: i32, theatres: &[u64]) -> Option<(Vec<(i32, i32)>, Vec<(u32, i32, i32)>)> {
        let mut runs = Vec::new();
        if theatres.contains(&1) {
            for region in REGIONS {
                let mut start: Option<usize> = None;
                for day in 0..=DAYS {
                    let held = day < DAYS && member(day).contains(&region);
                    match (held, start) {
                        (true, None) => start = Some(day),
                        (false, Some(from)) => {
                            runs.push((region, (from as i32 * DAY).max(first), (day as i32 * DAY - 1).min(last)));
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        Some((vec![(first, last)], runs))
    }
}

fn params(mode: AirMode, viewpoint: Option<&str>, country: bool, sides: &[&str], theatres: bool) -> AirTimelineParams {
    AirTimelineParams {
        branch_id: "1".into(),
        first_hour: 0,
        last_hour: LAST,
        mode: Some(mode),
        viewpoint: viewpoint.map(dashboard_base::products::Country::from),
        viewpoint_country: country.then_some(true),
        sides: (!sides.is_empty()).then(|| sides.iter().map(|key| (*key).to_owned()).collect()),
        theatres: theatres.then(|| vec!["1".to_owned()]),
        ..AirTimelineParams::default()
    }
}

#[test]
fn each_modes_ranking_from_the_tables_equals_the_ranking_composed_from_the_rows() {
    let world = world();
    let index = index();
    let holders = holders();
    let held = Held(world.clone());
    let days = reads(&world);
    let cancelled = AtomicBool::new(false);
    let theatres: Vec<Option<Result<HashSet<u32>, ()>>> = (0..DAYS).map(|day| Some(Ok(member(day)))).collect();
    let plain = params(AirMode::Sides, None, false, &[], false);
    let composer = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &plain, theatres: None };
    let mut activity = Vec::new();
    let mut before = vec![0u128; REGIONS.len()];
    for day in 1..=DAYS {
        let through = composer.totals(&REGIONS, day, &cancelled).unwrap();
        activity.push(REGIONS.iter().zip(through.iter().zip(&before)).map(|(&region, (now, was))| (region, (now - was) as i64)).collect::<HashMap<_, _>>());
        before = through;
    }
    let tables = Tables { world: world, activity };
    let stretches = [Stretch { run: 0, first: 0, last: LAST, join: Join::Start }];
    let rank = |sums: &HashMap<u32, u128>| {
        let mut ranked: Vec<(u32, u128)> = REGIONS.iter().map(|region| (*region, sums.get(region).copied().unwrap_or(0))).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
    };
    for with_theatres in [false, true] {
        for params in [
            params(AirMode::Sides, None, false, &[], with_theatres),
            params(AirMode::Sides, None, false, &["c:GER"], with_theatres),
            params(AirMode::Sides, None, false, &["f:7", "c:USA"], with_theatres),
            params(AirMode::Viewpoint, Some("f:7"), false, &[], with_theatres),
            params(AirMode::Viewpoint, Some("FRA"), true, &[], with_theatres),
            params(AirMode::Viewpoint, Some("GER"), true, &[], with_theatres),
        ] {
            let composed = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &params, theatres: with_theatres.then_some(theatres.as_slice()) };
            let expected: HashMap<u32, u128> = REGIONS.iter().copied().zip(composed.totals(&REGIONS, DAYS, &cancelled).unwrap()).collect();
            let read = sums(&tables, &params, &index, &holders, &stretches, LAST).expect("the tables give every mode here");
            assert_eq!(rank(&read), rank(&expected), "{params:?}");
        }
    }
}

#[test]
fn a_view_is_cut_where_its_reads_change_and_each_day_counts_once() {
    let stretches = [Stretch { run: 0, first: 0, last: 100, join: Join::Start }, Stretch { run: 1, first: 101, last: 130, join: Join::Break }];
    let cut = pieces(&stretches, 130, &[60, 120].into());
    assert_eq!(cut.iter().map(|piece| (piece.run, piece.first, piece.last, piece.counts_last)).collect::<Vec<_>>(),
        [(0, 0, 59, false), (0, 60, 100, false), (1, 101, 119, true), (1, 120, 130, false)]);
}
