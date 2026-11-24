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
    let kinds = FacetFilter { name: "kind".into(), values: vec![1, 7], ..Default::default() };
    let countries = FacetFilter { name: "country".into(), countries: vec!["ITA".into(), "GER".into()], ..Default::default() };
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
    assert!(reader.select(&[FacetFilter { name: "kind".into(), countries: vec!["GER".into()], ..Default::default() }], FIRST, last).is_err(), "a kind holds no country");
    assert!(reader.select(&[FacetFilter { name: "nope".into(), ..FacetFilter::default() }], FIRST, last).is_err());
}

#[test]
fn a_tag_filter_over_two_facets_preserves_each_pattern_by_default_and_deduplicates_when_opted_in() {
    let first = ITA_FROM - 120;
    let fixture = Fixture::with_gap(Data::Market, first..=LAST, FIRST + FIRST_HOURS..=SECOND - 1);
    let query = fixture.query();
    let reader = market(&query);
    let default_parties = FacetFilter { name: "country|receiver".into(), countries: vec!["ITA".into(), "GER".into()], ..Default::default() };
    let default_selection = reader.select(&[default_parties], query.timeline.first, LAST).unwrap();
    assert_eq!(default_selection.overlap, Overlap::EachPattern, "multi-facet country filter defaults to EachPattern for existing callers");

    let parties = FacetFilter { name: "country|receiver".into(), countries: vec!["ITA".into(), "GER".into()], deduplicate: true, ..Default::default() };
    let selection = reader.select(&[parties], query.timeline.first, LAST).unwrap();
    assert_eq!(selection.overlap, Overlap::FirstMatch);
    assert_eq!(selection.groups, vec![Some("ITA".into()), Some("GER".into())]);
    let mut wanted = Vec::new();
    let mut each_wanted = Vec::new();
    let mut both_matched = 0usize;
    for (hour, key, kind, country) in events(&query, first, LAST, |_, _, _| true) {
        let receiver = fixture::receiver(country, kind, hour);
        let ita = country == 9002 || receiver == 9002;
        let ger = country == 9001 || receiver == 9001;
        if ita && ger {
            both_matched += 1;
        }
        if ita {
            wanted.push((hour, 0usize, key));
            each_wanted.push((hour, 0usize, key));
        } else if ger {
            wanted.push((hour, 1usize, key));
        }
        if ger {
            each_wanted.push((hour, 1usize, key));
        }
    }
    wanted.sort_unstable();
    each_wanted.sort_unstable();
    assert!(both_matched > 0, "the fixture has rows both selected parties hold");
    assert_eq!(reader.total_with(&default_selection.patterns, first, LAST, default_selection.overlap).unwrap(), each_wanted.len() as u64);
    assert!(wanted.iter().any(|row| row.1 == 0) && wanted.iter().any(|row| row.1 == 1));
    let (mut got, mut cursor) = (Vec::new(), 0);
    let mut seen_keys = HashSet::new();
    loop {
        let page = reader.page_with(&selection.patterns, 2, first, LAST, cursor, 100, selection.overlap).unwrap();
        assert_eq!(page.total(), wanted.len() as u64);
        for (idx, paged) in page.rows.iter().enumerate() {
            assert_eq!(paged.ordinal, page.first + idx as u64);
            assert!(seen_keys.insert((paged.row.hour, paged.row.key)), "each event appears once");
            got.push((paged.row.hour, paged.group, paged.row.key));
        }
        match page.cursor() {
            Some(next) => cursor = next,
            None => break,
        }
    }
    assert_eq!(got, wanted);
    let mut display_wanted = wanted.clone();
    display_wanted.sort_by_key(|&(hour, group, key)| (Reverse(hour), group, key));
    let tail = reader.newest_page_with(&selection.patterns, 2, first, LAST, 0, 50, selection.overlap).unwrap();
    assert_eq!(tail.total, wanted.len() as u64);
    let tail_got: Vec<_> = tail.rows.iter().map(|paged| (paged.row.hour, paged.group, paged.row.key)).collect();
    assert_eq!(tail_got, display_wanted[..50]);
    for bin_hours in [3, 96] {
        let counts = reader.counts_with(&selection.patterns, 2, first, LAST, bin_hours, selection.overlap).unwrap();
        let sum: u64 = counts.iter().map(|c| c.bins.iter().map(|(_, count)| *count).sum::<u64>()).sum();
        assert_eq!(sum, wanted.len() as u64, "counts sum to deduplicated total, bins of {bin_hours}");
        for (group, counts) in counts.iter().enumerate() {
            assert_eq!(counts.bins.iter().map(|(_, count)| count).sum::<u64>(), wanted.iter().filter(|row| row.1 == group).count() as u64, "group {group}, bins of {bin_hours}");
        }
    }
    assert!(reader.select(&[FacetFilter { name: "kind|country".into(), values: vec![1], ..Default::default() }], first, LAST).is_err(), "values name one facet");
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

#[test]
fn tail_matches_the_newest_rows_of_page_in_display_order_across_ranges_and_limits() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let filters = [
        Vec::new(),
        vec![FacetFilter { name: "kind".into(), values: vec![1, 3], ..Default::default() }],
        vec![FacetFilter { name: "country".into(), countries: vec![Country::from("GER"), Country::from("ITA")], ..Default::default() }],
    ];
    for facets in filters {
        for (from, to, limit) in [
            (FIRST, LAST, 0),
            (FIRST, LAST, 1),
            (FIRST, LAST, 17),
            (FIRST, LAST, 64),
            (FIRST + 1_234, FIRST + 2_500, 13),
            (FIRST + 1_234, FIRST + 1_237, 50),
            (FIRST + FIRST_HOURS - 300, LAST, 500),
        ] {
            let selection = reader.select(&facets, query.timeline.first, to).unwrap();
            let tail = reader.newest_page_with(&selection.patterns, selection.groups.len(), from, to, 0, limit, selection.overlap).unwrap();
            let total = reader.total_with(&selection.patterns, from, to, selection.overlap).unwrap();
            assert_eq!(tail.total, total, "tail total at {from}..={to} limit={limit}");
            let whole = reader.page_with(&selection.patterns, selection.groups.len(), from, to, 0, total.max(1) as u32, selection.overlap).unwrap();
            let mut expected: Vec<_> = whole.rows.iter().map(|row| (row.row.hour, row.row.key, row.ordinal, row.own, row.group)).collect();
            expected.sort_by_key(|&(hour, key, _, _, group)| (Reverse(hour), group, key));
            expected.truncate(limit as usize);
            let got: Vec<_> = tail.rows.iter().map(|row| (row.row.hour, row.row.key, row.ordinal, row.own, row.group)).collect();
            assert_eq!(got, expected, "tail vs display-ordered page at {from}..={to} limit={limit}");
        }
    }
}

#[test]
fn total_matches_exact_row_count_including_partial_tail_bucket() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let facets = vec![FacetFilter { name: "kind".into(), values: vec![2], ..Default::default() }];
    let from = FIRST + 601;
    for offset in 0..18 {
        let to = from + offset;
        let selection = reader.select(&facets, query.timeline.first, to).unwrap();
        let got = reader.total_with(&selection.patterns, from, to, selection.overlap).unwrap();
        let expected = events(&query, from, to, |_, kind, _| kind == 2).len() as u64;
        assert_eq!(got, expected, "exact total for partial bin at {from}..={to}");
    }
}

#[test]
fn dense_bucket_tail_decodes_only_requested_rows_and_bounded_cache_matches_uncached() {
    let first = 600;
    let last = 620;
    let dense_hour = 602;
    let dense_count = 400u64;
    let fixture = Fixture::new(Data::DenseMarket(dense_hour, dense_count), first..=last);
    let query = fixture.query();
    let log = dashboard_declare::log("market-events").unwrap();
    let shared = Arc::new(Mutex::new(BucketCache::with_limits(1, 2048)));
    let uncached = Reader::new(&query, log).unwrap();
    let selection = uncached.select(&[], first, last).unwrap();

    let (cold_tail, cold_counts) = replay_columnar::unit::reader::count_decodes(|| {
        let reader = Reader::with_cache(&query, log, Arc::clone(&shared)).unwrap();
        reader.newest_page_with(&selection.patterns, selection.groups.len(), first, dense_hour, 0, 16, selection.overlap).unwrap()
    });
    assert_eq!(cold_tail.rows.len(), 16);
    assert_eq!(cold_counts.log_rows_at_rows, 16, "only the 16 tail rows are fully decoded from a 400-row bucket");

    let ((warm_tail, warm_examined), warm_counts) = replay_columnar::unit::reader::count_decodes(|| {
        count_examined(|| {
            let reader = Reader::with_cache(&query, log, Arc::clone(&shared)).unwrap();
            reader.newest_page_with(&selection.patterns, selection.groups.len(), first, dense_hour, 0, 16, selection.overlap).unwrap()
        })
    });
    assert_eq!(warm_counts.log_span_rows, 0, "warm immutable bucket cache skips facet re-scan");
    assert_eq!(warm_counts.log_rows_at_rows, 16, "warm step still decodes only the 16 tail rows");
    assert!(warm_examined <= 16, "warm single-group dense hour examines only requested tail entries, got {warm_examined}");
    assert_eq!(
        warm_tail.rows.iter().map(|r| &r.row).collect::<Vec<_>>(),
        cold_tail.rows.iter().map(|r| &r.row).collect::<Vec<_>>()
    );

    let (has_quiet, quiet_examined) = count_examined(|| {
        let reader = Reader::with_cache(&query, log, Arc::clone(&shared)).unwrap();
        reader.has_any_with(&selection.patterns, dense_hour + 1, dense_hour + 1, selection.overlap).unwrap()
    });
    assert!(!has_quiet, "quiet hour after dense hour has no new events");
    assert_eq!(quiet_examined, 0, "warm quiet check examines zero entries in the dense hour");

    let direct = uncached.newest_page_with(&selection.patterns, selection.groups.len(), first, dense_hour, 0, 16, selection.overlap).unwrap();
    assert_eq!(
        warm_tail.rows.iter().map(|r| &r.row).collect::<Vec<_>>(),
        direct.rows.iter().map(|r| &r.row).collect::<Vec<_>>()
    );

    let reader_evict = Reader::with_cache(&query, log, Arc::clone(&shared)).unwrap();
    let late = reader_evict.newest_page_with(&selection.patterns, selection.groups.len(), first, last, 0, 16, selection.overlap).unwrap();
    let late_direct = uncached.newest_page_with(&selection.patterns, selection.groups.len(), first, last, 0, 16, selection.overlap).unwrap();
    assert_eq!(
        late.rows.iter().map(|r| &r.row).collect::<Vec<_>>(),
        late_direct.rows.iter().map(|r| &r.row).collect::<Vec<_>>()
    );
    assert!(shared.lock().unwrap().len() <= 1, "bounded bucket cache respects its capacity limit");
}

#[test]
fn dated_membership_and_unobserved_spans_filter_per_row_hour() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let from = FIRST;
    let to = FIRST + 600;
    let join_hour = FIRST + 200;
    let unobs_first = FIRST + 300;
    let unobs_last = FIRST + 360;
    let filter = FacetFilter {
        name: "country".into(),
        membership: vec![CountrySpan { country: Country::from("GER"), first_hour: join_hour, last_hour: to }],
        unobserved: vec![UnobservedSpan { first_hour: unobs_first, last_hour: unobs_last }],
        ..Default::default()
    };
    let selection = reader.select(&[filter], from, to).unwrap();
    let wanted = events(&query, from, to, |hour, _, country| {
        country == 9001 && hour >= join_hour && !(unobs_first..=unobs_last).contains(&hour)
    });
    assert!(!wanted.is_empty());
    let pre_join = events(&query, from, join_hour - 1, |_, _, country| country == 9001);
    assert!(!pre_join.is_empty(), "fixture must have pre-join events to prove they are excluded");

    let page = reader.page_with(&selection.patterns, selection.groups.len(), from, to, 0, 500, selection.overlap).unwrap();
    assert_eq!(page.total(), wanted.len() as u64);
    assert!(page.rows.iter().all(|r| r.row.hour >= join_hour && !(unobs_first..=unobs_last).contains(&r.row.hour)));
    let tail = reader.newest_page_with(&selection.patterns, selection.groups.len(), from, to, 0, 64, selection.overlap).unwrap();
    assert_eq!(tail.total, wanted.len() as u64);
    assert_eq!(reader.total_with(&selection.patterns, from, to, selection.overlap).unwrap(), wanted.len() as u64);
    let counts = reader.counts_with(&selection.patterns, selection.groups.len(), from, to, 6, selection.overlap).unwrap();
    let counted: u64 = counts[0].bins.iter().map(|(_, c)| *c).sum();
    assert_eq!(counted, wanted.len() as u64);
}

#[test]
fn per_group_counts_credit_duplicate_country_groups_independently() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let from = FIRST;
    let to = FIRST + 600;
    let dup = FacetFilter { name: "country".into(), countries: vec![Country::from("GER"), Country::from("GER")], ..Default::default() };
    let selection = reader.select(&[dup], from, to).unwrap();
    assert_eq!(selection.overlap, Overlap::EachPattern);
    let counts = reader.counts(&selection.patterns, selection.groups.len(), from, to, 6).unwrap();
    assert_eq!(counts.len(), 2);
    let expected = events(&query, from, to, |_, _, country| country == 9001).len() as u64;
    assert!(expected > 0);
    for (idx, group_counts) in counts.iter().enumerate() {
        let sum: u64 = group_counts.bins.iter().map(|(_, c)| *c).sum();
        assert_eq!(sum, expected, "duplicate group {idx} must receive full independent count");
    }
}

#[test]
fn newest_page_preserves_same_hour_order_across_tail_and_page_boundary() {
    let first = 600;
    let last = 604;
    let dense_hour = 602;
    let dense_count = 100u64;
    let fixture = Fixture::new(Data::DenseMarket(dense_hour, dense_count), first..=last);
    let query = fixture.query();
    let reader = market(&query);
    let selection = reader.select(&[], first, last).unwrap();

    let tail_64 = reader.newest_page_with(&selection.patterns, selection.groups.len(), first, dense_hour, 0, 64, selection.overlap).unwrap();
    let page_0_128 = reader.newest_page_with(&selection.patterns, selection.groups.len(), first, dense_hour, 0, 128, selection.overlap).unwrap();
    let page_64_128 = reader.newest_page_with(&selection.patterns, selection.groups.len(), first, dense_hour, 64, 128, selection.overlap).unwrap();

    let expected_first_64: Vec<u128> = (0..64).map(|idx| 10_000 + idx).collect();
    let expected_rest_36: Vec<u128> = (64..100).map(|idx| 10_000 + idx).collect();

    assert_eq!(tail_64.rows.iter().map(|r| r.row.key).collect::<Vec<_>>(), expected_first_64);
    assert_eq!(
        page_0_128.rows[..64].iter().map(|r| r.row.key).collect::<Vec<_>>(),
        expected_first_64,
        "128-row page at offset 0 starts with the exact same 64 same-hour rows as the 64-row tail"
    );
    assert_eq!(
        page_0_128.rows[64..100].iter().map(|r| r.row.key).collect::<Vec<_>>(),
        expected_rest_36
    );
    assert_eq!(
        page_64_128.rows[..36].iter().map(|r| r.row.key).collect::<Vec<_>>(),
        expected_rest_36,
        "offset 64 continues seamlessly with the remaining 36 same-hour rows"
    );
    assert_eq!(page_64_128.rows[36].row.hour, 600, "earlier hour follows after all dense-hour ties");
    assert_eq!(
        tail_64.rows.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        page_0_128.rows[..64].iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        "ordinals match across tail and page"
    );
}

#[test]
fn projected_out_numeric_column_in_append_log_stays_unrecorded() {
    let first = 600;
    let last = 604;
    let fixture = Fixture::new(Data::DenseMarket(602, 5), first..=last);
    let query = fixture.query();
    let recording = query.replay.recording();
    let table = replay_data::rows::TableId::MarketEvents;
    let index = table.index();
    let kind_col = dashboard_declare::field(table, "kind").unwrap();
    let country_col = dashboard_declare::field(table, "country").unwrap();
    let amount_col = dashboard_declare::field(table, "amount_raw").unwrap();
    let kinds = replay_recording::schema::schema().unwrap().swap_remove(index);
    let recorded = recording.columns()[index].min(kinds.len());
    let selected = read_columns(recording, recorded, index, Some(&[kind_col, country_col]));
    assert!(!selected.contains(&amount_col), "projection excludes amount_raw");
    let pages = recording.table_pages(0, index, 0, 1).unwrap();
    let mut strings: HashMap<u32, Vec<u8>> = HashMap::new();
    let mut checked = 0usize;
    unit::reader::log_rows_at(recording.extents(), &pages, &kinds[..recorded], index, &selected, &[0, 1, 2], &mut |event| match event {
        LogEvent::Named { id, text } => {
            strings.insert(id, text.to_vec());
        }
        LogEvent::Appended { rows, at, .. } => {
            let cells = Reader::cells_from_log_state(rows, &kinds, recorded, table.field_count(), &selected, at, &strings);
            assert_eq!(cells[kind_col], Cell::Number(20));
            assert_eq!(cells[amount_col], Cell::Unrecorded, "projected-out numeric column must be Unrecorded, never 0");
            checked += 1;
        }
        LogEvent::Forgotten { .. } => {}
    })
    .unwrap();
    assert_eq!(checked, 3);
}

#[test]
fn staggered_membership_intervals_match_exact_counts_without_quadratic_sweep() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let from = FIRST;
    let to = FIRST + 1_800;
    let membership: Vec<CountrySpan> = (0..1_500)
        .map(|i| CountrySpan { country: Country::from("GER"), first_hour: from + i, last_hour: to })
        .collect();
    let filter = FacetFilter { name: "country".into(), membership, ..Default::default() };
    let selection = reader.select(&[filter], from, to).unwrap();
    let (total, checked) = count_patterns_checked(|| reader.total_with(&selection.patterns, from, to, selection.overlap).unwrap());
    let expected = events(&query, from, to, |_, _, country| country == 9001).len() as u64;
    assert_eq!(total, expected, "1,500 staggered intervals ending together count each matching event once");
    assert!(checked <= 20_000, "1,500 staggered intervals must not perform a quadratic active-set sweep, got {checked}");

    let run = selection.patterns[0].run;
    let country_shift = facet_shift(1);
    let mask = u128::from(u32::MAX) << country_shift;
    let distinct: Vec<Pattern> = (0..1_500u32)
        .map(|i| Pattern {
            run,
            first: from + i as i32,
            last: to,
            mask,
            value: u128::from(9001 + i) << country_shift,
            group: i as usize,
        })
        .collect();
    let (distinct_total, distinct_checked) = count_patterns_checked(|| reader.total_with(&distinct, from, to, Overlap::FirstMatch).unwrap());
    assert_eq!(distinct_total, expected, "1,500 distinct staggered patterns ending together count exactly");
    assert!(distinct_checked <= 20_000, "1,500 distinct staggered patterns must not perform a quadratic sweep, got {distinct_checked}");
}

#[test]
fn membership_group_order_matches_sorted_tags_not_input_order() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let shared_hour = (ITA_FROM..=LAST)
        .find(|&hour| {
            let list = fixture::market(hour);
            list.iter().any(|&(_, _, c)| c == 9001) && list.iter().any(|&(_, _, c)| c == 9002)
        })
        .expect("fixture has an hour where both GER and ITA emit events");
    let filter = FacetFilter {
        name: "country".into(),
        membership: vec![
            CountrySpan { country: Country::from("ITA"), first_hour: ITA_FROM, last_hour: shared_hour },
            CountrySpan { country: Country::from("GER"), first_hour: shared_hour - 10, last_hour: shared_hour },
        ],
        ..Default::default()
    };
    let selection = reader.select(&[filter], ITA_FROM, shared_hour).unwrap();
    assert_eq!(selection.groups, vec![Some(Country::from("GER")), Some(Country::from("ITA"))]);

    let page = reader.page_with(&selection.patterns, selection.groups.len(), shared_hour, shared_hour, 0, 16, selection.overlap).unwrap();
    let tail = reader.newest_page_with(&selection.patterns, selection.groups.len(), shared_hour, shared_hour, 0, 16, selection.overlap).unwrap();
    let page_groups: Vec<usize> = page.rows.iter().map(|r| r.group).collect();
    let tail_groups: Vec<usize> = tail.rows.iter().map(|r| r.group).collect();
    assert_eq!(page_groups, vec![0, 1], "GER (group 0) precedes ITA (group 1) at the same hour");
    assert_eq!(tail_groups, vec![0, 1], "newest_page_with matches page_with same-hour order");
}

#[test]
fn quiet_advance_does_not_walk_expired_historical_patterns() {
    let fixture = Fixture::campaign(Data::Market);
    let query = fixture.query();
    let reader = market(&query);
    let from = FIRST;
    let quiet_hour = (FIRST + 2_200..=FIRST + 2_300).find(|&hour| fixture::market(hour).is_empty()).unwrap();
    let mut membership: Vec<CountrySpan> = (0..1_000)
        .map(|i| CountrySpan { country: Country::from("GER"), first_hour: from + i * 2, last_hour: from + i * 2 })
        .collect();
    membership.push(CountrySpan { country: Country::from("GER"), first_hour: quiet_hour - 5, last_hour: quiet_hour + 5 });
    let filter = FacetFilter { name: "country".into(), membership, ..Default::default() };
    let selection = reader.select(&[filter], from, quiet_hour + 5).unwrap();
    assert!(selection.patterns.len() >= 1_000, "test must exercise at least 1,000 distinct historical patterns");
    let ((has, examined), checked) = count_patterns_checked(|| count_examined(|| reader.has_any_in(&selection, quiet_hour, quiet_hour).unwrap()));
    assert!(!has);
    assert_eq!(examined, 0);
    assert!(checked < 64, "quiet advance must skip expired historical patterns via interval index, checked {checked}");
}

#[test]
fn filtered_quiet_advance_skips_bucket_with_only_unselected_event_kinds() {
    let first = 600;
    let last = 612;
    let dense_hour = 602;
    let fixture = Fixture::new(Data::DenseMarket(dense_hour, 100), first..=last);
    let query = fixture.query();
    let reader = market(&query);
    let filter = FacetFilter { name: "kind".into(), values: vec![10], ..Default::default() };
    let selection = reader.select(&[filter], first, last).unwrap();
    let (has, examined) = count_examined(|| reader.has_any_in(&selection, dense_hour, dense_hour).unwrap());
    assert!(!has, "dense hour holds only kind 20, not kind 10");
    assert_eq!(examined, 0, "bucket summary with no matching facet must examine zero entries");
}

#[test]
fn stretch_boundary_windows_within_six_hours_cross_gaps_with_exact_bins_and_pages() {
    let first = 10_003;
    let run0_last = 10_203;
    let run1_first = 10_401;
    let last = 11_200;
    let fixture = Fixture::with_gap(Data::Market, first..=last, (run0_last + 1)..=(run1_first - 1));
    let query = fixture.query();
    let reader = market(&query);

    let all_sel = reader.select(&[], first, last).unwrap();
    let filtered_sel = reader
        .select(
            &[FacetFilter { name: "country|receiver".into(), countries: vec!["GER".into()], deduplicate: true, ..Default::default() }],
            first,
            last,
        )
        .unwrap();

    let windows = [
        (run0_last - 2, last),
        (run0_last, last),
        (first, run1_first + 1),
        (first, run1_first),
        (run0_last - 2, run1_first + 2),
        (run1_first, run1_first + 24),
    ];

    for (from, to) in windows {
        let wanted = events(&query, from, to, |_, _, _| true);
        let expected_before = events(&query, first, from - 1, |_, _, _| true).len() as u64;

        for (label, sel) in [("each", &all_sel), ("first-match", &filtered_sel)] {
            for bin_hours in [6, 96] {
                let counts = reader.counts_with(&sel.patterns, sel.groups.len(), from, to, bin_hours, sel.overlap).unwrap();
                assert_eq!(counts[0].before, expected_before, "{label} before for {from}..={to} bin={bin_hours}");
                let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
                let mut expected_bins: BTreeMap<u32, u64> = BTreeMap::new();
                for event in &wanted {
                    *expected_bins.entry(((i64::from(event.0) - first_bin) / bin_hours) as u32).or_default() += 1;
                }
                assert_eq!(
                    counts[0].bins,
                    expected_bins.into_iter().collect::<Vec<_>>(),
                    "{label} bins for {from}..={to} bin={bin_hours}"
                );
            }

            let mut forward = Vec::new();
            let mut cursor = 0;
            loop {
                let page = reader.page_with(&sel.patterns, sel.groups.len(), from, to, cursor, 32, sel.overlap).unwrap();
                assert_eq!((page.base, page.total()), (expected_before, wanted.len() as u64));
                forward.extend(page.rows.iter().map(|p| (p.row.hour, p.row.key)));
                match page.cursor() {
                    Some(next) => cursor = next,
                    None => break,
                }
            }
            let expected_keys: Vec<(i32, u128)> = wanted.iter().map(|e| (e.0, e.1)).collect();
            assert_eq!(forward, expected_keys, "{label} forward page for {from}..={to}");

            let tail = reader.newest_page_with(&sel.patterns, sel.groups.len(), from, to, 0, 128, sel.overlap).unwrap();
            assert_eq!(tail.total, wanted.len() as u64, "{label} tail total for {from}..={to}");
            let mut expected_tail = expected_keys.clone();
            expected_tail.sort_by_key(|&(hour, key)| (Reverse(hour), key));
            expected_tail.truncate(128);
            let got_tail: Vec<(i32, u128)> = tail.rows.iter().map(|p| (p.row.hour, p.row.key)).collect();
            assert_eq!(got_tail, expected_tail, "{label} tail rows for {from}..={to}");
        }
    }
}
