//! The series engine against this reader: identical synthetic streams, replies compared.
use super::*;
use crate::products::country_groups::HourSpan;
use dashboard_series as engine;
use engine::codec::Binary;
use engine::query::{Alias, EventView, Tail};

const SECTION: &str = "parity";
const KEYS: u64 = 6;

#[derive(Default)]
struct Pages(Vec<Vec<u8>>);
impl engine::PageRead for Pages {
    fn identity(&mut self, id: u64) -> io::Result<[u64; 3]> {
        let bytes = self
            .0
            .get(id as usize)
            .ok_or_else(|| invalid("missing page"))?;
        let hash = blake3::hash(bytes);
        let words: [u8; 32] = *hash.as_bytes();
        Ok([
            u64::from_le_bytes(words[..8].try_into().unwrap()),
            u64::from_le_bytes(words[8..16].try_into().unwrap()),
            bytes.len() as u64,
        ])
    }
    fn len(&mut self, id: u64) -> io::Result<u64> {
        Ok(self
            .0
            .get(id as usize)
            .ok_or_else(|| invalid("missing page"))?
            .len() as u64)
    }
    fn read(&mut self, id: u64, offset: u64, into: &mut [u8]) -> io::Result<()> {
        let bytes = self
            .0
            .get(id as usize)
            .ok_or_else(|| invalid("missing page"))?;
        let at = offset as usize;
        into.copy_from_slice(
            bytes
                .get(at..at + into.len())
                .ok_or_else(|| invalid("short page"))?,
        );
        Ok(())
    }
}
impl engine::PageWrite for Pages {
    fn append(&mut self, bytes: &[u8]) -> io::Result<u64> {
        self.0.push(bytes.to_vec());
        Ok(self.0.len() as u64 - 1)
    }
}

fn identity() -> engine::Identity {
    engine::Identity {
        source: [4; 32],
        content: [5; 32],
        branch: [6; 32],
        schema: 1,
    }
}
fn hours(first: i32, last: i32) -> engine::Span {
    engine::Span::new(first, last).unwrap()
}

/// One key's stream: runs as (first, last, value, break, supplying) and dated events.
struct Stream {
    runs: Vec<(i32, i32, i128, bool, bool)>,
    events: Vec<i32>,
}

fn streams(seed: &mut u64, last: i32) -> Vec<Stream> {
    let mut next = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    };
    (0..KEYS)
        .map(|_| {
            let mut runs = Vec::new();
            let mut events = Vec::new();
            let mut hour = (next() % 5) as i32;
            while hour <= last {
                let length = (next() % 9) as i32 + 1;
                let end = (hour + length - 1).min(last);
                let value = [0, 7, 7, -3, 1_000_000_000_000][(next() % 5) as usize];
                runs.push((hour, end, value, next() % 3 == 0, next() % 4 == 0));
                if next() % 3 == 0 {
                    events.push(hour);
                    if next() % 2 == 0 {
                        events.push(hour);
                    }
                }
                hour = end + 1 + [0, 0, 0, 1, 4][(next() % 5) as usize];
            }
            Stream { runs, events }
        })
        .collect()
}

#[test]
fn engine_answers_points_ranges_charts_navigation_and_events_as_the_reader_does() {
    let last = 179;
    let cuts = [23, 47, 48, 95, 131, 179];
    let mut seed = 0x51ed_270b_2747_4ff7;
    let streams = streams(&mut seed, last);
    let fixture = tests::Fixture::new();
    let mut values = Accumulator::default();
    let budget = engine::Budget::new(256 * 1024 * 1024);
    let cache = engine::Cache::new(&budget);
    let mut pages = Pages::default();
    let mut writer = engine::Writer::new(&budget, identity(), 0).unwrap();
    let keys: Vec<u64> = (0..KEYS)
        .map(|actor| {
            writer
                .intern(
                    engine::KeySpec {
                        schema: 1,
                        actor,
                        measure: 0,
                        dimensions: engine::Blob::EMPTY,
                    },
                    engine::Blob::EMPTY,
                    engine::RunMode::Exact,
                    2,
                )
                .unwrap()
        })
        .collect();
    assert_eq!(keys, (0..KEYS).collect::<Vec<_>>());
    let mut root = 0;
    let mut first = 0;
    for (segment, &end) in cuts.iter().enumerate() {
        values.through(first);
        for (key, stream) in streams.iter().enumerate() {
            let key = key as u64;
            for &(a, b, value, broken, supplying) in &stream.runs {
                let Some(part) = hours(a, b).intersect(hours(first, end)) else {
                    continue;
                };
                let head = part.first == a;
                values.push_boundary(
                    key,
                    Run::new(part.first, part.last, value, broken && head),
                    supplying && head,
                );
                writer
                    .run(key, part, value, broken && head, supplying && head)
                    .unwrap();
            }
            writer
                .evidence(
                    key,
                    engine::Evidence {
                        span: hours(first, end),
                        known: engine::Knownness::Known,
                        supplying_read: None,
                        holder: None,
                        identity_first: None,
                    },
                    None,
                )
                .unwrap();
            for &hour in stream.events.iter().filter(|h| (first..=end).contains(*h)) {
                let sequence = writer.sequence().unwrap();
                values.event(key, hour, &sequence).unwrap();
                writer
                    .event(engine::Event {
                        key,
                        native_hour: hour,
                        read_hour: None,
                        sequence,
                        kind: 0,
                        fields: engine::Blob::EMPTY,
                    })
                    .unwrap();
            }
        }
        values.through(end);
        let mut store = fixture.writer();
        values
            .flush(&mut store, SECTION, 2, &AtomicBool::new(false))
            .unwrap();
        store.commit().unwrap();
        let pending = writer
            .cut(
                engine::Publication {
                    generation: segment as u64 + 1,
                    segment: segment as u64,
                    accepted: hours(0, end),
                    reached: end,
                    refusal: None,
                },
                engine::Blob::EMPTY,
            )
            .unwrap();
        let sealed = pending.seal(&mut pages).unwrap();
        root = writer.publish(&mut pages, &cache, sealed).unwrap();
        first = end + 1;
    }
    let cancelled = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, SECTION.into(), &cancelled).unwrap();
    let mut snapshot = engine::Snapshot::open(pages, &cache, root, identity()).unwrap();
    let selection = |key: u64| engine::Selection {
        key,
        membership: None,
    };

    for hour in -1..=last + 1 {
        let old = reader.values_at(&keys, hour).unwrap();
        for (at, &key) in keys.iter().enumerate() {
            let new = snapshot.value_at(key, hour).unwrap();
            let run = match new {
                engine::Observation::Value { run, .. } => Some(run),
                _ => None,
            };
            assert_eq!(run.map(|r| r.value), old[at], "key {key} hour {hour}");
            let expected = reader.value_at(key, hour).unwrap();
            assert_eq!(
                run.map(|r| (r.span.first, r.span.last, r.value, r.break_before)),
                expected.map(|r| (r.first_hour, r.last_hour, r.value, r.break_before)),
                "key {key} hour {hour}"
            );
        }
    }
    let samples: Vec<i32> = (0..=last).step_by(3).collect();
    let old = reader.changes_at(&keys, &samples).unwrap();
    let mut new = vec![Vec::new(); keys.len()];
    snapshot
        .changes_at(&keys, &samples, |key, sample, value| {
            new[key].push((
                sample,
                match value {
                    engine::Observation::Value { run, .. } => Some(run.value),
                    _ => None,
                },
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(new, old);

    for &key in &keys {
        for (from, to) in [(0, last), (5, 60), (47, 49), (100, 100)] {
            let old: Vec<_> = reader
                .runs(key, from, to)
                .unwrap()
                .into_iter()
                .map(|r| (r.first_hour, r.last_hour, r.value))
                .collect();
            let mut new = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot
                    .run_page(selection(key), hours(from, to), cursor, 1 << 16, |run| {
                        new.push((run.span.first, run.span.last, run.value));
                        Ok(())
                    })
                    .unwrap();
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(new, old, "key {key} window {from}..={to}");
        }
    }

    let within = [
        HourSpan {
            first_hour: 10,
            last_hour: 40,
        },
        HourSpan {
            first_hour: 90,
            last_hour: 150,
        },
    ];
    let members = [hours(10, 40), hours(90, 150)];
    for &key in &keys {
        for (view, membership) in [
            ((0.0, f64::from(last), 7), false),
            ((0.0, f64::from(last), 61), false),
            ((-2.5, f64::from(last) + 3.5, 200), false),
            ((20.0, 60.0, 41), false),
            ((20.0, 60.0, 41), true),
            ((0.0, f64::from(last), 13), true),
            ((0.0, f64::from(last), 180), true),
        ] {
            let old = reader
                .summary_within(key, (0, last), view, 2, membership.then_some(&within[..]))
                .unwrap();
            let mut new = Vec::new();
            snapshot
                .summary_tiles(
                    &[engine::Selection {
                        key,
                        membership: membership.then_some(&members[..]),
                    }],
                    hours(0, last),
                    engine::Viewport {
                        from: view.0,
                        to: view.1,
                        width: view.2,
                    },
                    Tail::EMPTY,
                    None,
                    1 << 24,
                    |_, bucket| {
                        new.push(bucket.exact);
                        Ok(())
                    },
                )
                .unwrap();
            assert_eq!(new.len(), old.buckets.len());
            for (new, old) in new.iter().zip(&old.buckets) {
                assert_eq!(new.map(chart_bucket), *old, "key {key} view {view:?}");
            }
            if let Some(old) = old.runs {
                let mut runs = Vec::new();
                snapshot
                    .hourly_runs(
                        engine::Selection {
                            key,
                            membership: membership.then_some(&members[..]),
                        },
                        hours(0, last),
                        engine::Viewport {
                            from: view.0,
                            to: view.1,
                            width: view.2,
                        },
                        None,
                        1 << 20,
                        |run| {
                            runs.push(ChartRun {
                                first_hour: run.span.first,
                                last_hour: run.span.last,
                                value: run.value as f64 / 100.0,
                                break_before: run.break_before,
                            });
                            Ok(())
                        },
                    )
                    .unwrap();
                assert_eq!(runs, old, "key {key} view {view:?}");
            }
        }
    }

    let spans = [
        (0, 0, 60, None),
        (3, 61, 120, Some(false)),
        (5, 121, last, Some(true)),
    ];
    let aliases: Vec<_> = spans
        .iter()
        .map(|&(key, first, last, boundary)| Alias {
            key,
            span: hours(first, last),
            boundary,
        })
        .collect();
    for width in [9, 180] {
        let view = (0.0, f64::from(last), width);
        let old = reader.summary_spans(&spans, (0, last), view, 2).unwrap();
        let mut new = Vec::new();
        snapshot
            .summary_aliases(
                &aliases,
                hours(0, last),
                engine::Viewport {
                    from: view.0,
                    to: view.1,
                    width,
                },
                None,
                1 << 24,
                |_, bucket| {
                    new.push(bucket.exact.map(chart_bucket));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(new, old.buckets, "alias width {width}");
        if let Some(old) = old.runs {
            let mut runs = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot
                    .hourly_alias_runs(
                        &aliases,
                        hours(0, last),
                        engine::Viewport {
                            from: view.0,
                            to: view.1,
                            width,
                        },
                        cursor,
                        engine::Run::SIZE,
                        |run| {
                            runs.push(ChartRun {
                                first_hour: run.span.first,
                                last_hour: run.span.last,
                                value: run.value as f64 / 100.0,
                                break_before: run.break_before,
                            });
                            Ok(())
                        },
                    )
                    .unwrap();
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(runs, old, "alias runs width {width}");
        }
    }

    for &key in &keys {
        let mut old = Vec::new();
        for (from, to) in [(0, 49), (50, 179)] {
            old.extend(
                reader
                    .top_spans(
                        key,
                        (from, to),
                        (f64::from(from), f64::from(to), (to - from + 1) as u32),
                        7,
                        &[],
                    )
                    .unwrap()
                    .into_iter()
                    .map(|s| (s.first_hour, s.last_hour)),
            );
        }
        let mut new = Vec::new();
        for (from, to) in [(0, 49), (50, 179)] {
            snapshot
                .top_spans(selection(key), 7, hours(from, to), |s| {
                    new.push((s.first, s.last));
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(new, old, "key {key}");
    }

    for origin in [
        None,
        Some(-3.0),
        Some(0.0),
        Some(20.5),
        Some(47.0),
        Some(48.0),
        Some(100.25),
        Some(200.0),
    ] {
        for (direction, measure) in [
            (engine::Direction::Forward, MeasureKey::ArrowRight),
            (engine::Direction::Backward, MeasureKey::ArrowLeft),
            (engine::Direction::Home, MeasureKey::Home),
            (engine::Direction::End, MeasureKey::End),
        ] {
            for (window, membership) in [
                ((0.0, f64::from(last)), None),
                ((5.5, 150.0), Some(&within[..])),
            ] {
                let navigation = Navigation {
                    from: origin,
                    first_hour: window.0,
                    last_hour: window.1,
                    key: measure,
                };
                let old = navigate(
                    &mut [(&mut reader, keys.iter().map(|&k| (k, membership)).collect())],
                    &navigation,
                )
                .unwrap();
                let selections: Vec<_> = keys
                    .iter()
                    .map(|&key| engine::Selection {
                        key,
                        membership: membership.map(|_| &members[..]),
                    })
                    .collect();
                let new = snapshot
                    .navigate_window(&selections, origin, direction, window)
                    .unwrap();
                assert_eq!(new, old, "origin {origin:?} {direction:?} {window:?}");
            }
        }
    }

    for &key in &keys {
        for hour in [-1, 0, 47, 48, 100, last] {
            assert_eq!(
                snapshot.event_count_through(key, hour, None).unwrap(),
                reader.event_count_through(key, hour).unwrap(),
                "key {key} hour {hour}"
            );
            let old = reader.latest_value(key, hour).unwrap();
            let new = snapshot.latest_event(selection(key), hour, None).unwrap();
            assert_eq!(
                new.map(|e| (e.native_hour(), e.sequence())),
                old.map(|(hour, row)| (hour, row.as_u64().unwrap())),
                "key {key} hour {hour}"
            );
        }
        let count = reader.event_count_through(key, last).unwrap();
        for ordinal in 0..count {
            assert_eq!(
                snapshot.event_date(selection(key), None, ordinal).unwrap(),
                Some(reader.event_date(key, ordinal).unwrap())
            );
        }
        let old = reader
            .event_clusters(key, -0.5, f64::from(last) + 0.5, 300)
            .unwrap();
        let mut new = Vec::new();
        let selected = [selection(key)];
        snapshot
            .event_clusters(
                engine::EventQuery {
                    selection: &selected,
                    native: hours(i32::MIN, i32::MAX),
                    as_of: None,
                    order: engine::EventOrder::Native,
                },
                engine::Viewport {
                    from: -0.5,
                    to: f64::from(last) + 0.5,
                    width: 300,
                },
                None,
                1 << 20,
                |c| {
                    new.push(MarkCluster {
                        first_hour: c.first_hour.unwrap(),
                        last_hour: c.last_hour.unwrap(),
                        first_ordinal: c.first_ordinal.to_string(),
                        count: c.count.to_string(),
                    });
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(new, old.clusters, "key {key}");
    }
    let selections: Vec<_> = keys.iter().map(|&key| selection(key)).collect();
    let mut cursor = 0;
    let mut old = Vec::new();
    loop {
        let page = reader.events(&keys, 10, 150, cursor, 7).unwrap();
        old.extend(
            page.rows
                .iter()
                .map(|row| (row.hour, row.key, row.value().as_u64().unwrap())),
        );
        match page.cursor {
            Some(next) => cursor = next,
            None => break,
        }
    }
    let mut new = Vec::new();
    let mut next = None;
    loop {
        let page = snapshot
            .event_page(
                engine::EventQuery {
                    selection: &selections,
                    native: hours(10, 150),
                    as_of: None,
                    order: engine::EventOrder::Native,
                },
                next,
                7 * (engine::Event::SIZE + 1),
                |row: EventView| {
                    new.push((row.native_hour(), row.key(), row.sequence()));
                    Ok(())
                },
            )
            .unwrap();
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    assert_eq!(new, old);
}

/// Each key's ordered contributions in insertion order, drawn segment by segment:
/// later draws are backdated behind earlier ones, overlap them, or cross the cut.
fn ordered_streams(seed: &mut u64, cuts: &[i32]) -> Vec<Vec<(i32, i32, i128, bool, bool)>> {
    let mut next = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    };
    let last = cuts[cuts.len() - 1];
    (0..KEYS)
        .map(|_| {
            let mut runs = Vec::new();
            let mut first = 0;
            for &end in cuts {
                for _ in 0..next() % 6 {
                    let start = first + (next() % (end - first + 1) as u64) as i32;
                    let finish = (start + (next() % 12) as i32).min(last);
                    let value = [0, 7, 7, -3][(next() % 4) as usize];
                    runs.push((start, finish, value, next() % 3 == 0, next() % 5 == 0));
                }
                first = end + 1;
            }
            runs
        })
        .collect()
}

#[test]
fn engine_navigates_backdated_ordered_contributions_as_the_reader_does() {
    let last = 179;
    let cuts = [23, 47, 48, 95, 131, 179];
    let mut seed = 0x7a3c_91d2_04be_5f61;
    let mut streams = ordered_streams(&mut seed, &cuts);
    // A later run sharing the first selection's start, directly and across a gap.
    streams.push(vec![(0, 10, 1, true, false), (5, 5, 2, true, false), (0, 10, 3, true, false)]);
    streams.push(vec![(0, 2, 1, true, false), (5, 5, 2, true, false), (0, 10, 3, true, false)]);
    let fixture = tests::Fixture::new();
    let mut values = Accumulator::default();
    let budget = engine::Budget::new(256 * 1024 * 1024);
    let cache = engine::Cache::new(&budget);
    let mut pages = Pages::default();
    let mut writer = engine::Writer::new(&budget, identity(), 0).unwrap();
    let keys: Vec<u64> = (0..streams.len() as u64)
        .map(|actor| {
            writer
                .intern(
                    engine::KeySpec {
                        schema: 1,
                        actor,
                        measure: 1,
                        dimensions: engine::Blob::EMPTY,
                    },
                    engine::Blob::EMPTY,
                    engine::RunMode::OrderedContributions,
                    2,
                )
                .unwrap()
        })
        .collect();
    let mut root = 0;
    let mut first = 0;
    for (segment, &end) in cuts.iter().enumerate() {
        values.through(first);
        for (key, runs) in streams.iter().enumerate() {
            let key = key as u64;
            for &(a, b, value, broken, supplying) in runs {
                let Some(part) = hours(a, b).intersect(hours(first, end)) else {
                    continue;
                };
                let head = part.first == a;
                values.push_boundary(
                    key,
                    Run::new(part.first, part.last, value, broken && head),
                    supplying && head,
                );
                writer
                    .run(key, part, value, broken && head, supplying && head)
                    .unwrap();
            }
            writer
                .evidence(
                    key,
                    engine::Evidence {
                        span: hours(first, end),
                        known: engine::Knownness::Known,
                        supplying_read: None,
                        holder: None,
                        identity_first: None,
                    },
                    None,
                )
                .unwrap();
        }
        values.through(end);
        let mut store = fixture.writer();
        values
            .flush(&mut store, SECTION, 2, &AtomicBool::new(false))
            .unwrap();
        store.commit().unwrap();
        let pending = writer
            .cut(
                engine::Publication {
                    generation: segment as u64 + 1,
                    segment: segment as u64,
                    accepted: hours(0, end),
                    reached: end,
                    refusal: None,
                },
                engine::Blob::EMPTY,
            )
            .unwrap();
        let sealed = pending.seal(&mut pages).unwrap();
        root = writer.publish(&mut pages, &cache, sealed).unwrap();
        first = end + 1;
    }
    let cancelled = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, SECTION.into(), &cancelled).unwrap();
    let mut snapshot = engine::Snapshot::open(pages, &cache, root, identity()).unwrap();
    let selection = |key: u64| engine::Selection {
        key,
        membership: None,
    };

    for hour in -1..=last + 1 {
        let old = reader.values_at(&keys, hour).unwrap();
        for (at, &key) in keys.iter().enumerate() {
            let run = match snapshot.value_at(key, hour).unwrap() {
                engine::Observation::Value { run, .. } => Some(run),
                _ => None,
            };
            assert_eq!(run.map(|r| r.value), old[at], "key {key} hour {hour}");
            let expected = reader.value_at(key, hour).unwrap();
            assert_eq!(
                run.map(|r| (r.span.first, r.span.last, r.value, r.break_before)),
                expected.map(|r| (r.first_hour, r.last_hour, r.value, r.break_before)),
                "key {key} hour {hour}"
            );
        }
    }
    let samples: Vec<i32> = (0..=last).step_by(3).collect();
    let old = reader.changes_at(&keys, &samples).unwrap();
    let mut new = vec![Vec::new(); keys.len()];
    snapshot
        .changes_at(&keys, &samples, |key, sample, value| {
            new[key].push((
                sample,
                match value {
                    engine::Observation::Value { run, .. } => Some(run.value),
                    _ => None,
                },
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(new, old);

    let within = [
        HourSpan {
            first_hour: 10,
            last_hour: 40,
        },
        HourSpan {
            first_hour: 90,
            last_hour: 150,
        },
    ];
    let members = [hours(10, 40), hours(90, 150)];
    let all = hours(i32::MIN, i32::MAX);
    let mut stops = 0;
    for &key in &keys {
        for half in -6..=2 * (last + 3) {
            let hour = f64::from(half) / 2.0;
            for forward in [true, false] {
                let old = reader.next_change(key, hour, forward).map_err(|_| ());
                let new = snapshot
                    .next_change(selection(key), hour, forward, all)
                    .map_err(|_| ());
                assert_eq!(new, old, "key {key} hour {hour} forward {forward}");
                stops += usize::from(matches!(old, Ok(Some(_))));
            }
        }
        for (from, to) in [(0, last), (5, 60), (47, 49), (100, 100)] {
            let old: Vec<_> = reader
                .runs(key, from, to)
                .unwrap()
                .into_iter()
                .map(|r| (r.first_hour, r.last_hour, r.value))
                .collect();
            let mut new = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot
                    .run_page(
                        selection(key),
                        hours(from, to),
                        cursor,
                        engine::Run::SIZE,
                        |run| {
                            new.push((run.span.first, run.span.last, run.value));
                            Ok(())
                        },
                    )
                    .unwrap();
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(new, old, "key {key} window {from}..={to}");
        }
        let view = (0.0, f64::from(last), (last + 1) as u32);
        let old = reader
            .summary_within(key, (0, last), view, 2, None)
            .unwrap();
        let viewport = engine::Viewport {
            from: view.0,
            to: view.1,
            width: view.2,
        };
        let mut new = Vec::new();
        snapshot
            .summary_tiles(
                &[selection(key)],
                hours(0, last),
                viewport,
                Tail::EMPTY,
                None,
                1 << 24,
                |_, bucket| {
                    new.push(bucket.exact.map(chart_bucket));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(new, old.buckets, "key {key}");
        for (view, membership) in [
            ((0.0, f64::from(last), (last + 1) as u32), false),
            ((0.0, f64::from(last), (last + 1) as u32), true),
            ((20.0, 60.0, 41), false),
            ((20.0, 60.0, 41), true),
            ((-2.5, f64::from(last) + 3.5, 200), false),
        ] {
            let old = reader
                .summary_within(key, (0, last), view, 2, membership.then_some(&within[..]))
                .unwrap()
                .runs
                .unwrap();
            let selected = engine::Selection {
                key,
                membership: membership.then_some(&members[..]),
            };
            for bytes in [engine::Run::SIZE, 1 << 20] {
                let mut new = Vec::new();
                let mut cursor = None;
                loop {
                    let page = snapshot
                        .hourly_runs(
                            selected,
                            hours(0, last),
                            engine::Viewport {
                                from: view.0,
                                to: view.1,
                                width: view.2,
                            },
                            cursor,
                            bytes,
                            |run| {
                                new.push(ChartRun {
                                    first_hour: run.span.first,
                                    last_hour: run.span.last,
                                    value: run.value as f64 / 100.0,
                                    break_before: run.break_before,
                                });
                                Ok(())
                            },
                        )
                        .unwrap();
                    cursor = page.next;
                    if cursor.is_none() {
                        break;
                    }
                }
                assert_eq!(new, old, "key {key} view {view:?} pages of {bytes} bytes");
            }
        }
    }
    assert!(stops > 100);

    for origin in [
        None,
        Some(-3.0),
        Some(0.0),
        Some(20.5),
        Some(47.0),
        Some(48.0),
        Some(100.25),
        Some(200.0),
    ] {
        for (direction, measure) in [
            (engine::Direction::Forward, MeasureKey::ArrowRight),
            (engine::Direction::Backward, MeasureKey::ArrowLeft),
            (engine::Direction::Home, MeasureKey::Home),
            (engine::Direction::End, MeasureKey::End),
        ] {
            for (window, membership) in [
                ((0.0, f64::from(last)), None),
                ((5.5, 150.0), Some(&within[..])),
            ] {
                let navigation = Navigation {
                    from: origin,
                    first_hour: window.0,
                    last_hour: window.1,
                    key: measure,
                };
                let old = navigate(
                    &mut [(&mut reader, keys.iter().map(|&k| (k, membership)).collect())],
                    &navigation,
                )
                .map_err(|_| ());
                let selections: Vec<_> = keys
                    .iter()
                    .map(|&key| engine::Selection {
                        key,
                        membership: membership.map(|_| &members[..]),
                    })
                    .collect();
                let new = snapshot
                    .navigate_window(&selections, origin, direction, window)
                    .map_err(|_| ());
                assert_eq!(new, old, "origin {origin:?} {direction:?} {window:?}");
            }
        }
    }
}

fn chart_bucket(summary: engine::summary::Summary) -> ChartBucket {
    let total = summary.absolute_total.0;
    assert_eq!((total[2], total[3]), (0, 0));
    ChartBucket {
        absolute_total: u128::from(total[0]) | u128::from(total[1]) << 64,
        first_hour: summary.first_hour,
        last_hour: summary.last_hour,
        first_value: summary.first_value,
        last_value: summary.last_value,
        last_value_hour: summary.last_value_hour,
        min_hour: summary.min_hour,
        min_value: summary.min_value,
        max_hour: summary.max_hour,
        max_value: summary.max_value,
        first_break: summary.first_break,
        has_break: summary.has_break,
    }
}
