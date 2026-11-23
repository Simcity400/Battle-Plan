use super::*;
use crate::{
    fixture::{self, *},
    tile::{self, BINS, PRESENT, RUN},
};
use dashboard_declare::{Carry, Column, Key};
use replay_columnar::summary::Cells;
use replay_data::rows::TableId;

fn resources_line<'q, 'r>(query: &'q Query<'r>, measure: &str) -> Line<'q, 'r> {
    let series = dashboard_declare::series("country-resources").unwrap();
    Line::new(query, series, series.measure(measure).unwrap()).unwrap()
}

fn produced(query: &Query<'_>, id: u32, resource: u32, hour: i32) -> Option<i64> {
    (held(query, tag_of(id), hour) && fixture::resources(id, hour).contains(&resource)).then(|| fixture::resource(id, resource, hour).produced)
}

fn read(tag: &str, hour: i32, part: Option<u32>) -> SeriesRead {
    SeriesRead { country: tag.into(), hour, part: part.map(|part| part.to_string()) }
}

const HOURS: [i32; 9] = [FIRST, FIRST + 1_999, FIRST + 2_000, FIRST + 4_321, FIRST + 5_999, ITA_FROM + 3, FIRST + FIRST_HOURS - 1, SECOND + 17, LAST];

#[test]
fn a_state_of_one_part_is_the_parts_row_at_the_hour() {
    let fixture = Fixture::campaign(Data::Resources);
    let query = fixture.query();
    let line = resources_line(&query, "produced");
    let mut reads = Vec::new();
    for hour in HOURS.into_iter().chain([FIRST + FIRST_HOURS + 5]) {
        for (id, resource) in [(9001, 1), (9001, 4), (9002, 2)] {
            reads.push((id, resource, read(tag_of(id), hour, Some(resource))));
        }
    }
    let states = line.states(&reads.iter().map(|(.., read)| read.clone()).collect::<Vec<_>>(), None).unwrap();
    let mut valued = 0;
    for ((id, resource, read), state) in reads.iter().zip(&states) {
        assert_eq!(state.part.as_deref(), Some(resource.to_string().as_str()));
        let wanted = produced(&query, *id, *resource, read.hour);
        let got = state.run.as_ref().map(|run| run.value_raw.parse::<i64>().unwrap());
        assert_eq!(got, wanted, "{} part {resource} at {}: {state:?}", read.country, read.hour);
        if let Some(run) = &state.run {
            assert!(run.first_hour <= read.hour && read.hour <= run.last_hour);
            for hour in run.first_hour..=run.last_hour {
                assert_eq!(produced(&query, *id, *resource, hour), wanted, "the run holds one value at {hour}");
            }
            valued += 1;
        } else {
            assert!(wanted.is_none());
        }
    }
    assert!(valued >= 12, "{valued} values compared");
    assert!(states.iter().any(|state| state.missing.as_deref() == Some(GAP_REASON)));
}

#[test]
fn a_state_of_every_part_and_of_the_top_parts_is_the_owners_rows_at_the_hour() {
    let fixture = Fixture::campaign(Data::Resources);
    let query = fixture.query();
    let line = resources_line(&query, "produced");
    for hour in HOURS {
        for id in [9001, 9002] {
            let tag = tag_of(id);
            let states = line.states(&[read(tag, hour, None)], None).unwrap();
            let wanted: Vec<(u32, i64)> = fixture::resources(id, hour).into_iter().filter_map(|resource| Some((resource, produced(&query, id, resource, hour)?))).collect();
            if wanted.is_empty() {
                assert_eq!(states.len(), 1, "{tag} at {hour}: one state saying why");
                assert!(states[0].missing.is_some() && states[0].run.is_none());
                continue;
            }
            let got: Vec<(u32, i64)> = states.iter().map(|state| (state.part.as_ref().unwrap().parse().unwrap(), state.run.as_ref().unwrap().value_raw.parse().unwrap())).collect();
            assert_eq!(got, wanted, "{tag}'s parts at {hour}");
            let mut ranked = wanted.clone();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            ranked.truncate(2);
            let top = line.states(&[read(tag, hour, None)], Some(2)).unwrap();
            let got: Vec<(u32, i64)> = top.iter().map(|state| (state.part.as_ref().unwrap().parse().unwrap(), state.run.as_ref().unwrap().value_raw.parse().unwrap())).collect();
            assert_eq!(got, ranked, "{tag}'s top two parts at {hour}");
        }
    }
    let whole = resources_line(&query, "produced");
    assert!(whole.asks(&["GER".into()], &[]).is_err(), "a sum of parts is not offered");
}

#[test]
fn tiles_of_two_parts_hold_each_parts_values() {
    let fixture = Fixture::new(Data::Resources, FIRST..=FIRST + FIRST_HOURS - 1);
    let query = fixture.query();
    let line = resources_line(&query, "produced");
    let asks = vec![Ask { country: "GER".into(), part: 4 }, Ask { country: "ITA".into(), part: 2 }];
    let ids = [(9001, 4), (9002, 2)];
    let scale = line.scale();
    let mut compared = 0;
    for index in [i64::from(FIRST + 1_990) / 256, i64::from(FIRST + 5_990) / 256, i64::from(ITA_FROM) / 256] {
        let reply = tile::tile(&line, "1", &asks, None, 1, index, false).unwrap();
        assert_eq!(reply.lines.iter().map(|line| line.part.clone().unwrap()).collect::<Vec<_>>(), ["4", "2"]);
        let bytes = unbase64(&reply.bins);
        let runs = tile::decode_runs(&bytes, asks.len(), reply.first_hour, scale, reply.raw_values).unwrap();
        for (at, &(id, resource)) in ids.iter().enumerate() {
            for bin in 0..BINS {
                let hour = reply.first_hour + bin as i32;
                let flag = bytes[at * BINS + bin];
                let wanted = produced(&query, id, resource, hour);
                assert_eq!(flag & PRESENT != 0, wanted.is_some(), "{id} part {resource} at {hour}");
                if let Some(wanted) = wanted {
                    let run = runs[at].iter().find(|run| run.0 <= hour && hour <= run.1).expect("a run holds the hour");
                    assert_eq!(run.2, wanted as f64 / scale, "{id} part {resource} at {hour}");
                    compared += 1;
                }
                if flag & RUN != 0 {
                    assert!(runs[at].iter().any(|run| run.0 == hour));
                }
            }
        }
    }
    for index in [i64::from(FIRST + 1_000) / (256 * 6), i64::from(ITA_FROM) / (256 * 6)] {
        let reply = tile::tile(&line, "1", &asks, None, 6, index, false).unwrap();
        let buckets = tile::decode(&unbase64(&reply.bins), asks.len(), reply.first_hour, 6, scale, reply.raw_values, reply.hour_bytes).unwrap();
        for (at, &(id, resource)) in ids.iter().enumerate() {
            for bin in 0..BINS {
                let start = reply.first_hour + 6 * bin as i32;
                if query.timeline.at(start).zip(query.timeline.at(start + 5)).is_none_or(|(a, b)| a != b) {
                    continue;
                }
                let values: Vec<i64> = (start..start + 6).filter_map(|hour| produced(&query, id, resource, hour)).collect();
                let bucket = buckets[at * BINS + bin];
                assert_eq!(bucket.is_some(), !values.is_empty(), "{id} part {resource} bin from {start}");
                let Some(bucket) = bucket else { continue };
                let wanted = [values[0], *values.last().unwrap(), *values.iter().min().unwrap(), *values.iter().max().unwrap()].map(|value| value as f64 / scale);
                assert_eq!([bucket.first_value, bucket.last_value, bucket.min_value, bucket.max_value], wanted, "{id} part {resource} bin from {start}");
                compared += 1;
            }
        }
    }
    assert!(compared > 1_000, "{compared} hours and bins compared");
}

#[test]
fn a_chart_names_the_hours_its_line_holds_no_row_in_and_its_table_was_not_available() {
    let fixture = Fixture::campaign(Data::Resources);
    let query = fixture.query();
    let line = resources_line(&query, "produced");
    let rowless = |first: i32, last: i32| MissingSpan { first_hour: first, last_hour: last, reason: "GER's produced was not recorded here".into() };
    let gap = MissingSpan { first_hour: FIRST + FIRST_HOURS, last_hour: SECOND - 1, reason: GAP_REASON.into() };
    let chart = |part: u128, from: i32, to: i32, width: u32| line.summaries_within(&[(Ask { country: "GER".into(), part }, vec![(from, to)])], from, to, width).unwrap().remove(0);
    for width in [64, 600] {
        let (never, missing) = chart(5, FIRST, LAST, width);
        assert!(never.buckets.iter().all(Option::is_none), "no row, nothing drawn");
        assert_eq!(missing, [rowless(FIRST, FIRST + FIRST_HOURS - 1), gap.clone(), rowless(SECOND, LAST)], "a part never held, {width} wide");
        let (_, missing) = chart(4, FIRST, LAST, width);
        assert_eq!(
            missing,
            [rowless(FIRST, FIRST + 1_999), rowless(FIRST + 6_000, FIRST + FIRST_HOURS - 1), gap.clone(), rowless(SECOND, LAST)],
            "a part held for a while, {width} wide"
        );
    }
    let (_, hourly) = chart(5, FIRST, FIRST + 99, 100);
    assert_eq!(hourly, [rowless(FIRST, FIRST + 99)], "an hourly view");
    let (_, inside) = chart(4, FIRST + 2_000, FIRST + 2_099, 100);
    assert!(inside.is_empty(), "hours its row holds: {inside:?}");
    let index = i64::from(FIRST + 3_000).div_euclid(BINS as i64 * 6);
    let reply = tile::tile(&line, "1", &[Ask { country: "GER".into(), part: 5 }], None, 6, index, false).unwrap();
    let last = reply.first_hour + (BINS * 6) as i32 - 1;
    assert_eq!(reply.lines[0].missing, [rowless(reply.first_hour, last)], "a tile of a part never held");

    let experience = dashboard_declare::series("country-experience").unwrap();
    let line = Line::new(&query, experience, experience.measure("army").unwrap()).unwrap();
    let unavailable = |first: i32, last: i32| MissingSpan { first_hour: first, last_hour: last, reason: format!("{}: not available at this hour", TableId::CountryExperience.words()) };
    let (summary, missing) = line.summary("GER", FIRST, LAST, 64).unwrap();
    assert!(summary.buckets.iter().all(Option::is_none));
    assert_eq!(missing, [unavailable(FIRST, FIRST + FIRST_HOURS - 1), gap, unavailable(SECOND, LAST)], "a table the recording never held");
}

#[test]
fn the_next_change_of_a_part_is_the_next_hour_its_row_moves_comes_or_goes() {
    let fixture = Fixture::new(Data::Resources, FIRST..=FIRST + FIRST_HOURS - 1);
    let query = fixture.query();
    let line = resources_line(&query, "produced");
    let measure = line.series().measure("produced").unwrap();
    let bounds = (FIRST, FIRST + FIRST_HOURS - 1);
    for (id, resource, origin) in [(9001, 4, FIRST + 1_000), (9001, 4, FIRST + 5_950), (9001, 1, FIRST + 3_001), (9002, 2, ITA_FROM + 100), (9002, 1, ITA_FROM - 400)] {
        let ask = Ask { country: tag_of(id).into(), part: u128::from(resource) };
        let found = line.navigate_lines(std::slice::from_ref(&ask), &[measure], Some(origin), NavigationKey::ArrowRight, bounds, &Within::new()).unwrap();
        let held = |hour: i32| held(&query, tag_of(id), hour);
        let first_held = (bounds.0..=bounds.1).find(|&hour| held(hour)).unwrap();
        let wanted = (origin + 1..=bounds.1).find(|&hour| hour == first_held || (hour > first_held && produced(&query, id, resource, hour) != produced(&query, id, resource, hour - 1)));
        assert_eq!(line.change_lines(std::slice::from_ref(&ask), &[measure], Some(origin), NavigationKey::ArrowRight, bounds, &Within::new()).unwrap(), wanted, "{} part {resource} after {origin}: none when nothing changes after it", ask.country);
        assert_eq!(found, wanted.or(Some(origin)), "{} part {resource} after {origin}: the origin itself when nothing changes after it, as main", ask.country);
        let back = line.navigate_lines(std::slice::from_ref(&ask), &[measure], Some(origin), NavigationKey::ArrowLeft, bounds, &Within::new()).unwrap();
        let wanted = (bounds.0..origin).rev().find(|&hour| hour == first_held || (hour > first_held && produced(&query, id, resource, hour) != produced(&query, id, resource, hour - 1)));
        assert_eq!(back, wanted.or(Some(origin)), "{} part {resource} before {origin}: the origin itself when nothing changes before it, as main", ask.country);
    }
    let ask = Ask { country: tag_of(9001).into(), part: 4 };
    let step = |origin: Option<i32>, key| line.navigate_lines(std::slice::from_ref(&ask), &[measure], origin, key, bounds, &Within::new()).unwrap();
    assert_eq!(step(Some(bounds.0), NavigationKey::ArrowLeft), Some(bounds.0));
    assert_eq!(step(Some(bounds.1), NavigationKey::ArrowRight), Some(bounds.1));
    assert_eq!(step(Some(bounds.1 + 50), NavigationKey::ArrowRight), Some(bounds.1));
    assert_eq!(step(None, NavigationKey::ArrowLeft), line.change_lines(std::slice::from_ref(&ask), &[measure], None, NavigationKey::ArrowLeft, bounds, &Within::new()).unwrap());
    let within = Within::from([(ask.country.clone(), vec![(FIRST + 2_000, FIRST + 3_000)])]);
    let inside = |origin: i32, key| line.change_lines(std::slice::from_ref(&ask), &[measure], Some(origin), key, bounds, &within).unwrap();
    let changes = || (FIRST + 2_001..=FIRST + 3_000).filter(|&hour| produced(&query, 9001, 4, hour) != produced(&query, 9001, 4, hour - 1));
    assert_eq!(inside(FIRST + 1_000, NavigationKey::ArrowRight), Some(FIRST + 2_000), "a step from before the held run lands where it starts");
    assert_eq!(inside(FIRST + 2_000, NavigationKey::ArrowRight), changes().next(), "a change outside the held run is never a step");
    assert_eq!(inside(FIRST + 5_000, NavigationKey::ArrowLeft), changes().last().or(Some(FIRST + 2_000)), "a step back from after the held run stays inside it");
}

#[test]
fn an_absent_row_of_a_table_listing_only_nonzero_values_is_zero_only_where_its_owner_was_read() {
    assert!(dashboard_declare::absent_is_zero(TableId::RelationOpinions));
    assert_eq!(meet(&[(0, 9), (20, 29)], &[(5, 24), (28, 40)]), [(5, 9), (20, 24), (28, 29)]);
    let held: read::Runs = vec![(10, 12, Some(7)), (13, 14, Some(9)), (15, 40, None)];
    let zero = meet(&[(3, 30)], &[(0, 24), (27, 50)]);
    assert_eq!(zero_absent(&held, &zero, 0, 40), [(0, 2, None), (3, 9, Some(0)), (10, 12, Some(7)), (13, 14, Some(9)), (15, 24, Some(0)), (25, 26, None), (27, 30, Some(0)), (31, 40, None)]);
    assert_eq!(zero_absent(&Vec::new(), &[(5, 6)], 0, 9), [(0, 4, None), (5, 6, Some(0)), (7, 9, None)], "a pair never listed");
    assert_eq!(zero_absent(&Vec::new(), &[], 0, 9), [(0, 9, None)], "an owner never read is not known");
}

#[test]
fn a_pair_listed_under_other_raw_ids_of_its_countries_is_its_line() {
    let links: HashMap<u128, read::Runs> = HashMap::from([
        (1, vec![(0, 99, Some(10))]),
        (7, vec![(0, 19, None), (20, 60, Some(10)), (61, 99, Some(30))]),
        (2, vec![(0, 99, Some(20))]),
        (8, vec![(0, 99, Some(20))]),
        (9, vec![(70, 99, Some(10))]),
    ]);
    let aliases = Aliases::new(&links);
    assert_eq!(aliases.of(1, 0, 99), [(1, vec![(0, 99)]), (7, vec![(20, 60)]), (9, vec![(70, 99)])]);
    assert_eq!(aliases.of(8, 10, 50), [(2, vec![(10, 50)]), (8, vec![(10, 50)])]);
    assert_eq!(aliases.of(5, 0, 99), [(5, vec![(0, 99)])], "an id with no link is its own alias alone");
    assert_eq!(union(vec![(5, 9), (0, 3), (4, 4), (12, 20), (15, 18)]), [(0, 9), (12, 20)]);
    let alias_row: read::Runs = vec![(30, 49, Some(35)), (61, 80, Some(-90))];
    let own_row: read::Runs = vec![(50, 54, Some(40)), (55, 59, None)];
    let line = pair_rows(&[(own_row.as_slice(), &[(0, 99)][..]), (alias_row.as_slice(), &[(20, 60)][..])]);
    assert_eq!(line, [(30, 49, Some(35)), (50, 54, Some(40))]);
    assert_eq!(
        zero_absent(&line, &[(0, 99)], 0, 99),
        [(0, 29, Some(0)), (30, 49, Some(35)), (50, 54, Some(40)), (55, 99, Some(0))],
        "the pair's listed values stand, its hours listed under no alias are zero"
    );
    let (first, second) = ([(0, 9, Some(1))], [(5, 14, Some(2))]);
    assert_eq!(pair_rows(&[(&first[..], &[(0, 99)][..]), (&second[..], &[(0, 99)][..])]), [(0, 9, Some(1)), (10, 14, Some(2))], "two rows never overlap");
}

fn tension_fixture() -> Fixture {
    let end = FIRST + UNIT_HOURS - 1;
    Fixture::with_gap(Data::Tension, FIRST..=end + GAP + 24, end + 1..=end + GAP)
}

#[test]
fn the_next_read_of_a_table_is_the_next_hour_it_was_read_across_a_gap_and_never_the_origin() {
    let fixture = tension_fixture();
    let query = fixture.query();
    let end = query.timeline.stretches[0].last;
    let second = query.timeline.stretches[1].first;
    let last = query.timeline.last;
    let run = query.replay.recording().run(0).unwrap();
    let block = |hour| run.place(run.ordinal(hour).unwrap()).0;
    assert_eq!(block(second), block(end) + 1);
    let tension = TableId::WorldTension;
    let step = |origin: Option<i32>, key| next_read(&query, tension, origin, key, (FIRST, last)).unwrap();
    let still = (FIRST + 30..).find(|hour| fixture::tension(*hour) == fixture::tension(hour + 1)).unwrap();
    assert_eq!(step(Some(still), NavigationKey::ArrowRight), Some(still + 1), "read again though the tension holds");
    assert_eq!(step(Some(still + 1), NavigationKey::ArrowLeft), Some(still));
    assert_eq!(step(Some(end), NavigationKey::ArrowRight), Some(second), "the hours another history holds are no read");
    assert_eq!(step(Some(second), NavigationKey::ArrowLeft), Some(end));
    assert_eq!(step(Some(FIRST), NavigationKey::ArrowLeft), None, "nothing before the first read");
    assert_eq!(step(Some(last), NavigationKey::ArrowRight), None);
    assert_eq!(step(None, NavigationKey::Home), Some(FIRST));
    assert_eq!(step(Some(FIRST + 9), NavigationKey::End), Some(last));
    assert_eq!(step(Some(FIRST + 100), NavigationKey::ArrowRight), Some(FIRST + 101), "an origin far from either edge");
    assert_eq!(next_read(&query, tension, Some(FIRST + 10), NavigationKey::ArrowRight, (FIRST + 50, FIRST + 60)).unwrap(), Some(FIRST + 50), "an origin before the bounds steps into them");
    assert!(next_read(&query, TableId::NavalSupremacy, Some(FIRST), NavigationKey::ArrowRight, (FIRST, last)).unwrap().is_none(), "a table never available is never read");
}

#[test]
fn an_owners_parts_over_a_span_are_its_rows_lifetimes() {
    let fixture = Fixture::campaign_with_sealed_span(Data::Resources);
    let query = fixture.query();
    let line = resources_line(&query, "produced");
    let tags = ["GER".to_owned(), "ITA".to_owned()];
    let second = query.timeline.stretches[1].first;
    let last = query.timeline.last;
    for (from, to) in [(FIRST, last), (FIRST + 3_000, FIRST + 4_000), (FIRST + 5_500, second + 100)] {
        let owners = line.part_lives(&tags, from, to).unwrap();
        for (owner, id) in owners.iter().zip([9001, 9002]) {
            let mut wanted: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
            for hour in from..=to {
                for resource in [1, 2, 3, 4] {
                    if produced(&query, id, resource, hour).is_some() {
                        let spans = wanted.entry(resource).or_default();
                        match spans.last_mut() {
                            Some(span) if span.1 + 1 == hour => span.1 = hour,
                            _ => spans.push((hour, hour)),
                        }
                    }
                }
            }
            let mut got: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
            for life in &owner.parts {
                got.entry(life.part.parse().unwrap()).or_default().push((life.first_hour, life.last_hour));
            }
            assert_eq!(got, wanted, "{}'s parts over {from}..{to}: each span it is present, never one span across a gap", owner.country);
            assert!(owner.parts.iter().all(|life| life.id.as_deref() == Some(life.part.as_str())), "a resource part names its resource");
        }
        if to >= second {
            assert!(owners[0].missing.iter().any(|span| span.reason == GAP_REASON));
        }
    }
    let fuel = dashboard_declare::series("fuel-consumers").unwrap();
    let line = Line::new(&query, fuel, fuel.measure("requested").unwrap()).unwrap();
    let owners = line.part_lives(&tags[..1], FIRST, FIRST + 100).unwrap();
    assert!(owners[0].parts.is_empty());
    let missing: Vec<(i32, i32)> = owners[0].missing.iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(missing, [(FIRST, FIRST + 100)]);
    assert!(owners[0].missing[0].reason.contains("not available"), "{}", owners[0].missing[0].reason);
}

const SPECIAL_TOTAL: usize = 4;
const PRODUCTION_DEMAND: usize = 5;

fn surplus(cells: &dyn Cells, slot: u32) -> Option<i64> {
    cells.number(SPECIAL_TOTAL, slot)?.checked_add(cells.number(PRODUCTION_DEMAND, slot)?)
}

static SURPLUS: Series = Series {
    kind: "test-surplus",
    table: TableId::CountryResources,
    key: Key::Country,
    parts: Some(dashboard_declare::Parts { bits: 32, id: Some((0, 32)) }),
    measures: &[Measure {
        name: "surplus",
        column: Column::Computed { id: 900, inputs: &["special_total", "production_demand"], value: surplus },
        decimals: 5,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
    absent_none: false,
};

#[test]
fn a_computed_part_state_is_its_formula_over_the_row() {
    assert_eq!(dashboard_declare::field(TableId::CountryResources, "special_total").unwrap(), SPECIAL_TOTAL);
    assert_eq!(dashboard_declare::field(TableId::CountryResources, "production_demand").unwrap(), PRODUCTION_DEMAND);
    let fixture = Fixture::campaign(Data::Resources);
    let query = fixture.query();
    let line = Line::new(&query, &SURPLUS, &SURPLUS.measures[0]).unwrap();
    for hour in HOURS {
        let states = line.states(&[read("GER", hour, Some(2))], None).unwrap();
        let row = fixture::resource(9001, 2, hour);
        let wanted = held(&query, "GER", hour).then(|| row.special_total + row.production_demand);
        assert_eq!(states[0].run.as_ref().map(|run| run.value_raw.parse::<i64>().unwrap()), wanted, "GER's surplus of 2 at {hour}");
    }
}

const POWER: usize = 2;
const COMMAND: usize = 3;

fn negated(cells: &dyn Cells, slot: u32) -> Option<i64> {
    Some(cells.number(COMMAND, slot)? - cells.number(POWER, slot)?)
}

static NEGATED: Series = Series {
    kind: "test-negated",
    table: TableId::Countries,
    key: Key::Country,
    parts: None,
    measures: &[Measure {
        name: "negated",
        column: Column::Computed { id: 900, inputs: &["political_power_raw", "command_power_raw"], value: negated },
        decimals: 5,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
    absent_none: false,
};

#[test]
fn a_rank_at_an_hour_of_a_computed_column_ranks_the_formula_of_each_row() {
    assert_eq!(dashboard_declare::field(TableId::Countries, "political_power_raw").unwrap(), POWER);
    assert_eq!(dashboard_declare::field(TableId::Countries, "command_power_raw").unwrap(), COMMAND);
    let fixture = Fixture::with_gap(Data::Power, FIRST..=SECOND + 3, FIRST + FIRST_HOURS..=SECOND - 1);
    let query = fixture.query();
    let line = Line::new(&query, &NEGATED, &NEGATED.measures[0]).unwrap();
    let mut both = 0;
    for hour in (FIRST..FIRST + FIRST_HOURS).step_by(37).chain([ITA_FROM + 40 - ITA_FROM.rem_euclid(97) + 97, SECOND + 3]) {
        let mut wanted: Vec<(i64, u32)> = [9001, 9002].into_iter().filter(|&id| exists(id, hour)).filter_map(|id| Some((-power(id, hour)?, id))).collect();
        wanted.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let wanted: Vec<String> = wanted.into_iter().map(|(_, id)| tag_of(id).to_owned()).collect();
        both += usize::from(wanted.len() == 2);
        assert_eq!(line.rank_at(hour, 2).unwrap(), wanted, "at {hour}");
        let states = line.states(&[read("GER", hour, None)], None).unwrap();
        assert_eq!(states[0].run.as_ref().map(|run| run.value_raw.parse::<i64>().unwrap()), power(9001, hour).map(|power| -power), "GER's negated power at {hour}");
    }
    assert!(both > 10, "{both} hours ranked two countries");
    assert!(resources_line(&query, "used").rank_at(FIRST, 2).is_err(), "a sub-keyed series ranks parts, not countries");
}

#[test]
fn a_ranked_series_answers_its_top_countries_at_an_hour_over_a_view_and_by_key() {
    let fixture = Fixture::campaign_with_sealed_span(Data::Power);
    let query = fixture.query();
    let end = query.timeline.stretches[0].last;
    let second = query.timeline.stretches[1].first;
    let last = query.timeline.last;
    let ask = |product: &str, extra: serde_json::Value| {
        let mut params = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" } });
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        answer(&query, product, &params).unwrap().unwrap()
    };
    for hour in HOURS[..6].iter().copied().chain([end, second + 17, last]) {
        let reply: SeriesStateReply = serde_json::from_value(ask("series-state", serde_json::json!({ "top": 1, "hour": hour }))).unwrap();
        let mut wanted: Vec<(i64, &str)> = [9001, 9002]
            .into_iter()
            .filter(|&id| held(&query, tag_of(id), hour) && exists(id, hour))
            .filter_map(|id| Some((power(id, hour)?, tag_of(id))))
            .collect();
        wanted.sort_by(|a, b| b.0.cmp(&a.0));
        let got: Vec<(i64, &str)> = reply.states.iter().map(|state| (state.run.as_ref().unwrap().value_raw.parse().unwrap(), state.country.tag().unwrap())).collect();
        assert_eq!(got, wanted.into_iter().take(1).collect::<Vec<_>>(), "the top country at {hour}");
    }
    let chart: ranked::SeriesRankedReply = serde_json::from_value(ask("series-summary", serde_json::json!({ "top": 1, "from": FIRST, "to": last, "width": 64 }))).unwrap();
    assert!(!chart.lines.is_empty());
    for line in &chart.lines {
        assert!(!line.membership.is_empty() && !line.parts.is_empty(), "{} ranks somewhere and is held there", line.tag);
        for (bin, bucket) in line.summary.buckets.iter().enumerate() {
            let Some(bucket) = bucket else { continue };
            assert!(line.membership.iter().any(|span| span.first_hour <= bucket.last_hour && bucket.first_hour <= span.last_hour), "{} bin {bin} lies in its membership", line.tag);
        }
    }
    let mut checked = 0;
    for to in (FIRST + 300..end - 9).filter(|hour| (hour + 1).rem_euclid(6) != 0).take(24) {
        let edge: ranked::SeriesRankedReply = serde_json::from_value(ask("series-summary", serde_json::json!({ "top": 1, "from": FIRST, "to": to, "width": 64 }))).unwrap();
        for line in &edge.lines {
            let Some(Some(bucket)) = line.summary.buckets.last() else { continue };
            let id = [9001, 9002].into_iter().find(|&id| tag_of(id) == line.tag).unwrap();
            assert!(bucket.last_hour <= to, "{}'s last bin ends at the view's last hour {to}", line.tag);
            if let Some(raw) = power(id, to).filter(|_| exists(id, to) && held(&query, &line.tag, to)) {
                assert_eq!(bucket.last_hour, to, "{}'s last bin reaches the view's last hour {to}", line.tag);
                assert_eq!(bucket.last_value, raw as f64 / 10f64.powi(i32::from(edge.decimals)), "{}'s last value is the view's last hour's ({to})", line.tag);
                checked += 1;
            }
        }
    }
    assert!(checked > 10, "{checked} views whose member holds a value at the view's last hour");
    let scale = 10f64.powi(i32::from(chart.decimals));
    let mut ends = 0;
    for line in &chart.lines {
        let id = [9001, 9002].into_iter().find(|&id| tag_of(id) == line.tag).unwrap();
        let Some(bucket) = line.summary.buckets.iter().flatten().find(|bucket| bucket.first_hour <= end && end <= bucket.last_hour) else { continue };
        let values: Vec<i64> = (bucket.first_hour..=end).filter(|&hour| exists(id, hour) && held(&query, &line.tag, hour)).filter_map(|hour| power(id, hour)).collect();
        assert_eq!(bucket.last_hour, end, "{}'s bin ends at the stretch's last hour", line.tag);
        assert_eq!(bucket.last_value, power(id, end).unwrap() as f64 / scale, "{}'s last value is the stretch's last hour's", line.tag);
        assert_eq!((bucket.min_value, bucket.max_value), (*values.iter().min().unwrap() as f64 / scale, *values.iter().max().unwrap() as f64 / scale), "{}'s bin at {end}", line.tag);
        ends += 1;
    }
    assert_eq!(ends, 1, "the leader at the stretch's last hour draws its bin");
    let tiles = serde_json::json!({ "top": null, "from": FIRST, "to": last, "width": 64, "countries": ["GER"] });
    assert!(answer(&query, "series-summary", &{
        let mut params = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" } });
        params.as_object_mut().unwrap().extend(tiles.as_object().unwrap().clone());
        params
    })
    .is_err(), "chosen lines are tiles");
    let origin = ITA_FROM + 10;
    let next: SeriesKeyReply = serde_json::from_value(ask("series-next-change", serde_json::json!({ "top": 1, "key": "ArrowRight", "from": origin, "firstHour": FIRST, "lastHour": last }))).unwrap();
    let leader = line_of(&query).rank_at(origin, 1).unwrap();
    let wanted = line_of(&query).navigate(&leader, &[dashboard_declare::series("country-measure").unwrap().measure("politicalPower").unwrap()], Some(origin), NavigationKey::ArrowRight, (FIRST, last)).unwrap();
    assert_eq!(next.hour, wanted.map(f64::from));
    assert!(next.hour.is_some_and(|hour| hour > f64::from(origin)));
}

#[test]
fn a_request_naming_1500_countries_is_answered() {
    let (first, last) = (ITA_FROM - UNIT_HOURS, ITA_FROM + UNIT_HOURS);
    let fixture = Fixture::new(Data::Power, first..=last);
    let query = fixture.query();
    let ask = |product: &str, extra: serde_json::Value| {
        let mut params = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" } });
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        answer(&query, product, &params).unwrap().unwrap()
    };
    let tags: Vec<String> = TAGS.iter().map(|tag| (*tag).to_owned()).chain((0..1_498).map(|at| format!("X{at:04}"))).collect();
    let hour = ITA_FROM + 10;
    let reads: Vec<serde_json::Value> = tags.iter().map(|tag| serde_json::json!({ "country": tag, "hour": hour })).collect();
    let states: SeriesStateReply = serde_json::from_value(ask("series-state", serde_json::json!({ "reads": reads }))).unwrap();
    assert_eq!(states.states.len(), 1_500);
    assert!(states.states[..2].iter().all(|state| state.run.is_some()), "GER and ITA read at {hour}");
    assert!(states.states[2..].iter().all(|state| state.run.is_none() && state.missing.is_some()), "a tag no country holds is missing");
    let chart: ranked::SeriesRankedReply = serde_json::from_value(ask("series-summary", serde_json::json!({ "top": 1_500, "from": first, "to": last, "width": 7 }))).unwrap();
    assert_eq!(chart.lines.iter().map(|line| line.tag.as_str()).collect::<std::collections::BTreeSet<_>>(), TAGS.into_iter().collect());
    let next: SeriesKeyReply =
        serde_json::from_value(ask("series-next-change", serde_json::json!({ "countries": tags, "key": "ArrowRight", "from": hour, "firstHour": first, "lastHour": last }))).unwrap();
    assert!(next.hour.is_some_and(|next| next > f64::from(hour)));
    let held = serde_json::json!([["GER", [{ "firstHour": first, "lastHour": first }]]]);
    let outside: SeriesKeyReply =
        serde_json::from_value(ask("series-next-change", serde_json::json!({ "countries": ["GER"], "within": held, "key": "ArrowRight", "from": hour, "firstHour": first, "lastHour": last }))).unwrap();
    assert_eq!(outside.hour, None, "a step from outside the held runs, with nothing after it inside them, goes nowhere rather than back to its origin");
    ask("events-page", serde_json::json!({ "countries": tags, "from": first, "to": last, "pageSize": 16 }));
}

fn line_of<'q, 'r>(query: &'q Query<'r>) -> Line<'q, 'r> {
    let series = dashboard_declare::series("country-measure").unwrap();
    Line::new(query, series, series.measure("politicalPower").unwrap()).unwrap()
}

#[test]
fn a_ranked_view_of_stretches_shorter_than_a_bucket_reads_each_exactly() {
    use crate::timeline::{Join, Stretch, Timeline};
    let first = ITA_FROM.div_euclid(6) * 6 + 600;
    let last = first + 9;
    let fixture = Fixture::new(Data::Power, first..=last);
    let mut query = fixture.query();
    query.timeline = Arc::new(Timeline {
        id: 1,
        first,
        last,
        stretches: vec![Stretch { run: 0, first, last: first + 4, join: Join::Start }, Stretch { run: 0, first: first + 5, last, join: Join::Load { parent: first + 4 } }],
        unheld: Vec::new(),
    });
    let line = line_of(&query);
    let scale = 10f64.powi(i32::from(line.decimals()));
    let values: Vec<f64> = (first..=last).map(|hour| power(9002, hour).unwrap() as f64 / scale).collect();
    for width in [64, 1] {
        let ranked = line.ranked(first, last, width, 1).unwrap();
        assert_eq!(ranked.iter().map(|line| line.tag.as_str()).collect::<Vec<_>>(), ["ITA"], "the leader at width {width}");
        assert_eq!(ranked[0].membership, [(first, last)], "ITA ranks over both stretches at width {width}");
        let buckets: Vec<&ChartBucket> = ranked[0].summary.buckets.iter().flatten().collect();
        let (head, tail) = (buckets.first().unwrap(), buckets.last().unwrap());
        assert_eq!((head.first_hour, head.first_value), (first, values[0]), "width {width}");
        assert_eq!((tail.last_hour, tail.last_value), (last, values[9]), "width {width}");
        let least = buckets.iter().map(|bucket| bucket.min_value).fold(f64::INFINITY, f64::min);
        let greatest = buckets.iter().map(|bucket| bucket.max_value).fold(f64::NEG_INFINITY, f64::max);
        assert_eq!((least, greatest), (values.iter().copied().fold(f64::INFINITY, f64::min), values.iter().copied().fold(f64::NEG_INFINITY, f64::max)), "width {width}");
    }
}

#[test]
fn a_ranked_short_stretch_reads_its_winner_only_over_its_tags_hours() {
    use crate::identity::{Holder, HolderRun, Holders};
    use crate::timeline::{Join, Stretch, Timeline};
    let first = ITA_FROM.div_euclid(6) * 6 + 600;
    let last = first + 9;
    let fixture = Fixture::new(Data::Power, first..=last);
    let mut query = fixture.query();
    let from = first + 3;
    query.timeline = Arc::new(Timeline {
        id: 1,
        first,
        last,
        stretches: vec![Stretch { run: 0, first, last: first + 4, join: Join::Start }, Stretch { run: 0, first: first + 5, last, join: Join::Load { parent: first + 4 } }],
        unheld: Vec::new(),
    });
    let run = |first: i32, last: i32, starts_segment: bool| HolderRun { first, last, holder: Holder::Found { slot: 8, id: 9002 }, starts_segment };
    let mut holders = Holders::new(query.holders.tags.clone(), Vec::new());
    holders.tags.insert("AAA".to_owned(), vec![run(first, from - 1, true)]);
    holders.tags.insert("ITA".to_owned(), vec![run(from, last, true)]);
    query.holders = Arc::new(holders);
    let line = line_of(&query);
    let scale = 10f64.powi(i32::from(line.decimals()));
    let values: Vec<f64> = (from..=last).map(|hour| power(9002, hour).unwrap() as f64 / scale).collect();
    let ranked = line.ranked(first, last, 64, 2).unwrap();
    let ita = ranked.iter().find(|line| line.tag == "ITA").expect("ITA leads at each stretch's last hour");
    assert!(ranked.iter().all(|line| line.tag != "AAA"), "AAA holds the key at no bin's last hour");
    assert_eq!(ita.membership, [(from, last)], "ITA ranks only from the hour it holds the key");
    let buckets: Vec<&ChartBucket> = ita.summary.buckets.iter().flatten().collect();
    assert_eq!((buckets[0].first_hour, buckets[0].first_value), (from, values[0]), "ITA's first bin starts at its own first hour");
    let least = buckets.iter().map(|bucket| bucket.min_value).fold(f64::INFINITY, f64::min);
    let greatest = buckets.iter().map(|bucket| bucket.max_value).fold(f64::NEG_INFINITY, f64::max);
    assert_eq!((least, greatest), (values.iter().copied().fold(f64::INFINITY, f64::min), values.iter().copied().fold(f64::NEG_INFINITY, f64::max)));
}

#[test]
fn a_factions_line_is_its_row_and_its_bins_hold_its_members_exact_sums() {
    let bucket_hours = replay_columnar::summary::bucket_hours(1) as i32;
    let edge_first = ((ITA_FROM + UNIT_HOURS).div_euclid(bucket_hours) + 1) * bucket_hours;
    let (from, to) = (ITA_FROM - UNIT_HOURS, edge_first + 12);
    let last = edge_first + bucket_hours - 1;
    let fixture = Fixture::new(Data::Faction, from..=last);
    let query = fixture.query();
    let totals = dashboard_declare::series("faction-totals").unwrap();
    let line = Line::new(&query, totals, totals.measure("controlledStates").unwrap()).unwrap();
    let width = 7;
    let charts = line.summaries(&[FACTION.to_string()], from, to, width).unwrap();
    let edge = charts[0].0.buckets.iter().rev().flatten().next().unwrap();
    assert_eq!(edge.last_hour, to);
    assert!((to + 1..=last).filter_map(faction_sum).any(|sum| sum as f64 > edge.max_value));
    let mut checked = 0;
    for bucket in charts[0].0.buckets.iter().flatten() {
        let sums: Vec<i64> = (bucket.first_hour..=bucket.last_hour).filter_map(faction_sum).collect();
        let (least, most) = (*sums.iter().min().unwrap(), *sums.iter().max().unwrap());
        assert_eq!((bucket.min_value, bucket.max_value), (least as f64, most as f64), "bin {}..{}", bucket.first_hour, bucket.last_hour);
        checked += 1;
    }
    assert!(checked > 1, "the view's bins hold the faction's values");
    assert!(line.summaries(&["GER".to_owned()], from, to, width).unwrap()[0].0.buckets.iter().all(Option::is_none), "a faction is named by its id");
    assert!(line.rank_at(from, 1).is_err(), "factions are not ranked as countries are");
}

static WORLD: Series = Series {
    kind: "test-world",
    table: TableId::WorldTension,
    key: Key::Singleton,
    parts: None,
    measures: &[Measure { name: "tension", column: Column::Cell("total_raw"), decimals: 3, aggregates: 0, carry: Carry::None, rank: 0 }],
    drops: false,
    absent_none: false,
};

#[test]
fn a_singleton_line_reads_its_one_row_in_every_stretch_whatever_it_is_named_and_is_never_ranked() {
    let fixture = tension_fixture();
    let query = fixture.query();
    assert!(held(&query, "GER", FIRST));
    assert!(!held(&query, "no such tag", FIRST));
    let last = query.timeline.last;
    let gap_end = query.timeline.stretches[1].first - 1;
    let line = Line::new(&query, &WORLD, &WORLD.measures[0]).unwrap();
    let wanted: Vec<(u32, i32, i32, u128)> = query.timeline.within(FIRST, last).map(|stretch| (stretch.run, stretch.first, stretch.last, 0)).collect();
    for name in ["GER", "no such tag"] {
        let held: Vec<(u32, i32, i32, u128)> = line.held(&Ask::whole(name), FIRST, last).iter().map(|part| (part.run, part.first, part.last, part.key)).collect();
        assert_eq!(held, wanted, "{name}");
    }
    let states = line.states(&[read("no such tag", FIRST + 3, None), read("no such tag", gap_end, None)], None).unwrap();
    assert_eq!(states[0].missing, None, "a held hour is never refused for its name");
    assert_eq!(states[0].run.as_ref().map(|run| run.value_raw.clone()), Some(fixture::tension(FIRST + 3).to_string()), "the one row's value");
    assert_eq!(states[1].missing.as_deref(), Some(query.timeline.reason(gap_end).as_str()));
    assert!(line.rank_at(FIRST, 2).is_err());
    assert!(line.ranked(FIRST, last, 64, 2).is_err());
    let hour = FIRST + 3;
    let reply = crate::tile::tile(&line, "1", &[Ask::whole("World")], None, 1, i64::from(hour).div_euclid(BINS as i64), false).expect("the world's line is a tile");
    let runs = crate::tile::decode_runs(&unbase64(&reply.bins), 1, reply.first_hour, line.scale(), reply.raw_values).unwrap();
    let held = runs[0].iter().find(|run| run.0 <= hour && hour <= run.1).expect("a run holds the hour");
    assert_eq!(held.2, fixture::tension(hour) as f64 / 1_000.0);
}

#[test]
fn a_rank_names_the_greatest_countries_at_the_hour_with_their_states() {
    let fixture = Fixture::with_gap(Data::Power, FIRST..=SECOND + 3, FIRST + FIRST_HOURS..=SECOND - 1);
    let query = fixture.query();
    let ask = |hour: i32, top: u32| serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" }, "hour": hour, "top": top });
    let mut ranked_both = 0;
    for hour in [FIRST, ITA_FROM + 40, FIRST + FIRST_HOURS - 1, SECOND + 3] {
        let reply: SeriesStateReply = serde_json::from_value(answer(&query, "series-state", &ask(hour, 2)).unwrap().unwrap()).unwrap();
        let mut wanted: Vec<(i64, u32)> = [9001, 9002].into_iter().filter(|&id| exists(id, hour)).filter_map(|id| Some((power(id, hour)?, id))).collect();
        wanted.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked_both += usize::from(wanted.len() == 2);
        let shown: Vec<(String, Option<String>)> = reply.states.iter().map(|state| (state.country.to_string(), state.run.as_ref().map(|run| run.value_raw.clone()))).collect();
        let wanted: Vec<(String, Option<String>)> = wanted.into_iter().map(|(value, id)| (tag_of(id).to_owned(), Some(value.to_string()))).collect();
        assert_eq!(shown, wanted, "at {hour}");
    }
    assert!(ranked_both > 0);
    assert!(answer(&query, "series-state", &ask(FIRST, 0)).is_err());
}

#[test]
fn v1_published_supply_health_is_withheld_by_dashboard_query() {
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, schema::schema};
    use replay::campaign::Replay;
    use replay_journal::{columnar_file::{FileWriter, HourWriter}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::{CaptureMode, Identity, IdentityCodec, Roster, SampleBoundary, SessionStart, Verification},
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow,
            Recorded, TABLE_COUNT,
        },
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use std::io::Cursor;

    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let first = 100;
    let last = first + 11;
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"v1-supply".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000)),
    );
    writer.start_run(1, envelope::epoch::START, first).unwrap();
    let mut synthetic = Synthetic::new(1, first).unwrap();
    let campaign = [CampaignRow { id: b"v1-supply".to_vec() }];
    let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
    let divisions = [DivisionRow { id: 1, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() }];
    let land = [DivisionLandInputsRow { division: 1, naval_transfer: false, ..Default::default() }];
    let statuses = [DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(0),
        supply_ratio_exact: Recorded::Value(Some(15_000)),
        supply_grace_exact: Recorded::Value(Some(0)),
        ..Default::default()
    }];
    for hour in first..=last {
        synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionLandInputs, Input::Snapshot { rows: &land, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionStatus, Input::Snapshot { rows: &statuses, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
    }
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let v2_bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let v2_replay = Replay::open(v2_bytes.clone()).unwrap();
    let v2_timeline = Arc::new(crate::timeline::Timeline {
        id: 1,
        first,
        last,
        stretches: vec![crate::timeline::Stretch { run: 0, first, last, join: crate::timeline::Join::Start }],
        unheld: Vec::new(),
    });
    let v2_holders = Arc::new(crate::identity::holders(v2_replay.recording(), &v2_timeline).unwrap());
    let v2_query = Query { replay: &v2_replay, timeline: v2_timeline, holders: v2_holders };
    let series = dashboard_declare::series("supply-health").unwrap();
    let v2_line = Line::new(&v2_query, series, series.measure("noSupply").unwrap()).unwrap();
    let v2_states = v2_line.states(&[read("GER", first + 2, None)], None).unwrap();
    assert_eq!(v2_states[0].missing, None, "v2 published SupplyHealth stays readable: {:?}", v2_states[0]);
    assert_eq!(v2_states[0].run.as_ref().map(|run| run.value_raw.as_str()), Some("1"));

    let (extent, length) = container::locate(v2_bytes.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(v2_bytes.as_slice(), &extent).unwrap();
    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);
    assert!(root.section(key).is_some_and(|section| section.version == 2));
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut rewritten = container::ContainerWriter::resume(Cursor::new(v2_bytes), root, at, 1, None).unwrap();
    rewritten.commit(None, true, |_| Ok(())).unwrap();
    let v1_bytes = rewritten.into_inner().into_inner();

    let replay = Replay::open(v1_bytes).unwrap();
    let timeline = Arc::new(crate::timeline::Timeline {
        id: 1,
        first,
        last,
        stretches: vec![crate::timeline::Stretch { run: 0, first, last, join: crate::timeline::Join::Start }],
        unheld: Vec::new(),
    });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };
    let line = Line::new(&query, series, series.measure("noSupply").unwrap()).unwrap();

    let states = line.states(&[read("GER", first + 2, None)], None).unwrap();
    assert_eq!(states.len(), 1);
    assert!(states[0].run.is_none(), "v1 SupplyHealth must not report definitive count: {:?}", states[0]);
    assert!(
        states[0].missing.as_deref().is_some_and(|why| why.contains("SupplyHealth")),
        "{:?}",
        states[0]
    );

    let (summary, missing) = line.summary("GER", first, last, 2).unwrap();
    assert!(summary.buckets.iter().all(Option::is_none), "binned summary must withhold v1 counts: {:?}", summary.buckets);
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!((missing[0].first_hour, missing[0].last_hour), (first, last));
    assert!(missing[0].reason.contains("SupplyHealth"), "{missing:?}");

    let tile_hourly = tile::tile(&line, "1", &[Ask::whole("GER")], None, 1, i64::from(first).div_euclid(BINS as i64), false).unwrap();
    let flags = unbase64(&tile_hourly.bins);
    assert!(flags[..BINS].iter().all(|&flag| flag & PRESENT == 0), "hourly tile must withhold v1 counts");
    assert!(tile_hourly.lines[0].missing.iter().any(|span| span.reason.contains("SupplyHealth")), "{:?}", tile_hourly.lines[0].missing);

    let tile_binned = tile::tile(&line, "1", &[Ask::whole("GER")], None, 6, i64::from(first).div_euclid(BINS as i64 * 6), false).unwrap();
    let flags = unbase64(&tile_binned.bins);
    assert!(flags[..BINS].iter().all(|&flag| flag & PRESENT == 0), "binned tile must withhold v1 counts");
    assert!(tile_binned.lines[0].missing.iter().any(|span| span.reason.contains("SupplyHealth")), "{:?}", tile_binned.lines[0].missing);

    assert!(line.rank_at_by(first + 2, 5, false).unwrap().is_empty());

    let top_err = replay_lanes::lanes::top(
        replay.recording(),
        0,
        TableId::SupplyHealth,
        0,
        first,
        last,
        2,
        5,
        replay_lanes::lanes::By::Last,
    )
    .unwrap_err();
    assert!(
        matches!(
            replay_recording::columnar_frames::Damage::of(&top_err).map(|damage| &damage.scope),
            Some(replay_recording::columnar_frames::DamageScope::Withheld { table }) if *table == TableId::SupplyHealth.index() as u32
        ),
        "lanes::top must withhold unmarked v1 SupplyHealth: {top_err}"
    );

    let board = board::at(
        &[board::Column { line: Line::new(&query, series, series.measure("noSupply").unwrap()).unwrap(), partial: None }],
        board::Rows::Countries(&["GER".into()]),
        first + 2,
    )
    .unwrap();
    assert_eq!(board.rows.len(), 1);
    assert_eq!(board.rows[0].cells[0].value_raw, None, "{:?}", board.rows[0]);
    assert!(
        board.rows[0].cells[0].missing.as_deref().is_some_and(|why| why.contains("SupplyHealth")),
        "{:?}",
        board.rows[0]
    );
}

#[test]
fn resumed_v1_to_v2_supply_health_admits_v2_and_withholds_v1_and_spanning_bin() {
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, columnar_frames::{Damage, DamageScope}, schema::schema};
    use replay::campaign::Replay;
    use replay_journal::{columnar_file::{FileWriter, HourWriter, Resumed}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::{CaptureMode, Identity, IdentityCodec, Roster, SampleBoundary, SessionStart, Verification},
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow,
            Recorded, TABLE_COUNT,
        },
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use std::io::Cursor;

    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let first_v1 = 100;
    let last_v1 = 111;
    let first_v2 = 112;
    let last_v2 = 123;
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first_v1,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"mixed-supply".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let definitions = || {
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000))
    };
    let campaign = [CampaignRow { id: b"mixed-supply".to_vec() }];
    let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
    let divisions = [DivisionRow { id: 1, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() }];
    let land = [DivisionLandInputsRow { division: 1, naval_transfer: false, ..Default::default() }];
    let statuses = [DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(0),
        supply_ratio_exact: Recorded::Value(Some(15_000)),
        supply_grace_exact: Recorded::Value(Some(0)),
        ..Default::default()
    }];
    let write_hours = |writer: &mut HourWriter<Cursor<Vec<u8>>>, start: i32, end: i32| {
        writer.start_run(1, envelope::epoch::START, start).unwrap();
        let mut synthetic = Synthetic::new(1, start).unwrap();
        for hour in start..=end {
            synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::DivisionLandInputs, Input::Snapshot { rows: &land, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::DivisionStatus, Input::Snapshot { rows: &statuses, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
        }
    };

    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(definitions());
    write_hours(&mut writer, first_v1, last_v1);
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let v1_with_marker = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let (extent, length) = container::locate(v1_with_marker.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(v1_with_marker.as_slice(), &extent).unwrap();
    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut stripped = container::ContainerWriter::resume(Cursor::new(v1_with_marker), root, at, 1, None).unwrap();
    stripped.commit(None, true, |_| Ok(())).unwrap();
    let v1_bytes = stripped.into_inner().into_inner();

    let (extent, length) = container::locate(v1_bytes.as_slice(), container::Open::Newest).unwrap().unwrap();
    let root = container::read_root(v1_bytes.as_slice(), &extent).unwrap();
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let resumed = Resumed { root, at, last: Some((1, last_v1)), open: None };
    let file = FileWriter::resume(Cursor::new(v1_bytes), resumed, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(definitions());
    write_hours(&mut writer, first_v2, last_v2);
    let file = writer.finish().unwrap();
    let mixed_bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let replay = Replay::open(mixed_bytes).unwrap();
    let timeline = Arc::new(crate::timeline::Timeline {
        id: 1,
        first: first_v1,
        last: last_v2,
        stretches: vec![
            crate::timeline::Stretch { run: 0, first: first_v1, last: last_v1, join: crate::timeline::Join::Start },
            crate::timeline::Stretch { run: 1, first: first_v2, last: last_v2, join: crate::timeline::Join::Break },
        ],
        unheld: Vec::new(),
    });
    let holders = Arc::new(crate::identity::holders(replay.recording(), &timeline).unwrap());
    let query = Query { replay: &replay, timeline, holders };
    let series = dashboard_declare::series("supply-health").unwrap();
    let line = Line::new(&query, series, series.measure("noSupply").unwrap()).unwrap();

    let states = line.states(&[read("GER", last_v1, None), read("GER", first_v2, None)], None).unwrap();
    assert!(states[0].run.is_none(), "last v1 hour must be withheld: {:?}", states[0]);
    assert!(states[0].missing.as_deref().is_some_and(|why| why.contains("SupplyHealth")), "{:?}", states[0]);
    assert_eq!(states[1].missing, None, "first v2 hour must be admitted: {:?}", states[1]);
    assert_eq!(states[1].run.as_ref().map(|run| run.value_raw.as_str()), Some("1"));

    assert!(line.rank_at_by(last_v1, 5, false).unwrap().is_empty());
    assert_eq!(line.rank_at_by(first_v2, 5, false).unwrap(), vec!["GER"]);

    let v1_top = replay_lanes::lanes::top(replay.recording(), 0, TableId::SupplyHealth, 0, first_v1, last_v1, 2, 5, replay_lanes::lanes::By::Last).unwrap_err();
    assert!(
        matches!(
            Damage::of(&v1_top).map(|damage| &damage.scope),
            Some(DamageScope::Withheld { table }) if *table == TableId::SupplyHealth.index() as u32
        ),
        "v1 run must be withheld by lanes::top: {v1_top}"
    );
    let v2_top = replay_lanes::lanes::top(replay.recording(), 1, TableId::SupplyHealth, 0, first_v2, last_v2, 2, 5, replay_lanes::lanes::By::Last).unwrap();
    assert!(v2_top.iter().any(|bin| !bin.is_empty()), "v2 run must be admitted by lanes::top: {v2_top:?}");

    let (summary, missing) = line.summary("GER", first_v1, last_v2, 4).unwrap();
    assert_eq!(summary.buckets.len(), 5, "{:?}", summary.buckets);
    assert!(summary.buckets[0].is_none(), "pure v1 bin [96,101] must be withheld");
    assert!(summary.buckets[1].is_none(), "pure v1 bin [102,107] must be withheld");
    assert!(summary.buckets[2].is_none(), "spanning v1+v2 bin [108,113] must be withheld: {:?}", summary.buckets[2]);
    assert_eq!(summary.buckets[3].map(|bucket| bucket.last_value), Some(1.0), "pure v2 bin [114,119] must be admitted");
    assert_eq!(summary.buckets[4].map(|bucket| bucket.last_value), Some(1.0), "pure v2 bin [120,125] must be admitted");
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!((missing[0].first_hour, missing[0].last_hour), (first_v1, last_v1));
    assert!(missing[0].reason.contains("SupplyHealth"), "{missing:?}");

    let tile_binned = tile::tile(&line, "1", &[Ask::whole("GER")], None, 6, i64::from(first_v1).div_euclid(BINS as i64 * 6), false).unwrap();
    let flags = unbase64(&tile_binned.bins);
    let bin_of = |hour: i32| ((i64::from(hour) - i64::from(tile_binned.first_hour)) / 6) as usize;
    assert_eq!(flags[bin_of(108)] & PRESENT, 0, "spanning v1+v2 tile bin [108,113] must be withheld");
    assert_ne!(flags[bin_of(114)] & PRESENT, 0, "pure v2 tile bin [114,119] must be admitted");

    let tile_hourly = tile::tile(&line, "1", &[Ask::whole("GER")], None, 1, i64::from(first_v1).div_euclid(BINS as i64), false).unwrap();
    let hourly_flags = unbase64(&tile_hourly.bins);
    let hour_slot = |hour: i32| (hour - tile_hourly.first_hour) as usize;
    assert_eq!(hourly_flags[hour_slot(last_v1)] & PRESENT, 0, "last v1 hour must be withheld in hourly tile");
    assert_ne!(hourly_flags[hour_slot(first_v2)] & PRESENT, 0, "first v2 hour must be admitted in hourly tile");

    let file = FileWriter::create(Cursor::new(Vec::new()), replay.recording().root().identity.clone(), TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(definitions());
    write_hours(&mut writer, first_v1, last_v2);
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let single_run = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();
    let (extent, length) = container::locate(single_run.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(single_run.as_slice(), &extent).unwrap();
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut rewritten = container::ContainerWriter::resume(Cursor::new(single_run), root, at, 1, None).unwrap();
    rewritten.section(key, 2).unwrap();
    let mut payload = replay_columnar::leaf::Writer::new();
    payload.varint(0).unwrap();
    payload.varint(12).unwrap();
    let payload = payload.into_bytes();
    let payload_extent = rewritten.put(Kind::Stored, Tags::default(), payload.len() as u64, &payload).unwrap();
    rewritten.append(key, payload_extent).unwrap();
    rewritten.commit(None, true, |_| Ok(())).unwrap();
    let mid_run_bytes = rewritten.into_inner().into_inner();

    let mid_replay = Replay::open(mid_run_bytes).unwrap();
    let mid_top = replay_lanes::lanes::top(mid_replay.recording(), 0, TableId::SupplyHealth, 0, first_v1, last_v2, 4, 5, replay_lanes::lanes::By::Last).unwrap();
    assert_eq!(mid_top.len(), 5, "{mid_top:?}");
    assert!(mid_top[0].is_empty() && mid_top[1].is_empty() && mid_top[2].is_empty(), "v1 and spanning bins must be empty in mid-run lanes::top: {mid_top:?}");
    assert!(!mid_top[3].is_empty() && !mid_top[4].is_empty(), "pure v2 bins must be admitted in mid-run lanes::top: {mid_top:?}");
    let mid_timeline = Arc::new(crate::timeline::Timeline {
        id: 1,
        first: first_v1,
        last: last_v2,
        stretches: vec![crate::timeline::Stretch { run: 0, first: first_v1, last: last_v2, join: crate::timeline::Join::Start }],
        unheld: Vec::new(),
    });
    let mid_holders = Arc::new(crate::identity::holders(mid_replay.recording(), &mid_timeline).unwrap());
    let mid_query = Query { replay: &mid_replay, timeline: mid_timeline, holders: mid_holders };
    let mid_line = Line::new(&mid_query, series, series.measure("noSupply").unwrap()).unwrap();
    let mid_states = mid_line.states(&[read("GER", last_v1, None), read("GER", first_v2, None)], None).unwrap();
    assert!(mid_states[0].run.is_none() && mid_states[0].missing.is_some());
    assert_eq!(mid_states[1].missing, None);
    assert_eq!(mid_states[1].run.as_ref().map(|run| (run.first_hour, run.value_raw.as_str())), Some((first_v2, "1")));
    let (mid_summary, mid_missing) = mid_line.summary("GER", first_v1, last_v2, 4).unwrap();
    assert!(mid_summary.buckets[0].is_none() && mid_summary.buckets[1].is_none() && mid_summary.buckets[2].is_none());
    assert_eq!(mid_summary.buckets[3].map(|bucket| bucket.last_value), Some(1.0));
    assert_eq!(mid_summary.buckets[4].map(|bucket| bucket.last_value), Some(1.0));
    assert_eq!(mid_missing.len(), 1);
    assert_eq!((mid_missing[0].first_hour, mid_missing[0].last_hour), (first_v1, last_v1));

    let mid_chart = replay_lanes::lanes::chart_at(mid_replay.recording(), 0, TableId::SupplyHealth, 0, None, first_v1, last_v2, 0, 6).unwrap();
    let row = mid_chart.keys.iter().position(|&k| k == 7).unwrap();
    assert_eq!(mid_chart.values(row)[3].hours, 6, "6h bin [114,119] must not be double-counted: {:?}", mid_chart.values(row)[3]);
    assert_eq!(mid_chart.life(row)[3], 6, "6h bin [114,119] life must be 6: {:?}", mid_chart.life(row));
    assert_eq!(mid_chart.values(row)[4].hours, 4, "4h bin [120,123] must not be double-counted: {:?}", mid_chart.values(row)[4]);
    assert_eq!(mid_chart.life(row)[4], 4, "4h bin [120,123] life must be 4: {:?}", mid_chart.life(row));

    let v2_edge_chart = replay_lanes::lanes::chart_at(mid_replay.recording(), 0, TableId::SupplyHealth, 0, None, first_v2, last_v2, 0, 6).unwrap();
    let row = v2_edge_chart.keys.iter().position(|&k| k == 7).unwrap();
    assert_eq!(v2_edge_chart.values(row)[0].hours, 2, "leading walked v2 hours [112,113]: {:?}", v2_edge_chart.values(row)[0]);
    assert_eq!(v2_edge_chart.life(row)[0], 2);
    assert_eq!(v2_edge_chart.values(row)[1].hours, 6, "stored v2 bin [114,119]: {:?}", v2_edge_chart.values(row)[1]);
    assert_eq!(v2_edge_chart.life(row)[1], 6);
    assert_eq!(v2_edge_chart.values(row)[2].hours, 4, "stored v2 bin [120,123]: {:?}", v2_edge_chart.values(row)[2]);
    assert_eq!(v2_edge_chart.life(row)[2], 4);

    let reads = [replay_lanes::at::Read::all(TableId::SupplyHealth)];
    let mut cursor = replay_lanes::at::Cursor::new(mid_replay.recording(), &reads).unwrap();
    let at_v1 = cursor.frame(0, last_v1).unwrap();
    assert!(
        matches!(&*at_v1.tables[TableId::SupplyHealth.index()], replay_frame::frame::TableView::Unavailable { reason } if reason.contains("SupplyHealth")),
        "at::Cursor must withhold last v1 hour: {:?}",
        at_v1.tables[TableId::SupplyHealth.index()]
    );
    let at_v2 = cursor.frame(0, first_v2).unwrap();
    assert!(
        matches!(&*at_v2.tables[TableId::SupplyHealth.index()], replay_frame::frame::TableView::Complete(_)),
        "at::Cursor must admit first v2 hour: {:?}",
        at_v2.tables[TableId::SupplyHealth.index()]
    );
    let back_to_v1 = cursor.frame(0, last_v1).unwrap();
    assert!(
        matches!(&*back_to_v1.tables[TableId::SupplyHealth.index()], replay_frame::frame::TableView::Unavailable { reason } if reason.contains("SupplyHealth")),
        "at::Cursor backward seek must withhold v1 hour: {:?}",
        back_to_v1.tables[TableId::SupplyHealth.index()]
    );
    let forward_v2 = cursor.frame(0, first_v2 + 1).unwrap();
    assert!(
        matches!(&*forward_v2.tables[TableId::SupplyHealth.index()], replay_frame::frame::TableView::Complete(_)),
        "at::Cursor forward seek after backward seek must admit v2 hour: {:?}",
        forward_v2.tables[TableId::SupplyHealth.index()]
    );

    let mut hour_cursor = replay::campaign::HourCursor::new(Arc::new(mid_replay), &[TableId::SupplyHealth], false);
    let view_at = |cursor: &mut replay::campaign::HourCursor, hour: i32| {
        let replay::campaign::SeekResult::Frame(frame) = cursor.seek(1, hour).unwrap() else {
            panic!("expected frame at hour {hour}");
        };
        frame.tables[TableId::SupplyHealth.index()].clone()
    };
    let h111 = view_at(&mut hour_cursor, last_v1);
    assert!(
        matches!(&*h111, replay_frame::frame::TableView::Unavailable { reason } if reason.contains("SupplyHealth")),
        "HourCursor must withhold last v1 hour {last_v1}: {h111:?}"
    );
    let h112 = view_at(&mut hour_cursor, first_v2);
    assert!(
        matches!(&*h112, replay_frame::frame::TableView::Complete(_)),
        "HourCursor stepping from v1 to v2 must admit hour {first_v2}: {h112:?}"
    );
    let h113 = view_at(&mut hour_cursor, first_v2 + 1);
    assert!(
        matches!(&*h113, replay_frame::frame::TableView::Complete(_)),
        "HourCursor stepping to hour {} must stay complete: {h113:?}",
        first_v2 + 1
    );
    let back111 = view_at(&mut hour_cursor, last_v1);
    assert!(
        matches!(&*back111, replay_frame::frame::TableView::Unavailable { reason } if reason.contains("SupplyHealth")),
        "HourCursor backward seek must withhold v1 hour {last_v1}: {back111:?}"
    );
    let again112 = view_at(&mut hour_cursor, first_v2);
    assert!(
        matches!(&*again112, replay_frame::frame::TableView::Complete(_)),
        "HourCursor stepping back into v2 must admit hour {first_v2}: {again112:?}"
    );
}

#[test]
fn wholly_v2_top_query_does_not_rank_earlier_peak_before_from() {
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, schema::schema};
    use replay::campaign::Replay;
    use replay_journal::{columnar_file::{FileWriter, HourWriter}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::{CaptureMode, Identity, IdentityCodec, Roster, SampleBoundary, SessionStart, Verification},
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow,
            Recorded, TABLE_COUNT,
        },
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use std::io::Cursor;

    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let first = 100;
    let last = 189;
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"top-peak".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000)),
    );
    writer.start_run(1, envelope::epoch::START, first).unwrap();
    let mut synthetic = Synthetic::new(1, first).unwrap();
    let campaign = [CampaignRow { id: b"top-peak".to_vec() }];
    let countries = [
        CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() },
        CountryRow { id: 9002, canonical_slot: 8, tag: b"FRA".to_vec(), ..Default::default() },
    ];
    let links = [
        CountryLinkRow { id: 9001, canonical_slot: 7 },
        CountryLinkRow { id: 9002, canonical_slot: 8 },
    ];
    let divisions = [
        DivisionRow { id: 1, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() },
        DivisionRow { id: 2, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() },
        DivisionRow { id: 3, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() },
        DivisionRow { id: 4, roster_owner: 9002, logical_owner: 9002, province: 2, ..Default::default() },
    ];
    let land = [
        DivisionLandInputsRow { division: 1, naval_transfer: false, ..Default::default() },
        DivisionLandInputsRow { division: 2, naval_transfer: false, ..Default::default() },
        DivisionLandInputsRow { division: 3, naval_transfer: false, ..Default::default() },
        DivisionLandInputsRow { division: 4, naval_transfer: false, ..Default::default() },
    ];
    let status = |division: u64, exact: i64| DivisionStatusRow {
        division,
        supply_ratio: None,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(0),
        supply_ratio_exact: Recorded::Value(Some(exact)),
        supply_grace_exact: Recorded::Value(Some(0)),
        ..Default::default()
    };
    for hour in first..=last {
        let ger_ratio = if hour == 112 || (156..=161).contains(&hour) { 10_000 } else { 90_000 };
        let statuses = [
            status(1, ger_ratio),
            status(2, ger_ratio),
            status(3, ger_ratio),
            status(4, 10_000),
        ];
        synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionLandInputs, Input::Snapshot { rows: &land, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionStatus, Input::Snapshot { rows: &statuses, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
    }
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);
    let (extent, length) = container::locate(bytes.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(bytes.as_slice(), &extent).unwrap();
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut rewritten = container::ContainerWriter::resume(Cursor::new(bytes), root, at, 1, None).unwrap();
    rewritten.section(key, 2).unwrap();
    let mut payload = replay_columnar::leaf::Writer::new();
    payload.varint(0).unwrap();
    payload.varint(12).unwrap();
    let payload = payload.into_bytes();
    let payload_extent = rewritten.put(Kind::Stored, Tags::default(), payload.len() as u64, &payload).unwrap();
    rewritten.append(key, payload_extent).unwrap();
    rewritten.commit(None, true, |_| Ok(())).unwrap();
    let mixed = rewritten.into_inner().into_inner();

    let replay = Replay::open(mixed).unwrap();
    let top = replay_lanes::lanes::top(replay.recording(), 0, TableId::SupplyHealth, 0, 166, 189, 2, 1, replay_lanes::lanes::By::Max).unwrap();
    assert_eq!(top[0].len(), 1, "{top:?}");
    assert_eq!(top[0][0].0, 8, "earlier peak in 156..=161 must not win By::Max for 166..=189: {top:?}");
    assert_eq!(top[0][0].1.max, 1, "{top:?}");

    let edge_top = replay_lanes::lanes::top(replay.recording(), 0, TableId::SupplyHealth, 0, 112, 135, 2, 1, replay_lanes::lanes::By::Max).unwrap();
    assert_eq!(edge_top[0].len(), 1, "{edge_top:?}");
    assert_eq!(edge_top[0][0].0, 7, "valid v2 edge peak at hour 112 must win By::Max for 112..=135: {edge_top:?}");
    assert_eq!(edge_top[0][0].1.max, 3, "{edge_top:?}");
}

#[test]
fn walked_edge_top_preserves_cross_piece_by_last_order() {
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, schema::schema};
    use replay::campaign::Replay;
    use replay_journal::{columnar_file::{FileWriter, HourWriter}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::{CaptureMode, Identity, IdentityCodec, Roster, SampleBoundary, SessionStart, Verification},
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow,
            Recorded, TABLE_COUNT,
        },
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use std::io::Cursor;

    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let first = 100;
    let last = 1545;
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"cross-piece-last".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000)),
    );
    writer.start_run(1, envelope::epoch::START, first).unwrap();
    let mut synthetic = Synthetic::new(1, first).unwrap();
    let campaign = [CampaignRow { id: b"cross-piece-last".to_vec() }];
    let both_countries = [
        CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() },
        CountryRow { id: 9002, canonical_slot: 8, tag: b"FRA".to_vec(), ..Default::default() },
    ];
    let fra_only_countries = [CountryRow { id: 9002, canonical_slot: 8, tag: b"FRA".to_vec(), ..Default::default() }];
    let both_links = [
        CountryLinkRow { id: 9001, canonical_slot: 7 },
        CountryLinkRow { id: 9002, canonical_slot: 8 },
    ];
    let fra_only_links = [CountryLinkRow { id: 9002, canonical_slot: 8 }];
    let both_divisions: Vec<DivisionRow> = (1..=10)
        .map(|id| DivisionRow { id, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() })
        .chain((11..=12).map(|id| DivisionRow { id, roster_owner: 9002, logical_owner: 9002, province: 2, ..Default::default() }))
        .collect();
    let fra_only_divisions: Vec<DivisionRow> = (11..=12)
        .map(|id| DivisionRow { id, roster_owner: 9002, logical_owner: 9002, province: 2, ..Default::default() })
        .collect();
    let both_land: Vec<DivisionLandInputsRow> = (1..=12).map(|division| DivisionLandInputsRow { division, naval_transfer: false, ..Default::default() }).collect();
    let fra_only_land: Vec<DivisionLandInputsRow> = (11..=12).map(|division| DivisionLandInputsRow { division, naval_transfer: false, ..Default::default() }).collect();
    let status = |division: u64| DivisionStatusRow {
        division,
        supply_ratio: None,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(0),
        supply_ratio_exact: Recorded::Value(Some(10_000)),
        supply_grace_exact: Recorded::Value(Some(0)),
        ..Default::default()
    };
    let both_statuses: Vec<DivisionStatusRow> = (1..=12).map(status).collect();
    let fra_only_statuses: Vec<DivisionStatusRow> = (11..=12).map(status).collect();

    for hour in first..=last {
        let ger_present = hour <= 1539;
        synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: if ger_present { &both_countries } else { &fra_only_countries }, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: if ger_present { &both_links } else { &fra_only_links }, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Divisions, Input::Snapshot { rows: if ger_present { &both_divisions } else { &fra_only_divisions }, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionLandInputs, Input::Snapshot { rows: if ger_present { &both_land } else { &fra_only_land }, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionStatus, Input::Snapshot { rows: if ger_present { &both_statuses } else { &fra_only_statuses }, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
    }
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);
    let (extent, length) = container::locate(bytes.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(bytes.as_slice(), &extent).unwrap();
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut rewritten = container::ContainerWriter::resume(Cursor::new(bytes), root, at, 1, None).unwrap();
    rewritten.section(key, 2).unwrap();
    let mut payload = replay_columnar::leaf::Writer::new();
    payload.varint(0).unwrap();
    payload.varint(720).unwrap();
    let payload = payload.into_bytes();
    let payload_extent = rewritten.put(Kind::Stored, Tags::default(), payload.len() as u64, &payload).unwrap();
    rewritten.append(key, payload_extent).unwrap();
    rewritten.commit(None, true, |_| Ok(())).unwrap();
    let mixed = rewritten.into_inner().into_inner();

    let replay = Replay::open(mixed).unwrap();
    let stored_only = replay_lanes::lanes::top(replay.recording(), 0, TableId::SupplyHealth, 0, 822, 1545, 120, 1, replay_lanes::lanes::By::Last).unwrap();
    let walked_edge = replay_lanes::lanes::top(replay.recording(), 0, TableId::SupplyHealth, 0, 820, 1545, 121, 1, replay_lanes::lanes::By::Last).unwrap();
    assert_eq!(stored_only[119].len(), 1, "{:?}", stored_only[119]);
    assert_eq!(stored_only[119][0].0, 8, "stored-only cross-piece bin 1536..=1541 must rank FRA first: {:?}", stored_only[119]);
    assert_eq!(stored_only[119][0].1.last, 2, "{:?}", stored_only[119]);
    assert_eq!(walked_edge[120].len(), 1, "{:?}", walked_edge[120]);
    assert_eq!(walked_edge[120][0].0, 8, "edge-walked cross-piece bin 1536..=1541 must rank FRA first: {:?}", walked_edge[120]);
    assert_eq!(walked_edge[120][0].1.last, 2, "{:?}", walked_edge[120]);
    assert_eq!(walked_edge[0].len(), 1, "{:?}", walked_edge[0]);
    assert_eq!(walked_edge[0][0].0, 7, "walked leading edge bin 820..=821 must rank GER first: {:?}", walked_edge[0]);
    assert_eq!(walked_edge[0][0].1.last, 10, "{:?}", walked_edge[0]);
}

#[test]
fn negative_hours_mixed_chart_does_not_double_count_six_hour_bin() {
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, schema::schema};
    use replay::campaign::Replay;
    use replay_journal::{columnar_file::{FileWriter, HourWriter}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::{CaptureMode, Identity, IdentityCodec, Roster, SampleBoundary, SessionStart, Verification},
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow,
            Recorded, TABLE_COUNT,
        },
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use std::io::Cursor;

    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let first = -24;
    let last = -1;
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"neg-hours".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000)),
    );
    writer.start_run(1, envelope::epoch::START, first).unwrap();
    let mut synthetic = Synthetic::new(1, first).unwrap();
    let campaign = [CampaignRow { id: b"neg-hours".to_vec() }];
    let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
    let divisions = [DivisionRow { id: 1, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() }];
    let land = [DivisionLandInputsRow { division: 1, naval_transfer: false, ..Default::default() }];
    let statuses = [DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(0),
        supply_ratio_exact: Recorded::Value(Some(15_000)),
        supply_grace_exact: Recorded::Value(Some(0)),
        ..Default::default()
    }];
    for hour in first..=last {
        synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionLandInputs, Input::Snapshot { rows: &land, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::DivisionStatus, Input::Snapshot { rows: &statuses, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
    }
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);
    let (extent, length) = container::locate(bytes.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(bytes.as_slice(), &extent).unwrap();
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut rewritten = container::ContainerWriter::resume(Cursor::new(bytes), root, at, 1, None).unwrap();
    rewritten.section(key, 2).unwrap();
    let mut payload = replay_columnar::leaf::Writer::new();
    payload.varint(0).unwrap();
    payload.varint(12).unwrap();
    let payload = payload.into_bytes();
    let payload_extent = rewritten.put(Kind::Stored, Tags::default(), payload.len() as u64, &payload).unwrap();
    rewritten.append(key, payload_extent).unwrap();
    rewritten.commit(None, true, |_| Ok(())).unwrap();
    let mixed = rewritten.into_inner().into_inner();

    let replay = Replay::open(mixed).unwrap();
    let chart = replay_lanes::lanes::chart_at(replay.recording(), 0, TableId::SupplyHealth, 0, None, first, last, 0, 6).unwrap();
    let row = chart.keys.iter().position(|&k| k == 7).unwrap();
    assert_eq!(chart.values(row)[0].hours, 0);
    assert_eq!(chart.values(row)[1].hours, 0);
    assert_eq!(chart.values(row)[2].hours, 6, "negative 6h bin [-12,-7] must have 6 hours: {:?}", chart.values(row)[2]);
    assert_eq!(chart.life(row)[2], 6, "negative 6h bin [-12,-7] must have life 6: {:?}", chart.life(row));
    assert_eq!(chart.values(row)[3].hours, 6, "negative 6h bin [-6,-1] must have 6 hours: {:?}", chart.values(row)[3]);
    assert_eq!(chart.life(row)[3], 6, "negative 6h bin [-6,-1] must have life 6: {:?}", chart.life(row));
}

#[test]
fn damaged_derived_version_marker_withholds_table_without_failing_open() {
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, columnar_frames::DamageScope, schema::schema};
    use replay::campaign::Replay;
    use replay_journal::{columnar_file::{FileWriter, HourWriter}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::{CaptureMode, Identity, IdentityCodec, Roster, SampleBoundary, SessionStart, Verification},
        rows::{CampaignRow, ClockRow, CountryLinkRow, CountryRow, TABLE_COUNT},
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use std::io::Cursor;

    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let first = 100;
    let last = 111;
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"damaged-marker".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000)),
    );
    writer.start_run(1, envelope::epoch::START, first).unwrap();
    let mut synthetic = Synthetic::new(1, first).unwrap();
    let campaign = [CampaignRow { id: b"damaged-marker".to_vec() }];
    let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
    for hour in first..=last {
        synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
    }
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let base = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);
    let with_marker = |payload: Vec<u8>, corrupt: bool| -> Vec<u8> {
        let (extent, length) = container::locate(base.as_slice(), container::Open::Newest).unwrap().unwrap();
        let mut root = container::read_root(base.as_slice(), &extent).unwrap();
        root.sections.retain(|section| section.key != key);
        let at = container::Revision { number: root.revision, length, root: extent.digest };
        let mut rewritten = container::ContainerWriter::resume(Cursor::new(base.clone()), root, at, 1, None).unwrap();
        rewritten.section(key, 2).unwrap();
        let extent = rewritten.put(Kind::Stored, Tags::default(), payload.len() as u64, &payload).unwrap();
        rewritten.append(key, extent).unwrap();
        rewritten.commit(None, true, |_| Ok(())).unwrap();
        let mut out = rewritten.into_inner().into_inner();
        if corrupt {
            out[extent.offset as usize] ^= 0xff;
        }
        out
    };

    let mut valid_payload = replay_columnar::leaf::Writer::new();
    valid_payload.varint(0).unwrap();
    valid_payload.varint(6).unwrap();
    let corrupted_bytes = with_marker(valid_payload.into_bytes(), true);
    let corrupted = Replay::open(corrupted_bytes).expect("damaged SupplyHealth marker must not fail open");
    assert!(
        corrupted.recording().withheld_reason(TableId::SupplyHealth.index()).is_some_and(|why| why.contains("SupplyHealth")),
        "{:?}",
        corrupted.recording().withheld()
    );
    assert!(
        corrupted.recording().damage().iter().any(|damage| matches!(damage.scope, DamageScope::Withheld { table } if table == TableId::SupplyHealth.index() as u32)),
        "{:?}",
        corrupted.recording().damage()
    );
    let countries_snap = replay_lanes::lanes::at(corrupted.recording(), 0, TableId::Countries, first, None).unwrap();
    assert_eq!(countries_snap.rows.len(), 1);

    let mut out_of_range = replay_columnar::leaf::Writer::new();
    out_of_range.varint(0).unwrap();
    out_of_range.varint(999).unwrap();
    let invalid_boundary = Replay::open(with_marker(out_of_range.into_bytes(), false)).expect("out-of-range marker must not fail open");
    assert!(
        invalid_boundary.recording().withheld_reason(TableId::SupplyHealth.index()).is_some_and(|why| why.contains("SupplyHealth")),
        "{:?}",
        invalid_boundary.recording().withheld()
    );
    let countries_snap = replay_lanes::lanes::at(invalid_boundary.recording(), 0, TableId::Countries, first, None).unwrap();
    assert_eq!(countries_snap.rows.len(), 1);
}
