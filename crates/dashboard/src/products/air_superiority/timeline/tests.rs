use super::*;
use crate::products::air_superiority::assignment::{FactionHistory, Groups, Identity, Run, WarRun};
use dashboard_query::identity::{Holder, HolderRun};

const TAGS: [(&str, u32); 4] = [("ENG", 1), ("FRA", 2), ("GER", 3), ("USA", 4)];
const DAYS: usize = 12;
const LAST: i32 = DAYS as i32 * DAY - 1;

fn run(tag: &str) -> Run {
    Run { tag: tag.into(), first_hour: 0, last_hour: LAST }
}

/// ENG leads faction 7 with FRA; both fight GER; USA is at peace. Each tag's raw id is its slot.
fn index() -> AirIndex {
    let groups = Groups {
        factions: [(7, FactionHistory { first_hour: 0, members: vec![run("ENG"), run("FRA")], leaders: vec![run("ENG")], incomplete: vec![] })].into(),
        wars: ["ENG", "FRA"].map(|tag| WarRun { first: tag.into(), second: "GER".into(), first_hour: 0, last_hour: LAST }).into(),
        unobserved_factions: vec![],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: LAST, slot: *id, country_id: *id }])).collect(),
    };
    index::build(groups, 0, LAST)
}

fn holders() -> Holders {
    Holders {
        tags: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last: LAST, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        unknown: vec![],
    }
}

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

/// Each day's rows at its end. Region 1 has one big day (ENG, 1,000 on day 3); region 2 many
/// small ones (GER, 150 on days 0 to 9); in region 3 FRA, a member its leader speaks for, scores
/// 5,000 every day and ENG 10 on day 0, so its bound is far above its sum; region 4 has USA's
/// 1,100 on the last day alone. Enemy scores give the viewpoints something to read. ENG, GER and
/// USA hold a zero row in region 9 every day: each has an air object, so a region they have no row
/// in is a known zero for them.
fn world() -> Vec<Vec<RaidAirScoreRow>> {
    (0..DAYS as i32)
        .map(|day| {
            let mut rows = vec![score(1, 9, 0, 0), score(3, 9, 0, 0), score(4, 9, 0, 0)];
            if day == 3 {
                rows.push(score(1, 1, 1000, 40));
            }
            if day <= 9 {
                rows.push(score(3, 2, 150, 20));
            }
            rows.push(score(2, 3, 5000, 7));
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

struct Held(Vec<Vec<RaidAirScoreRow>>);

impl RegionRows for Held {
    fn rows(&self, day: usize, region: u32, out: &mut Vec<RaidAirScoreRow>) {
        out.extend(self.0[day].iter().filter(|row| row.region == region).cloned());
    }
}

fn reads(world: &[Vec<RaidAirScoreRow>], last: i32) -> Vec<Option<DayRead>> {
    let links = Arc::new(TAGS.iter().map(|(_, id)| CountryLinkRow { id: *id, canonical_slot: *id }).collect::<Vec<_>>());
    world
        .iter()
        .enumerate()
        .map(|(day, rows)| {
            let hour = (day as i32 * DAY + DAY - 1).min(last);
            Some(DayRead { hour, witnesses: rows.iter().map(|row| (row.country, row.region, row.friendly, row.enemy)).collect(), complete: true, links: Some(Arc::clone(&links)) })
        })
        .collect()
}

fn params(mode: AirMode, viewpoint: Option<&str>, country: bool, sides: Option<&[&str]>) -> AirTimelineParams {
    AirTimelineParams {
        branch_id: "1".into(),
        first_hour: 0,
        last_hour: LAST,
        mode: Some(mode),
        viewpoint: viewpoint.map(String::from),
        viewpoint_country: country.then_some(true),
        sides: sides.map(|keys| keys.iter().map(|key| (*key).to_owned()).collect()),
        ..AirTimelineParams::default()
    }
}

/// What `AirRegionActivity` bounds a region by over the view: every owner's scores summed, a
/// negative one as zero, at the end of every day (the friendly ones alone for the sides).
fn bounds(world: &[Vec<RaidAirScoreRow>], viewpoint: bool) -> HashMap<u32, u128> {
    let mut out: HashMap<u32, u128> = HashMap::new();
    for row in world.iter().flatten() {
        *out.entry(row.region).or_default() += row.friendly.max(0) as u128 + if viewpoint { row.enemy.max(0) as u128 } else { 0 };
    }
    out
}

const REGIONS: [u32; 4] = [1, 2, 3, 4];

/// Main ranks the most active regions by the sum over the view's completed days of each day's
/// composed activity: many small days above one big day, a region whose scores are a member's
/// its leader speaks for low however great they are, and a last day not yet over not counted.
#[test]
fn regions_rank_by_their_sum_of_composed_days_as_main_ranks_them() {
    let world = world();
    let index = index();
    let holders = holders();
    let held = Held(world.clone());
    let cancelled = AtomicBool::new(false);
    for (last, expected) in [(LAST, [2, 4, 1, 3]), (LAST - 18, [2, 1, 3, 4])] {
        let days = reads(&world, last);
        let params = AirTimelineParams { last_hour: last, ..params(AirMode::Sides, None, false, None) };
        let timeline = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &params, theatres: None };
        let counted = if last.rem_euclid(DAY) == DAY - 1 { DAYS } else { DAYS - 1 };
        let totals = timeline.totals(&REGIONS, counted, &cancelled).unwrap();
        assert_eq!(totals[..3], [1000, 1500, 10], "ENG's big day, GER's small ones, ENG's ten under FRA's member scores");
        let bounds = bounds(&world, false);
        for top in 0..=REGIONS.len() {
            let ranked = timeline.rank(&REGIONS, Some(&bounds), counted, Some(top), &cancelled).unwrap();
            assert_eq!(ranked, expected[..top], "the {top} most active through hour {last}");
            assert_eq!(timeline.rank(&REGIONS, None, counted, Some(top), &cancelled).unwrap(), ranked, "the bound only saves composing");
        }
        assert_eq!(timeline.rank(&[4, 1], None, counted, None, &cancelled).unwrap(), expected.iter().copied().filter(|region| [4, 1].contains(region)).collect::<Vec<_>>(), "chosen regions are ranked too");
    }
}

/// No composition exceeds the sums of every owner's scores, in any mode: the bound holds.
#[test]
fn every_composition_is_within_every_owners_summed_scores() {
    let world = world();
    let index = index();
    let holders = holders();
    let held = Held(world.clone());
    let days = reads(&world, LAST);
    let cancelled = AtomicBool::new(false);
    let theatres: Vec<Option<Result<HashSet<u32>, ()>>> = (0..DAYS).map(|day| Some(Ok(if day < 6 { [1, 2, 3].into() } else { [4].into() }))).collect();
    let cases = [
        params(AirMode::Sides, None, false, None),
        params(AirMode::Sides, None, false, Some(&["c:GER"])),
        params(AirMode::Viewpoint, Some("f:7"), false, None),
        params(AirMode::Viewpoint, Some("FRA"), true, None),
        params(AirMode::Viewpoint, Some("GER"), true, None),
    ];
    for params in &cases {
        for theatres in [None, Some(theatres.as_slice())] {
            let timeline = Days { index: &index, holders: &holders, days: &days, rows: &held, params, theatres };
            let bounds = bounds(&world, params.mode == Some(AirMode::Viewpoint));
            let totals = timeline.totals(&REGIONS, DAYS, &cancelled).unwrap();
            for (region, total) in REGIONS.iter().zip(totals) {
                assert!(total <= bounds[region], "region {region}: {total} over its bound {} ({params:?}, theatres {})", bounds[region], theatres.is_some());
            }
            let all = timeline.rank(&REGIONS, None, DAYS, Some(2), &cancelled).unwrap();
            assert_eq!(timeline.rank(&REGIONS, Some(&bounds), DAYS, Some(2), &cancelled).unwrap(), all, "{params:?}");
        }
    }
    let params = params(AirMode::Sides, None, false, None);
    let timeline = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &params, theatres: Some(&theatres) };
    assert_eq!(timeline.totals(&REGIONS, DAYS, &cancelled).unwrap(), [1000, 900, 10, 1100], "a day counts only where its theatres hold the region");
}

/// Each day's cell is the moment's composition at its end; a day not read has none, equal days
/// are one run, and the sides named are each named once.
#[test]
fn a_regions_days_are_the_moments_composition_at_each_days_end() {
    let world = world();
    let index = index();
    let holders = holders();
    let held = Held(world.clone());
    let mut days = reads(&world, LAST);
    days[5] = None;
    let cancelled = AtomicBool::new(false);
    let params = params(AirMode::Sides, None, false, None);
    let timeline = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &params, theatres: None };
    let shown = timeline.shown(&[2, 3], &index, &cancelled).unwrap();
    let keys: Vec<&str> = shown.sides.iter().map(|side| side.key.as_str()).collect();
    let ger = keys.iter().position(|key| *key == "c:GER").unwrap() as u32;
    let eng = keys.iter().position(|key| *key == "f:7").unwrap() as u32;
    let sides = |parts: Vec<AirDayPart>| Some(AirDayCell::Sides(AirDaySides { parts, viewpoints: vec![], unknown: vec![], no_air_object: vec![], filtered: false }));
    let ger_part = sides(vec![AirDayPart { side: ger, tag: "GER".into(), friendly: "150".into() }]);
    let none = sides(vec![]);
    assert_eq!(shown.cells[0], [(5, ger_part.clone()), (1, None), (4, ger_part), (2, none.clone())], "region 2: GER's days, a day not read, then no activity");
    let eng_part = sides(vec![AirDayPart { side: eng, tag: "ENG".into(), friendly: "10".into() }]);
    assert_eq!(shown.cells[1], [(1, eng_part), (4, none.clone()), (1, None), (6, none)], "region 3: FRA's scores are its leader's read, which is ENG's");
    assert!(shown.excluded.iter().all(|&excluded| excluded == 0));
    let mut named = keys.clone();
    named.dedup();
    assert_eq!(named.len(), keys.len(), "each side named once");
}

/// A viewpoint's cells carry its friendly and enemy scores, zero where it has no row; a side
/// filter keeps the chosen sides' parts alone and says so.
#[test]
fn a_viewpoints_days_carry_its_scores_and_a_filter_keeps_its_sides() {
    let world = world();
    let index = index();
    let holders = holders();
    let held = Held(world.clone());
    let days = reads(&world, LAST);
    let cancelled = AtomicBool::new(false);
    let country = params(AirMode::Viewpoint, Some("GER"), true, None);
    let timeline = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &country, theatres: None };
    let shown = timeline.shown(&[2], &index, &cancelled).unwrap();
    let view = |friendly: &str, enemy: &str| Some(AirDayCell::View { side: None, leader: Some("GER".into()), friendly: friendly.into(), enemy: enemy.into() });
    assert_eq!(shown.cells[0], [(10, view("150", "20")), (2, view("0", "0"))]);
    let filtered = params(AirMode::Sides, None, false, Some(&["c:GER"]));
    let timeline = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &filtered, theatres: None };
    let shown = timeline.shown(&[3], &index, &cancelled).unwrap();
    assert_eq!(shown.cells[0], [(DAYS as u32, Some(AirDayCell::Sides(AirDaySides { parts: vec![], viewpoints: vec![], unknown: vec![], no_air_object: vec![], filtered: true })))], "ENG's part is not a chosen side's");
}

/// Main counts each day once, at its last recorded hour: a stretch cut by a load inside a day leaves
/// that day to the stretch after it, and a last day not yet over is not counted. `AirRegionDays`'
/// rows at each stretch's two edges give exactly those sums.
#[test]
fn a_regions_days_are_summed_once_each_from_two_reads_a_stretch() {
    let row = |day, activity, total| DayActivity { day, activity, total };
    // Days 0 and 1 end at 5; day 2 reaches 7 before the load at hour 60 cuts it.
    let cut = StretchDays { first_day: 0, last_day: 2, counts_last: false, start: [(1, row(0, 5, 0))].into(), end: [(1, row(2, 7, 10)), (2, row(2, 3, 0))].into() };
    // The run after the load: day 2 ends at 7, day 3 at 8, day 4 (not over) reads 9.
    let after = StretchDays { first_day: 2, last_day: 4, counts_last: false, start: [(1, row(2, 7, 0))].into(), end: [(1, row(4, 9, 15))].into() };
    let sums = day_sums(&[cut, after]);
    assert_eq!(sums[&1], 5 + 5 + 7 + 8, "days 0 to 3, day 2 once, day 4 not over");
    assert_eq!(sums[&2], 0, "a region first scored in a day the next stretch ends counts there");
    let over = StretchDays { first_day: 2, last_day: 4, counts_last: true, start: [(1, row(2, 7, 0))].into(), end: [(1, row(4, 9, 15))].into() };
    assert_eq!(day_sums(&[over])[&1], 7 + 8 + 9, "a last day over counts");
}
