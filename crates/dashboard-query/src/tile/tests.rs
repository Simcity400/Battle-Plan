use super::*;
use crate::{Query, chart::ChartRun, fixture::*, identity::{Country, Holder, HolderRun}, series::grid};

fn whole(tags: &[&str]) -> Vec<Ask> {
    tags.iter().map(|tag| Ask::whole(*tag)).collect()
}

fn line<'q, 'r>(query: &'q Query<'r>) -> Line<'q, 'r> {
    let series = dashboard_declare::series("country-measure").unwrap();
    Line::new(query, series, series.measure("politicalPower").unwrap()).unwrap()
}

fn wire(line: &Line<'_, '_>, bin_hours: i64, index: i64) -> (SeriesTileReply, Vec<Vec<Option<ChartBucket>>>) {
    let tags = whole(&TAGS);
    let reply = tile(line, "1", &tags, None, bin_hours, index, false).unwrap();
    let buckets = decode(&unbase64(&reply.bins), tags.len(), reply.first_hour, bin_hours, line.scale(), reply.raw_values, reply.hour_bytes).unwrap();
    let by_tag = buckets.chunks(BINS).map(<[_]>::to_vec).collect();
    (reply, by_tag)
}

fn joined(spans: impl IntoIterator<Item = MissingSpan>) -> Vec<MissingSpan> {
    let mut out: Vec<MissingSpan> = Vec::new();
    for span in spans {
        match out.last_mut() {
            Some(held) if held.reason == span.reason && held.last_hour + 1 == span.first_hour => held.last_hour = span.last_hour,
            _ => out.push(span),
        }
    }
    out
}

const LONG_SECOND: i32 = FIRST + 17 * UNIT_HOURS + GAP;
const LONG_LAST: i32 = LONG_SECOND + 2 * UNIT_HOURS - 1;

fn long_campaign() -> Fixture {
    Fixture::with_gap(Data::Power, FIRST..=LONG_LAST, LONG_SECOND - GAP..=LONG_SECOND - 1)
}

#[test]
fn a_tile_holds_the_bins_a_view_of_its_hours_reads_at_every_level() {
    let fixture = long_campaign();
    let query = fixture.query();
    let line = line(&query);
    let mut compared = 0;
    for bin_hours in [3, 6, 18, 96, 288, 1_536, 73_728] {
        let span = BINS as i64 * bin_hours;
        let (first_tile, last_tile) = (i64::from(FIRST).div_euclid(span), i64::from(LONG_LAST).div_euclid(span));
        let (first, last) = (first_tile * span, (last_tile + 1) * span - 1);
        let from = first - bin_hours;
        let width = ((last - from + 1) / bin_hours) as u32;
        assert_eq!(grid(last - from + 1, width).1, bin_hours);
        let drawn: Vec<(Ask, Vec<(i32, i32)>)> = TAGS.iter().map(|tag| (Ask::whole(*tag), vec![(from as i32, last as i32)])).collect();
        let view = line.summaries_within(&drawn, from as i32, last as i32, width).unwrap();
        for index in first_tile..=last_tile {
            let (reply, tiles) = wire(&line, bin_hours, index);
            for (at, tag) in TAGS.iter().enumerate() {
                let offset = ((index - first_tile) as usize) * BINS + 1;
                let expected: Vec<Option<ChartBucket>> = view[at].0.buckets[offset..offset + BINS]
                    .iter()
                    .map(|bucket| bucket.map(|bucket| ChartBucket { absolute_total: 0, ..bucket }))
                    .collect();
                assert_eq!(tiles[at], expected, "{tag}, bins of {bin_hours} hours, tile {index}");
                compared += tiles[at].iter().flatten().count();
                assert_eq!(reply.lines[at].country, Country::from(*tag));
            }
        }
        for (at, tag) in TAGS.iter().enumerate() {
            let pieces = (first_tile..=last_tile).flat_map(|index| wire(&line, bin_hours, index).0.lines.swap_remove(at).missing);
            let parts = line.parts(*tag, first as i32, last as i32);
            assert_eq!(joined(pieces), line.missing(*tag, first as i32, last as i32, &parts).unwrap(), "{tag}'s hours not recorded, bins of {bin_hours}");
        }
    }
    assert!(compared > 1_000, "{compared} bins compared");
}

#[test]
fn a_tile_across_a_gap_marks_its_bins_not_recorded() {
    let gap = (FIRST + FIRST_HOURS, SECOND - 1);
    let fixture = Fixture::with_gap(Data::Power, gap.0 - 24..=gap.1 + 24, gap.0..=gap.1);
    let query = fixture.query();
    let line = line(&query);
    let bin_hours = 6;
    let index = i64::from(FIRST + FIRST_HOURS).div_euclid(BINS as i64 * bin_hours);
    let (reply, tiles) = wire(&line, bin_hours, index);
    for (at, tag) in TAGS.iter().enumerate() {
        let mut inside = 0;
        for (bin, bucket) in tiles[at].iter().enumerate() {
            let start = i64::from(reply.first_hour) + bin as i64 * bin_hours;
            if start >= i64::from(gap.0) && start + bin_hours - 1 <= i64::from(gap.1) {
                assert_eq!(*bucket, None, "{tag}'s bin from {start} is in the gap");
                inside += 1;
            }
        }
        assert!(inside >= 60, "{inside} bins inside the gap");
        let missing = &reply.lines[at].missing;
        assert!(
            missing.iter().any(|span| span.first_hour <= gap.0 && gap.1 <= span.last_hour && span.reason == GAP_REASON),
            "{tag}: {missing:?}"
        );
    }
    assert!(tiles[0].iter().any(Option::is_some) && tiles[0].iter().any(Option::is_none));
}

#[test]
fn the_last_tile_says_how_far_it_is_filled_and_is_not_immutable() {
    let bin_hours = 18;
    let span = BINS as i64 * bin_hours;
    let seam = ((i64::from(ITA_FROM).div_euclid(span) + 1) * span) as i32;
    let last_hour = seam + 16;
    let fixture = Fixture::new(Data::Power, seam - 16..=last_hour);
    let query = fixture.query();
    let line = line(&query);
    let last = i64::from(last_hour).div_euclid(span);
    let (reply, _) = wire(&line, bin_hours, last);
    assert!(!reply.immutable);
    assert_eq!(reply.filled, Some(last_hour));
    let (before, _) = wire(&line, bin_hours, last - 1);
    assert!(before.immutable);
    assert_eq!(before.filled, Some(before.first_hour + span as i32 - 1));
    let (after, tiles) = wire(&line, bin_hours, last + 1);
    assert!(!after.immutable);
    assert_eq!(after.filled, None);
    assert!(tiles.iter().flatten().all(Option::is_none));
}

#[test]
fn a_tile_of_a_tag_the_recording_lacks_is_refused_by_name() {
    let hour = 10 * BINS as i32 * 6;
    let fixture = Fixture::new(Data::Power, hour..=hour);
    let query = fixture.query();
    let line = line(&query);
    let error = tile(&line, "1", &whole(&["GER", "XYZ"]), None, 6, 10, false).unwrap_err();
    assert!(error.to_string().contains("XYZ"), "{error}");
}

#[test]
fn bins_round_trip_through_every_width() {
    let bucket = |value: f64| ChartBucket {
        absolute_total: 0,
        first_hour: 70_000,
        last_hour: 140_000,
        first_value: value,
        last_value: 1.5,
        last_value_hour: 100_000,
        min_hour: 70_000,
        min_value: -2.25,
        max_hour: 139_999,
        max_value: value.max(1.5),
        first_break: true,
        has_break: false,
    };
    for (value, raw) in [(3.0, true), (0.1, true), (0.123, false), (5e12, false)] {
        let mut buckets = vec![None; BINS];
        buckets[0] = Some(bucket(value));
        let (bytes, raw_values, hour_bytes) = encode(&buckets, 70_000, 73_728, 100.0);
        assert_eq!((raw_values, hour_bytes), (raw, 4));
        assert_eq!(decode(&bytes, 1, 70_000, 73_728, 100.0, raw_values, hour_bytes).unwrap(), buckets);
    }
    let made = |start: i32, hours: i32, values: [f64; 4]| ChartBucket {
        absolute_total: 0,
        first_hour: start,
        last_hour: start + hours - 1,
        first_value: values[0],
        last_value: values[1],
        last_value_hour: start + hours / 2,
        min_hour: start,
        min_value: values[2],
        max_hour: start + hours - 1,
        max_value: values[3],
        first_break: false,
        has_break: true,
    };
    for (bin_hours, hours) in [(6, 1), (96 * 3, 2), (24_576 * 3, 4)] {
        let mut buckets = vec![None; 2 * BINS];
        buckets[3] = Some(made(3 * bin_hours as i32, bin_hours as i32, [-7.5; 4]));
        buckets[BINS + 1] = Some(made(bin_hours as i32, bin_hours as i32, [4.25, 3.0, 1.5, 6.0]));
        let (bytes, raw_values, hour_bytes) = encode(&buckets, 0, bin_hours, 100.0);
        assert_eq!(hour_bytes, hours);
        assert_eq!(bytes.len(), 2 * BINS + (1 + 4) * 4 + 2 * 5 * usize::from(hours), "a level bin carries one value");
        assert_eq!(decode(&bytes, 2, 0, bin_hours, 100.0, raw_values, hour_bytes).unwrap(), buckets);
    }
    assert_eq!(unbase64(&base64(b"tiles!?")), b"tiles!?");
}

#[test]
fn bins_of_no_view_are_refused() {
    for bin_hours in [0, -6, 100, 6 * 16 * 15 + 6] {
        assert!(level_of(bin_hours).is_err(), "{bin_hours}");
    }
    assert_eq!(level_of(5).unwrap(), replay_lanes::lanes::WALKED);
    assert_eq!(level_of(90).unwrap(), 0);
    assert_eq!(level_of(96 * 15).unwrap(), 1);
}

#[test]
fn a_tile_carries_its_drops_per_bin_and_the_drops_before_it() {
    let fixture = Fixture::campaign(Data::Power);
    let query = fixture.query();
    let line = line(&query);
    let mut compared = 0;
    for bin_hours in [1, 3, 6, 18, 96, 288] {
        let span = BINS as i64 * bin_hours;
        for index in i64::from(FIRST).div_euclid(span)..=i64::from(LAST).div_euclid(span) {
            let reply = tile(&line, "1", &whole(&TAGS), None, bin_hours, index, true).unwrap();
            let first = reply.first_hour;
            for (at, tag) in TAGS.iter().enumerate() {
                let id = 9001 + at as u32;
                let marks = reply.lines[at].marks.clone().expect("asked with marks");
                let before = (FIRST..first).filter(|&hour| held(&query, tag, hour) && dropped(id, hour)).count();
                assert_eq!(marks.before, before.to_string(), "{tag}, bins of {bin_hours}, tile {index}");
                let mut bins: Vec<(u32, u64)> = Vec::new();
                for hour in first..first + span as i32 {
                    if held(&query, tag, hour) && dropped(id, hour) {
                        let bin = ((i64::from(hour) - i64::from(first)) / bin_hours) as u32;
                        match bins.last_mut() {
                            Some(last) if last.0 == bin => last.1 += 1,
                            _ => bins.push((bin, 1)),
                        }
                    }
                }
                compared += bins.len();
                assert_eq!(marks.bins, bins, "{tag}, bins of {bin_hours}, tile {index}");
            }
        }
    }
    assert!(compared > 100, "{compared} bins with drops compared");
    let plain = tile(&line, "1", &whole(&["GER"]), None, 6, 30, false).unwrap();
    assert!(plain.lines[0].marks.is_none(), "marks only when asked");
}

#[test]
fn an_hourly_tile_holds_the_runs_a_view_of_its_hours_reads() {
    let fixture = Fixture::campaign(Data::Power);
    let query = fixture.query();
    let line = line(&query);
    let tags: Vec<String> = TAGS.map(str::to_owned).to_vec();
    let seam = (FIRST / 256 + 20) * 256;
    let views = [(FIRST, FIRST + 167), (FIRST + 1_003, FIRST + 1_170), (seam - 30, seam + 90), (SECOND - 100, SECOND + 50), (ITA_FROM - 20, ITA_FROM + 100), (LAST - 150, LAST), (FIRST + 41, FIRST + 300)];
    let mut runs_compared = 0;
    for (from, to) in views {
        let today = line.summaries(&tags, from, to, 600).unwrap();
        let mut joined: Vec<Vec<ChartRun>> = vec![Vec::new(); tags.len()];
        let mut parts: Vec<Vec<TilePart>> = vec![Vec::new(); tags.len()];
        for index in from.div_euclid(256)..=to.div_euclid(256) {
            let reply = tile(&line, "1", &whole(&TAGS), None, 1, i64::from(index), false).unwrap();
            assert_eq!(reply.hour_bytes, 0, "an hourly tile carries runs");
            let runs = decode_runs(&unbase64(&reply.bins), tags.len(), reply.first_hour, line.scale(), reply.raw_values).unwrap();
            for (at, held) in runs.into_iter().enumerate() {
                for (first, last, value, flags) in held {
                    match joined[at].last_mut() {
                        Some(run) if flags & SEAM != 0 && run.last_hour + 1 == first => run.last_hour = last,
                        _ => joined[at].push(ChartRun { first_hour: first, last_hour: last, value, break_before: flags & FIRST_BREAK != 0 }),
                    }
                }
                parts[at].extend(reply.lines[at].parts.iter().cloned());
            }
        }
        for (at, tag) in TAGS.iter().enumerate() {
            let segment = parts[at].iter().find(|part| part.first_hour <= from && from <= part.last_hour).map(|part| part.segment_first_hour);
            let ours: Vec<ChartRun> = joined[at]
                .iter()
                .filter(|run| run.last_hour >= from && run.first_hour <= to)
                .map(|run| match run.first_hour <= from {
                    true => ChartRun { first_hour: from, last_hour: run.last_hour.min(to), break_before: segment == Some(from), ..run.clone() },
                    false => ChartRun { last_hour: run.last_hour.min(to), ..run.clone() },
                })
                .collect();
            assert_eq!(Some(&ours), today[at].0.runs.as_ref(), "{tag}'s runs over {from}..{to}");
            runs_compared += ours.len();
            let mut bins = vec![None; (to - from + 1) as usize];
            for run in &ours {
                for hour in run.first_hour..=run.last_hour {
                    bins[(hour - from) as usize] = Some(ChartBucket {
                        absolute_total: 0,
                        first_hour: hour,
                        last_hour: hour,
                        first_value: run.value,
                        last_value: run.value,
                        last_value_hour: hour,
                        min_hour: hour,
                        min_value: run.value,
                        max_hour: hour,
                        max_value: run.value,
                        first_break: hour == run.first_hour && run.break_before,
                        has_break: false,
                    });
                }
            }
            let expected: Vec<Option<ChartBucket>> = today[at].0.buckets.iter().map(|bucket| bucket.map(|bucket| ChartBucket { absolute_total: 0, ..bucket })).collect();
            assert_eq!(bins, expected, "{tag}'s hourly bins over {from}..{to}");
        }
    }
    assert!(runs_compared > 20, "{runs_compared} runs compared");
}

#[test]
fn a_view_assembled_from_tiles_holds_the_chart_of_its_whole_bins() {
    let fixture = long_campaign();
    let query = fixture.query();
    let line = line(&query);
    let held = |tag: &str, hour: i32| query.holders.at(tag, hour).is_some() && query.timeline.at(hour).is_some();
    let seam = (FIRST / 256 + 20) * 256;
    let views = [
        (FIRST + 7, LONG_LAST - 11, 64),
        (FIRST + 5, LONG_LAST, 600),
        (FIRST + 101, FIRST + 1_900, 600),
        (LONG_SECOND - 2_000, LONG_SECOND + 700, 333),
        (ITA_FROM - 500, ITA_FROM + 2_500, 100),
        (FIRST, LONG_LAST, 3),
        (LONG_LAST - 700, LONG_LAST, 128),
        (FIRST, FIRST + 100, 64),
        (seam - 30, seam + 90, 64),
        (LONG_SECOND - 40, LONG_SECOND + 20, 64),
        (ITA_FROM - 20, ITA_FROM + 30, 64),
        (LONG_LAST - 50, LONG_LAST, 64),
    ];
    let (mut bins_compared, mut runs_compared, mut marks_compared) = (0, 0, 0);
    let mut levels = std::collections::BTreeSet::new();
    for (at, (from, to, width)) in views.into_iter().enumerate() {
        let keys: Vec<&str> = match at < 2 {
            true => TAGS.iter().copied().cycle().take(view::GROUP + 2).collect(),
            false => TAGS.to_vec(),
        };
        let chart = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" },
            "countries": keys, "from": from, "to": to, "width": width, "withMarks": true });
        let (asked, requests) = view::requests(&chart).unwrap();
        assert_eq!(asked, view::View { from, to, width });
        let bin_hours = asked.bin_hours();
        levels.insert(bin_hours);
        assert_eq!(requests.len(), asked.tiles().count() * keys.len().div_ceil(view::GROUP), "{from}..{to} at {width}");
        let replies: Vec<SeriesTileReply> = requests
            .iter()
            .map(|request| {
                assert!(request.get("from").is_none() && request.get("width").is_none() && request["withMarks"] == true);
                serde_json::from_value(serde_json::to_value(reply(&line, "1", request).unwrap()).unwrap()).unwrap()
            })
            .collect();
        let assembled = view::assemble(asked, &replies).unwrap();
        assert_eq!(assembled.bin_hours, bin_hours);
        assert_eq!(assembled.series.iter().map(|line| line.country.tag().unwrap()).collect::<Vec<_>>(), keys);
        let tags: Vec<String> = TAGS.map(str::to_owned).to_vec();
        let (first_bin, last_bin) = (i64::from(from).div_euclid(bin_hours) * bin_hours, i64::from(to).div_euclid(bin_hours) * bin_hours + bin_hours - 1);
        let walked = match bin_hours {
            1 => line.summaries(&tags, from, to, (to - from + 1) as u32).unwrap(),
            _ => {
                let whole = ((last_bin - first_bin + 1) / bin_hours) as u32;
                assert_eq!(grid(last_bin - first_bin + 1, whole).1, bin_hours);
                line.summaries(&tags, first_bin as i32, last_bin as i32, whole).unwrap()
            }
        };
        let drops: Vec<Vec<(i32, i32, String, String)>> = TAGS
            .iter()
            .enumerate()
            .map(|(own, tag)| {
                let id = 9001 + own as u32;
                let fell = |hour: i32| held(tag, hour) && dropped(id, hour);
                let mut before = (FIRST..first_bin as i32).filter(|&hour| fell(hour)).count();
                let (mut out, mut start) = (Vec::new(), first_bin);
                while start <= i64::from(to) {
                    let (first, last) = (start as i32, (start + bin_hours - 1) as i32);
                    let count = (first..=last).filter(|&hour| fell(hour)).count();
                    if count > 0 {
                        out.push((first, last, before.to_string(), count.to_string()));
                    }
                    before += count;
                    start += bin_hours;
                }
                out
            })
            .collect();
        for assembled in &assembled.series {
            let tag = assembled.country.tag().unwrap();
            let own = TAGS.iter().position(|held| *held == tag).unwrap();
            let expected: Vec<Option<ChartBucket>> = walked[own].0.buckets.iter().map(|bucket| bucket.map(|bucket| ChartBucket { absolute_total: 0, ..bucket })).collect();
            assert_eq!(assembled.summary.buckets, expected, "{tag}'s bins over {from}..{to} at {width}");
            bins_compared += expected.iter().flatten().count();
            if bin_hours == 1 {
                assert_eq!(assembled.summary.runs, walked[own].0.runs, "{tag}'s runs over {from}..{to}");
                runs_compared += assembled.summary.runs.as_ref().map_or(0, Vec::len);
            }
            let parts = line.parts(tag, from, to);
            assert_eq!(assembled.missing, line.missing(tag, from, to, &parts).unwrap(), "{tag}'s hours not recorded over {from}..{to}");
            let marks = assembled.marks.as_ref().expect("asked with marks");
            let clusters: Vec<_> = marks.clusters.iter().map(|cluster| (cluster.first_hour, cluster.last_hour, cluster.first_ordinal.clone(), cluster.count.clone())).collect();
            assert_eq!(clusters, drops[own], "{tag}'s drops over {from}..{to} at {width}");
            marks_compared += clusters.len();
        }
    }
    assert!(levels.len() >= 8, "{levels:?}");
    assert!(bins_compared > 5_000 && runs_compared > 20 && marks_compared > 100, "{bins_compared} bins, {runs_compared} runs, {marks_compared} marks");
}

fn together(query: &Query<'_>, tags: &[&str], bin_hours: i64, index: i64) -> Vec<Vec<Option<ChartBucket>>> {
    let line = line(query);
    let reply = tile(&line, "1", &whole(tags), None, bin_hours, index, false).unwrap();
    let buckets = decode(&unbase64(&reply.bins), tags.len(), reply.first_hour, bin_hours, line.scale(), reply.raw_values, reply.hour_bytes).unwrap();
    buckets.chunks(BINS).map(<[_]>::to_vec).collect()
}

#[test]
fn a_line_has_the_same_bins_asked_alone_or_beside_tags_held_over_other_hours() {
    let first = ITA_FROM - 2 * UNIT_HOURS;
    let fixture = Fixture::with_gap(Data::Power, first..=LAST, FIRST + FIRST_HOURS..=SECOND - 1);
    let mut renamed = fixture.query();
    let change = ITA_FROM + 7;
    let run = |first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot: 7, id: 9001 }, starts_segment: true };
    let mut holders = (*renamed.holders).clone();
    holders.tags.insert("GER".to_owned(), vec![run(first, change - 1)]);
    holders.tags.insert("PRU".to_owned(), vec![HolderRun { starts_segment: false, ..run(change, FIRST + FIRST_HOURS - 1) }, run(SECOND, LAST)]);
    renamed.holders = std::sync::Arc::new(holders);
    let mut compared = 0;
    for (query, tags) in [(fixture.query(), vec!["GER", "ITA"]), (renamed, vec!["GER", "PRU", "ITA"])] {
        for bin_hours in [3, 6, 18, 96] {
            let index = i64::from(ITA_FROM).div_euclid(BINS as i64 * bin_hours);
            let both = together(&query, &tags, bin_hours, index);
            for (at, tag) in tags.iter().enumerate() {
                let alone = together(&query, &[tag], bin_hours, index).remove(0);
                assert_eq!(both[at], alone, "{tag} beside {tags:?}, bins of {bin_hours} hours");
                if level_of(bin_hours).unwrap() != replay_lanes::lanes::WALKED && bucket_hours(level_of(bin_hours).unwrap()) == bin_hours {
                    for bucket in alone.iter().flatten() {
                        assert_eq!((bucket.min_hour, bucket.max_hour), (bucket.first_hour, bucket.first_hour), "{tag}'s extremes in a bin of {bin_hours} hours");
                    }
                }
                compared += alone.iter().flatten().count();
            }
        }
    }
    assert!(compared > 1_000, "{compared} bins compared");
}

#[test]
fn a_top_of_tiles_names_the_countries_ranked_at_its_hour_and_how_long_they_stay_the_top() {
    let fixture = long_campaign();
    let query = fixture.query();
    let line = line(&query);
    let bin_hours = 96;
    let ask = |hour: i32| reply(&line, "1", &serde_json::json!({ "binHours": bin_hours, "index": i64::from(hour).div_euclid(BINS as i64 * bin_hours), "top": 1, "hour": hour })).unwrap();
    let mut proven = 0;
    for hour in (FIRST + 24..LONG_SECOND - GAP).step_by(37) {
        let top = ask(hour);
        let ranking = top.ranking.clone().expect("a top says who it is");
        if [9001, 9002].into_iter().any(|id| exists(id, hour) && power(id, hour).is_none()) {
            assert!(ranking.refused.is_some() && top.lines.is_empty(), "an unrecorded contender refuses the top at {hour}");
            continue;
        }
        assert_eq!(ranking.refused, None, "hour {hour}");
        let ranked = line.rank_at(hour, 1, false).unwrap().unwrap();
        assert_eq!(top.lines.iter().map(|line| line.country.to_string()).collect::<Vec<_>>(), ranked, "the reply names exactly the top at {hour}");
        let plain = tile(&line, "1", &whole(&ranked.iter().map(String::as_str).collect::<Vec<_>>()), None, bin_hours, top.index, false).unwrap();
        assert_eq!(top.bins, plain.bins, "the top's bins are its countries' tiles at {hour}");
        assert!(ranking.from <= hour && hour <= ranking.until);
        for at in ranking.from..=ranking.until {
            assert_eq!(line.rank_at(at, 1, false).unwrap().unwrap(), ranked, "the same top through its proven hours ({at} for {hour})");
        }
        let walked_edge = |at: i32| (at - FIRST) % UNIT_HOURS == 0;
        if ranking.from > FIRST && !walked_edge(ranking.from) {
            assert_ne!(line.rank_at(ranking.from - 1, 1, false).unwrap(), Ok(ranked.clone()), "the hour before the proof is another top");
        }
        if ranking.until < LONG_SECOND - GAP - 1 && !walked_edge(ranking.until + 1) {
            assert_ne!(line.rank_at(ranking.until + 1, 1, false).unwrap(), Ok(ranked.clone()), "the hour after the proof is another top");
        }
        proven += usize::try_from(ranking.until - ranking.from).unwrap();
    }
    assert!(proven > 50, "{proven} hours proven");
    let refused = ask(LONG_SECOND - 2);
    assert!(refused.lines.is_empty(), "no line in a gap");
    assert!(refused.ranking.unwrap().refused.is_some(), "a gap's top is refused with its reason");
    assert!(reply(&line, "1", &serde_json::json!({ "binHours": bin_hours, "index": 0, "top": 1, "hour": FIRST, "countries": ["GER"] })).is_err());
    let series = dashboard_declare::series("country-measure").unwrap();
    let other = Line::new(&query, series, series.measure("commandPower").unwrap()).unwrap();
    let hour = FIRST + 24;
    let index = i64::from(hour).div_euclid(BINS as i64 * bin_hours);
    let by = reply(&other, "1", &serde_json::json!({ "binHours": bin_hours, "index": index, "top": 2, "hour": hour, "rankBy": "politicalPower" })).unwrap();
    let ranked = line.rank_at(hour, 2, false).unwrap().unwrap();
    assert_eq!(by.lines.iter().map(|line| line.country.to_string()).collect::<Vec<_>>(), ranked, "ranked by the named measure");
    let plain = tile(&other, "1", &whole(&ranked.iter().map(String::as_str).collect::<Vec<_>>()), None, bin_hours, index, false).unwrap();
    assert_eq!(by.bins, plain.bins, "the tiles are of the asked measure");
    assert_eq!(by.measure, "commandPower");
    assert!(reply(&other, "1", &serde_json::json!({ "binHours": bin_hours, "index": index, "top": 2, "hour": hour, "rankBy": "noSuchMeasure" })).is_err());
}

#[test]
fn quiet_hours_and_many_tiles_of_one_top_share_one_ranking_pass() {
    let fixture = long_campaign();
    let query = fixture.query();
    let line = line(&query);
    let hour = ITA_FROM + 101;
    let top = |at: i32, index: i64| reply(&line, "1", &serde_json::json!({ "binHours": 1, "index": index, "top": 2, "hour": at })).unwrap();
    let first = top(hour, i64::from(hour).div_euclid(BINS as i64));
    let ranking = first.ranking.clone().unwrap();
    assert_eq!(ranking.refused, None);
    assert!(ranking.until - ranking.from >= 40, "a proof of {}..{}", ranking.from, ranking.until);
    let members: Vec<String> = first.lines.iter().map(|line| line.country.to_string()).collect();
    let asks: Vec<(i32, i64)> = (ranking.from..=ranking.until).flat_map(|at| (0..4).map(move |tile| (at, i64::from(at).div_euclid(BINS as i64) + tile - 1))).collect();
    assert!(asks.len() > 150, "{} asks", asks.len());
    crate::cost::take_calls();
    for &(at, index) in &asks {
        let again = top(at, index);
        let held = again.ranking.unwrap();
        assert_eq!((held.from, held.until, held.refused), (ranking.from, ranking.until, None), "the same proof at {at}, tile {index}");
        assert_eq!(again.lines.iter().map(|line| line.country.to_string()).collect::<Vec<_>>(), members);
    }
    let topped = crate::cost::take_calls();
    let named = whole(&members.iter().map(String::as_str).collect::<Vec<_>>());
    for &(_, index) in &asks {
        tile(&line, "1", &named, None, 1, index, false).unwrap();
    }
    let plain = crate::cost::take_calls();
    assert_eq!(topped, plain, "quiet hours and tiles of a proven top read nothing beyond their tiles: no ranking pass ({topped} reads against {plain})");
    let rank = |params: serde_json::Value| -> u64 {
        let mut request = serde_json::json!({ "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" }, "hour": hour });
        request.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
        crate::cost::take_calls();
        let reply: crate::series::ranked::SeriesRankReply = serde_json::from_value(crate::series::answer(&query, crate::series::ranked::PRODUCT, &request).unwrap().unwrap()).unwrap();
        assert_eq!(reply.rows.iter().map(|row| row.country.to_string()).collect::<Vec<_>>(), line.rank_at(hour, 2, false).unwrap().unwrap(), "rows in rank order at the hour");
        assert!(ranking.from <= reply.from && reply.until <= ranking.until);
        crate::cost::take_calls()
    };
    assert_eq!(rank(serde_json::json!({ "top": 2 })), rank(serde_json::json!({ "countries": members })), "a rank of the proven top reads what a rank of its named countries reads");
}
