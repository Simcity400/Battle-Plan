use super::*;
use crate::products::air_superiority::assignment::{FactionHistory, Groups, Identity, Run, WarRun};
use dashboard_query::identity::{Holder, HolderRun};

const TAGS: [(&str, u32); 4] = [("ENG", 1), ("FRA", 2), ("GER", 3), ("USA", 4)];
const DAYS: usize = 12;
const LAST: i32 = DAYS as i32 * DAY - 1;

fn run(tag: &str) -> Run {
    Run { tag: tag.into(), first_hour: 0, last_hour: LAST }
}

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
        viewpoint: viewpoint.map(dashboard_base::products::Country::from),
        viewpoint_country: country.then_some(true),
        sides: sides.map(|keys| keys.iter().map(|key| (*key).to_owned()).collect()),
        ..AirTimelineParams::default()
    }
}

fn bounds(world: &[Vec<RaidAirScoreRow>], viewpoint: bool) -> HashMap<u32, u128> {
    let mut out: HashMap<u32, u128> = HashMap::new();
    for row in world.iter().flatten() {
        *out.entry(row.region).or_default() += row.friendly.max(0) as u128 + if viewpoint { row.enemy.max(0) as u128 } else { 0 };
    }
    out
}

const REGIONS: [u32; 4] = [1, 2, 3, 4];

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

#[test]
fn tile_viewpoint_clears_when_a_later_recorded_roster_shows_the_side_absent() {
    let world = world();
    let holders = holders();
    let held = Held(world.clone());
    let days = reads(&world, LAST);
    let cancelled = AtomicBool::new(false);
    let dissolved = Groups {
        factions: [(7, FactionHistory { first_hour: 0, members: vec![Run { tag: "ENG".into(), first_hour: 0, last_hour: 5 * DAY - 1 }], leaders: vec![Run { tag: "ENG".into(), first_hour: 0, last_hour: 5 * DAY - 1 }], incomplete: vec![] })].into(),
        wars: vec![],
        unobserved_factions: vec![],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: LAST, slot: *id, country_id: *id }])).collect(),
    };
    let dissolved_index = index::build(dissolved, 0, LAST);
    let side_view = params(AirMode::Viewpoint, Some("f:7"), false, None);
    let shown = Days { index: &dissolved_index, holders: &holders, days: &days, rows: &held, params: &side_view, theatres: None }
        .shown(&[1], &dissolved_index, &cancelled)
        .unwrap();
    assert_eq!(shown.viewpoint, None, "a later recorded roster without f:7 clears the carried viewpoint");

    let still_present = index();
    let theatres: Vec<Option<Result<HashSet<u32>, ()>>> = (0..DAYS).map(|day| Some(Ok(if day + 1 < DAYS { [1].into() } else { [99].into() }))).collect();
    let shown = Days { index: &still_present, holders: &holders, days: &days, rows: &held, params: &side_view, theatres: Some(&theatres) }
        .shown(&[1], &still_present, &cancelled)
        .unwrap();
    assert!(shown.viewpoint.is_some(), "a side still present in the roster keeps its viewpoint even when the region is outside theatres");

    let unobserved_end = Groups {
        factions: [(7, FactionHistory { first_hour: 0, members: vec![run("ENG")], leaders: vec![run("ENG")], incomplete: vec![] })].into(),
        wars: vec![],
        unobserved_factions: vec![(10 * DAY, LAST)],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: LAST, slot: *id, country_id: *id }])).collect(),
    };
    let unobserved_index = index::build(unobserved_end, 0, LAST);
    let shown = Days { index: &unobserved_index, holders: &holders, days: &days, rows: &held, params: &side_view, theatres: None }
        .shown(&[1], &unobserved_index, &cancelled)
        .unwrap();
    assert!(shown.viewpoint.is_some(), "an unobserved final roster does not establish side absence");
}

struct MockResolver {
    countries: HashMap<(String, i32), (Option<String>, Option<[u8; 3]>)>,
    factions: HashMap<(u64, i32), (Option<String>, Option<String>)>,
}

impl SideResolver for MockResolver {
    fn country(&mut self, tag: &str, hour: i32) -> dashboard_world::products::styles::ResolvedCountryLook {
        match self.countries.get(&(tag.to_owned(), hour)) {
            Some((name, color)) => dashboard_world::products::styles::ResolvedCountryLook {
                name: name.clone(),
                color: *color,
                name_missing: name.is_none().then(|| "Country name not recorded".to_owned()),
                color_missing: color.is_none().then(|| "No recorded map colour".to_owned()),
            },
            None => dashboard_world::products::styles::ResolvedCountryLook {
                name: None,
                color: None,
                name_missing: Some("Country not resolved".to_owned()),
                color_missing: Some("No recorded map colour".to_owned()),
            },
        }
    }

    fn faction_name(&self, faction_id: u64, hour: i32) -> (Option<String>, Option<String>) {
        self.factions
            .get(&(faction_id, hour))
            .cloned()
            .unwrap_or((None, Some("Faction name was not recorded at this hour".to_owned())))
    }
}

#[test]
fn acceptance_cases_2_3_4_8_12_cell_colours_names_and_legend_never_leak_future_or_off_chart() {
    let groups = Groups {
        factions: [
            (10, FactionHistory { first_hour: 0, members: vec![Run { tag: "GER".into(), first_hour: 0, last_hour: 191 }], leaders: vec![Run { tag: "GER".into(), first_hour: 0, last_hour: 191 }], incomplete: vec![] }),
            (20, FactionHistory { first_hour: 72, members: vec![Run { tag: "USA".into(), first_hour: 72, last_hour: 191 }, Run { tag: "FRA".into(), first_hour: 72, last_hour: 191 }], leaders: vec![Run { tag: "USA".into(), first_hour: 72, last_hour: 191 }], incomplete: vec![] }),
        ]
        .into(),
        wars: vec![
            WarRun { first: "GER".into(), second: "ENG".into(), first_hour: 72, last_hour: 191 },
            WarRun { first: "USA".into(), second: "ENG".into(), first_hour: 120, last_hour: 191 },
            WarRun { first: "FRA".into(), second: "ENG".into(), first_hour: 120, last_hour: 191 },
        ],
        unobserved_factions: vec![],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: 191, slot: *id, country_id: *id }])).collect(),
    };
    let index = index::build(groups, 0, 191);
    let holders = Holders::new(
        TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last: 191, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        vec![],
    );
    let mut world = vec![Vec::new(); 8];
    world[1] = vec![score(3, 5, 100, 0), score(1, 99, 500, 0)];
    world[3] = vec![score(3, 5, 100, 0), score(4, 5, 80, 0)];
    world[5] = vec![score(3, 5, 100, 0), score(4, 5, 80, 0)];
    let held = Held(world.clone());
    let mut resolver = MockResolver {
        countries: HashMap::from([
            (("GER".into(), 0), (Some("German Reich".into()), Some([90, 90, 90]))),
            (("USA".into(), 72), (Some("United States".into()), Some([220, 200, 140]))),
            (("ENG".into(), 47), (Some("United Kingdom".into()), Some([200, 60, 80]))),
        ]),
        factions: HashMap::from([
            ((10, 47), (None, Some("Unnamed before hour 72".into()))),
            ((10, 95), (Some("Axis".into()), None)),
            ((10, 143), (Some("Axis".into()), None)),
            ((20, 95), (Some("New League".into()), None)),
            ((20, 143), (Some("New League".into()), None)),
        ]),
    };
    let cancelled = AtomicBool::new(false);

    let early_days = reads(&world[..2], 47);
    let early_params = AirTimelineParams { last_hour: 47, ..params(AirMode::Sides, None, false, None) };
    let early = Days { index: &index, holders: &holders, days: &early_days, rows: &held, params: &early_params, theatres: None }
        .shown_with_resolver(&[5], &index, &mut resolver, &cancelled)
        .unwrap();
    assert_eq!(early.legend.len(), 1, "off-chart region 99 and future coalition never enter legend at H=47");
    let solo_at_47 = &early.sides[early.legend[0] as usize];
    assert_eq!(solo_at_47.key, "f:10");
    assert_eq!(solo_at_47.color, Some([90, 90, 90]));
    assert_eq!(solo_at_47.name, None, "unnamed faction at H=47 never falls back to later Axis name");
    assert_eq!(solo_at_47.name_missing.as_deref(), Some("Unnamed before hour 72"));

    let full_days = reads(&world[..6], 143);
    let full_params = AirTimelineParams { last_hour: 143, ..params(AirMode::Sides, None, false, None) };
    let full = Days { index: &index, holders: &holders, days: &full_days, rows: &held, params: &full_params, theatres: None }
        .shown_with_resolver(&[5], &index, &mut resolver, &cancelled)
        .unwrap();
    let day1_cell = full.cells[0].iter().flat_map(|(n, c)| std::iter::repeat_n(c, *n as usize)).nth(1).unwrap().as_ref().unwrap();
    let day5_cell = full.cells[0].iter().flat_map(|(n, c)| std::iter::repeat_n(c, *n as usize)).nth(5).unwrap().as_ref().unwrap();
    let (AirDayCell::Sides(d1), AirDayCell::Sides(d5)) = (day1_cell, day5_cell) else { panic!("expected sides cells") };
    assert_eq!(full.sides[d1.parts[0].side as usize].color, Some([90, 90, 90]), "early solo cell stays German grey after coalition forms");
    assert_eq!(full.sides[d5.parts[0].side as usize].color, Some([220, 200, 140]), "coalition cell uses majority colour anchor");
    assert_eq!(full.sides[d5.parts[0].side as usize].name.as_deref(), Some("Axis / New League"));
    let legend_f10 = full.legend.iter().map(|&idx| &full.sides[idx as usize]).find(|s| s.key == "f:10").unwrap();
    assert_eq!(legend_f10.name.as_deref(), Some("Axis / New League"), "legend reflects latest displayed metadata at H=143");
    assert_eq!(legend_f10.color, Some([220, 200, 140]));
}

#[test]
fn acceptance_case_8_two_founding_hours_of_same_tag_keep_distinct_colours() {
    let groups = Groups {
        factions: [
            (10, FactionHistory { first_hour: 0, members: vec![Run { tag: "GER".into(), first_hour: 0, last_hour: 47 }, Run { tag: "FRA".into(), first_hour: 0, last_hour: 119 }], leaders: vec![Run { tag: "GER".into(), first_hour: 0, last_hour: 47 }, Run { tag: "FRA".into(), first_hour: 48, last_hour: 119 }], incomplete: vec![] }),
            (20, FactionHistory { first_hour: 72, members: vec![Run { tag: "GER".into(), first_hour: 72, last_hour: 119 }], leaders: vec![Run { tag: "GER".into(), first_hour: 72, last_hour: 119 }], incomplete: vec![] }),
        ]
        .into(),
        wars: vec![],
        unobserved_factions: vec![],
        unobserved_wars: vec![],
        identities: TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![Identity { first_hour: 0, last_hour: 119, slot: *id, country_id: *id }])).collect(),
    };
    let index = index::build(groups, 0, 119);
    let holders = Holders::new(
        TAGS.iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last: 119, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        vec![],
    );
    let mut world = vec![Vec::new(); 4];
    world[3] = vec![score(2, 5, 100, 0), score(3, 5, 80, 0)];
    let held = Held(world.clone());
    let days = reads(&world, 95);
    let mut resolver = MockResolver {
        countries: HashMap::from([
            (("GER".into(), 0), (Some("German Empire".into()), Some([200, 40, 40]))),
            (("GER".into(), 72), (Some("German Republic".into()), Some([40, 80, 220]))),
        ]),
        factions: HashMap::from([
            ((10, 95), (Some("Old Pact".into()), None)),
            ((20, 95), (Some("New Pact".into()), None)),
        ]),
    };
    let p = AirTimelineParams { last_hour: 95, ..params(AirMode::Sides, None, false, None) };
    let shown = Days { index: &index, holders: &holders, days: &days, rows: &held, params: &p, theatres: None }
        .shown_with_resolver(&[5], &index, &mut resolver, &AtomicBool::new(false))
        .unwrap();
    let f10 = shown.sides.iter().find(|s| s.key == "f:10").unwrap();
    let f20 = shown.sides.iter().find(|s| s.key == "f:20").unwrap();
    assert_eq!(f10.color, Some([200, 40, 40]));
    assert_eq!(f20.color, Some([40, 80, 220]));
}

#[test]
fn a_known_side_beside_an_unknown_one_is_not_a_leader() {
    let cell = AirDayCell::Sides(AirDaySides { parts: vec![AirDayPart { side: 0, tag: "GER".into(), friendly: "71".into() }], viewpoints: vec![], unknown: vec![1], no_air_object: vec![], filtered: false });
    assert_eq!(readout_of(Some(&cell), Some(100)), AirReadout::NotRecorded { reason: "A side's air score was not recorded".into() });
}
