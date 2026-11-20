use super::*;
use crate::{
    fixture::{self, *},
    tile::{self, BINS, PRESENT, RUN},
};
use dashboard_declare::{Carry, Column, Key};
use replay_columnar::summary::Cells;
use replay_data::rows::TableId;

fn resources_line<'q>(query: &'q Query<'static>, measure: &str) -> Line<'q, 'static> {
    let series = dashboard_declare::series("country-resources").unwrap();
    Line::new(query, series, series.measure(measure).unwrap()).unwrap()
}

/// `id`'s line of `resource` at `hour` as the fixture wrote it, where the timeline and the tag's
/// holder hold the hour: its row's `produced`, or none without a row.
fn produced(query: &Query<'_>, id: u32, resource: u32, hour: i32) -> Option<i64> {
    (held(query, tag_of(id), hour) && fixture::resources(id, hour).contains(&resource)).then(|| fixture::resource(id, resource, hour).produced)
}

fn read(tag: &str, hour: i32, part: Option<u32>) -> SeriesRead {
    SeriesRead { country: tag.into(), hour, part: part.map(|part| part.to_string()) }
}

const HOURS: [i32; 9] = [FIRST, FIRST + 1_999, FIRST + 2_000, FIRST + 4_321, FIRST + 5_999, ITA_FROM + 3, FIRST + FIRST_HOURS - 1, SECOND + 17, LAST];

/// One part's state at hours across a part coming and going, a country coming into being and the
/// hours the timeline does not hold, against the fixture's rows.
#[test]
fn a_state_of_one_part_is_the_parts_row_at_the_hour() {
    let query = query();
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

/// A read without a part reads every part the owner holds at the hour, in key order; with a top,
/// its greatest parts first.
#[test]
fn a_state_of_every_part_and_of_the_top_parts_is_the_owners_rows_at_the_hour() {
    let query = query();
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

/// Tiles of two parts: hourly tiles carry each part's hours as the fixture wrote them, and six-hour
/// tiles each bin's first, last, least and greatest of them, across a part coming and going.
#[test]
fn tiles_of_two_parts_hold_each_parts_values() {
    let query = query();
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
                // A bin at a stretch's edge holds its whole bucket: only bins inside one are judged.
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

/// A chart of a line its table holds no row of names every hour its holder holds as not recorded,
/// never blank; one of a part that comes and goes names the hours outside its rows; one over a table
/// never available names that. Bins of every width and a tile say the same hours.
#[test]
fn a_chart_names_the_hours_its_line_holds_no_row_in_and_its_table_was_not_available() {
    let query = query();
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

/// The next change of a part's line is the next hour its value moves or its row comes or goes.
#[test]
fn the_next_change_of_a_part_is_the_next_hour_its_row_moves_comes_or_goes() {
    let query = query();
    let line = resources_line(&query, "produced");
    let measure = line.series().measure("produced").unwrap();
    let bounds = (FIRST, FIRST + FIRST_HOURS - 1);
    for (id, resource, origin) in [(9001, 4, FIRST + 1_000), (9001, 4, FIRST + 5_950), (9001, 1, FIRST + 3_001), (9002, 2, ITA_FROM + 100), (9002, 1, ITA_FROM - 400)] {
        let ask = Ask { country: tag_of(id).into(), part: u128::from(resource) };
        let found = line.navigate_lines(std::slice::from_ref(&ask), &[measure], Some(origin), NavigationKey::ArrowRight, bounds).unwrap();
        let held = |hour: i32| held(&query, tag_of(id), hour);
        let first_held = (bounds.0..=bounds.1).find(|&hour| held(hour)).unwrap();
        let wanted = (origin + 1..=bounds.1).find(|&hour| hour == first_held || (hour > first_held && produced(&query, id, resource, hour) != produced(&query, id, resource, hour - 1)));
        assert_eq!(line.change_lines(std::slice::from_ref(&ask), &[measure], Some(origin), NavigationKey::ArrowRight, bounds).unwrap(), wanted, "{} part {resource} after {origin}: none when nothing changes after it", ask.country);
        assert_eq!(found, wanted.or(Some(origin)), "{} part {resource} after {origin}: the origin itself when nothing changes after it, as main", ask.country);
        let back = line.navigate_lines(std::slice::from_ref(&ask), &[measure], Some(origin), NavigationKey::ArrowLeft, bounds).unwrap();
        let wanted = (bounds.0..origin).rev().find(|&hour| hour == first_held || (hour > first_held && produced(&query, id, resource, hour) != produced(&query, id, resource, hour - 1)));
        assert_eq!(back, wanted.or(Some(origin)), "{} part {resource} before {origin}: the origin itself when nothing changes before it, as main", ask.country);
    }
    // Main's reached-hour fallback: a step off either edge of the bounds answers the edge, an origin past them is held
    // within them, and a step with no origin and nothing to find is none.
    let ask = Ask { country: tag_of(9001).into(), part: 4 };
    let step = |origin: Option<i32>, key| line.navigate_lines(std::slice::from_ref(&ask), &[measure], origin, key, bounds).unwrap();
    assert_eq!(step(Some(bounds.0), NavigationKey::ArrowLeft), Some(bounds.0));
    assert_eq!(step(Some(bounds.1), NavigationKey::ArrowRight), Some(bounds.1));
    assert_eq!(step(Some(bounds.1 + 50), NavigationKey::ArrowRight), Some(bounds.1));
    assert_eq!(step(None, NavigationKey::ArrowLeft), line.change_lines(std::slice::from_ref(&ask), &[measure], None, NavigationKey::ArrowLeft, bounds).unwrap());
}

/// A row the game lists only while nonzero reads 0 where it is absent while the table is complete
/// and its owner's slice was read, and has no value where either is not so; its own values stay.
#[test]
fn an_absent_row_of_a_table_listing_only_nonzero_values_is_zero_only_where_its_owner_was_read() {
    assert!(dashboard_declare::absent_is_zero(TableId::RelationOpinions));
    assert_eq!(meet(&[(0, 9), (20, 29)], &[(5, 24), (28, 40)]), [(5, 9), (20, 24), (28, 29)]);
    // A row from 10 to 14 (one value), absent until 10 and again from 15; the owner read from 3 to 30, the table
    // complete throughout but 25 to 26.
    let held: read::Runs = vec![(10, 12, Some(7)), (13, 14, Some(9)), (15, 40, None)];
    let zero = meet(&[(3, 30)], &[(0, 24), (27, 50)]);
    assert_eq!(zero_absent(&held, &zero, 0, 40), [(0, 2, None), (3, 9, Some(0)), (10, 12, Some(7)), (13, 14, Some(9)), (15, 24, Some(0)), (25, 26, None), (27, 30, Some(0)), (31, 40, None)]);
    assert_eq!(zero_absent(&Vec::new(), &[(5, 6)], 0, 9), [(0, 4, None), (5, 6, Some(0)), (7, 9, None)], "a pair never listed");
    assert_eq!(zero_absent(&Vec::new(), &[], 0, 9), [(0, 9, None)], "an owner never read is not known");
}

/// The game lists a pair's opinion under any raw engine reference of its two countries: a pair's
/// line reads its rows under every raw id linked to either country while the link holds, never
/// only the holders' own ids (which would read the pair as never listed: zero).
#[test]
fn a_pair_listed_under_other_raw_ids_of_its_countries_is_its_line() {
    // Raw 1 and 7 link to slot 10 (7 only from 20 to 60); raw 2 and 8 to slot 20; raw 9 to slot 10 from 70.
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
    // The pair (1, 2): listed under 7:8 from 30 to 49 at 35, under 1:2 from 50 to 54 at 40; a row under
    // 7:8 after 60 (7 is another country's then) is not the pair's.
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

/// A step to the next read of a table lands on the next hour the table was read, held or not by a
/// change of its value, across the hours the timeline does not hold, never on the origin.
#[test]
fn the_next_read_of_a_table_is_the_next_hour_it_was_read_across_a_gap_and_never_the_origin() {
    let query = query();
    let tension = TableId::WorldTension;
    let step = |origin: Option<i32>, key| next_read(&query, tension, origin, key, (FIRST, LAST)).unwrap();
    let still = (FIRST + 30..).find(|hour| fixture::tension(*hour) == fixture::tension(hour + 1)).unwrap();
    assert_eq!(step(Some(still), NavigationKey::ArrowRight), Some(still + 1), "read again though the tension holds");
    assert_eq!(step(Some(still + 1), NavigationKey::ArrowLeft), Some(still));
    let end = FIRST + FIRST_HOURS - 1;
    assert_eq!(step(Some(end), NavigationKey::ArrowRight), Some(SECOND), "the hours another history holds are no read");
    assert_eq!(step(Some(SECOND), NavigationKey::ArrowLeft), Some(end));
    assert_eq!(step(Some(FIRST), NavigationKey::ArrowLeft), None, "nothing before the first read");
    assert_eq!(step(Some(LAST), NavigationKey::ArrowRight), None);
    assert_eq!(step(None, NavigationKey::Home), Some(FIRST));
    assert_eq!(step(Some(FIRST + 9), NavigationKey::End), Some(LAST));
    assert_eq!(step(Some(FIRST + 100), NavigationKey::ArrowRight), Some(FIRST + 101), "an origin far from either edge");
    assert_eq!(next_read(&query, tension, Some(FIRST + 10), NavigationKey::ArrowRight, (FIRST + 50, FIRST + 60)).unwrap(), Some(FIRST + 50), "an origin before the bounds steps into them");
    assert!(next_read(&query, TableId::NavalSupremacy, Some(FIRST), NavigationKey::ArrowRight, (FIRST, LAST)).unwrap().is_none(), "a table never available is never read");
}

/// The parts each owner held over a span, with the hours it held each, cut to its holders.
#[test]
fn an_owners_parts_over_a_span_are_its_rows_lifetimes() {
    let query = query();
    let line = resources_line(&query, "produced");
    let tags = ["GER".to_owned(), "ITA".to_owned()];
    for (from, to) in [(FIRST, LAST), (FIRST + 3_000, FIRST + 4_000), (FIRST + 5_500, SECOND + 100)] {
        let owners = line.part_lives(&tags, from, to).unwrap();
        for (owner, id) in owners.iter().zip([9001, 9002]) {
            let mut wanted: BTreeMap<u32, (i32, i32)> = BTreeMap::new();
            for hour in from..=to {
                for resource in [1, 2, 3, 4] {
                    if produced(&query, id, resource, hour).is_some() {
                        let span = wanted.entry(resource).or_insert((hour, hour));
                        span.1 = hour;
                    }
                }
            }
            let got: BTreeMap<u32, (i32, i32)> = owner.parts.iter().map(|life| (life.part.parse().unwrap(), (life.first_hour, life.last_hour))).collect();
            assert_eq!(got, wanted, "{}'s parts over {from}..{to}", owner.country);
            assert!(owner.parts.iter().all(|life| life.id.as_deref() == Some(life.part.as_str())), "a resource part names its resource");
        }
        if to > FIRST + FIRST_HOURS {
            assert!(owners[0].missing.iter().any(|span| span.reason == GAP_REASON));
        }
    }
    // A table never available over the span holds no parts, and says each hour is missing, never that none were held.
    let fuel = dashboard_declare::series("fuel-consumers").unwrap();
    let line = Line::new(&query, fuel, fuel.measure("requested").unwrap()).unwrap();
    let owners = line.part_lives(&tags[..1], FIRST, FIRST + 100).unwrap();
    assert!(owners[0].parts.is_empty());
    let missing: Vec<(i32, i32)> = owners[0].missing.iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(missing, [(FIRST, FIRST + 100)]);
    assert!(owners[0].missing[0].reason.contains("not available"), "{}", owners[0].missing[0].reason);
}

/// CountryResources' `special_total` and `production_demand`, as the schema numbers them.
const SPECIAL_TOTAL: usize = 4;
const PRODUCTION_DEMAND: usize = 5;

fn surplus(cells: &dyn Cells, slot: u32) -> Option<i64> {
    cells.number(SPECIAL_TOTAL, slot)?.checked_add(cells.number(PRODUCTION_DEMAND, slot)?)
}

/// A computed column of a sub-keyed series no declaration summarizes: what remains of each
/// resource after special projects less what production lines need.
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
};

/// A computed column of one row: the surplus of each part, at every hour asked.
#[test]
fn a_computed_part_state_is_its_formula_over_the_row() {
    assert_eq!(dashboard_declare::field(TableId::CountryResources, "special_total").unwrap(), SPECIAL_TOTAL);
    assert_eq!(dashboard_declare::field(TableId::CountryResources, "production_demand").unwrap(), PRODUCTION_DEMAND);
    let query = query();
    let line = Line::new(&query, &SURPLUS, &SURPLUS.measures[0]).unwrap();
    for hour in HOURS {
        let states = line.states(&[read("GER", hour, Some(2))], None).unwrap();
        let row = fixture::resource(9001, 2, hour);
        let wanted = held(&query, "GER", hour).then(|| row.special_total + row.production_demand);
        assert_eq!(states[0].run.as_ref().map(|run| run.value_raw.parse::<i64>().unwrap()), wanted, "GER's surplus of 2 at {hour}");
    }
}

/// CountryRow's `political_power_raw` and `command_power_raw`, as the schema numbers them.
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
};

/// A rank at an hour of a computed column ranks the formula's value of each row read at the hour
/// (a row whose input is not recorded is not ranked), as walked rows rank.
#[test]
fn a_rank_at_an_hour_of_a_computed_column_ranks_the_formula_of_each_row() {
    assert_eq!(dashboard_declare::field(TableId::Countries, "political_power_raw").unwrap(), POWER);
    assert_eq!(dashboard_declare::field(TableId::Countries, "command_power_raw").unwrap(), COMMAND);
    let query = query();
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

/// A series without parts asked for its top N: at an hour, the countries greatest there with their states, greatest
/// first; over a view, lines of the members of each bin only; the keyboard steps through the changes of the
/// countries ranking first at the cursor.
#[test]
fn a_ranked_series_answers_its_top_countries_at_an_hour_over_a_view_and_by_key() {
    let query = query();
    let ask = |product: &str, extra: serde_json::Value| {
        let mut params = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" } });
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        answer(&query, product, &params).unwrap().unwrap()
    };
    for hour in HOURS {
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
    let chart: ranked::SeriesRankedReply = serde_json::from_value(ask("series-summary", serde_json::json!({ "top": 1, "from": FIRST, "to": LAST, "width": 64 }))).unwrap();
    assert!(!chart.lines.is_empty());
    for line in &chart.lines {
        assert!(!line.membership.is_empty() && !line.parts.is_empty(), "{} ranks somewhere and is held there", line.tag);
        for (bin, bucket) in line.summary.buckets.iter().enumerate() {
            let Some(bucket) = bucket else { continue };
            assert!(line.membership.iter().any(|span| span.first_hour <= bucket.last_hour && bucket.first_hour <= span.last_hour), "{} bin {bin} lies in its membership", line.tag);
        }
    }
    // A view ending inside a summary bucket ends its members' last bins at its last hour, with the value held there,
    // never one read after it, whatever the edge's hours are on any grid of their own length.
    let mut checked = 0;
    for to in (FIRST + 300..FIRST + FIRST_HOURS - 10).filter(|hour| (hour + 1).rem_euclid(6) != 0).take(24) {
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
    // A stretch's last bin, before another history's hours, ranks at its last hour and holds only its own hours' values.
    let end = FIRST + FIRST_HOURS - 1;
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
    let tiles = serde_json::json!({ "top": null, "from": FIRST, "to": LAST, "width": 64, "countries": ["GER"] });
    assert!(answer(&query, "series-summary", &{
        let mut params = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" } });
        params.as_object_mut().unwrap().extend(tiles.as_object().unwrap().clone());
        params
    })
    .is_err(), "chosen lines are tiles");
    let origin = ITA_FROM + 10;
    let next: SeriesKeyReply = serde_json::from_value(ask("series-next-change", serde_json::json!({ "top": 1, "key": "ArrowRight", "from": origin, "firstHour": FIRST, "lastHour": LAST }))).unwrap();
    let leader = line_of(&query).rank_at(origin, 1).unwrap();
    let wanted = line_of(&query).navigate(&leader, &[dashboard_declare::series("country-measure").unwrap().measure("politicalPower").unwrap()], Some(origin), NavigationKey::ArrowRight, (FIRST, LAST)).unwrap();
    assert_eq!(next.hour, wanted.map(f64::from));
    assert!(next.hour.is_some_and(|hour| hour > f64::from(origin)));
}

/// A selection of 1,500 countries, as a campaign with that many may make, is answered whole: its states at an hour,
/// a ranked chart of its top 1,500, its navigation and its drops. A tag no country holds reads as missing.
#[test]
fn a_request_naming_1500_countries_is_answered() {
    let query = query();
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
    let chart: ranked::SeriesRankedReply = serde_json::from_value(ask("series-summary", serde_json::json!({ "top": 1_500, "from": FIRST, "to": LAST, "width": 64 }))).unwrap();
    assert_eq!(chart.lines.iter().map(|line| line.tag.as_str()).collect::<std::collections::BTreeSet<_>>(), TAGS.into_iter().collect());
    let next: SeriesKeyReply =
        serde_json::from_value(ask("series-next-change", serde_json::json!({ "countries": tags, "key": "ArrowRight", "from": hour, "firstHour": FIRST, "lastHour": LAST }))).unwrap();
    assert!(next.hour.is_some_and(|next| next > f64::from(hour)));
    ask("events-page", serde_json::json!({ "countries": tags, "from": FIRST, "to": LAST, "pageSize": 16 }));
}

fn line_of<'q>(query: &'q Query<'static>) -> Line<'q, 'static> {
    let series = dashboard_declare::series("country-measure").unwrap();
    Line::new(query, series, series.measure("politicalPower").unwrap()).unwrap()
}

/// A ranked view of stretches each shorter than a summary bucket ranks and reads every one of them exactly, drawn
/// hourly or in bins: two five-hour stretches either side of a load are one line over all ten hours.
#[test]
fn a_ranked_view_of_stretches_shorter_than_a_bucket_reads_each_exactly() {
    use crate::timeline::{Join, Stretch, Timeline};
    let mut query = query();
    let first = ITA_FROM.div_euclid(6) * 6 + 600;
    let last = first + 9;
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

/// A short stretch's ranked bin names a key by its tag at the bin's last hour and reads only the hours that tag holds
/// it: ITA's key held by AAA for the stretch's first three hours lends ITA none of AAA's values or membership.
#[test]
fn a_ranked_short_stretch_reads_its_winner_only_over_its_tags_hours() {
    use crate::identity::{Holder, HolderRun, Holders};
    use crate::timeline::{Join, Stretch, Timeline};
    let mut query = query();
    let first = ITA_FROM.div_euclid(6) * 6 + 600;
    let last = first + 9;
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

/// A faction's line is its own row of the recorder's totals, named by its id (never a country's tag), and each bin's
/// least and greatest are the faction's own: its members' exact sums at the bin's hours, not their bounds.
#[test]
fn a_factions_line_is_its_row_and_its_bins_hold_its_members_exact_sums() {
    let query = query();
    let totals = dashboard_declare::series("faction-totals").unwrap();
    let line = Line::new(&query, totals, totals.measure("controlledStates").unwrap()).unwrap();
    let (from, to) = (FIRST, FIRST + FIRST_HOURS - 1);
    let charts = line.summaries(&[FACTION.to_string()], from, to, 64).unwrap();
    let mut checked = 0;
    for bucket in charts[0].0.buckets.iter().flatten() {
        let sums: Vec<i64> = (bucket.first_hour..=bucket.last_hour).filter_map(faction_sum).collect();
        let (least, most) = (*sums.iter().min().unwrap(), *sums.iter().max().unwrap());
        assert_eq!((bucket.min_value, bucket.max_value), (least as f64, most as f64), "bin {}..{}", bucket.first_hour, bucket.last_hour);
        checked += 1;
    }
    assert!(checked > 1, "the view's bins hold the faction's values");
    assert!(line.summaries(&["GER".to_owned()], from, to, 64).unwrap()[0].0.buckets.iter().all(Option::is_none), "a faction is named by its id");
    assert!(line.rank_at(from, 1).is_err(), "factions are not ranked as countries are");
}

static WORLD: Series = Series {
    kind: "test-world",
    table: TableId::WorldTension,
    key: Key::Singleton,
    parts: None,
    measures: &[Measure { name: "tension", column: Column::Cell("total_raw"), decimals: 3, aggregates: 0, carry: Carry::None, rank: 0 }],
    drops: false,
};

/// A singleton's line is its one row (key 0) in every stretch whatever the line is named, an hour
/// the timeline does not hold is missing with the recording's reason, and it is never ranked.
#[test]
fn a_singleton_line_reads_its_one_row_in_every_stretch_whatever_it_is_named_and_is_never_ranked() {
    let query = query();
    let line = Line::new(&query, &WORLD, &WORLD.measures[0]).unwrap();
    let wanted: Vec<(u32, i32, i32, u128)> = query.timeline.within(FIRST, LAST).map(|stretch| (stretch.run, stretch.first, stretch.last, 0)).collect();
    for name in ["GER", "no such tag"] {
        let held: Vec<(u32, i32, i32, u128)> = line.held(&Ask::whole(name), FIRST, LAST).iter().map(|part| (part.run, part.first, part.last, part.key)).collect();
        assert_eq!(held, wanted, "{name}");
    }
    let states = line.states(&[read("no such tag", FIRST + 3, None), read("no such tag", SECOND - 1, None)], None).unwrap();
    assert_eq!(states[0].missing, None, "a held hour is never refused for its name");
    assert_eq!(states[0].run.as_ref().map(|run| run.value_raw.clone()), Some(fixture::tension(FIRST + 3).to_string()), "the one row's value");
    assert_eq!(states[1].missing.as_deref(), Some(query.timeline.reason(SECOND - 1).as_str()));
    assert!(line.rank_at(FIRST, 2).is_err());
    assert!(line.ranked(FIRST, LAST, 64, 2).is_err());
    // Its tile is its one row, though no country holds the name it is asked by.
    let hour = FIRST + 3;
    let reply = crate::tile::tile(&line, "1", &[Ask::whole("World")], None, 1, i64::from(hour).div_euclid(BINS as i64), false).expect("the world's line is a tile");
    let runs = crate::tile::decode_runs(&unbase64(&reply.bins), 1, reply.first_hour, line.scale(), reply.raw_values).unwrap();
    let held = runs[0].iter().find(|run| run.0 <= hour && hour <= run.1).expect("a run holds the hour");
    assert_eq!(held.2, fixture::tension(hour) as f64 / 1_000.0);
}

/// A rank at an hour names the countries greatest there, greatest first (ties by key), each with
/// its state (a ranked `series-state`); a top of none is refused.
#[test]
fn a_rank_names_the_greatest_countries_at_the_hour_with_their_states() {
    let query = query();
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
