use super::*;
use crate::{
    fixture::{self, *},
    series::Line,
};

fn market<'q, 'r>(query: &'q Query<'r>) -> Reader<'q, 'r> {
    Reader::new(query, dashboard_declare::log("market-events").unwrap()).unwrap()
}

fn events(query: &Query<'_>, from: i32, to: i32, keep: impl Fn(i32, u8, u32) -> bool) -> Vec<(i32, u128, u8, u32)> {
    let mut out = Vec::new();
    for hour in from..=to {
        if query.timeline.at(hour).is_none() {
            continue;
        }
        let mut held: Vec<(i32, u128, u8, u32)> = fixture::market(hour).into_iter().filter(|&(_, kind, country)| keep(hour, kind, country)).map(|(id, kind, country)| (hour, u128::from(id), kind, country)).collect();
        held.sort_by_key(|event| event.1);
        out.extend(held);
    }
    out
}

fn number(row: &Row, reader: &Reader<'_, '_>, name: &str) -> i64 {
    let at = reader.columns().iter().position(|column| *column == name).unwrap();
    match row.cells[at] {
        Cell::Number(value) => value,
        ref other => panic!("{name} holds {other:?}"),
    }
}

#[test]
fn pages_forward_and_backward_hold_the_logs_rows_from_their_ordinals() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let selection = reader.select(&[], query.timeline.first, LAST).unwrap();
    assert_eq!(selection.groups, vec![None]);
    for (from, size) in [(FIRST, 7), (FIRST + 1_234, 50), (FIRST + FIRST_HOURS - 300, 13)] {
        let before = events(&query, FIRST, from - 1, |_, _, _| true).len() as u64;
        let wanted = events(&query, from, LAST, |_, _, _| true);
        let mut cursor = 0;
        let mut forward = Vec::new();
        loop {
            let page = reader.page(&selection.patterns, 1, from, LAST, cursor, size).unwrap();
            assert_eq!((page.base, page.total()), (before, wanted.len() as u64));
            assert_eq!(page.first, cursor.max(before));
            for (at, paged) in page.rows.iter().enumerate() {
                assert_eq!((paged.ordinal, paged.own), (page.first + at as u64, page.first + at as u64));
            }
            forward.extend(page.rows.iter().map(|paged| (paged.row.hour, paged.row.key, number(&paged.row, &reader, "kind") as u8, number(&paged.row, &reader, "country") as u32)));
            match page.cursor() {
                Some(next) => cursor = next,
                None => break,
            }
        }
        assert_eq!(forward, wanted, "pages from {from}, {size} a page");
        let mut start = before + wanted.len() as u64;
        while start > before {
            start = start.saturating_sub(u64::from(size)).max(before);
            let page = reader.page(&selection.patterns, 1, from, LAST, start, size).unwrap();
            let slice = &wanted[(start - before) as usize..((start - before) as usize + size as usize).min(wanted.len())];
            let got: Vec<(i32, u128)> = page.rows.iter().map(|paged| (paged.row.hour, paged.row.key)).collect();
            assert_eq!(got, slice.iter().map(|event| (event.0, event.1)).collect::<Vec<_>>(), "the page at {start} from {from}");
        }
    }
}

#[test]
fn walks_in_any_order_read_each_ranges_rows() {
    let last = FIRST + 17 * UNIT_HOURS - 1;
    let fixture = Fixture::new(Data::Market, FIRST..=last);
    let query = fixture.query();
    let reader = market(&query);
    let selection = reader.select(&[], query.timeline.first, last).unwrap();
    let run = selection.patterns[0].run;
    for (from, to) in [
        (FIRST + 3 * UNIT_HOURS + 5, FIRST + 3 * UNIT_HOURS + 40),
        (FIRST + 9 * UNIT_HOURS - 20, FIRST + 9 * UNIT_HOURS + 30),
        (FIRST + 100, FIRST + 130),
        (FIRST + 16 * UNIT_HOURS, last),
        (FIRST + 3 * UNIT_HOURS + 41, FIRST + 4 * UNIT_HOURS),
    ] {
        let got: Vec<(i32, u128)> = reader.rows(run, &selection.patterns, &[(from, to)]).unwrap().iter().map(|(_, row)| (row.hour, row.key)).collect();
        let wanted: Vec<(i32, u128)> = events(&query, from, to, |_, _, _| true).iter().map(|event| (event.0, event.1)).collect();
        assert!(!wanted.is_empty());
        assert_eq!(got, wanted, "{from}..={to}");
    }
}

#[test]
fn a_pass_over_a_run_gives_each_row_it_inserted_once_as_a_read_does() {
    let first = ITA_FROM - UNIT_HOURS;
    let (event_hour, event_key) = (first + 2 * UNIT_HOURS..).find_map(|hour| {
        fixture::market(hour).first().map(|&(key, ..)| (hour, u128::from(key)))
    }).unwrap();
    let fixture = Fixture::new(Data::Market, first..=event_hour);
    let query = fixture.query();
    let reader = market(&query);
    let run = reader.select(&[], query.timeline.first, query.timeline.last).unwrap().patterns[0].run;
    let (first, last) = reader.run_hours(run).unwrap();
    let mut got = Vec::new();
    reader.each(run, |row| {
        got.push(row);
        Ok(())
    }).unwrap();
    assert!(reader.walked.lock().unwrap().is_empty());
    assert!(got.windows(2).all(|pair| pair[0].hour <= pair[1].hour));
    assert!(got.iter().any(|row| row.hour == event_hour && row.key == event_key));
    got.sort_by_key(|row| (row.hour, row.key));
    let wanted = reader.inserted(run, &[(first as i32, last as i32)]).unwrap();
    assert!(!wanted.is_empty());
    assert_eq!(got, wanted);
}

#[test]
fn a_page_reads_only_the_buckets_holding_its_rows() {
    let buckets: BTreeMap<i64, u64> = [(600, 2), (612, 3), (630, 1), (660, 4)].into_iter().collect();
    assert_eq!(page_buckets(&buckets, 10, 12, 16, (600, 695)), (2, vec![(612, 617), (630, 635)]));
    assert_eq!(page_buckets(&buckets, 10, 10, 100, (604, 695)), (0, vec![(604, 605), (612, 617), (630, 635), (660, 665)]));
    let touching: BTreeMap<i64, u64> = [(612, 1), (618, 1)].into_iter().collect();
    assert_eq!(page_buckets(&touching, 0, 0, 2, (600, 695)), (0, vec![(612, 623)]));
    assert_eq!(page_buckets(&BTreeMap::new(), 0, 0, 2, (600, 695)), (0, vec![(600, 695)]));
}

#[test]
fn a_facet_filter_keeps_its_values_and_its_tags_holders_rows() {
    let fixture = Fixture::campaign_with_sealed_span(Data::Market);
    let query = fixture.query();
    let last = query.timeline.last;
    let second = query.timeline.stretches[1].first;
    let reader = market(&query);
    let kinds = FacetFilter { name: "kind".into(), values: vec![1, 7], countries: Vec::new() };
    let countries = FacetFilter { name: "country".into(), values: Vec::new(), countries: vec!["ITA".into(), "GER".into()] };
    for (filters, keep) in [
        (vec![kinds.clone()], Box::new(|_: i32, kind: u8, _: u32| kind == 1 || kind == 7) as Box<dyn Fn(i32, u8, u32) -> bool>),
        (vec![countries.clone()], Box::new(|_: i32, _: u8, _: u32| true)),
        (vec![kinds, countries], Box::new(|_: i32, kind: u8, _: u32| kind == 1 || kind == 7)),
    ] {
        let selection = reader.select(&filters, query.timeline.first, last).unwrap();
        let wanted = events(&query, FIRST, last, &keep);
        let tagged = filters.iter().any(|filter| !filter.countries.is_empty());
        let groups = selection.groups.len();
        let (mut rows, mut cursor) = (Vec::new(), 0);
        loop {
            let page = reader.page(&selection.patterns, groups, FIRST, last, cursor, 256).unwrap();
            assert_eq!(page.total(), wanted.len() as u64);
            rows.extend(page.rows.iter().cloned());
            match page.cursor() {
                Some(next) => cursor = next,
                None => break,
            }
        }
        let mut ordered = wanted.clone();
        let group_of = |country: u32| if tagged { usize::from(country == 9001) } else { 0 };
        ordered.sort_by_key(|event| (event.0, group_of(event.3), event.1));
        let got: Vec<(i32, u128)> = rows.iter().map(|paged| (paged.row.hour, paged.row.key)).collect();
        assert_eq!(got, ordered.iter().map(|event| (event.0, event.1)).collect::<Vec<_>>());
        let every: Vec<(i32, u128)> = market(&query).every(&selection.patterns, FIRST, last).unwrap().iter().map(|(_, row)| (row.hour, row.key)).collect();
        assert_eq!(every, got, "every row of {filters:?}");
        let mut own = vec![0u64; groups];
        for (paged, event) in rows.iter().zip(&ordered) {
            assert_eq!(paged.group, group_of(event.3));
            assert_eq!(paged.own, own[paged.group]);
            own[paged.group] += 1;
        }
        for bin_hours in [3, 6, 96] {
            let (first, last) = (FIRST + 700, second + 200);
            let counts = reader.counts(&selection.patterns, groups, first, last, bin_hours).unwrap();
            let first_bin = i64::from(first).div_euclid(bin_hours) * bin_hours;
            for (group, counts) in counts.iter().enumerate() {
                let mine = |event: &&(i32, u128, u8, u32)| group_of(event.3) == group;
                assert_eq!(counts.before, events(&query, FIRST, first - 1, &keep).iter().filter(mine).count() as u64, "bins of {bin_hours}");
                let mut bins: BTreeMap<u32, u64> = BTreeMap::new();
                for event in events(&query, first, last, &keep).iter().filter(mine) {
                    *bins.entry(((i64::from(event.0) - first_bin) / bin_hours) as u32).or_default() += 1;
                }
                assert_eq!(counts.bins, bins.into_iter().collect::<Vec<_>>(), "group {group}, bins of {bin_hours}");
            }
        }
    }
    assert!(reader.select(&[FacetFilter { name: "kind".into(), values: Vec::new(), countries: vec!["GER".into()] }], FIRST, last).is_err(), "a kind holds no country");
    assert!(reader.select(&[FacetFilter { name: "nope".into(), ..FacetFilter::default() }], FIRST, last).is_err());
}

#[test]
fn a_tag_filter_over_two_facets_keeps_the_rows_either_names() {
    let first = ITA_FROM - 120;
    let fixture = Fixture::with_gap(Data::Market, first..=LAST, FIRST + FIRST_HOURS..=SECOND - 1);
    let query = fixture.query();
    let reader = market(&query);
    let parties = FacetFilter { name: "country|receiver".into(), values: Vec::new(), countries: vec!["ITA".into(), "GER".into()] };
    let selection = reader.select(&[parties], query.timeline.first, LAST).unwrap();
    assert_eq!(selection.groups, vec![Some("ITA".into()), Some("GER".into())]);
    let mut wanted = Vec::new();
    for (hour, key, kind, country) in events(&query, first, LAST, |_, _, _| true) {
        let receiver = fixture::receiver(country, kind, hour);
        for (group, id) in [(0usize, 9002u32), (1, 9001)] {
            if country == id || receiver == id {
                wanted.push((hour, group, key));
            }
        }
    }
    wanted.sort_unstable();
    assert!(wanted.iter().any(|row| row.1 == 0) && wanted.windows(2).any(|pair| pair[0].0 == pair[1].0 && pair[0].2 == pair[1].2), "the fixture has rows both tags hold");
    let (mut got, mut cursor) = (Vec::new(), 0);
    loop {
        let page = reader.page(&selection.patterns, 2, first, LAST, cursor, 100).unwrap();
        assert_eq!(page.total(), wanted.len() as u64);
        got.extend(page.rows.iter().map(|paged| (paged.row.hour, paged.group, paged.row.key)));
        match page.cursor() {
            Some(next) => cursor = next,
            None => break,
        }
    }
    assert_eq!(got, wanted);
    for bin_hours in [3, 96] {
        let counts = reader.counts(&selection.patterns, 2, first, LAST, bin_hours).unwrap();
        for (group, counts) in counts.iter().enumerate() {
            assert_eq!(counts.bins.iter().map(|(_, count)| count).sum::<u64>(), wanted.iter().filter(|row| row.1 == group).count() as u64, "group {group}, bins of {bin_hours}");
        }
    }
    assert!(reader.select(&[FacetFilter { name: "kind|country".into(), values: vec![1], countries: Vec::new() }], first, LAST).is_err(), "values name one facet");
}

#[test]
fn balance_drops_page_and_mark_as_the_values_fall() {
    let fixture = Fixture::campaign(Data::Power);
    let query = fixture.query();
    let series = dashboard_declare::series("country-measure").unwrap();
    let line = Line::new(&query, series, series.measure("politicalPower").unwrap()).unwrap();
    let tags: Vec<String> = TAGS.map(str::to_owned).to_vec();
    let timeline = &query;
    let drops = |from: i32, to: i32| -> Vec<(i32, usize)> {
        (from..=to).flat_map(|hour| (0..2).filter(move |&at| held(timeline, TAGS[at], hour) && dropped(9001 + at as u32, hour)).map(move |at| (hour, at))).collect()
    };
    let from = FIRST + 3_000;
    let wanted = drops(from, LAST);
    let before = drops(FIRST, from - 1);
    let mut own = [0u64; 2];
    for &(_, at) in &before {
        own[at] += 1;
    }
    let mut seen = Vec::new();
    let mut cursor = 0u64;
    loop {
        let page = line.page(&tags, from, LAST, cursor, 31).unwrap();
        assert_eq!(page.total, wanted.len().to_string());
        assert_eq!(page.first_ordinal, cursor.max(before.len() as u64).to_string());
        for event in &page.rows {
            let at = TAGS.iter().position(|tag| event.country.tag() == Some(*tag)).unwrap();
            assert_eq!(event.ordinal, own[at].to_string(), "{}'s drop at {}", event.country, event.hour);
            own[at] += 1;
            let id = 9001 + at as u32;
            assert_eq!((event.row.before_raw.clone(), event.row.after_raw.clone()), (power(id, event.hour - 1).unwrap().to_string(), power(id, event.hour).unwrap().to_string()));
            seen.push((event.hour, at));
        }
        match page.cursor {
            Some(next) => cursor = next.parse().unwrap(),
            None => break,
        }
    }
    assert_eq!(seen, wanted);
    let (view_from, view_to) = (FIRST + 2_000, FIRST + 2_500);
    let marks = line.marks_of(&tags, view_from, view_to, 120).unwrap();
    for (at, summary) in marks.iter().enumerate() {
        let total: u64 = summary.clusters.iter().map(|cluster| cluster.count.parse::<u64>().unwrap()).sum();
        let mine = |hours: Vec<(i32, usize)>| hours.into_iter().filter(|&(_, held)| held == at).count() as u64;
        assert_eq!(total, mine(drops(view_from, view_to)));
        if let Some(first) = summary.clusters.first() {
            assert_eq!(first.first_ordinal, mine(drops(FIRST, view_from - 1)).to_string());
        }
    }
}
