use crate::{
    budget::Budget,
    cache::Cache,
    codec::{Binary, Decode, Encode},
    pages::{self, Kind, PageRead, PageWrite},
    query::{Bucket, Cluster, EventView, Tail, Viewport},
    summary::{Summary, U256},
    *,
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Default)]
struct Memory {
    pages: Vec<Vec<u8>>,
    reads: usize,
    bytes: u64,
    fail: Option<usize>,
    kinds: [usize; 8],
}
impl PageRead for Memory {
    fn identity(&mut self, id: u64) -> io::Result<[u64; 3]> {
        let bytes = self
            .pages
            .get(id as usize)
            .ok_or_else(|| crate::invalid("missing test page"))?;
        let hash = blake3::hash(bytes);
        let mut d = Decode::new(hash.as_bytes());
        Ok([d.u64()?, d.u64()?, bytes.len() as u64])
    }
    fn len(&mut self, id: u64) -> io::Result<u64> {
        Ok(self
            .pages
            .get(id as usize)
            .ok_or_else(|| crate::invalid("missing test page"))?
            .len() as u64)
    }
    fn read(&mut self, id: u64, offset: u64, into: &mut [u8]) -> io::Result<()> {
        self.reads += 1;
        self.bytes += into.len() as u64;
        let bytes = self
            .pages
            .get(id as usize)
            .ok_or_else(|| crate::invalid("missing test page"))?;
        if offset == 0 && bytes.len() >= 12 {
            let kind = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
            if kind < self.kinds.len() {
                self.kinds[kind] += 1;
            }
        }
        let at = offset as usize;
        into.copy_from_slice(
            bytes
                .get(at..at + into.len())
                .ok_or_else(|| crate::invalid("truncated test page"))?,
        );
        Ok(())
    }
}
impl PageWrite for Memory {
    fn append(&mut self, bytes: &[u8]) -> io::Result<u64> {
        if self.fail == Some(self.pages.len()) {
            return Err(io::Error::other("synthetic write failure"));
        }
        let id = self.pages.len() as u64;
        self.pages.push(bytes.to_vec());
        Ok(id)
    }
}
fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: 7,
    }
}
fn span(first: i32, last: i32) -> Span {
    Span::new(first, last).unwrap()
}
fn publication(generation: u64, segment: u64, first: i32, last: i32, reached: i32) -> Publication {
    Publication {
        generation,
        segment,
        accepted: span(first, last),
        reached,
        refusal: None,
    }
}
fn known(first: i32, last: i32, read: Option<i32>, holder: Option<u64>) -> Evidence {
    Evidence {
        span: span(first, last),
        known: Knownness::Known,
        supplying_read: read,
        holder,
        identity_first: Some(first),
    }
}
fn spec(actor: u64, measure: u64) -> KeySpec {
    KeySpec {
        schema: 7,
        actor,
        measure,
        dimensions: Blob::EMPTY,
    }
}
fn selection(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}
/// Cuts, seals and publishes the writer's open segment.
fn publish(
    writer: &mut Writer<'_>,
    memory: &mut Memory,
    cache: &Cache<'_>,
    publication: Publication,
    checkpoint: Blob,
) -> u64 {
    let pending = writer.cut(publication, checkpoint).unwrap();
    let sealed = pending.seal(memory).unwrap();
    writer.publish(memory, cache, sealed).unwrap()
}
const BIG: usize = 96 * 1024 * 1024;

fn reference_add(a: U256, b: U256) -> U256 {
    let mut sum = [0; 4];
    let mut carry = 0_u128;
    for (i, out) in sum.iter_mut().enumerate() {
        let value = u128::from(a.0[i]) + u128::from(b.0[i]) + carry;
        *out = value as u64;
        carry = value >> 64;
    }
    assert_eq!(carry, 0);
    U256(sum)
}
fn reference_piece(run: Run, span: Span, decimals: u8) -> Summary {
    let magnitude = run.value.unsigned_abs();
    let amount = U256([magnitude as u64, (magnitude >> 64) as u64, 0, 0]);
    let mut total = U256([0; 4]);
    for _ in 0..span.hours() {
        total = reference_add(total, amount);
    }
    let value = run.value as f64 / 10_f64.powi(i32::from(decimals));
    Summary {
        absolute_total: total,
        covered: span.hours(),
        first_hour: span.first,
        last_hour: span.last,
        last_value_hour: span.first,
        min_hour: span.first,
        max_hour: span.first,
        first_value: value,
        last_value: value,
        min_value: value,
        max_value: value,
        raw_min: run.value,
        raw_max: run.value,
        first_break: run.break_before && span.first == run.span.first,
        has_break: false,
    }
}
fn reference_merge(a: Option<Summary>, b: Option<Summary>) -> Option<Summary> {
    match (a, b) {
        (None, None) => None,
        (Some(mut a), None) | (None, Some(mut a)) => {
            a.has_break = true;
            Some(a)
        }
        (Some(a), Some(b)) => Some(Summary {
            absolute_total: reference_add(a.absolute_total, b.absolute_total),
            covered: a.covered + b.covered,
            first_hour: a.first_hour,
            last_hour: b.last_hour,
            first_value: a.first_value,
            last_value: b.last_value,
            last_value_hour: if !b.has_break
                && !b.first_break
                && i64::from(a.last_hour) + 1 == i64::from(b.first_hour)
                && b.min_value == b.max_value
                && a.last_value == b.first_value
            {
                a.last_value_hour
            } else {
                b.last_value_hour
            },
            min_hour: if a.min_value <= b.min_value {
                a.min_hour
            } else {
                b.min_hour
            },
            min_value: a.min_value.min(b.min_value),
            max_hour: if a.max_value >= b.max_value {
                a.max_hour
            } else {
                b.max_hour
            },
            max_value: a.max_value.max(b.max_value),
            raw_min: a.raw_min.min(b.raw_min),
            raw_max: a.raw_max.max(b.raw_max),
            first_break: a.first_break,
            has_break: a.has_break
                || b.has_break
                || b.first_break
                || i64::from(a.last_hour) + 1 != i64::from(b.first_hour),
        }),
    }
}

#[derive(Default)]
struct Logical {
    runs: Vec<Run>,
    evidence: Vec<Evidence>,
    events: Vec<EventView>,
    decimals: u8,
}
#[derive(Default)]
struct Oracle {
    keys: BTreeMap<u64, Logical>,
    accepted: Option<Span>,
}
impl Oracle {
    fn value(&self, key: u64, hour: i32) -> Observation {
        let accepted = self.accepted.unwrap();
        if hour > accepted.last {
            return Observation::Pending;
        }
        if hour < accepted.first {
            return Observation::Unknown(None);
        }
        let Some(key) = self.keys.get(&key) else {
            return Observation::Unknown(None);
        };
        let Some(evidence) = key.evidence.iter().find(|e| e.span.contains(hour)).copied() else {
            return Observation::Unknown(None);
        };
        if matches!(evidence.known, Knownness::Unknown(_)) {
            return Observation::Unknown(Some(evidence));
        }
        key.runs
            .partition_point(|r| r.span.first <= hour)
            .checked_sub(1)
            .and_then(|i| key.runs.get(i))
            .filter(|r| r.span.contains(hour))
            .map_or(Observation::KnownAbsent(evidence), |&run| {
                Observation::Value { run, evidence }
            })
    }
    fn events(&self, query: EventQuery<'_>) -> Vec<EventView> {
        let mut rows = Vec::new();
        let keys: std::collections::BTreeSet<_> = query.selection.iter().map(|s| s.key).collect();
        for key in keys {
            for &row in &self.keys[&key].events {
                let hour = row.native_hour();
                let read = match row {
                    EventView::Family(e) => e.read_hour,
                    EventView::Evidence { value, .. } => value.supplying_read,
                };
                if !query.native.contains(hour)
                    || !query.selection.iter().any(|s| {
                        s.key == key
                            && s.membership
                                .is_none_or(|members| members.iter().any(|m| m.contains(hour)))
                    })
                {
                    continue;
                }
                if let Some(as_of) = query.as_of {
                    if read.is_none_or(|r| r > as_of.min(self.accepted.unwrap().last)) {
                        continue;
                    }
                } else if read.is_some_and(|r| r > self.accepted.unwrap().last) {
                    continue;
                }
                rows.push(row);
            }
        }
        rows.sort_by_key(|r| match query.order {
            EventOrder::Native => (i64::from(r.native_hour()), r.key(), r.sequence()),
            EventOrder::Publication => (0, 0, r.sequence()),
        });
        rows
    }
    fn clusters(&self, query: EventQuery<'_>, view: Viewport) -> Vec<Cluster> {
        let rows = self.events(EventQuery {
            order: EventOrder::Native,
            ..query
        });
        let unmasked: Vec<_> = query.selection.iter().map(|s| selection(s.key)).collect();
        let all = self.events(EventQuery {
            selection: &unmasked,
            native: span(i32::MIN, i32::MAX),
            as_of: query.as_of,
            order: EventOrder::Native,
        });
        let cells = view.width.div_ceil(18);
        let mut result = Vec::new();
        for cell in 0..cells {
            let start = view.from + (view.to - view.from) * f64::from(cell) / f64::from(cells);
            let end = view.from + (view.to - view.from) * f64::from(cell + 1) / f64::from(cells);
            let first = start.ceil() as i32;
            let last = if cell + 1 == cells {
                end.floor()
            } else {
                end.ceil() - 1.0
            } as i32;
            if first > last {
                continue;
            }
            let Some(extent) = span(first, last).intersect(query.native) else {
                continue;
            };
            let within: Vec<_> = rows
                .iter()
                .copied()
                .filter(|r| extent.contains(r.native_hour()))
                .collect();
            let Some(first) = within.first() else {
                continue;
            };
            result.push(Cluster {
                span: extent,
                first_hour: Some(first.native_hour()),
                last_hour: within.last().map(|e| e.native_hour()),
                first_ordinal: all
                    .iter()
                    .position(|r| r.sequence() == first.sequence() && r.key() == first.key())
                    .unwrap() as u64,
                count: within.len() as u64,
            });
        }
        result
    }
    fn raw_bucket(&self, key: u64, bin: Span, members: Option<&[Span]>) -> Option<Summary> {
        let mut result = None;
        let data = &self.keys[&key];
        for &run in &data.runs {
            for hour in bin.first..=bin.last {
                if run.span.contains(hour)
                    && self.accepted.unwrap().contains(hour)
                    && members.is_none_or(|m| m.iter().any(|s| s.contains(hour)))
                    && data
                        .evidence
                        .iter()
                        .any(|e| e.known == Knownness::Known && e.span.contains(hour))
                {
                    let next = Some(reference_piece(run, span(hour, hour), data.decimals));
                    result = if result.is_some() {
                        reference_merge(result, next)
                    } else {
                        next
                    };
                }
            }
        }
        result
    }
    fn numeric_next(&self, key: u64, at: f64, forward: bool) -> Option<i32> {
        let mut intervals = Vec::new();
        for run in &self.keys[&key].runs {
            for known in self.known(key) {
                if let Some(span) = run
                    .span
                    .intersect(known)
                    .and_then(|s| s.intersect(self.accepted.unwrap()))
                {
                    intervals.push(span);
                }
            }
        }
        intervals.sort_by_key(|s| s.first);
        if forward {
            if let Some(span) = intervals
                .iter()
                .find(|s| at >= f64::from(s.first) && at < f64::from(s.last) + 1.0)
            {
                span.last.checked_add(1)
            } else {
                intervals
                    .iter()
                    .find(|s| f64::from(s.first) > at)
                    .map(|s| s.first)
            }
        } else {
            intervals
                .iter()
                .rev()
                .find(|s| f64::from(s.first) < at)
                .map(|s| {
                    s.last
                        .checked_add(1)
                        .filter(|h| f64::from(*h) < at)
                        .unwrap_or(s.first)
                })
        }
    }
    fn navigation(
        &self,
        selected: &[Selection<'_>],
        origin: Option<f64>,
        direction: Direction,
        bounds: Span,
    ) -> Option<f64> {
        let forward = matches!(direction, Direction::Forward | Direction::Home);
        let at = match direction {
            Direction::Home => f64::from(bounds.first) - 1.0,
            Direction::End => f64::from(bounds.last) + 1.0,
            _ => origin.unwrap_or(if forward {
                f64::from(bounds.first) - 1.0
            } else {
                f64::from(bounds.last) + 1.0
            }),
        };
        let mut result = None;
        for selected in selected {
            let whole = [bounds];
            for member in selected.membership.unwrap_or(&whole) {
                let Some(member) = member.intersect(bounds) else {
                    continue;
                };
                let within = if forward {
                    at.max(f64::from(member.first) - 1.0)
                } else {
                    at.min(f64::from(member.last) + 1.0)
                };
                let candidate = self
                    .numeric_next(selected.key, within, forward)
                    .filter(|h| member.contains(*h));
                let edge = (self
                    .numeric_next(selected.key, f64::from(member.first), false)
                    .is_some()
                    && ((forward && at < f64::from(member.first))
                        || (!forward && at > f64::from(member.first))))
                .then_some(member.first);
                for hour in [candidate, edge].into_iter().flatten() {
                    let hour = f64::from(hour);
                    result = Some(result.map_or(hour, |old: f64| {
                        if forward {
                            old.min(hour)
                        } else {
                            old.max(hour)
                        }
                    }));
                }
            }
        }
        result.or_else(|| origin.map(|h| h.clamp(f64::from(bounds.first), f64::from(bounds.last))))
    }
    fn known(&self, key: u64) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        for e in &self.keys[&key].evidence {
            if e.known == Knownness::Known {
                if let Some(previous) = spans.last_mut() {
                    if i64::from(previous.last) + 1 == i64::from(e.span.first) {
                        previous.last = e.span.last;
                        continue;
                    }
                }
                spans.push(e.span);
            }
        }
        spans
    }
    fn ranges(&self, key: u64, window: Span) -> Vec<Run> {
        let mut result: Vec<Run> = Vec::new();
        for run in &self.keys[&key].runs {
            for known in self.known(key) {
                if let Some(span) = run
                    .span
                    .intersect(window)
                    .and_then(|s| s.intersect(known))
                    .and_then(|s| s.intersect(self.accepted.unwrap()))
                {
                    if let Some(previous) = result.last_mut() {
                        if !run.break_before
                            && previous.value == run.value
                            && i64::from(previous.span.last) + 1 == i64::from(span.first)
                        {
                            previous.span.last = span.last;
                            continue;
                        }
                    }
                    result.push(Run { span, ..*run });
                }
            }
        }
        result
    }
    fn bucket(&self, key: u64, bin: Span, members: Option<&[Span]>) -> Option<Summary> {
        let original = self.range_bucket(key, bin)?;
        let Some(members) = members else {
            return Some(original);
        };
        let mut result = None;
        for member in members {
            let first = original.first_hour.max(member.first);
            let last = original.last_hour.min(member.last);
            if first <= last {
                let next = self.range_bucket(key, span(first, last));
                if result.is_some() {
                    result = reference_merge(result, next);
                } else {
                    result = next;
                }
            }
        }
        result
    }
    fn range_bucket(&self, key: u64, bin: Span) -> Option<Summary> {
        let mut result = None;
        for known in self.known(key) {
            let Some(known) = known
                .intersect(bin)
                .and_then(|s| s.intersect(self.accepted.unwrap()))
            else {
                continue;
            };
            let mut date = i64::from(known.first);
            let mut value = None;
            let mut initialized = false;
            while date <= i64::from(known.last) {
                if let Some(&size) = [16_i64, 256, 4096, 65536].iter().rev().find(|&&size| {
                    date.rem_euclid(size) == 0 && date + size - 1 <= i64::from(known.last)
                }) {
                    let next =
                        self.raw_bucket(key, span(date as i32, (date + size - 1) as i32), None);
                    value = if initialized {
                        reference_merge(value, next)
                    } else {
                        next
                    };
                    initialized = true;
                    date += size;
                } else {
                    let end = i64::from(known.last).min((date.div_euclid(16) + 1) * 16 - 1);
                    let mut partial = None;
                    let mut seen = false;
                    while date <= end {
                        let run = match self.value(key, date as i32) {
                            Observation::Value { run, .. } => Some(run),
                            _ => None,
                        };
                        let until = run.map_or(date, |r| i64::from(r.span.last).min(end));
                        let next = run.map(|r| {
                            reference_piece(
                                r,
                                span(date as i32, until as i32),
                                self.keys[&key].decimals,
                            )
                        });
                        partial = if seen {
                            reference_merge(partial, next)
                        } else {
                            next
                        };
                        seen = true;
                        date = until + 1;
                    }
                    value = if initialized {
                        reference_merge(value, partial)
                    } else {
                        partial
                    };
                    initialized = true;
                }
            }
            if let Some(next) = value {
                result = if result.is_some() {
                    reference_merge(result, Some(next))
                } else {
                    Some(next)
                };
            }
        }
        if let Some(value) = result.as_mut() {
            value.has_break |= value.first_hour > bin.first || value.last_hour < bin.last;
        }
        result
    }
}

/// Splits whole runs at segment ends: the head keeps its flags, later parts
/// continue it. Returns each part with its segment, and the logical runs with
/// the canonical and row ordinals the writer assigns.
#[allow(clippy::type_complexity)]
fn split(
    runs: &[(i32, i32, i128, bool, bool)],
    cuts: &[i32],
    first: i32,
) -> (Vec<(usize, Span, i128, bool, bool)>, Vec<Run>) {
    let mut parts = Vec::new();
    let mut logical = Vec::new();
    let mut rows = 0;
    for (canonical, &(a, b, value, broken, supplying)) in runs.iter().enumerate() {
        logical.push(Run {
            span: span(a, b),
            value,
            break_before: broken,
            supplying_boundary: supplying,
            canonical: canonical as u64,
            sequence: rows,
        });
        let mut start = first;
        for (segment, &last) in cuts.iter().enumerate() {
            if let Some(part) = span(a, b).intersect(span(start, last)) {
                let head = part.first == a;
                parts.push((segment, part, value, broken && head, supplying && head));
                rows += 1;
            }
            start = last + 1;
        }
    }
    (parts, logical)
}
fn segment_of(cuts: &[i32], hour: i32) -> usize {
    cuts.partition_point(|last| *last < hour)
}

/// Three keys over hours -8..=12, published in segments ending at `cuts`.
/// Runs and evidence crossing a cut continue in the next segment; each key's
/// events are appended in the segment holding hour 4, so earlier ones are late.
fn fixture(cuts: &[i32]) -> (Memory, u64, Oracle, Vec<u64>) {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), -8).unwrap();
    let mut oracle = Oracle {
        accepted: Some(span(-8, 12)),
        ..Oracle::default()
    };
    let mut keys = Vec::new();
    for actor in [99, 7, 123] {
        let key = writer
            .intern(spec(actor, 1), Blob::EMPTY, RunMode::Exact, 0)
            .unwrap();
        keys.push(key);
        oracle.keys.insert(key, Logical::default());
    }
    let data = [
        vec![
            (-8, -5, 0, true, false),
            (-4, -2, 0, false, true),
            (1, 3, 9, true, false),
            (4, 5, 9, true, false),
            (8, 10, -4, true, false),
        ],
        vec![
            (-8, -6, i128::MIN, true, false),
            (-5, -2, i128::MAX, false, false),
            (1, 2, 9_007_199_254_740_992, true, false),
            (3, 5, 9_007_199_254_740_993, false, false),
            (7, 9, 9_007_199_254_740_993, false, false),
        ],
        vec![(1, 5, 1, true, false), (7, 12, 1, true, false)],
    ];
    let mut parts = Vec::new();
    for (at, &key) in keys.iter().enumerate() {
        let (pieces, logical) = split(&data[at], cuts, -8);
        oracle.keys.get_mut(&key).unwrap().runs = logical;
        parts.push(pieces);
    }
    let evidence = |at: u64| {
        [
            known(-8, -2, Some(-8), Some(10 + at)),
            Evidence {
                known: Knownness::Unknown(44),
                ..known(-1, 0, None, None)
            },
            known(1, 5, None, Some(10 + at)),
            known(7, 12, Some(7), Some(90 + at)),
        ]
    };
    let events_at = segment_of(cuts, 4);
    let mut first = -8;
    for (segment, &last) in cuts.iter().enumerate() {
        let hours = span(first, last);
        if segment == 0 {
            writer.catalogue(3, Cell::Integer(i128::MAX)).unwrap();
            writer.catalogue(8, Cell::Null).unwrap();
        }
        for (at, &key) in keys.iter().enumerate() {
            for &(part_segment, part, value, broken, supplying) in &parts[at] {
                if part_segment == segment {
                    writer.run(key, part, value, broken, supplying).unwrap();
                }
            }
            for row in evidence(at as u64) {
                let Some(part) = row.span.intersect(hours) else {
                    continue;
                };
                let piece = Evidence { span: part, ..row };
                let logical = oracle.keys.get_mut(&key).unwrap();
                if part.first == row.span.first {
                    let sequence = writer.sequence().unwrap();
                    writer.evidence(key, piece, Some(sequence)).unwrap();
                    logical.evidence.push(row);
                    logical.events.push(EventView::Evidence {
                        key,
                        sequence,
                        value: piece,
                    });
                } else {
                    writer.evidence(key, piece, None).unwrap();
                }
            }
            if segment == events_at {
                for (native, read) in [
                    (4, Some(4)),
                    (-5, Some(9)),
                    (4, Some(5)),
                    (-5, Some(10)),
                    (0, None),
                ] {
                    let sequence = writer.sequence().unwrap();
                    let event = Event {
                        key,
                        native_hour: native,
                        read_hour: read,
                        sequence,
                        kind: 101,
                        fields: Blob::EMPTY,
                    };
                    writer.event(event).unwrap();
                    oracle
                        .keys
                        .get_mut(&key)
                        .unwrap()
                        .events
                        .push(EventView::Family(event));
                }
            }
        }
        let end = segment + 1 == cuts.len();
        let checkpoint = if end {
            writer
                .cells([Cell::Unsigned(123), Cell::List(Blob::EMPTY)])
                .unwrap()
        } else {
            Blob::EMPTY
        };
        let publication = publication(
            segment as u64 + 1,
            segment as u64,
            -8,
            last,
            if end { 16 } else { last },
        );
        let root = publish(&mut writer, &mut memory, &cache, publication, checkpoint);
        if end {
            drop(writer);
            drop(cache);
            assert_eq!(budget.used(), 0);
            return (memory, root, oracle, keys);
        }
        first = last + 1;
    }
    unreachable!()
}
fn every_hour() -> Vec<i32> {
    (-8..=12).collect()
}
fn partitions() -> [Vec<i32>; 3] {
    [vec![12], vec![-3, 4, 12], every_hour()]
}

/// The current reader's chart grid: every bin of `view` over `bounds`.
fn grid(bounds: Span, view: Viewport) -> Vec<Span> {
    let first = view.from.floor().max(f64::from(bounds.first));
    let last = view.to.floor().min(f64::from(bounds.last));
    if first > last {
        return Vec::new();
    }
    let (first, last) = (first as i64, last as i64);
    let count = last - first + 1;
    let width = i64::from(view.width);
    let per_pixel = (count / width).max(1);
    let (size, base) = if per_pixel <= 16 {
        let size = 1_i64 << per_pixel.ilog2();
        let from = i64::from(bounds.first);
        (size, from + (first - from) / size * size)
    } else {
        ((count + width - 1) / width, first)
    };
    (0..)
        .map(|bin| base + bin * size)
        .take_while(|start| *start <= last)
        .map(|start| span(start.max(first) as i32, (start + size - 1).min(last) as i32))
        .collect()
}
/// Every bucket `summary_tiles` returns across its pages, with its key index and bin.
fn all_tiles(
    snapshot: &mut Snapshot<'_, Memory>,
    selections: &[Selection<'_>],
    bounds: Span,
    view: Viewport,
    per_page: usize,
) -> Vec<(usize, u64, Bucket)> {
    let mut all = Vec::new();
    let mut cursor = None;
    loop {
        let mut page = Vec::new();
        let tile = snapshot
            .summary_tiles(
                selections,
                bounds,
                view,
                Tail::EMPTY,
                cursor,
                per_page * std::mem::size_of::<Bucket>(),
                |bin, bucket| {
                    page.push((bin, bucket));
                    Ok(())
                },
            )
            .unwrap();
        all.extend(
            page.into_iter()
                .map(|(bin, bucket)| (tile.key_index, bin, bucket)),
        );
        cursor = tile.page.next;
        if cursor.is_none() {
            return all;
        }
    }
}
/// What `all_tiles` must return: every selected key's every bin, from the oracle.
fn expected_tiles(
    oracle: &Oracle,
    selections: &[Selection<'_>],
    bounds: Span,
    view: Viewport,
) -> Vec<(usize, u64, Bucket)> {
    let accepted = oracle.accepted.unwrap();
    let mut expected = Vec::new();
    for (index, selected) in selections.iter().enumerate() {
        for (bin, extent) in grid(bounds, view).into_iter().enumerate() {
            expected.push((
                index,
                bin as u64,
                Bucket {
                    span: extent,
                    exact: oracle.bucket(selected.key, extent, selected.membership),
                    pending: accepted.last.checked_add(1).and_then(|first| {
                        extent.intersect(Span {
                            first,
                            last: bounds.last,
                        })
                    }),
                },
            ));
        }
    }
    expected
}

#[test]
fn every_point_batch_sample_range_and_evidence_edge_matches_logical_oracle() {
    for cuts in partitions() {
        let (memory, root, oracle, keys) = fixture(&cuts);
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let hours: Vec<_> = (-10..=16).collect();
        for &hour in &hours {
            for &key in &keys {
                assert_eq!(
                    snapshot.value_at(key, hour).unwrap(),
                    oracle.value(key, hour)
                );
            }
            snapshot
                .values_at(&keys, hour, |index, value| {
                    assert_eq!(value.unwrap(), oracle.value(keys[index], hour));
                    Ok(())
                })
                .unwrap();
        }
        let mut expected = Vec::new();
        let mut observed = Vec::new();
        for (i, &key) in keys.iter().enumerate() {
            let mut previous = None;
            let mut seen = None;
            for (sample, &hour) in hours.iter().enumerate() {
                let value = oracle.value(key, hour);
                let amount = match value {
                    Observation::Value { run, .. } => Some(run.value),
                    _ => None,
                };
                if previous != amount {
                    expected.push((i, sample, value));
                    previous = amount;
                }
                if seen != Some(value) {
                    observed.push((i, sample, value));
                    seen = Some(value);
                }
            }
        }
        let mut actual = Vec::new();
        snapshot
            .changes_at(&keys, &hours, |key, sample, value| {
                actual.push((key, sample, value));
                Ok(())
            })
            .unwrap();
        assert_eq!(actual, expected);
        let mut actual = Vec::new();
        snapshot
            .observation_changes_at(&keys, &hours, |key, sample, value| {
                actual.push((key, sample, value));
                Ok(())
            })
            .unwrap();
        assert_eq!(actual, observed);
        assert!(
            snapshot
                .changes_at(&keys, &[1, 1], |_, _, _| Ok(()))
                .is_err()
        );
        for &key in &keys {
            for first in -9..=13 {
                for last in first..=13 {
                    let mut actual = Vec::new();
                    let mut cursor = None;
                    loop {
                        let page = snapshot
                            .run_page(
                                selection(key),
                                span(first, last),
                                cursor,
                                Run::SIZE,
                                |run| {
                                    actual.push(run);
                                    Ok(())
                                },
                            )
                            .unwrap();
                        cursor = page.next;
                        if cursor.is_none() {
                            break;
                        }
                    }
                    assert_eq!(actual, oracle.ranges(key, span(first, last)));
                }
            }
            let mut evidence = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot
                    .evidence_page(key, span(-8, 12), cursor, Evidence::SIZE, |value| {
                        evidence.push(value);
                        Ok(())
                    })
                    .unwrap();
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(evidence, oracle.keys[&key].evidence);
        }
        let mut rows = Vec::new();
        snapshot
            .runs_each(
                &keys.iter().map(|&key| selection(key)).collect::<Vec<_>>(),
                span(-8, 12),
                Run::SIZE,
                |key, row| {
                    rows.push((key, row.unwrap()));
                    Ok(())
                },
            )
            .unwrap();
        assert!(!rows.is_empty());
        let n = cuts.len() as u64;
        assert_eq!(snapshot.publication(), publication(n, n - 1, -8, 12, 16));
        let through = cuts[segment_of(&cuts, 0)];
        assert_eq!(snapshot.batch_through(0).unwrap(), Some(through));
        assert_eq!(snapshot.batch_through(13).unwrap(), None);
    }
}

#[test]
fn all_chart_grids_membership_display_ties_and_top_rank_queries_match_oracle() {
    for cuts in partitions() {
        let (memory, root, oracle, keys) = fixture(&cuts);
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let members = [span(-7, -4), span(0, 2), span(4, 11)];
        for within in [None, Some(&members[..]), Some(&[][..])] {
            for width in [1, 2, 7, 25, 100] {
                let selections: Vec<_> = keys
                    .iter()
                    .map(|&key| Selection {
                        key,
                        membership: within,
                    })
                    .collect();
                let view = Viewport {
                    from: -8.0,
                    to: 16.0,
                    width,
                };
                let expected = expected_tiles(&oracle, &selections, span(-8, 16), view);
                assert!(!expected.is_empty());
                assert_eq!(
                    all_tiles(&mut snapshot, &selections, span(-8, 16), view, 2),
                    expected,
                    "width {width} membership {within:?}"
                );
            }
        }
        let ranks = [(777, keys[2])];
        snapshot
            .rank_at(&ranks, 3, |actor, value| {
                assert_eq!(actor, 777);
                assert_eq!(value, oracle.value(keys[2], 3));
                Ok(())
            })
            .unwrap();
        let mut spans = Vec::new();
        snapshot
            .top_spans(selection(keys[2]), 1, span(-8, 12), |s| {
                spans.push(s);
                Ok(())
            })
            .unwrap();
        assert_eq!(spans, [span(1, 5), span(7, 12)]);
    }
}

#[test]
fn event_pages_storage_order_counts_latest_ordinals_clusters_and_as_of_match_oracle() {
    for cuts in [vec![12], every_hour()] {
        let (memory, root, oracle, keys) = fixture(&cuts);
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let members = [span(-8, -3), span(3, 12)];
        for within in [None, Some(&members[..]), Some(&[][..])] {
            for as_of in [None, Some(-8), Some(4), Some(9), Some(12)] {
                for order in [EventOrder::Native, EventOrder::Publication] {
                    let selections: Vec<_> = keys
                        .iter()
                        .map(|&key| Selection {
                            key,
                            membership: within,
                        })
                        .collect();
                    let query = EventQuery {
                        selection: &selections,
                        native: span(-8, 12),
                        as_of,
                        order,
                    };
                    let expected = oracle.events(query);
                    assert_eq!(snapshot.event_count(query).unwrap(), expected.len() as u64);
                    let mut cursor = None;
                    let mut actual = Vec::new();
                    loop {
                        let page = snapshot
                            .event_page(query, cursor, 2 * (Event::SIZE + 1), |value| {
                                actual.push(value);
                                Ok(())
                            })
                            .unwrap();
                        assert_eq!(page.total, Some(expected.len() as u64));
                        cursor = page.next;
                        if cursor.is_none() {
                            break;
                        }
                    }
                    assert_eq!(actual, expected);
                    for &s in &selections {
                        let one = [s];
                        let expected = oracle.events(EventQuery {
                            selection: &one,
                            order: EventOrder::Native,
                            ..query
                        });
                        for ordinal in 0..=expected.len() {
                            assert_eq!(
                                snapshot.event_raw(s, as_of, ordinal as u64).unwrap(),
                                expected.get(ordinal).copied()
                            );
                            assert_eq!(
                                snapshot.event_date(s, as_of, ordinal as u64).unwrap(),
                                expected.get(ordinal).map(|r| r.native_hour())
                            );
                        }
                        assert_eq!(
                            snapshot.latest_event(s, 12, as_of).unwrap(),
                            expected.last().copied()
                        );
                    }
                    snapshot
                        .latest_event_rows(&selections, 12, as_of, |at, row| {
                            let one = [selections[at]];
                            assert_eq!(
                                row,
                                oracle
                                    .events(EventQuery {
                                        selection: &one,
                                        order: EventOrder::Native,
                                        ..query
                                    })
                                    .last()
                                    .copied()
                            );
                            Ok(())
                        })
                        .unwrap();
                    for width in [7, 72, 199] {
                        let view = Viewport {
                            from: -8.0,
                            to: 12.0,
                            width,
                        };
                        let mut clusters = Vec::new();
                        let mut cursor = None;
                        loop {
                            let page = snapshot
                                .event_clusters(
                                    query,
                                    view,
                                    cursor,
                                    2 * (std::mem::size_of::<Cluster>()
                                        + std::mem::size_of::<(i32, u64, u64)>()),
                                    |c| {
                                        clusters.push(c);
                                        Ok(())
                                    },
                                )
                                .unwrap();
                            cursor = page.next;
                            if cursor.is_none() {
                                break;
                            }
                        }
                        assert_eq!(clusters, oracle.clusters(query, view));
                    }
                }
            }
        }
    }
}

#[test]
fn typed_dictionary_catalogue_checkpoint_lists_and_giant_cells_are_bounded() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let text = vec![b'x'; 3 * pages::PAGE_BYTES + 19];
    let blob = writer.blob(&mut &text[..], text.len() as u64).unwrap();
    let mut cells = vec![
        Cell::Null,
        Cell::Integer(i128::MIN),
        Cell::Unsigned(u64::MAX),
        Cell::Number(0.125),
        Cell::Boolean(true),
        Cell::Boolean(false),
        Cell::Text(blob),
    ];
    let child = writer.cells(cells.iter().copied()).unwrap();
    cells.push(Cell::List(child));
    let dimensions = writer.cells(cells.iter().copied()).unwrap();
    let a = writer
        .intern(
            KeySpec {
                dimensions,
                ..spec(2, 4)
            },
            blob,
            RunMode::Exact,
            5,
        )
        .unwrap();
    let dimensions2 = writer.cells(cells.iter().copied()).unwrap();
    let same = writer
        .intern(
            KeySpec {
                dimensions: dimensions2,
                ..spec(2, 4)
            },
            Blob::EMPTY,
            RunMode::Exact,
            5,
        )
        .unwrap();
    assert_eq!(same, a);
    let b = writer
        .intern(spec(1, 4), Blob::EMPTY, RunMode::Exact, 5)
        .unwrap();
    assert_eq!((a, b), (0, 1));
    writer.catalogue(9, Cell::List(dimensions)).unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 1, 1),
        child,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let mut buffer = [0; 4096];
    for offset in (0..text.len()).step_by(buffer.len()) {
        let count = buffer.len().min(text.len() - offset);
        snapshot
            .cell_bytes(blob, offset as u64, &mut buffer[..count])
            .unwrap();
        assert_eq!(&buffer[..count], &text[offset..offset + count]);
    }
    for (index, &cell) in cells.iter().enumerate() {
        assert_eq!(snapshot.field(dimensions, index as u64).unwrap(), cell);
    }
    assert_eq!(
        snapshot.field(snapshot.checkpoint(), 1).unwrap(),
        Cell::Integer(i128::MIN)
    );
    let mut keys = Vec::new();
    let first = snapshot
        .catalogue_page(None, None, KeyDefinition::SIZE, |key| {
            keys.push(key);
            Ok(())
        })
        .unwrap();
    assert!(first.next.is_some());
    snapshot
        .catalogue_page(None, first.next, KeyDefinition::SIZE, |key| {
            keys.push(key);
            Ok(())
        })
        .unwrap();
    assert_eq!(keys.iter().map(|k| k.id).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(keys[0].spec.dimensions, dimensions);
    assert_eq!(keys[0].label, blob);
    let mut meta = Vec::new();
    snapshot
        .metadata_page(None, 128, |id, value| {
            meta.push((id, value));
            Ok(())
        })
        .unwrap();
    assert_eq!(meta, [(9, Cell::List(dimensions))]);
    let mut filtered = Vec::new();
    snapshot
        .catalogue_page(Some((7, 4)), None, 1024, |key| {
            filtered.push(key.id);
            Ok(())
        })
        .unwrap();
    assert_eq!(filtered, [0, 1]);
    let mut bytes = [0; Cursor::SIZE];
    first
        .next
        .unwrap()
        .encode(&mut Encode::new(&mut bytes))
        .unwrap();
    assert_eq!(
        Cursor::decode(&mut Decode::new(&bytes)).unwrap(),
        first.next.unwrap()
    );
}

#[test]
fn shared_evidence_read_refreshes_identity_reuse_and_publication_prefixes_are_exact() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), -3).unwrap();
    let key = writer
        .intern(spec(22, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.bind_domain(key, 99).unwrap();
    let rows = [
        writer.domain(99, known(-3, 0, Some(-3), Some(22))).unwrap(),
        writer.domain(99, known(1, 3, Some(1), Some(22))).unwrap(),
    ];
    writer.run(key, span(-3, 3), 0, true, false).unwrap();
    for row in rows {
        let sequence = writer.sequence().unwrap();
        writer.present_domain(key, row, sequence).unwrap();
    }
    let first = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, -3, 3, 6),
        Blob::EMPTY,
    );
    drop(writer);
    let mut writer = Writer::resume(&mut memory, &cache, first, identity()).unwrap();
    writer.run(key, span(4, 6), 0, false, false).unwrap();
    writer.domain(99, known(4, 6, Some(4), Some(888))).unwrap();
    let reused = writer
        .intern(spec(888, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    assert_ne!(key, reused);
    assert!(writer.bind_domain(key, 98).is_err());
    let second = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(2, 1, -3, 6, 8),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory.clone(), &cache, second, identity()).unwrap();
    assert!(matches!(
        snapshot.value_at(key, 4).unwrap(),
        Observation::Value {
            run: Run {
                span: Span { first: -3, last: 6 },
                ..
            },
            evidence: Evidence {
                holder: Some(888),
                ..
            }
        }
    ));
    let mut prefix = Snapshot::open(memory, &cache, second, identity())
        .unwrap()
        .through(4)
        .unwrap()
        .unwrap();
    assert_eq!(prefix.publication().generation, 1);
    assert_eq!(prefix.value_at(key, 4).unwrap(), Observation::Pending);
    assert_eq!(
        prefix.value_at(reused, 1).unwrap(),
        Observation::Unknown(None)
    );
    assert!(matches!(
        prefix.value_at(key, -3).unwrap(),
        Observation::Value {
            run: Run {
                span: Span { first: -3, last: 3 },
                ..
            },
            ..
        }
    ));
    let selections = [selection(key)];
    assert_eq!(
        prefix
            .event_count(EventQuery {
                selection: &selections,
                native: span(-3, 3),
                as_of: None,
                order: EventOrder::Native
            })
            .unwrap(),
        2
    );
}

/// The current reader's `next_change` over one batch's rows in insertion order;
/// `None` where it refuses with a missing preceding run.
fn reader_next_change(
    runs: &[Run],
    accepted: Span,
    hour: f64,
    forward: bool,
) -> Option<Option<i32>> {
    let value_at = |hour: i32| {
        if !accepted.contains(hour) {
            return None;
        }
        let index = runs.partition_point(|r| r.span.first <= hour);
        index
            .checked_sub(1)
            .map(|i| runs[i])
            .filter(|r| r.span.last >= hour)
    };
    if forward {
        if let Some(run) = value_at(hour.floor() as i32) {
            return Some(run.span.last.checked_add(1));
        }
        if f64::from(accepted.last) <= hour {
            return Some(None);
        }
        return Some(
            runs.get(runs.partition_point(|r| f64::from(r.span.first) <= hour))
                .map(|r| r.span.first),
        );
    }
    if f64::from(accepted.first) >= hour {
        return Some(None);
    }
    let index = runs.partition_point(|r| f64::from(r.span.first) < hour);
    let Some(previous) = index.checked_sub(1) else {
        return Some(None);
    };
    let run = value_at(runs[previous].span.last)?;
    Some(Some(
        run.span
            .last
            .checked_add(1)
            .filter(|last| f64::from(*last) < hour)
            .unwrap_or(run.span.first),
    ))
}

#[test]
fn insertion_order_overlaps_and_backdated_endpoints_keep_ordered_contributions() {
    for inputs in [
        vec![(8, 12, 19), (-3, 2, 7)],
        vec![(-5, 8, 19), (3, 10, 7)],
        vec![
            (8, 12, 19),
            (-3, 2, 7),
            (0, 9, 4),
            (5, 5, 1),
            (-5, -4, 2),
            (11, 12, 6),
        ],
        vec![(4, 6, 1), (-5, 12, 2), (0, 1, 3), (7, 9, 4), (2, 2, 5)],
    ] {
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut memory = Memory::default();
        let mut writer = Writer::new(&budget, identity(), -5).unwrap();
        let key = writer
            .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
            .unwrap();
        let mut oracle = Oracle {
            accepted: Some(span(-5, 12)),
            ..Oracle::default()
        };
        let mut logical = Logical::default();
        logical.evidence.push(known(-5, 12, None, None));
        writer.evidence(key, logical.evidence[0], None).unwrap();
        for (sequence, (first, last, value)) in inputs.into_iter().enumerate() {
            writer
                .run(key, span(first, last), value, true, false)
                .unwrap();
            logical.runs.push(Run {
                span: span(first, last),
                value,
                break_before: true,
                supplying_boundary: false,
                canonical: sequence as u64,
                sequence: sequence as u64,
            });
        }
        oracle.keys.insert(key, logical);
        let root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(1, 0, -5, 12, 12),
            Blob::EMPTY,
        );
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let selections = [selection(key)];
        for width in [1, 5, 18] {
            let view = Viewport {
                from: -5.0,
                to: 12.0,
                width,
            };
            assert_eq!(
                all_tiles(&mut snapshot, &selections, span(-5, 12), view, 4),
                expected_tiles(&oracle, &selections, span(-5, 12), view)
            );
        }
        for hour in -6..=13 {
            assert_eq!(
                snapshot.value_at(key, hour).unwrap(),
                oracle.value(key, hour),
                "hour {hour}"
            );
        }
        let runs = &oracle.keys[&key].runs;
        for half in -16..=30 {
            let hour = f64::from(half) / 2.0;
            for forward in [true, false] {
                let expected = reader_next_change(runs, span(-5, 12), hour, forward);
                let actual =
                    snapshot.next_change(selection(key), hour, forward, span(i32::MIN, i32::MAX));
                match expected {
                    Some(expected) => {
                        assert_eq!(actual.unwrap(), expected, "hour {hour} forward {forward}")
                    }
                    None => assert!(actual.is_err(), "hour {hour} forward {forward}"),
                }
            }
        }
    }
}

#[test]
fn backdated_contributions_navigate_as_the_reader_partition_point_does() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), -5).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    writer
        .evidence(key, known(-5, 12, None, None), None)
        .unwrap();
    writer.run(key, span(8, 12), 19, true, false).unwrap();
    writer.run(key, span(-3, 2), 7, true, false).unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, -5, 12, 12),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let all = span(i32::MIN, i32::MAX);
    assert_eq!(
        snapshot.next_change(selection(key), 5, false, all).unwrap(),
        Some(3)
    );
    assert_eq!(
        snapshot.next_change(selection(key), 5, true, all).unwrap(),
        None
    );
    assert_eq!(
        snapshot.next_change(selection(key), -4, true, all).unwrap(),
        Some(8)
    );
}

#[test]
fn partition_point_probes_rows_as_the_standard_search_does() {
    for len in 0..=12_u64 {
        for pattern in 0..1_u64 << len {
            let rows: Vec<u64> = (0..len).collect();
            let expected = rows.partition_point(|&at| pattern >> at & 1 != 0) as u64;
            let actual =
                crate::source::partition_point(len, |at| Ok(pattern >> at & 1 != 0)).unwrap();
            assert_eq!(actual, expected, "len {len} pattern {pattern:b}");
        }
    }
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    for len in [33_u64, 100, 1000, 4097] {
        let bits: Vec<bool> = (0..len)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                seed >> 63 != 0
            })
            .collect();
        let expected = bits.partition_point(|bit| *bit) as u64;
        let actual = crate::source::partition_point(len, |at| Ok(bits[at as usize])).unwrap();
        assert_eq!(actual, expected, "len {len}");
    }
}

#[test]
fn fifty_year_and_full_hour_domain_constants_cost_runs_and_exact_bytes() {
    let mut work = Vec::new();
    for extent in [span(0, 0), span(0, 438_000), span(i32::MIN, i32::MAX)] {
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut memory = Memory::default();
        let mut writer = Writer::new(&budget, identity(), extent.first).unwrap();
        let key = writer
            .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
            .unwrap();
        writer.run(key, extent, i128::MIN, true, false).unwrap();
        writer
            .evidence(key, known(extent.first, extent.last, None, None), None)
            .unwrap();
        let root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(1, 0, extent.first, extent.last, extent.last),
            Blob::EMPTY,
        );
        work.push(memory.pages.len());
        memory.reads = 0;
        memory.bytes = 0;
        let reader = Cache::new(&budget);
        let mut snapshot = Snapshot::open(memory, &reader, root, identity()).unwrap();
        let mut count = 0;
        snapshot
            .summary_tiles(
                &[selection(key)],
                extent,
                Viewport {
                    from: f64::from(extent.first),
                    to: f64::from(extent.last),
                    width: 1,
                },
                Tail::EMPTY,
                None,
                4096,
                |_, bucket| {
                    count += 1;
                    let b = bucket.exact.unwrap();
                    assert_eq!(b.covered, extent.hours());
                    let total = 1_u128 << 127;
                    let low = (total as u64 as u128) * u128::from(extent.hours());
                    let high = (total >> 64) * u128::from(extent.hours()) + (low >> 64);
                    assert_eq!(
                        b.absolute_total,
                        U256([low as u64, high as u64, (high >> 64) as u64, 0])
                    );
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(count, 1);
        let memory = snapshot.into_io();
        assert!(memory.reads < 100);
    }
    assert!(work.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn rank_builder_batches_ties_unknowns_extremes_and_displaced_actors_match_sort_oracle() {
    use crate::rank::{Actor, Amount, RankBuilder};
    let budget = Budget::new(BIG);
    let actors = [
        Actor {
            identity: 1,
            tag: "ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦",
        },
        Actor {
            identity: 2,
            tag: "AAA",
        },
        Actor {
            identity: 3,
            tag: "ZZZ",
        },
        Actor {
            identity: 4,
            tag: "BBB",
        },
    ];
    for descending in [false, true] {
        for include_unknown in [false, true] {
            let mut ranks =
                RankBuilder::new(&budget, &actors, descending, include_unknown).unwrap();
            let mut amounts = vec![Amount::Unknown; actors.len()];
            let mut forward = BTreeMap::new();
            let mut inverse = BTreeMap::new();
            for (hour, changes) in [
                vec![(0, Amount::Known(i128::MIN)), (2, Amount::Known(i128::MAX))],
                vec![(1, Amount::Known(3)), (3, Amount::Known(3))],
                vec![(0, Amount::Known(i128::MAX)), (2, Amount::Unknown)],
                vec![(1, Amount::Excluded), (2, Amount::Known(-4))],
                vec![(1, Amount::Unknown)],
            ]
            .into_iter()
            .enumerate()
            {
                for &(handle, amount) in &changes {
                    amounts[handle] = amount;
                }
                ranks
                    .changes(
                        hour as i32,
                        &changes,
                        |position, actor| {
                            if let Some(actor) = actor {
                                forward.insert(position, actor);
                            } else {
                                forward.remove(&position);
                            }
                            Ok(())
                        },
                        |actor, position| {
                            if let Some(position) = position {
                                inverse.insert(actor, position);
                            } else {
                                inverse.remove(&actor);
                            }
                            Ok(())
                        },
                    )
                    .unwrap();
                let mut expected: Vec<_> = (0..actors.len())
                    .filter(|&i| {
                        matches!(amounts[i], Amount::Known(_))
                            || (include_unknown && amounts[i] == Amount::Unknown)
                    })
                    .collect();
                expected.sort_by(|&a, &b| {
                    let amount = match (amounts[a], amounts[b]) {
                        (Amount::Known(a), Amount::Known(b)) => {
                            if descending {
                                b.cmp(&a)
                            } else {
                                a.cmp(&b)
                            }
                        }
                        (Amount::Known(_), _) => std::cmp::Ordering::Less,
                        (_, Amount::Known(_)) => std::cmp::Ordering::Greater,
                        _ => std::cmp::Ordering::Equal,
                    };
                    amount
                        .then_with(|| actors[a].tag.as_bytes().cmp(actors[b].tag.as_bytes()))
                        .then(a.cmp(&b))
                });
                assert_eq!(
                    forward
                        .iter()
                        .map(|(&rank, &actor)| (rank, actor))
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .enumerate()
                        .map(|(rank, &i)| (rank, actors[i].identity))
                        .collect::<Vec<_>>()
                );
                for i in 0..actors.len() {
                    assert_eq!(
                        ranks.position(i).unwrap(),
                        expected.iter().position(|&e| e == i)
                    );
                    assert_eq!(
                        inverse.get(&actors[i].identity).copied(),
                        expected.iter().position(|&e| e == i)
                    );
                }
            }
        }
    }
}

#[test]
fn damaged_pages_bad_lengths_wrong_roots_and_admission_are_errors() {
    let (memory, root, _, keys) = fixture(&[-3, 4, 12]);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    for offset in [0, 8, 10, 12, 16, 20, 24, 55, 56] {
        let mut broken = memory.clone();
        broken.pages[root as usize][offset] ^= 0x80;
        assert!(Snapshot::open(broken, &cache, root, identity()).is_err());
    }
    let mut broken = memory.clone();
    broken.pages[root as usize].pop();
    assert!(Snapshot::open(broken, &cache, root, identity()).is_err());
    let small = Budget::new(1);
    assert!(Snapshot::open(memory.clone(), &Cache::new(&small), root, identity()).is_err());
    let mut wrong = identity();
    wrong.branch[0] ^= 1;
    assert!(Snapshot::open(memory.clone(), &cache, root, wrong).is_err());
    let mut damaged = memory;
    for page in &mut damaged.pages {
        if page.len() > 60 && u16::from_le_bytes([page[10], page[11]]) == Kind::Rows as u16 {
            page[57] ^= 1;
        }
    }
    let fresh = Cache::new(&budget);
    let mut snapshot = Snapshot::open(damaged, &fresh, root, identity()).unwrap();
    assert!(snapshot.value_at(keys[0], -8).is_err());
    drop(snapshot);
    drop(fresh);
    drop(cache);
    assert_eq!(budget.used(), 0);
}

#[test]
fn failed_seal_retries_and_cursors_bind_every_query_operand() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.run(key, span(0, 3), 7, true, false).unwrap();
    writer
        .evidence(key, known(0, 3, Some(0), Some(1)), None)
        .unwrap();
    let pending = writer.cut(publication(1, 0, 0, 3, 6), Blob::EMPTY).unwrap();
    memory.fail = Some(memory.pages.len() + 2);
    assert!(pending.seal(&mut memory).is_err());
    memory.fail = None;
    let sealed = pending.seal(&mut memory).unwrap();
    drop(pending);
    let root = writer.publish(&mut memory, &cache, sealed).unwrap();
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let selections = [selection(key)];
    let view = Viewport {
        from: 0.0,
        to: 6.0,
        width: 7,
    };
    let page = snapshot
        .summary_tiles(
            &selections,
            span(0, 6),
            view,
            Tail::EMPTY,
            None,
            std::mem::size_of::<Bucket>(),
            |_, _| Ok(()),
        )
        .unwrap();
    let cursor = page.page.next.unwrap();
    assert!(
        snapshot
            .summary_tiles(
                &selections,
                span(0, 6),
                Viewport { width: 8, ..view },
                Tail::EMPTY,
                Some(cursor),
                4096,
                |_, _| Ok(())
            )
            .is_err()
    );
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let mut snapshot = snapshot.with_cancellation(&cancelled);
    assert_eq!(
        snapshot.value_at(key, 0).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn fifteen_hundred_keys_without_evidence_read_unknown_and_have_no_cap() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let mut keys = Vec::new();
    for country in 0..1500 {
        keys.push(
            writer
                .intern(spec(country, 1), Blob::EMPTY, RunMode::Exact, 0)
                .unwrap(),
        );
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 1, 1),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let mut visited = 0;
    snapshot
        .values_at(&keys, 0, |_, value| {
            assert_eq!(value.unwrap(), Observation::Unknown(None));
            visited += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(visited, 1500);
}

#[test]
fn navigation_home_end_membership_unknown_stops_and_reached_fallback_match_boundaries() {
    for cuts in partitions() {
        let (memory, root, oracle, keys) = fixture(&cuts);
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let members = [span(-6, -3), span(2, 11)];
        for membership in [None, Some(&members[..]), Some(&[][..])] {
            let selections: Vec<_> = keys
                .iter()
                .map(|&key| Selection { key, membership })
                .collect();
            for origin in -10..=18 {
                for direction in [
                    Direction::Forward,
                    Direction::Backward,
                    Direction::Home,
                    Direction::End,
                ] {
                    let expected = oracle.navigation(
                        &selections,
                        Some(f64::from(origin)),
                        direction,
                        span(-8, 16),
                    );
                    assert_eq!(
                        snapshot
                            .navigate(&selections, Some(origin), direction, span(-8, 16))
                            .unwrap()
                            .map(f64::from),
                        expected
                    );
                }
            }
        }
        assert_eq!(
            snapshot
                .next_change(selection(keys[0]), 16, true, span(-8, 16))
                .unwrap(),
            None
        );
    }
}

#[test]
fn progressive_exact_and_approximate_tails_keep_pending_and_refuse_false_authentication() {
    use crate::query::TailRun;
    let (memory, root, oracle, keys) = fixture(&[12]);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let tail = [TailRun {
        run: Run {
            span: span(13, 16),
            value: 5,
            break_before: false,
            supplying_boundary: false,
            canonical: 100,
            sequence: 100,
        },
        evidence: known(13, 16, Some(13), Some(100)),
    }];
    let approximation = [Approximation {
        span: span(17, 20),
        value: 8.5,
    }];
    let view = Tail {
        exact: &tail,
        approximate: &approximation,
    };
    assert!(matches!(
        snapshot.value_with_tail(keys[0], 13, view).unwrap(),
        Observation::Value {
            run: Run { value: 5, .. },
            ..
        }
    ));
    assert_eq!(
        snapshot.value_with_tail(keys[0], 17, view).unwrap(),
        Observation::Pending
    );
    assert_eq!(
        snapshot.value_with_tail(keys[0], 2, view).unwrap(),
        oracle.value(keys[0], 2)
    );
    let mut approximate = Vec::new();
    snapshot
        .approximate_spans(view, span(15, 18), |a| {
            approximate.push(a);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        approximate,
        [Approximation {
            span: span(17, 18),
            value: 8.5
        }]
    );
    let mut got_tail = false;
    snapshot
        .summary_tiles(
            &[selection(keys[0])],
            span(13, 20),
            Viewport {
                from: 13.0,
                to: 20.0,
                width: 8,
            },
            view,
            None,
            4096,
            |_, bucket| {
                if bucket.span.first <= 16 {
                    assert_eq!(bucket.exact.unwrap().first_value, 5.0);
                    got_tail = true;
                } else {
                    assert_eq!(bucket.exact, None);
                    assert!(bucket.pending.is_some());
                }
                Ok(())
            },
        )
        .unwrap();
    assert!(got_tail);
    let bad = [TailRun {
        evidence: Evidence {
            known: Knownness::Unknown(8),
            ..tail[0].evidence
        },
        ..tail[0]
    }];
    assert!(
        snapshot
            .value_with_tail(
                keys[0],
                13,
                Tail {
                    exact: &bad,
                    approximate: &[]
                }
            )
            .is_err()
    );
}

#[test]
fn codecs_enforce_declared_widths_and_late_cursor_generations_cannot_mix() {
    fn round_trip<T: Binary + PartialEq + std::fmt::Debug>(value: T) {
        let mut bytes = vec![0; T::SIZE];
        let mut encoder = Encode::new(&mut bytes);
        value.encode(&mut encoder).unwrap();
        assert_eq!(encoder.len(), T::SIZE);
        let mut decoder = Decode::new(&bytes);
        assert_eq!(T::decode(&mut decoder).unwrap(), value);
        decoder.end().unwrap();
    }
    round_trip(known(-3, 5, Some(i32::MIN), Some(u64::MAX)));
    round_trip(Summary::run(
        Run {
            span: span(i32::MIN, i32::MAX),
            value: i128::MIN,
            break_before: true,
            supplying_boundary: true,
            canonical: u64::MAX,
            sequence: u64::MAX,
        },
        span(i32::MIN, i32::MAX),
        0,
    ));
    round_trip(Cell::Text(Blob { at: 7, len: 9 }));
    round_trip(crate::format::Late(EventView::Evidence {
        key: 3,
        sequence: u64::MAX,
        value: known(i32::MIN, i32::MAX, None, Some(1)),
    }));
    round_trip(crate::format::NodeRef {
        page: 9,
        level: 2,
        first: 256,
        segments: 256,
        hours: span(-5, 5),
    });
    let (memory, root, _, keys) = fixture(&[-3, 4, 12]);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let selections = [selection(keys[0])];
    let query = EventQuery {
        selection: &selections,
        native: span(-8, 12),
        as_of: None,
        order: EventOrder::Native,
    };
    let mut snapshot = Snapshot::open(memory.clone(), &cache, root, identity()).unwrap();
    let cursor = snapshot
        .event_page(query, None, Event::SIZE + 1, |_| Ok(()))
        .unwrap()
        .next
        .unwrap();
    assert_eq!(
        snapshot
            .event_page(
                EventQuery {
                    as_of: Some(12),
                    ..query
                },
                Some(cursor),
                4096,
                |_| Ok(())
            )
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    drop(snapshot);
    let mut memory = memory;
    let mut writer = Writer::resume(&mut memory, &cache, root, identity()).unwrap();
    let sequence = writer.sequence().unwrap();
    writer
        .event(Event {
            key: keys[0],
            native_hour: -7,
            read_hour: Some(13),
            sequence,
            kind: 1,
            fields: Blob::EMPTY,
        })
        .unwrap();
    let next = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(4, 3, -8, 13, 16),
        Blob::EMPTY,
    );
    let mut newer = Snapshot::open(memory, &cache, next, identity()).unwrap();
    assert_eq!(
        newer
            .event_page(query, Some(cursor), 4096, |_| Ok(()))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        newer
            .latest_event(selection(keys[0]), i32::MAX, Some(i32::MAX))
            .unwrap()
            .unwrap()
            .native_hour(),
        7
    );
    let late = [selection(keys[0])];
    let mut rows = Vec::new();
    newer
        .event_page(
            EventQuery {
                selection: &late,
                native: span(-7, -7),
                as_of: Some(13),
                order: EventOrder::Native,
            },
            None,
            4096,
            |row| {
                rows.push(row.sequence());
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(rows, [sequence]);
}

#[test]
fn seeded_sparse_runs_have_partition_independent_points_ranges_tiles_and_stops() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for case in 0..16 {
        let mut runs = Vec::new();
        let mut hour = -40;
        for _ in 0..24 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let length = (seed % 3) as i32 + 1;
            let value = match seed % 7 {
                0 => 0,
                1 => i128::MIN,
                2 => i128::MAX,
                _ => (seed as i64) as i128,
            };
            runs.push((hour, hour + length - 1, value, true, seed & 8 != 0));
            hour += length + (seed % 2) as i32;
        }
        let end = hour;
        let step = (case % 5) as i32 + 1;
        let mut cuts: Vec<i32> = (-40..end).filter(|h| (h + 40) % step == step - 1).collect();
        cuts.push(end);
        let (parts, logical) = split(&runs, &cuts, -40);
        let budget = Budget::new(BIG);
        let cache = Cache::new(&budget);
        let mut memory = Memory::default();
        let mut writer = Writer::new(&budget, identity(), -40).unwrap();
        let key = writer
            .intern(spec(case, 1), Blob::EMPTY, RunMode::Exact, 5)
            .unwrap();
        let evidence = known(-40, end, None, Some(case));
        let mut root = 0;
        let mut first = -40;
        for (segment, &last) in cuts.iter().enumerate() {
            for &(part_segment, part, value, broken, supplying) in &parts {
                if part_segment == segment {
                    writer.run(key, part, value, broken, supplying).unwrap();
                }
            }
            writer
                .evidence(
                    key,
                    Evidence {
                        span: span(first, last),
                        ..evidence
                    },
                    None,
                )
                .unwrap();
            root = publish(
                &mut writer,
                &mut memory,
                &cache,
                publication(segment as u64 + 1, segment as u64, -40, last, last + 1),
                Blob::EMPTY,
            );
            first = last + 1;
        }
        drop(writer);
        let mut oracle = Oracle {
            accepted: Some(span(-40, end)),
            ..Oracle::default()
        };
        oracle.keys.insert(
            key,
            Logical {
                runs: logical,
                evidence: vec![evidence],
                decimals: 5,
                ..Logical::default()
            },
        );
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        for h in -41..=end + 1 {
            assert_eq!(snapshot.value_at(key, h).unwrap(), oracle.value(key, h));
        }
        for (first, last) in [(-40, end), (-33, -20), (-12, end - 3)] {
            let mut actual = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot
                    .run_page(
                        selection(key),
                        span(first, last),
                        cursor,
                        2 * Run::SIZE,
                        |run| {
                            actual.push(run);
                            Ok(())
                        },
                    )
                    .unwrap();
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(actual, oracle.ranges(key, span(first, last)));
        }
        for width in [9, end as u32 + 41] {
            let view = Viewport {
                from: -39.5,
                to: f64::from(end) - 0.25,
                width,
            };
            let selections = [selection(key)];
            let expected = expected_tiles(&oracle, &selections, span(-40, end), view);
            assert!(!expected.is_empty());
            assert_eq!(
                all_tiles(&mut snapshot, &selections, span(-40, end), view, 3),
                expected
            );
        }
        let selections = [selection(key)];
        for origin in -42..=end + 2 {
            for direction in [Direction::Forward, Direction::Backward] {
                assert_eq!(
                    snapshot
                        .navigate(&selections, Some(origin), direction, span(-40, end))
                        .unwrap()
                        .map(f64::from),
                    oracle.navigation(
                        &selections,
                        Some(f64::from(origin)),
                        direction,
                        span(-40, end)
                    )
                );
            }
        }
    }
}

#[test]
fn aliases_and_publication_iteration_have_bounded_query_shapes() {
    use crate::query::Alias;
    let (memory, root, oracle, keys) = fixture(&[-3, 4, 12]);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let aliases = [
        Alias {
            key: keys[0],
            span: span(-8, -3),
            boundary: None,
        },
        Alias {
            key: keys[1],
            span: span(-2, 12),
            boundary: None,
        },
    ];
    let mut cursor = None;
    loop {
        let page = snapshot
            .summary_aliases(
                &aliases,
                span(-8, 12),
                Viewport {
                    from: -8.0,
                    to: 12.0,
                    width: 100,
                },
                cursor,
                2 * std::mem::size_of::<Bucket>(),
                |_, bucket| {
                    let alias = aliases
                        .iter()
                        .find(|a| a.span.contains(bucket.span.first))
                        .unwrap();
                    assert_eq!(
                        bucket.exact,
                        oracle.bucket(
                            alias.key,
                            bucket.span,
                            Some(std::slice::from_ref(&alias.span))
                        )
                    );
                    Ok(())
                },
            )
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    let selections: Vec<_> = keys.iter().map(|&key| selection(key)).collect();
    let query = EventQuery {
        selection: &selections,
        native: span(-8, 12),
        as_of: None,
        order: EventOrder::Native,
    };
    let mut rows = Vec::new();
    snapshot
        .each_event(query, 2 * (Event::SIZE + 1), |row| {
            rows.push(row);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        rows,
        oracle.events(EventQuery {
            order: EventOrder::Publication,
            ..query
        })
    );
    let one = [selection(keys[0])];
    assert_eq!(
        snapshot.event_count_through(keys[0], 4, None).unwrap(),
        oracle
            .events(EventQuery {
                selection: &one,
                native: span(i32::MIN, 4),
                as_of: None,
                order: EventOrder::Native
            })
            .len() as u64
    );
}

#[test]
fn simultaneous_rank_swaps_skip_the_unchanged_middle() {
    use crate::rank::{Actor, Amount, RankBuilder};
    let budget = Budget::new(BIG);
    let tags: Vec<_> = (0..1500).map(|i| format!("tag-{i:04}")).collect();
    let actors: Vec<_> = tags
        .iter()
        .enumerate()
        .map(|(i, tag)| Actor {
            identity: i as u64,
            tag,
        })
        .collect();
    let mut ranks = RankBuilder::new(&budget, &actors, false, false).unwrap();
    let changes: Vec<_> = (0..1500).map(|i| (i, Amount::Known(i as i128))).collect();
    ranks
        .changes(-1, &changes, |_, _| Ok(()), |_, _| Ok(()))
        .unwrap();
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    ranks
        .changes(
            0,
            &[(0, Amount::Known(1499)), (1499, Amount::Known(0))],
            |rank, actor| {
                forward.push((rank, actor));
                Ok(())
            },
            |actor, rank| {
                inverse.push((actor, rank));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(forward, [(0, Some(1499)), (1499, Some(0))]);
    assert_eq!(inverse, [(1499, Some(0)), (0, Some(1499))]);
    assert!(ranks.visited() < 100);
}

#[test]
fn hourly_overlaps_match_points_and_coarse_edges_match_the_current_reader() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), -16).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    writer.run(key, span(-5, 8), 19, true, false).unwrap();
    writer.run(key, span(3, 10), 7, false, false).unwrap();
    let evidence = known(-16, 31, None, None);
    writer.evidence(key, evidence, None).unwrap();
    let logical = Logical {
        runs: vec![
            Run {
                span: span(-5, 8),
                value: 19,
                break_before: true,
                supplying_boundary: false,
                canonical: 0,
                sequence: 0,
            },
            Run {
                span: span(3, 10),
                value: 7,
                break_before: false,
                supplying_boundary: false,
                canonical: 1,
                sequence: 1,
            },
        ],
        evidence: vec![evidence],
        ..Logical::default()
    };
    let oracle = Oracle {
        keys: BTreeMap::from([(key, logical)]),
        accepted: Some(span(-16, 31)),
    };
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, -16, 31, 31),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    for view in [
        Viewport {
            from: -5.0,
            to: 10.0,
            width: 16,
        },
        Viewport {
            from: -5.0,
            to: 10.0,
            width: 1,
        },
        Viewport {
            from: -16.0,
            to: 15.0,
            width: 1,
        },
    ] {
        snapshot
            .summary_tiles(
                &[selection(key)],
                span(-16, 31),
                view,
                Tail::EMPTY,
                None,
                16384,
                |_, bucket| {
                    assert_eq!(bucket.exact, oracle.bucket(key, bucket.span, None));
                    if bucket.span == span(4, 4) {
                        let value = bucket.exact.unwrap();
                        assert_eq!((value.min_value, value.max_value), (7.0, 7.0));
                        assert_eq!(value.absolute_total, U256([7, 0, 0, 0]));
                        assert_eq!(value.covered, 1);
                    }
                    if bucket.span == span(-16, 15) {
                        assert_eq!(bucket.exact.unwrap().absolute_total, U256([322, 0, 0, 0]));
                    }
                    Ok(())
                },
            )
            .unwrap();
    }
}

#[test]
fn repeating_overlap_blocks_preserve_last_origin_without_expanding_hours() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    let last = 3 * 65536 - 1;
    writer.run(key, span(0, last), 19, true, false).unwrap();
    writer.run(key, span(0, last), 7, false, false).unwrap();
    writer
        .evidence(key, known(0, last, None, None), None)
        .unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, last, last),
        Blob::EMPTY,
    );
    memory.reads = 0;
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    snapshot
        .summary_tiles(
            &[selection(key)],
            span(0, last),
            Viewport {
                from: 0.0,
                to: f64::from(last),
                width: 1,
            },
            Tail::EMPTY,
            None,
            4096,
            |_, bucket| {
                let b = bucket.exact.unwrap();
                assert_eq!(b.absolute_total, U256([26 * 3 * 65536, 0, 0, 0]));
                assert_eq!(b.covered, 2 * 3 * 65536);
                assert_eq!(b.last_value_hour, 2 * 65536);
                assert!(b.has_break);
                Ok(())
            },
        )
        .unwrap();
    assert!(snapshot.into_io().reads < 100);
}

#[test]
fn alias_boundaries_can_suppress_a_stored_cut_or_force_one_inside_a_run() {
    use crate::query::Alias;
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let mut keys = Vec::new();
    for (actor, extent, broken) in [
        (1, span(0, 15), true),
        (2, span(16, 31), true),
        (3, span(0, 31), false),
    ] {
        let key = writer
            .intern(spec(actor, 1), Blob::EMPTY, RunMode::Exact, 0)
            .unwrap();
        writer.run(key, extent, 5, broken, false).unwrap();
        writer
            .evidence(key, known(0, 31, None, Some(actor)), None)
            .unwrap();
        keys.push(key);
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 31, 31),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let joined = [
        Alias {
            key: keys[0],
            span: span(0, 15),
            boundary: None,
        },
        Alias {
            key: keys[1],
            span: span(16, 31),
            boundary: Some(false),
        },
    ];
    let cut = [
        Alias {
            key: keys[2],
            span: span(0, 15),
            boundary: None,
        },
        Alias {
            key: keys[2],
            span: span(16, 31),
            boundary: Some(true),
        },
    ];
    for (aliases, broken, origin) in [(&joined[..], false, 0), (&cut[..], true, 16)] {
        let mut cursor = None;
        let mut runs = Vec::new();
        loop {
            let page = snapshot
                .hourly_alias_runs(
                    aliases,
                    span(0, 31),
                    Viewport {
                        from: 0.0,
                        to: 31.0,
                        width: 32,
                    },
                    cursor,
                    Run::SIZE,
                    |run| {
                        runs.push(run);
                        Ok(())
                    },
                )
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].span, span(0, 15));
        assert_eq!(runs[1].span, span(16, 31));
        assert_eq!(runs[1].break_before, broken);
        snapshot
            .summary_aliases(
                aliases,
                span(0, 31),
                Viewport {
                    from: 0.0,
                    to: 31.0,
                    width: 1,
                },
                None,
                4096,
                |_, bucket| {
                    let b = bucket.exact.unwrap();
                    assert_eq!(b.has_break, broken);
                    assert_eq!(b.last_value_hour, origin);
                    assert_eq!(b.absolute_total, U256([160, 0, 0, 0]));
                    Ok(())
                },
            )
            .unwrap();
        let mut cursor = None;
        loop {
            let page = snapshot
                .summary_aliases(
                    aliases,
                    span(0, 31),
                    Viewport {
                        from: 0.0,
                        to: 31.0,
                        width: 32,
                    },
                    cursor,
                    2 * std::mem::size_of::<Bucket>(),
                    |_, bucket| {
                        if bucket.span.first == 16 {
                            assert_eq!(bucket.exact.unwrap().first_break, broken);
                        }
                        Ok(())
                    },
                )
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
    }
    let page = snapshot
        .summary_aliases(
            &joined,
            span(0, 31),
            Viewport {
                from: 0.0,
                to: 31.0,
                width: 32,
            },
            None,
            std::mem::size_of::<Bucket>(),
            |_, _| Ok(()),
        )
        .unwrap();
    let mut changed = joined;
    changed[1].boundary = Some(true);
    assert_eq!(
        snapshot
            .summary_aliases(
                &changed,
                span(0, 31),
                Viewport {
                    from: 0.0,
                    to: 31.0,
                    width: 32
                },
                page.next,
                4096,
                |_, _| Ok(())
            )
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn a_fifty_year_chart_reads_coalesced_coverage_without_hourly_evidence() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let last = 438_000 - 1;
    writer.run(key, span(0, last), 5, true, false).unwrap();
    for hour in 0..=last {
        writer
            .evidence(
                key,
                Evidence {
                    identity_first: Some(0),
                    ..known(hour, hour, Some(hour), Some(1))
                },
                None,
            )
            .unwrap();
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, last, last),
        Blob::EMPTY,
    );
    memory.reads = 0;
    memory.kinds = [0; 8];
    let cold = Cache::new(&budget);
    let mut snapshot = Snapshot::open(memory, &cold, root, identity()).unwrap();
    let mut bins = 0;
    snapshot
        .summary_tiles(
            &[selection(key)],
            span(0, last),
            Viewport {
                from: 0.0,
                to: f64::from(last),
                width: 1200,
            },
            Tail::EMPTY,
            None,
            1_000_000,
            |_, bucket| {
                bins += 1;
                let value = bucket.exact.unwrap();
                assert_eq!(
                    value.absolute_total,
                    U256([5 * bucket.span.hours(), 0, 0, 0])
                );
                assert!(!value.has_break);
                Ok(())
            },
        )
        .unwrap();
    let memory = snapshot.into_io();
    assert_eq!(bins, 1200);
    assert_eq!(memory.kinds[Kind::Rows as usize], 0);
    assert!(memory.reads < 50);
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let mut coverage = Vec::new();
    snapshot
        .coverage_page(key, span(0, last), None, Span::SIZE, |s| {
            coverage.push(s);
            Ok(())
        })
        .unwrap();
    assert_eq!(coverage, [span(0, last)]);
    assert!(matches!(
        snapshot.value_at(key, 200_000).unwrap(),
        Observation::Value {
            evidence: Evidence {
                supplying_read: Some(200_000),
                holder: Some(1),
                ..
            },
            ..
        }
    ));
    let mut detailed = Vec::new();
    snapshot
        .evidence_page(key, span(last - 2, last), None, 3 * Evidence::SIZE, |e| {
            detailed.push(e);
            Ok(())
        })
        .unwrap();
    assert_eq!(detailed.len(), 3);
    for (index, e) in detailed.iter().enumerate() {
        assert_eq!(e.supplying_read, Some(last - 2 + index as i32));
    }
}

#[test]
fn shared_coverage_survives_resume_and_prefixes_keep_their_own_roots() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.bind_domain(key, 9).unwrap();
    writer.run(key, span(0, 1023), 5, true, false).unwrap();
    for h in 0..1024 {
        writer
            .domain(
                9,
                Evidence {
                    identity_first: Some(0),
                    ..known(h, h, Some(h), Some(1))
                },
            )
            .unwrap();
    }
    let first = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 1023, 2047),
        Blob::EMPTY,
    );
    let mut writer = Writer::resume(&mut memory, &cache, first, identity()).unwrap();
    writer.run(key, span(1024, 2047), 5, false, false).unwrap();
    for h in 1024..2048 {
        writer
            .domain(
                9,
                Evidence {
                    identity_first: Some(0),
                    ..known(h, h, Some(h), Some(1))
                },
            )
            .unwrap();
    }
    let second = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(2, 1, 0, 2047, 2047),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory.clone(), &cache, second, identity()).unwrap();
    assert_eq!(snapshot.batch_through(500).unwrap(), Some(1023));
    assert_eq!(snapshot.batch_through(1500).unwrap(), Some(2047));
    let mut spans = Vec::new();
    snapshot
        .coverage_page(key, span(100, 1800), None, Span::SIZE, |s| {
            spans.push(s);
            Ok(())
        })
        .unwrap();
    assert_eq!(spans, [span(100, 1800)]);
    let mut prefix = Snapshot::open(memory, &cache, second, identity())
        .unwrap()
        .through(1500)
        .unwrap()
        .unwrap();
    assert_eq!(prefix.publication().generation, 1);
    assert_eq!(prefix.value_at(key, 1024).unwrap(), Observation::Pending);
    let mut spans = Vec::new();
    prefix
        .coverage_page(key, span(0, 2047), None, Span::SIZE, |s| {
            spans.push(s);
            Ok(())
        })
        .unwrap();
    assert_eq!(spans, [span(0, 1023)]);
}

#[test]
fn duplicate_date_pages_seek_full_positions_and_retain_the_frontier() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let count = 8192_u64;
    for _ in 0..count {
        let sequence = writer.sequence().unwrap();
        writer
            .event(Event {
                key,
                native_hour: 7,
                read_hour: Some(7),
                sequence,
                kind: 1,
                fields: Blob::EMPTY,
            })
            .unwrap();
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 7, 7),
        Blob::EMPTY,
    );
    memory.reads = 0;
    memory.kinds = [0; 8];
    let selected = [selection(key)];
    let query = EventQuery {
        selection: &selected,
        native: span(7, 7),
        as_of: None,
        order: EventOrder::Native,
    };
    let cold = Cache::new(&budget);
    let mut snapshot = Snapshot::open(memory.clone(), &cold, root, identity()).unwrap();
    let mut cursor = None;
    let mut rows = Vec::new();
    loop {
        let page = snapshot
            .event_page(query, cursor, 127 * (Event::SIZE + 1), |row| {
                rows.push(row.sequence());
                Ok(())
            })
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows, (0..count).collect::<Vec<_>>());
    let used = snapshot.into_io();
    assert!(used.reads < 64);
    let mut cursor = None;
    let mut rows = Vec::new();
    let mut pages = 0;
    loop {
        let fresh = Cache::new(&budget);
        let mut snapshot = Snapshot::open(memory, &fresh, root, identity()).unwrap();
        pages += 1;
        let page = snapshot
            .event_page(query, cursor, 127 * (Event::SIZE + 1), |row| {
                rows.push(row.sequence());
                Ok(())
            })
            .unwrap();
        cursor = page.next;
        memory = snapshot.into_io();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows, (0..count).collect::<Vec<_>>());
    assert!(memory.reads < pages * 32);
}

#[test]
fn exact_tails_reject_multiple_keys_before_emitting_any_bucket() {
    use crate::query::TailRun;
    let (memory, root, _, keys) = fixture(&[-3, 4, 12]);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let tail = [TailRun {
        run: Run {
            span: span(13, 16),
            value: 500,
            break_before: false,
            supplying_boundary: false,
            canonical: 99,
            sequence: 99,
        },
        evidence: known(13, 16, Some(13), Some(1)),
    }];
    let selected = [selection(keys[0]), selection(keys[1])];
    let mut emitted = 0;
    assert!(
        snapshot
            .summary_tiles(
                &selected,
                span(0, 16),
                Viewport {
                    from: 0.0,
                    to: 16.0,
                    width: 17
                },
                Tail {
                    exact: &tail,
                    approximate: &[]
                },
                None,
                4096,
                |_, _| {
                    emitted += 1;
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(emitted, 0);
    snapshot
        .summary_tiles(
            &selected[..1],
            span(13, 16),
            Viewport {
                from: 13.0,
                to: 16.0,
                width: 4,
            },
            Tail {
                exact: &tail,
                approximate: &[],
            },
            None,
            4096,
            |_, bucket| {
                assert_eq!(bucket.exact.unwrap().first_value, 500.0);
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn numeric_navigation_and_samples_ignore_read_only_refreshes() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.run(key, span(0, 100), 5, true, false).unwrap();
    for evidence in [
        known(0, 9, Some(0), Some(1)),
        Evidence {
            identity_first: Some(0),
            ..known(10, 100, Some(10), Some(1))
        },
    ] {
        let sequence = writer.sequence().unwrap();
        writer.evidence(key, evidence, Some(sequence)).unwrap();
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 100, 101),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    assert_eq!(
        snapshot
            .next_change(selection(key), 0, true, span(0, 101))
            .unwrap(),
        Some(101)
    );
    assert_eq!(
        snapshot
            .next_change(selection(key), 10, true, span(0, 101))
            .unwrap(),
        Some(101)
    );
    assert_eq!(
        snapshot
            .next_change(selection(key), 10.5, false, span(0, 101))
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        snapshot
            .next_evidence_change(selection(key), 0, true, span(0, 101))
            .unwrap(),
        Some(10)
    );
    let hours = [0, 9, 10, 99, 100, 101];
    let mut numeric = Vec::new();
    snapshot
        .changes_at(&[key], &hours, |_, sample, value| {
            numeric.push((
                sample,
                match value {
                    Observation::Value { run, .. } => Some(run.value),
                    _ => None,
                },
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(numeric, [(0, Some(5)), (5, None)]);
    let mut evidence = Vec::new();
    snapshot
        .observation_changes_at(&[key], &hours, |_, sample, _| {
            evidence.push(sample);
            Ok(())
        })
        .unwrap();
    assert_eq!(evidence, [0, 2, 5]);
    assert_eq!(snapshot.event_count_through(key, 100, None).unwrap(), 2);
    for direction in [Direction::Home, Direction::End] {
        assert_eq!(
            snapshot
                .navigate_window(&[selection(key)], None, direction, (10.5, 20.5))
                .unwrap(),
            Some(10.5)
        );
    }
    assert_eq!(
        snapshot
            .navigate_window(
                &[selection(key)],
                Some(15.25),
                Direction::Forward,
                (10.5, 20.5),
            )
            .unwrap(),
        Some(15.25)
    );
}

#[test]
fn keyed_and_economy_windows_clip_ranges_while_hourly_charts_keep_original_runs() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.run(key, span(0, 49), 5, false, false).unwrap();
    writer.run(key, span(50, 100), 5, false, true).unwrap();
    writer
        .evidence(key, known(0, 100, Some(0), Some(1)), None)
        .unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 100, 100),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let mut ranges = Vec::new();
    snapshot
        .run_page(selection(key), span(25, 75), None, Run::SIZE, |run| {
            ranges.push(run);
            Ok(())
        })
        .unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].span, span(25, 75));
    assert!(!ranges[0].break_before);
    let mut runs = Vec::new();
    snapshot
        .hourly_runs(
            selection(key),
            span(0, 100),
            Viewport {
                from: 25.0,
                to: 75.0,
                width: 51,
            },
            None,
            4 * Run::SIZE,
            |run| {
                runs.push(run);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        runs.iter().map(|r| r.span).collect::<Vec<_>>(),
        [span(0, 49), span(50, 100)]
    );
    let member = [span(27, 29)];
    let selected = Selection {
        key,
        membership: Some(&member),
    };
    let mut runs = Vec::new();
    snapshot
        .hourly_runs(
            selected,
            span(0, 100),
            Viewport {
                from: 25.0,
                to: 30.0,
                width: 6,
            },
            None,
            Run::SIZE,
            |run| {
                runs.push(run);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(runs[0].span, span(27, 29));
    assert!(runs[0].break_before);
    snapshot
        .summary_tiles(
            &[selected],
            span(0, 100),
            Viewport {
                from: 0.0,
                to: 100.0,
                width: 1,
            },
            Tail::EMPTY,
            None,
            4096,
            |_, bucket| {
                let b = bucket.exact.unwrap();
                assert_eq!((b.first_hour, b.last_hour), (27, 29));
                assert!(!b.has_break);
                Ok(())
            },
        )
        .unwrap();
    let mut evidence = Vec::new();
    snapshot
        .evidence_page(key, span(25, 30), None, Evidence::SIZE, |e| {
            evidence.push(e);
            Ok(())
        })
        .unwrap();
    assert_eq!(evidence[0].span, span(25, 30));
}

#[test]
fn event_membership_unions_pixel_cells_and_unfiltered_ordinals_match_current_reader() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), -3).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    for date in [-3, 1, 2, 5, 7, 12, 17, 20, 25] {
        let sequence = writer.sequence().unwrap();
        writer
            .event(Event {
                key,
                native_hour: date,
                read_hour: Some(30),
                sequence,
                kind: 1,
                fields: Blob::EMPTY,
            })
            .unwrap();
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, -3, 30, 30),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let members = [span(17, 20), span(5, 12), span(7, 9)];
    let selected = [Selection {
        key,
        membership: Some(&members),
    }];
    let query = EventQuery {
        selection: &selected,
        native: span(0, 20),
        as_of: None,
        order: EventOrder::Native,
    };
    let mut cursor = None;
    let mut clusters = Vec::new();
    loop {
        let page = snapshot
            .event_clusters(
                query,
                Viewport {
                    from: -0.25,
                    to: 20.25,
                    width: 72,
                },
                cursor,
                2 * (std::mem::size_of::<Cluster>() + std::mem::size_of::<(i32, u64, u64)>()),
                |c| {
                    clusters.push(c);
                    Ok(())
                },
            )
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        clusters,
        vec![
            Cluster {
                span: span(5, 9),
                first_hour: Some(5),
                last_hour: Some(7),
                first_ordinal: 3,
                count: 2
            },
            Cluster {
                span: span(10, 15),
                first_hour: Some(12),
                last_hour: Some(12),
                first_ordinal: 5,
                count: 1
            },
            Cluster {
                span: span(16, 20),
                first_hour: Some(17),
                last_hour: Some(20),
                first_ordinal: 6,
                count: 2
            }
        ]
    );
    let duplicate = [selection(key), selection(key)];
    let plain = EventQuery {
        selection: &duplicate,
        ..query
    };
    let mut rows = Vec::new();
    let page = snapshot
        .event_page(plain, None, 20 * (Event::SIZE + 1), |row| {
            rows.push(row.native_hour());
            Ok(())
        })
        .unwrap();
    assert_eq!(rows, [1, 2, 5, 7, 12, 17, 20]);
    assert_eq!((page.first_ordinal, page.total), (1, Some(7)));
    let mut rows = Vec::new();
    let page = snapshot
        .event_page(query, None, 20 * (Event::SIZE + 1), |row| {
            rows.push(row.native_hour());
            Ok(())
        })
        .unwrap();
    assert_eq!(rows, [5, 7, 12, 17, 20]);
    assert_eq!((page.first_ordinal, page.total), (0, Some(5)));
}

#[test]
fn native_resume_orders_keys_before_duplicate_sequences_and_skips_finished_keys() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let a = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let b = writer
        .intern(spec(2, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let keys = [a, b];
    for (index, date, read) in [
        (1, 7, 7),
        (0, 7, 7),
        (1, 7, 7),
        (0, 7, 7),
        (0, 6, 8),
        (1, 8, 8),
    ] {
        let sequence = writer.sequence().unwrap();
        let key = keys[index];
        writer
            .event(Event {
                key,
                native_hour: date,
                read_hour: Some(read),
                sequence,
                kind: 1,
                fields: Blob::EMPTY,
            })
            .unwrap();
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 8, 8),
        Blob::EMPTY,
    );
    let selected = [selection(b), selection(a), selection(b)];
    let query = EventQuery {
        selection: &selected,
        native: span(7, 7),
        as_of: None,
        order: EventOrder::Native,
    };
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let page = snapshot
            .event_page(query, cursor, Event::SIZE + 1, |row| {
                rows.push(row.sequence());
                Ok(())
            })
            .unwrap();
        assert_eq!(page.first_ordinal, rows.len() as u64);
        assert_eq!(page.total, Some(4));
        cursor = page.next;
        memory = snapshot.into_io();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows, [1, 3, 0, 2]);
}

#[test]
fn top_spans_keep_zero_negative_display_ties_and_selected_exact_tails() {
    use crate::query::TailRun;
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), -1).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    for (first, last, value) in [
        (-1, -1, 0),
        (0, 0, -2),
        (1, 1, 5),
        (2, 3, 9_007_199_254_740_993),
    ] {
        writer
            .run(key, span(first, last), value, true, false)
            .unwrap();
    }
    writer
        .evidence(key, known(-1, 3, None, None), None)
        .unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, -1, 3, 6),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let mut all = Vec::new();
    snapshot
        .top_spans(selection(key), 9_007_199_254_740_992, span(-1, 3), |s| {
            all.push(s);
            Ok(())
        })
        .unwrap();
    assert_eq!(all, [span(-1, 3)]);
    let exact = [TailRun {
        run: Run {
            span: span(4, 6),
            value: 0,
            break_before: false,
            supplying_boundary: false,
            canonical: 4,
            sequence: 4,
        },
        evidence: known(4, 6, Some(4), None),
    }];
    let mut spans = Vec::new();
    snapshot
        .top_spans_with_tail(
            selection(key),
            0,
            span(-1, 6),
            Tail {
                exact: &exact,
                approximate: &[],
            },
            |s| {
                spans.push(s);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(spans, [span(-1, 0), span(4, 6)]);
}

/// One key over hours 0..400: a constant run, a read every ten hours, an on-time
/// event per ten hours and, from hour 40, one dated 37 hours earlier.
/// `segments` hours per segment; every query must answer as one segment does.
fn chained(segments: i32) -> (Memory, u64, u64) {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let mut root = 0;
    let mut first = 0;
    let mut generation = 0;
    for tens in 0..40 {
        let start = tens * 10;
        writer
            .run(key, span(start, start + 9), 3, tens == 0, false)
            .unwrap();
        let sequence = writer.sequence().unwrap();
        writer
            .evidence(
                key,
                known(start, start + 9, Some(start), Some(5)),
                Some(sequence),
            )
            .unwrap();
        let sequence = writer.sequence().unwrap();
        writer
            .event(Event {
                key,
                native_hour: start + 5,
                read_hour: Some(start + 5),
                sequence,
                kind: 1,
                fields: Blob::EMPTY,
            })
            .unwrap();
        if start >= 40 {
            let sequence = writer.sequence().unwrap();
            writer
                .event(Event {
                    key,
                    native_hour: start - 37,
                    read_hour: Some(start + 9),
                    sequence,
                    kind: 2,
                    fields: Blob::EMPTY,
                })
                .unwrap();
        }
        let last = start + 9;
        if (last + 1 - first) % segments == 0 || last == 399 {
            generation += 1;
            root = publish(
                &mut writer,
                &mut memory,
                &cache,
                publication(generation, generation - 1, 0, last, last),
                Blob::EMPTY,
            );
            first = last + 1;
        }
    }
    (memory, root, key)
}

/// An observation without its row sequence, which counts stored rows and so follows the segmentation.
fn unsequenced(observation: Observation) -> Observation {
    match observation {
        Observation::Value { run, evidence } => Observation::Value {
            run: Run { sequence: 0, ..run },
            evidence,
        },
        other => other,
    }
}

/// Every query family over `hours` answers from a split publication as from one segment.
fn same_answers<'a>(
    one: &mut Snapshot<'a, Memory>,
    many: &mut Snapshot<'a, Memory>,
    key: u64,
    hours: Span,
) {
    fn collect_runs(snapshot: &mut Snapshot<'_, Memory>, key: u64, window: Span) -> Vec<Run> {
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot
                .run_page(selection(key), window, cursor, Run::SIZE, |run| {
                    runs.push(run);
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                return runs
                    .into_iter()
                    .map(|run| Run { sequence: 0, ..run })
                    .collect();
            }
        }
    }
    fn collect_evidence(
        snapshot: &mut Snapshot<'_, Memory>,
        key: u64,
        window: Span,
    ) -> Vec<Evidence> {
        let mut rows = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot
                .evidence_page(key, window, cursor, 3 * Evidence::SIZE, |row| {
                    rows.push(row);
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                return rows;
            }
        }
    }
    for hour in hours.first - 1..=hours.last + 1 {
        assert_eq!(
            unsequenced(many.value_at(key, hour).unwrap()),
            unsequenced(one.value_at(key, hour).unwrap()),
            "key {key} hour {hour}"
        );
    }
    for window in [hours, span(hours.first + 17, hours.last - 66)] {
        assert_eq!(
            collect_runs(many, key, window),
            collect_runs(one, key, window),
            "key {key} window {window:?}"
        );
        let evidence = collect_evidence(one, key, window);
        assert!(!evidence.is_empty());
        assert_eq!(collect_evidence(many, key, window), evidence);
    }
    let mut coverage = [Vec::new(), Vec::new()];
    for (at, snapshot) in [&mut *one, &mut *many].into_iter().enumerate() {
        snapshot
            .coverage_page(key, hours, None, 1 << 20, |known| {
                coverage[at].push(known);
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(coverage[1], coverage[0], "key {key}");
    let selections = [selection(key)];
    for width in [7, hours.hours() as u32] {
        let view = Viewport {
            from: f64::from(hours.first),
            to: f64::from(hours.last),
            width,
        };
        let expected = all_tiles(one, &selections, hours, view, 64);
        assert_eq!(expected.len(), grid(hours, view).len());
        assert_eq!(all_tiles(many, &selections, hours, view, 64), expected);
    }
    for as_of in [None, Some(hours.first + 100), Some(hours.last)] {
        for native in [
            hours,
            span(hours.first + 3, hours.first + 3),
            span(hours.first + 37, hours.first + 251),
        ] {
            for order in [EventOrder::Native, EventOrder::Publication] {
                let query = EventQuery {
                    selection: &selections,
                    native,
                    as_of,
                    order,
                };
                assert_eq!(
                    many.event_count(query).unwrap(),
                    one.event_count(query).unwrap()
                );
                let mut expected = Vec::new();
                let mut actual = Vec::new();
                for (snapshot, out) in [(&mut *one, &mut expected), (&mut *many, &mut actual)] {
                    let mut cursor = None;
                    loop {
                        let page = snapshot
                            .event_page(query, cursor, 3 * (Event::SIZE + 1), |row| {
                                out.push(row);
                                Ok(())
                            })
                            .unwrap();
                        cursor = page.next;
                        if cursor.is_none() {
                            break;
                        }
                    }
                }
                assert_eq!(actual, expected);
                let view = Viewport {
                    from: f64::from(hours.first) - 0.5,
                    to: f64::from(hours.last) + 1.5,
                    width: 90,
                };
                let mut expected = Vec::new();
                let mut actual = Vec::new();
                for (snapshot, out) in [(&mut *one, &mut expected), (&mut *many, &mut actual)] {
                    snapshot
                        .event_clusters(query, view, None, 1 << 20, |c| {
                            out.push(c);
                            Ok(())
                        })
                        .unwrap();
                }
                assert_eq!(actual, expected);
            }
            assert_eq!(
                many.latest_event(selection(key), native.last, as_of)
                    .unwrap(),
                one.latest_event(selection(key), native.last, as_of)
                    .unwrap()
            );
        }
        for ordinal in [0, 1, 40, 78, 200] {
            assert_eq!(
                many.event_raw(selection(key), as_of, ordinal).unwrap(),
                one.event_raw(selection(key), as_of, ordinal).unwrap()
            );
            assert_eq!(
                many.event_date(selection(key), as_of, ordinal).unwrap(),
                one.event_date(selection(key), as_of, ordinal).unwrap()
            );
        }
    }
    for origin in [
        hours.first - 3,
        hours.first,
        hours.first + 5,
        hours.first + 150,
        hours.first + 199,
        hours.last,
        hours.last + 5,
    ] {
        for direction in [Direction::Forward, Direction::Backward] {
            assert_eq!(
                many.navigate(&selections, Some(origin), direction, hours)
                    .unwrap(),
                one.navigate(&selections, Some(origin), direction, hours)
                    .unwrap(),
                "key {key} origin {origin} {direction:?}"
            );
            assert_eq!(
                many.next_evidence_change(
                    selection(key),
                    origin,
                    direction == Direction::Forward,
                    hours
                )
                .unwrap(),
                one.next_evidence_change(
                    selection(key),
                    origin,
                    direction == Direction::Forward,
                    hours
                )
                .unwrap()
            );
        }
    }
    let mut metadata = [Vec::new(), Vec::new()];
    for (at, snapshot) in [&mut *one, &mut *many].into_iter().enumerate() {
        let mut cursor = None;
        loop {
            let page = snapshot
                .metadata_page(cursor, 2 * (Cell::SIZE + 8), |id, value| {
                    metadata[at].push((id, value));
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
    }
    assert_eq!(metadata[1], metadata[0]);
}

#[test]
fn hierarchy_merges_late_tables_and_chains_answer_as_one_segment_does() {
    let (whole, whole_root, key) = chained(400);
    let (split, split_root, _) = chained(10);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut one = Snapshot::open(whole, &cache, whole_root, identity()).unwrap();
    let mut many = Snapshot::open(split, &cache, split_root, identity()).unwrap();
    assert_eq!(many.reader.root().nodes.len(), 10);
    assert_eq!(many.reader.root().nodes[0].level, 1);
    assert!(matches!(
        many.value_at(key, 250).unwrap(),
        Observation::Value {
            run: Run {
                span: Span {
                    first: 0,
                    last: 399
                },
                canonical: 0,
                ..
            },
            ..
        }
    ));
    same_answers(&mut one, &mut many, key, span(0, 399));
    for window in [span(0, 399), span(17, 233)] {
        let mut runs = 0;
        let mut cursor = None;
        loop {
            let page = many
                .run_page(selection(key), window, cursor, Run::SIZE, |_| {
                    runs += 1;
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(runs, 1);
    }
    let mut coverage = Vec::new();
    many.coverage_page(key, span(0, 399), None, Span::SIZE, |s| {
        coverage.push(s);
        Ok(())
    })
    .unwrap();
    assert_eq!(coverage, [span(0, 399)]);
    let mut prefix = many.through(155).unwrap().unwrap();
    assert_eq!(prefix.publication().accepted, span(0, 149));
    assert!(matches!(
        prefix.value_at(key, 149).unwrap(),
        Observation::Value {
            run: Run {
                span: Span {
                    first: 0,
                    last: 149
                },
                ..
            },
            ..
        }
    ));
    assert_eq!(prefix.value_at(key, 150).unwrap(), Observation::Pending);
}

/// Two keys over hours 0..300, a segment every `per` hours. An exact run breaks
/// once and is read every hour, its reads alike for ten hours; an ordered
/// contribution runs to hour 150, where an overlapping one starts and is
/// carried from there. An on-time event every seven hours, a late one every
/// five, and catalogue values rewritten every hour.
fn hourly(per: i32) -> (Memory, u64, [u64; 2]) {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let keys = [
        writer
            .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
            .unwrap(),
        writer
            .intern(spec(2, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
            .unwrap(),
    ];
    let mut root = 0;
    let mut first = 0;
    let mut generation = 0;
    for hour in 0..300 {
        let tens = hour - hour % 10;
        for key in keys {
            writer
                .evidence(
                    key,
                    Evidence {
                        identity_first: Some(tens),
                        ..known(hour, hour, Some(tens), Some(3))
                    },
                    None,
                )
                .unwrap();
            if hour % 7 == 0 {
                let sequence = writer.sequence().unwrap();
                writer
                    .event(Event {
                        key,
                        native_hour: hour,
                        read_hour: Some(hour),
                        sequence,
                        kind: 1,
                        fields: Blob::EMPTY,
                    })
                    .unwrap();
            }
            if hour % 5 == 0 && hour >= 40 {
                let sequence = writer.sequence().unwrap();
                writer
                    .event(Event {
                        key,
                        native_hour: hour - 37,
                        read_hour: Some(hour),
                        sequence,
                        kind: 2,
                        fields: Blob::EMPTY,
                    })
                    .unwrap();
            }
        }
        writer
            .run(
                keys[0],
                span(hour, hour),
                if hour < 120 { 4 } else { 9 },
                hour == 0 || hour == 120,
                false,
            )
            .unwrap();
        if hour <= 150 {
            writer
                .run(keys[1], span(hour, hour), 1, hour == 0, false)
                .unwrap();
        }
        if hour >= 150 {
            writer
                .run(keys[1], span(hour, hour), 2, hour == 150, false)
                .unwrap();
        }
        writer
            .catalogue((hour % 3) as u64, Cell::Integer(i128::from(hour)))
            .unwrap();
        if (hour + 1 - first) % per == 0 || hour == 299 {
            generation += 1;
            root = publish(
                &mut writer,
                &mut memory,
                &cache,
                publication(generation, generation - 1, 0, hour, hour),
                Blob::EMPTY,
            );
            first = hour + 1;
        }
    }
    (memory, root, keys)
}

#[test]
fn a_level_two_merge_answers_as_one_segment_does() {
    let (whole, whole_root, keys) = hourly(300);
    let (split, split_root, _) = hourly(1);
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut one = Snapshot::open(whole, &cache, whole_root, identity()).unwrap();
    let mut many = Snapshot::open(split, &cache, split_root, identity()).unwrap();
    let frontier = many.reader.root().nodes.clone();
    assert_eq!((frontier[0].level, frontier[0].segments), (2, 256));
    assert_eq!(frontier.len(), 15);
    same_answers(&mut one, &mut many, keys[0], span(0, 299));
    // Ordered navigation follows each segment's partition point, so only segment-free answers compare.
    for hour in -1..=300 {
        assert_eq!(
            unsequenced(many.value_at(keys[1], hour).unwrap()),
            unsequenced(one.value_at(keys[1], hour).unwrap()),
            "hour {hour}"
        );
    }
    for window in [span(0, 299), span(140, 160)] {
        let mut pages = [Vec::new(), Vec::new()];
        for (at, snapshot) in [&mut one, &mut many].into_iter().enumerate() {
            let mut cursor = None;
            loop {
                let page = snapshot
                    .run_page(selection(keys[1]), window, cursor, Run::SIZE, |run| {
                        pages[at].push(Run { sequence: 0, ..run });
                        Ok(())
                    })
                    .unwrap();
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
        }
        assert_eq!(pages[1], pages[0]);
        assert_eq!(pages[0].len(), 2);
    }
    let bounds = |observation: Observation| match observation {
        Observation::Value { run, .. } => Some((run.span, run.value, run.canonical)),
        _ => None,
    };
    for snapshot in [&mut one, &mut many] {
        assert_eq!(
            bounds(snapshot.value_at(keys[0], 5).unwrap()),
            Some((span(0, 119), 4, 0))
        );
        assert_eq!(
            bounds(snapshot.value_at(keys[0], 200).unwrap()),
            Some((span(120, 299), 9, 1))
        );
        assert_eq!(
            bounds(snapshot.value_at(keys[1], 5).unwrap()),
            Some((span(0, 150), 1, 0))
        );
        for hour in [150, 151, 299] {
            assert_eq!(
                bounds(snapshot.value_at(keys[1], hour).unwrap()),
                Some((span(150, 299), 2, 1))
            );
        }
    }
    let mut metadata = Vec::new();
    many.metadata_page(None, 1 << 20, |id, value| {
        metadata.push((id, value));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        metadata,
        [
            (0, Cell::Integer(297)),
            (1, Cell::Integer(298)),
            (2, Cell::Integer(299))
        ]
    );
}

#[test]
fn overlapping_contributions_chain_only_through_the_run_carried_out() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    let mut root = 0;
    for (segment, rows) in [
        vec![(1, true)],
        vec![(1, false), (2, true)],
        vec![(2, false)],
    ]
    .into_iter()
    .enumerate()
    {
        let first = segment as i32 * 10;
        writer
            .evidence(key, known(first, first + 9, None, None), None)
            .unwrap();
        for (value, broken) in rows {
            writer
                .run(key, span(first, first + 9), value, broken, false)
                .unwrap();
        }
        root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(segment as u64 + 1, segment as u64, 0, first + 9, first + 9),
            Blob::EMPTY,
        );
    }
    let bounds = |observation: Observation| match observation {
        Observation::Value { run, .. } => Some((run.span, run.value, run.canonical)),
        _ => None,
    };
    let mut snapshot = Snapshot::open(memory.clone(), &cache, root, identity()).unwrap();
    assert_eq!(
        bounds(snapshot.value_at(key, 5).unwrap()),
        Some((span(0, 19), 1, 0))
    );
    for hour in [10, 15, 25] {
        assert_eq!(
            bounds(snapshot.value_at(key, hour).unwrap()),
            Some((span(10, 29), 2, 1))
        );
    }
    for (window, expected) in [
        (span(0, 29), [(span(0, 19), 1, 0), (span(10, 29), 2, 1)]),
        (span(12, 25), [(span(12, 19), 1, 0), (span(12, 25), 2, 1)]),
    ] {
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot
                .run_page(selection(key), window, cursor, Run::SIZE, |run| {
                    runs.push((run.span, run.value, run.canonical));
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(runs, expected);
    }
    let all = span(i32::MIN, i32::MAX);
    assert_eq!(
        snapshot.next_change(selection(key), 5, true, all).unwrap(),
        Some(20)
    );
    for (origin, stop) in [(10, 0), (12, 10), (25, 10)] {
        assert_eq!(
            snapshot
                .next_change(selection(key), origin, false, all)
                .unwrap(),
            Some(stop)
        );
    }
    let mut prefix = Snapshot::open(memory, &cache, root, identity())
        .unwrap()
        .through(19)
        .unwrap()
        .unwrap();
    assert_eq!(
        bounds(prefix.value_at(key, 5).unwrap()),
        Some((span(0, 19), 1, 0))
    );
    assert_eq!(
        bounds(prefix.value_at(key, 15).unwrap()),
        Some((span(10, 19), 2, 1))
    );
}

#[test]
fn ordered_contribution_pages_seek_their_position_and_decode_bounded_rows() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    let mut canonicals: Vec<(Span, i128)> = Vec::new();
    let mut root = 0;
    for segment in 0..12_i32 {
        let first = segment * 100;
        writer
            .evidence(key, known(first, first + 99, None, None), None)
            .unwrap();
        let mut rows = Vec::new();
        if (3..7).contains(&segment) {
            rows.push((span(first, first + 99), 77, segment == 3));
        } else {
            for i in 0..40 {
                let start = first + (i * 37) % 90;
                rows.push((span(start, start + i % 9), i128::from(first + i), true));
            }
        }
        for (extent, value, broken) in rows {
            writer.run(key, extent, value, broken, false).unwrap();
            match canonicals.last_mut() {
                Some((last, held))
                    if !broken && *held == value && last.last + 1 == extent.first =>
                {
                    last.last = extent.last;
                }
                _ => canonicals.push((extent, value)),
            }
        }
        root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(
                segment as u64 + 1,
                segment as u64,
                0,
                first + 99,
                first + 99,
            ),
            Blob::EMPTY,
        );
    }
    assert_eq!(canonicals.len(), 8 * 40 + 1);
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let members = [span(150, 420), span(650, 1000)];
    for (window, membership) in [
        (span(0, 1199), None),
        (span(150, 1100), None),
        (span(150, 1100), Some(&members[..])),
    ] {
        let mut expected = Vec::new();
        for &(extent, value) in &canonicals {
            let Some(part) = extent.intersect(window) else {
                continue;
            };
            match membership {
                None => expected.push((part, value)),
                Some(members) => expected.extend(
                    members
                        .iter()
                        .filter_map(|member| part.intersect(*member))
                        .map(|part| (part, value)),
                ),
            }
        }
        let selected = Selection { key, membership };
        let mut actual = Vec::new();
        let mut most = 0;
        let mut cursor = None;
        loop {
            let before = crate::read::DECODED_RUNS.with(|decoded| decoded.get());
            let page = snapshot
                .run_page(selected, window, cursor, Run::SIZE, |run| {
                    actual.push((run.span, run.value));
                    Ok(())
                })
                .unwrap();
            most = most.max(crate::read::DECODED_RUNS.with(|decoded| decoded.get()) - before);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(actual, expected, "window {window:?}");
        assert!(actual.len() > 250);
        assert!(most <= 6 * 32, "{most} rows decoded for one page");
    }
}

#[test]
fn domain_presentations_append_through_their_row_handles() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let other = writer
        .intern(spec(2, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.bind_domain(key, 5).unwrap();
    writer.bind_domain(other, 6).unwrap();
    let values = [known(0, 1, Some(0), Some(1)), known(2, 3, Some(2), Some(2))];
    let rows = [
        writer.domain(5, values[0]).unwrap(),
        writer.domain(5, values[1]).unwrap(),
    ];
    let foreign = writer.domain(6, known(0, 3, Some(0), Some(9))).unwrap();
    writer.run(key, span(0, 3), 1, true, false).unwrap();
    let mut expected = Vec::new();
    for (row, value) in rows.into_iter().zip(values) {
        let sequence = writer.sequence().unwrap();
        writer.present_domain(key, row, sequence).unwrap();
        expected.push(EventView::Evidence {
            key,
            sequence,
            value,
        });
    }
    let sequence = writer.sequence().unwrap();
    assert!(writer.present_domain(key, foreign, sequence).is_err());
    publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 3, 3),
        Blob::EMPTY,
    );
    let sequence = writer.sequence().unwrap();
    assert!(writer.present_domain(key, rows[0], sequence).is_err());
    writer.present_domain(key, rows[1], sequence).unwrap();
    expected.push(EventView::Evidence {
        key,
        sequence,
        value: values[1],
    });
    let carried = known(4, 5, Some(4), Some(3));
    let row = writer.domain(5, carried).unwrap();
    writer.run(key, span(4, 5), 1, false, false).unwrap();
    let sequence = writer.sequence().unwrap();
    writer.present_domain(key, row, sequence).unwrap();
    expected.push(EventView::Evidence {
        key,
        sequence,
        value: carried,
    });
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(2, 1, 0, 5, 5),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let selections = [selection(key)];
    let mut actual = Vec::new();
    snapshot
        .event_page(
            EventQuery {
                selection: &selections,
                native: span(0, 5),
                as_of: None,
                order: EventOrder::Publication,
            },
            None,
            1 << 20,
            |row| {
                actual.push(row);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn a_domain_bound_after_a_run_covers_that_run() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.run(key, span(0, 5), 3, true, false).unwrap();
    writer.bind_domain(key, 7).unwrap();
    let row = writer.domain(7, known(0, 5, Some(0), Some(1))).unwrap();
    let sequence = writer.sequence().unwrap();
    writer.present_domain(key, row, sequence).unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 5, 5),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    for hour in 0..=5 {
        assert_eq!(
            snapshot.value_at(key, hour).unwrap(),
            Observation::Value {
                run: Run {
                    span: span(0, 5),
                    value: 3,
                    break_before: true,
                    supplying_boundary: false,
                    canonical: 0,
                    sequence: 0,
                },
                evidence: known(0, 5, Some(0), Some(1)),
            }
        );
    }
    assert_eq!(snapshot.value_at(key, 6).unwrap(), Observation::Pending);
    let mut coverage = Vec::new();
    snapshot
        .coverage_page(key, span(0, 5), None, Span::SIZE, |known| {
            coverage.push(known);
            Ok(())
        })
        .unwrap();
    assert_eq!(coverage, [span(0, 5)]);
    let selections = [selection(key)];
    assert_eq!(
        snapshot
            .event_count(EventQuery {
                selection: &selections,
                native: span(0, 5),
                as_of: None,
                order: EventOrder::Native,
            })
            .unwrap(),
        1
    );
}

#[test]
fn seal_work_follows_the_segment_rows_not_the_history_and_runs_off_the_emit_thread() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let keys: Vec<_> = (0..200)
        .map(|actor| {
            writer
                .intern(spec(actor, 1), Blob::EMPTY, RunMode::Exact, 0)
                .unwrap()
        })
        .collect();
    let mut work = Vec::new();
    for segment in 0..20_i32 {
        let start = segment * 10;
        for &key in &keys {
            writer
                .evidence(key, known(start, start + 9, Some(start), None), None)
                .unwrap();
            writer
                .run(
                    key,
                    span(start, start + 9),
                    i128::from(key % 3),
                    false,
                    false,
                )
                .unwrap();
        }
        let before = memory.pages.len();
        let pending = writer
            .cut(
                publication(segment as u64 + 1, segment as u64, 0, start + 9, start + 9),
                Blob::EMPTY,
            )
            .unwrap();
        assert_eq!(memory.pages.len(), before);
        let (sealed, counted) = std::thread::scope(|scope| {
            scope
                .spawn(|| pending.seal_counted(&mut memory))
                .join()
                .unwrap()
        })
        .unwrap();
        drop(pending);
        writer.publish(&mut memory, &cache, sealed).unwrap();
        work.push(counted);
    }
    let rows = 2 * keys.len() as u64;
    for counted in &work {
        assert!(counted.pages <= 24, "{counted:?}");
        assert!(counted.bytes <= 64 * 1024 + 64 * rows, "{counted:?}");
    }
    let first = work[1];
    let last = work[work.len() - 1];
    assert!(last.pages <= first.pages + 2, "{first:?} {last:?}");
    assert!(last.bytes <= first.bytes * 2, "{first:?} {last:?}");
    for &key in &keys {
        for hour in 0..20 {
            writer
                .evidence(
                    key,
                    known(200 + hour, 200 + hour, Some(200 + hour), None),
                    None,
                )
                .unwrap();
            writer
                .run(
                    key,
                    span(200 + hour, 200 + hour),
                    i128::from(hour),
                    true,
                    false,
                )
                .unwrap();
        }
    }
    let pending = writer
        .cut(publication(21, 20, 0, 219, 219), Blob::EMPTY)
        .unwrap();
    let (_, dense) = pending.seal_counted(&mut memory).unwrap();
    let rows = 40 * keys.len() as u64;
    assert!(dense.bytes <= 64 * 1024 + 32 * rows, "{dense:?}");
    assert!(
        dense.pages <= 24 + dense.bytes / pages::PAGE_BYTES as u64,
        "{dense:?}"
    );
}

#[test]
fn runs_outside_known_coverage_and_rows_outside_their_segment_are_refused() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.run(key, span(0, 5), 1, true, false).unwrap();
    writer
        .evidence(key, known(0, 3, Some(0), None), None)
        .unwrap();
    let pending = writer.cut(publication(1, 0, 0, 5, 5), Blob::EMPTY).unwrap();
    assert!(pending.seal(&mut memory).is_err());
    drop(pending);

    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.run(key, span(0, 5), 1, true, false).unwrap();
    assert!(writer.cut(publication(1, 0, 0, 4, 4), Blob::EMPTY).is_err());
    writer
        .evidence(key, known(0, 5, Some(0), None), None)
        .unwrap();
    let sequence = writer.sequence().unwrap();
    writer
        .event(Event {
            key,
            native_hour: 6,
            read_hour: None,
            sequence,
            kind: 1,
            fields: Blob::EMPTY,
        })
        .unwrap();
    assert!(writer.cut(publication(1, 0, 0, 5, 5), Blob::EMPTY).is_err());
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 6, 6),
        Blob::EMPTY,
    );
    assert!(writer.run(key, span(6, 8), 2, true, false).is_err());
    assert!(writer.evidence(key, known(5, 8, None, None), None).is_err());
    let unbound = writer
        .intern(spec(2, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.bind_domain(unbound, 4).unwrap();
    assert!(
        writer
            .evidence(unbound, known(7, 8, None, None), None)
            .is_err()
    );
    let sequence = writer.sequence().unwrap();
    for _ in 0..2 {
        writer
            .event(Event {
                key,
                native_hour: 7,
                read_hour: None,
                sequence,
                kind: 1,
                fields: Blob::EMPTY,
            })
            .unwrap();
    }
    let pending = writer.cut(publication(2, 1, 0, 8, 8), Blob::EMPTY).unwrap();
    assert!(pending.seal(&mut memory).is_err());
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    assert!(matches!(
        snapshot.value_at(key, 5).unwrap(),
        Observation::Value { .. }
    ));
    assert_eq!(snapshot.value_at(key, 7).unwrap(), Observation::Pending);
}

fn refused<T: std::fmt::Debug>(result: io::Result<T>) {
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
}

#[test]
fn refused_appends_change_nothing_and_a_retry_continues_the_last_admitted_run() {
    const LIMIT: usize = 1 << 20;
    let budget = Budget::new(LIMIT);
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let keys: Vec<_> = (0..8)
        .map(|actor| {
            writer
                .intern(spec(actor, 1), Blob::EMPTY, RunMode::Exact, 0)
                .unwrap()
        })
        .collect();
    let key = keys[0];
    writer.bind_domain(keys[7], 9).unwrap();
    let shared = writer.domain(9, known(0, 0, Some(0), Some(1))).unwrap();
    writer.run(key, span(0, 0), 0, true, false).unwrap();
    let presented = writer.sequence().unwrap();
    let family = writer.sequence().unwrap();
    let hog = budget.admit(LIMIT - budget.used()).unwrap();
    let mut hour = 1;
    let cut = loop {
        match writer.run(key, span(hour, hour), i128::from(hour), true, false) {
            Ok(()) => hour += 1,
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                break hour;
            }
        }
    };
    assert!(cut > 1);
    refused(writer.run(key, span(cut + 1, cut + 1), i128::from(cut), false, false));
    writer
        .run(key, span(cut, cut), i128::from(cut - 1), false, false)
        .unwrap();
    let mut domain_hour = 1;
    let domain_cut = loop {
        match writer.domain(9, known(domain_hour, domain_hour, Some(domain_hour), Some(1))) {
            Ok(_) => domain_hour += 1,
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                break domain_hour;
            }
        }
    };
    refused(writer.evidence(key, known(0, cut + 1, None, None), None));
    let event = Event {
        key,
        native_hour: 3,
        read_hour: None,
        sequence: family,
        kind: 1,
        fields: Blob::EMPTY,
    };
    refused(writer.event(event));
    refused(writer.present_domain(keys[7], shared, presented));
    refused(writer.catalogue(1, Cell::Integer(1)));
    refused(writer.text("kept"));
    refused(writer.intern(spec(99, 1), Blob::EMPTY, RunMode::Exact, 0));
    drop(hog);

    writer
        .run(key, span(cut + 1, cut + 1), 99, true, false)
        .unwrap();
    writer
        .domain(9, known(domain_cut, domain_cut, Some(domain_cut), Some(1)))
        .unwrap();
    writer
        .evidence(key, known(0, cut + 1, None, None), None)
        .unwrap();
    writer.event(event).unwrap();
    writer.present_domain(keys[7], shared, presented).unwrap();
    writer.catalogue(1, Cell::Integer(1)).unwrap();
    let text = writer.text("kept").unwrap();
    assert_eq!(
        writer
            .intern(spec(99, 1), Blob::EMPTY, RunMode::Exact, 0)
            .unwrap(),
        8
    );
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let last = cut.max(domain_cut) + 1;
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, last, last),
        Blob::EMPTY,
    );
    drop(writer);
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let mut runs = Vec::new();
    snapshot
        .run_page(selection(key), span(0, last), None, 1 << 16, |run| {
            runs.push((run.span, run.value));
            Ok(())
        })
        .unwrap();
    let mut expected: Vec<_> = (0..cut - 1).map(|h| (span(h, h), i128::from(h))).collect();
    expected.push((span(cut - 1, cut), i128::from(cut - 1)));
    expected.push((span(cut + 1, cut + 1), 99));
    assert_eq!(runs, expected);
    let Observation::Value { run, .. } = snapshot.value_at(key, cut).unwrap() else {
        panic!("the continued run is missing");
    };
    assert_eq!(
        (run.canonical, run.sequence),
        ((cut - 1) as u64, (cut - 1) as u64)
    );
    let selected = [selection(key), selection(keys[7])];
    let mut events = Vec::new();
    snapshot
        .event_page(
            EventQuery {
                selection: &selected,
                native: span(0, last),
                as_of: None,
                order: EventOrder::Publication,
            },
            None,
            1 << 16,
            |row| {
                events.push(row);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        events,
        [
            EventView::Evidence {
                key: keys[7],
                sequence: presented,
                value: known(0, 0, Some(0), Some(1)),
            },
            EventView::Family(event),
        ]
    );
    let mut metadata = Vec::new();
    snapshot
        .metadata_page(None, 1 << 16, |id, value| {
            metadata.push((id, value));
            Ok(())
        })
        .unwrap();
    assert_eq!(metadata, [(1, Cell::Integer(1))]);
    let mut bytes = [0; 4];
    snapshot.cell_bytes(text, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"kept");
    let mut defined = 0;
    snapshot
        .catalogue_page(None, None, 1 << 16, |_| {
            defined += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(defined, 9);
    drop(snapshot);
    drop(cache);
    assert_eq!(budget.used(), 0);
}

#[test]
fn a_resumed_writer_presents_the_domain_row_it_carried_with_its_exact_evidence() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let other = writer
        .intern(spec(2, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer.bind_domain(key, 5).unwrap();
    let shared = Evidence {
        identity_first: Some(0),
        ..known(0, 1, Some(0), Some(7))
    };
    let rows = [
        shared,
        Evidence {
            span: span(2, 3),
            ..shared
        },
    ];
    let handles = [
        writer.domain(5, rows[0]).unwrap(),
        writer.domain(5, rows[1]).unwrap(),
    ];
    writer.run(key, span(0, 3), 4, true, false).unwrap();
    let mut root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 3, 3),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory.clone(), &cache, root, identity()).unwrap();
    let mut extents = Vec::new();
    snapshot
        .evidence_page(key, span(0, 3), None, 1 << 16, |row| {
            extents.push(row.span);
            Ok(())
        })
        .unwrap();
    assert_eq!(extents, [span(0, 3)]);
    drop(snapshot);
    let mut resumed = Writer::resume(&mut memory, &cache, root, identity()).unwrap();
    let mut presented = [0; 2];
    for (at, writer) in [&mut writer, &mut resumed].into_iter().enumerate() {
        let sequence = writer.sequence().unwrap();
        assert!(writer.present_domain(key, handles[0], sequence).is_err());
        writer.present_domain(key, handles[1], sequence).unwrap();
        presented[at] = sequence;
    }
    drop(writer);
    let mut expected = vec![EventView::Evidence {
        key,
        sequence: presented[1],
        value: rows[1],
    }];
    let mut writer = resumed;
    for segment in 1..=16_u64 {
        let hour = 3 + segment as i32;
        writer
            .evidence(other, known(hour, hour, None, None), None)
            .unwrap();
        writer
            .run(other, span(hour, hour), 1, segment == 1, false)
            .unwrap();
        root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(segment + 1, segment, 0, hour, hour),
            Blob::EMPTY,
        );
    }
    let sequence = writer.sequence().unwrap();
    writer.present_domain(key, handles[1], sequence).unwrap();
    drop(writer);
    let mut resumed = Writer::resume(&mut memory, &cache, root, identity()).unwrap();
    let sequence = resumed.sequence().unwrap();
    assert!(resumed.present_domain(key, handles[0], sequence).is_err());
    resumed.present_domain(key, handles[1], sequence).unwrap();
    expected.push(EventView::Evidence {
        key,
        sequence,
        value: rows[1],
    });
    root = publish(
        &mut resumed,
        &mut memory,
        &cache,
        publication(18, 17, 0, 21, 21),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let selected = [selection(key)];
    let mut events = Vec::new();
    snapshot
        .event_page(
            EventQuery {
                selection: &selected,
                native: span(0, 21),
                as_of: None,
                order: EventOrder::Publication,
            },
            None,
            1 << 16,
            |row| {
                events.push(row);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(events, expected);
}

#[test]
fn an_ordered_contribution_ending_at_the_last_hour_does_not_end_its_page() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    writer
        .evidence(key, known(0, i32::MAX, None, None), None)
        .unwrap();
    writer.run(key, span(0, i32::MAX), 1, true, false).unwrap();
    writer.run(key, span(0, 0), 2, true, false).unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, i32::MAX, i32::MAX),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    for bytes in [Run::SIZE, 1 << 16] {
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot
                .run_page(selection(key), span(0, i32::MAX), cursor, bytes, |run| {
                    runs.push((run.span, run.value));
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(runs, [(span(0, i32::MAX), 1), (span(0, 0), 2)], "{bytes}");
    }
}

/// Every hourly run page of `selection`, as (extent, amount, break).
fn hourly_pages(
    snapshot: &mut Snapshot<'_, Memory>,
    selection: Selection<'_>,
    bounds: Span,
    view: Viewport,
    bytes: usize,
) -> Vec<(Span, i128, bool)> {
    let mut runs = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .hourly_runs(selection, bounds, view, cursor, bytes, |run| {
                runs.push((run.span, run.value, run.break_before));
                Ok(())
            })
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            return runs;
        }
    }
}

#[test]
fn hourly_runs_keep_only_a_selection_starting_elsewhere_than_the_last_across_pages() {
    use crate::query::Alias;
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let mut keys = Vec::new();
    for (actor, rows) in [
        (1, [(span(0, 10), 1), (span(5, 5), 2), (span(0, 10), 3)]),
        (2, [(span(0, 2), 1), (span(5, 5), 2), (span(0, 10), 3)]),
    ] {
        let key = writer
            .intern(spec(actor, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
            .unwrap();
        writer
            .evidence(key, known(0, 10, None, None), None)
            .unwrap();
        for (extent, value) in rows {
            writer.run(key, extent, value, true, false).unwrap();
        }
        keys.push(key);
    }
    let exact = writer
        .intern(spec(3, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    writer
        .evidence(exact, known(0, 40, None, None), None)
        .unwrap();
    writer.run(exact, span(10, 30), 4, true, false).unwrap();
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, 40, 40),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    for (hour, value) in [(4, 1), (5, 3)] {
        assert!(matches!(
            snapshot.value_at(keys[0], hour).unwrap(),
            Observation::Value { run, .. } if run.value == value
        ));
    }
    let view = |from: f64, to: f64| Viewport {
        from,
        to,
        width: (to - from) as u32 + 1,
    };
    let members = [span(10, 15), span(25, 40)];
    let masked = Selection {
        key: exact,
        membership: Some(&members),
    };
    let bounds = span(0, 40);
    for bytes in [Run::SIZE, 1 << 16] {
        assert_eq!(
            hourly_pages(&mut snapshot, selection(keys[0]), bounds, view(0.0, 10.0), bytes),
            [(span(0, 10), 1, true)]
        );
        assert_eq!(
            hourly_pages(&mut snapshot, selection(keys[0]), bounds, view(5.0, 10.0), bytes),
            [(span(0, 10), 3, true)]
        );
        assert_eq!(
            hourly_pages(&mut snapshot, selection(keys[1]), bounds, view(0.0, 10.0), bytes),
            [(span(0, 2), 1, true)]
        );
        assert_eq!(
            hourly_pages(&mut snapshot, masked, bounds, view(20.0, 30.0), bytes),
            [(span(10, 15), 4, true), (span(25, 30), 4, true)]
        );
        let aliases = [span(12, 14), span(18, 22), span(31, 35)].map(|span| Alias {
            key: exact,
            span,
            boundary: None,
        });
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot
                .hourly_alias_runs(&aliases, bounds, view(20.0, 30.0), cursor, bytes, |run| {
                    runs.push((run.span, run.value, run.break_before));
                    Ok(())
                })
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(runs, [(span(18, 22), 4, true)]);
    }
}

#[test]
fn hourly_runs_list_each_grid_hours_point_selection_once_per_first_hour() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as i32
    };
    let keys: Vec<_> = (0..4)
        .map(|actor| {
            let mode = if actor == 3 {
                RunMode::Exact
            } else {
                RunMode::OrderedContributions
            };
            writer.intern(spec(actor, 1), Blob::EMPTY, mode, 0).unwrap()
        })
        .collect();
    let mut root = 0;
    let mut first = 0;
    let mut carried = [None; 3];
    let mut exact = 0;
    for (segment, end) in [19, 40, 41, 77, 99].into_iter().enumerate() {
        for (at, &key) in keys.iter().enumerate() {
            writer
                .evidence(key, known(first, end, None, None), None)
                .unwrap();
            if at == 3 {
                while exact <= end {
                    let last = (exact + next() % 7).min(end);
                    let value = i128::from(next() % 3);
                    let broken = next() % 2 == 0;
                    writer
                        .run(key, span(exact, last), value, broken, false)
                        .unwrap();
                    exact = last + 1 + next() % 3;
                }
                continue;
            }
            if let Some(value) = carried[at].take() {
                let last = (first + next() % 4).min(end);
                writer
                    .run(key, span(first, last), value, false, false)
                    .unwrap();
            }
            for _ in 0..next() % 9 {
                let start = first + next() % (end - first + 1);
                let last = (start + next() % 12).min(end);
                let value = i128::from(next() % 4);
                let broken = next() % 3 == 0;
                writer
                    .run(key, span(start, last), value, broken, false)
                    .unwrap();
            }
            if next() % 2 == 0 {
                let value = i128::from(next() % 4);
                let start = (end - next() % 5).max(first);
                writer.run(key, span(start, end), value, true, false).unwrap();
                carried[at] = Some(value);
            }
        }
        root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(segment as u64 + 1, segment as u64, 0, end, end),
            Blob::EMPTY,
        );
        first = end + 1;
    }
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let bounds = span(0, 99);
    let members = [span(5, 30), span(60, 85)];
    let sparse = [span(45, 50)];
    let views = [
        (0.0, 99.0, 100, span(0, 99)),
        (35.0, 70.0, 36, span(35, 70)),
        (-3.5, 120.5, 125, span(0, 99)),
    ];
    let mut compared = 0;
    for &key in &keys {
        for membership in [None, Some(&members[..]), Some(&sparse[..])] {
            for (from, to, width, grid) in views {
                let mut listed: Vec<Run> = Vec::new();
                for hour in grid.first..=grid.last {
                    if let Observation::Value { run, .. } = snapshot.value_at(key, hour).unwrap() {
                        if listed.last().is_none_or(|p| p.span.first != run.span.first) {
                            listed.push(run);
                        }
                    }
                }
                let mut expected = Vec::new();
                for run in &listed {
                    match membership {
                        None => expected.push((run.span, run.value, run.break_before)),
                        Some(spans) => {
                            for part in spans.iter().filter_map(|m| m.intersect(run.span)) {
                                expected.push((
                                    part,
                                    run.value,
                                    run.break_before || part.first != run.span.first,
                                ));
                            }
                        }
                    }
                }
                let selected = Selection { key, membership };
                let view = Viewport { from, to, width };
                for bytes in [Run::SIZE, 3 * Run::SIZE, 1 << 16] {
                    assert_eq!(
                        hourly_pages(&mut snapshot, selected, bounds, view, bytes),
                        expected,
                        "key {key} membership {membership:?} view {view:?} bytes {bytes}"
                    );
                }
                compared += expected.len();
            }
        }
    }
    assert!(compared > 50);
}

#[test]
fn fresh_snapshot_pages_at_one_date_decode_bounded_rows_in_both_streams() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::Exact, 0)
        .unwrap();
    let count = 2048;
    let mut expected = Vec::new();
    let mut root = 0;
    for (native, read, segment) in [(7, 7, 0), (5, 8, 1)] {
        for _ in 0..count {
            let sequence = writer.sequence().unwrap();
            writer
                .event(Event {
                    key,
                    native_hour: native,
                    read_hour: Some(read),
                    sequence,
                    kind: 1,
                    fields: Blob::EMPTY,
                })
                .unwrap();
            expected.push((native, sequence));
        }
        root = publish(
            &mut writer,
            &mut memory,
            &cache,
            publication(segment + 1, segment, 0, read, read),
            Blob::EMPTY,
        );
    }
    expected.sort_unstable();
    let selected = [selection(key)];
    let query = EventQuery {
        selection: &selected,
        native: span(0, 8),
        as_of: None,
        order: EventOrder::Native,
    };
    let decoded = || crate::read::DECODED_EVENTS.with(|decoded| decoded.get());
    let mut rows = Vec::new();
    let mut cursor = None;
    let (mut most, mut total, mut pages) = (0, 0, 0);
    loop {
        let before = decoded();
        let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
        let page = snapshot
            .event_page(query, cursor, Event::SIZE + 1, |row| {
                rows.push((row.native_hour(), row.sequence()));
                Ok(())
            })
            .unwrap();
        let used = decoded() - before;
        most = most.max(used);
        total += used;
        pages += 1;
        cursor = page.next;
        memory = snapshot.into_io();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows, expected);
    assert!(most <= 128, "{most} event rows decoded for one page");
    assert!(total <= 128 * pages, "{total} event rows decoded for {pages} pages");
}

#[test]
fn hourly_ordered_runs_decode_rows_linear_in_the_segment() {
    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let key = writer
        .intern(spec(1, 1), Blob::EMPTY, RunMode::OrderedContributions, 0)
        .unwrap();
    let n = 8192_i32;
    writer
        .evidence(key, known(0, n - 1, None, None), None)
        .unwrap();
    for i in 0..n {
        writer
            .run(key, span(i, n - 1), i128::from(i), true, false)
            .unwrap();
    }
    let root = publish(
        &mut writer,
        &mut memory,
        &cache,
        publication(1, 0, 0, n - 1, n - 1),
        Blob::EMPTY,
    );
    let mut snapshot = Snapshot::open(memory, &cache, root, identity()).unwrap();
    let view = Viewport {
        from: 0.0,
        to: f64::from(n - 1),
        width: n as u32,
    };
    let expected: Vec<_> = (0..n)
        .map(|i| (span(i, n - 1), i128::from(i), true))
        .collect();
    let decoded = || crate::read::DECODED_RUNS.with(|decoded| decoded.get());
    for (bytes, per_run) in [(n as usize * Run::SIZE, 4), (64 * Run::SIZE, 8)] {
        let before = decoded();
        let runs = hourly_pages(&mut snapshot, selection(key), span(0, n - 1), view, bytes);
        let used = decoded() - before;
        assert_eq!(runs, expected);
        assert!(
            used <= per_run * n as u64,
            "{used} run rows decoded for {n} runs in pages of {bytes} bytes"
        );
    }
}

/// The bytes `fill` writes.
fn written(fill: impl Fn(&mut crate::codec::Sink) -> io::Result<()>) -> Vec<u8> {
    let mut sink = crate::codec::Sink::default();
    fill(&mut sink).unwrap();
    sink.as_slice().to_vec()
}

#[test]
fn malformed_deltas_lengths_and_indices_are_invalid_data_not_overflow() {
    use crate::{
        format::{
            BLOCK, Block, Detail, Directory, Entry, List, Node, Origin, Segment, State, Table,
            Track,
        },
        read::{self, Fragment},
        source::{Rows, Source},
    };
    let malformed = |result: io::Result<usize>| {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    };
    let block = Block {
        page: 0,
        offset: 0,
        rows: 1,
        first: 1,
        last: 1,
    };
    let mut runs = [Fragment::default(); BLOCK];
    for row in [
        written(|s| {
            s.zig(i64::MAX)?;
            s.var(0)?;
            s.u8(0)?;
            s.zig128(0)
        }),
        written(|s| {
            s.zig(0)?;
            s.var(i64::MAX as u64)?;
            s.u8(0)?;
            s.zig128(0)
        }),
    ] {
        malformed(read::runs(&row, 0, block, &mut runs));
    }
    let mut spans = [Span::default(); BLOCK];
    for row in [
        written(|s| {
            s.zig(i64::MAX)?;
            s.var(0)
        }),
        written(|s| {
            s.zig(0)?;
            s.var(i64::MAX as u64)
        }),
    ] {
        malformed(read::coverage(&row, 0, block, &mut spans));
    }
    let mut evidence = [(known(0, 0, None, None), false); BLOCK];
    for row in [
        written(|s| {
            s.zig(i64::MAX)?;
            s.var(0)?;
            s.u8(1)
        }),
        written(|s| {
            s.zig(0)?;
            s.var(i64::MAX as u64)?;
            s.u8(1)
        }),
        written(|s| {
            s.zig(0)?;
            s.var(0)?;
            s.u8(1 | 8)?;
            s.zig(i64::MIN)
        }),
        written(|s| {
            s.zig(0)?;
            s.var(0)?;
            s.u8(1 | 2)?;
            s.zig(i64::MAX)
        }),
    ] {
        malformed(read::evidence(&row, 0, block, &mut evidence));
    }
    let mut events = [EventView::Family(Event {
        key: 0,
        native_hour: 0,
        read_hour: None,
        sequence: 0,
        kind: 0,
        fields: Blob::EMPTY,
    }); BLOCK];
    for row in [
        written(|s| {
            s.zig(i64::MAX)?;
            s.zig(0)?;
            s.u8(0)?;
            s.var(0)?;
            s.var(0)
        }),
        written(|s| {
            s.zig(0)?;
            s.zig(0)?;
            s.u8(2)?;
            s.zig(i64::MAX)
        }),
        written(|s| {
            s.zig(0)?;
            s.zig(0)?;
            s.u8(1)?;
            s.var(i64::MAX as u64)
        }),
        written(|s| {
            s.zig(0)?;
            s.zig(0)?;
            s.u8(0)?;
            s.var(0)?;
            s.var(Cell::SIZE as u64)?;
            s.var(u64::MAX)
        }),
    ] {
        malformed(read::events(&row, 0, 1, block, &mut events));
    }
    let mut crowded = vec![0xff; 4];
    crowded.extend([0; 16]);
    malformed(read::entry_count(&crowded));

    let budget = Budget::new(BIG);
    let cache = Cache::new(&budget);
    let mut memory = Memory::default();
    let index = pages::seal(&mut memory, Kind::List, &[0; 8]).unwrap();
    let mut reader = read::Reader::detached(&cache);
    let list = List {
        root: index,
        levels: 1,
        pages: u64::MAX,
    };
    assert_eq!(
        reader
            .list_page(&mut memory, list, 1 << 62)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let directory = Directory {
        list: List::EMPTY,
        fences: Table::EMPTY,
        count: 0,
    };
    let node = Node {
        level: 0,
        first: 0,
        segments: 1,
        hours: span(0, 0),
        keys: (0, 0),
        heap: (0, 0),
        sequences: (0, 0),
        children: Vec::new(),
        entries: directory,
        domains: directory,
        late: Table::EMPTY,
        metadata: Table::EMPTY,
        segment: Some(Segment {
            state: State {
                publication: publication(1, 0, 0, 0, 0),
                next_key: 0,
                next_event: 0,
                heap: 0,
                checkpoint: Blob::EMPTY,
            },
            dictionary: Table::EMPTY,
            blocks: [Table::EMPTY; 4],
            data: List::EMPTY,
            heap: List::EMPTY,
        }),
    };
    let entry = Entry {
        runs: Track {
            count: 64,
            first: 0,
            last: 0,
            head: Default::default(),
        },
        detail: Some(Detail {
            blocks: [(u64::MAX, 2); 4],
            origin: Origin::default(),
            evidence_origin: 0,
            tail: None,
            evidence_tail: None,
        }),
        ..Entry::empty(0)
    };
    let mut defs = std::collections::HashMap::new();
    let mut s = Source {
        r: &mut reader,
        io: &mut memory,
        defs: &mut defs,
        accepted: span(0, 0),
    };
    let check = |kind: io::ErrorKind| assert_eq!(kind, io::ErrorKind::InvalidData);
    check(s.run_block(&node, &entry, 1).unwrap_err().kind());
    check(
        s.plain_block(&node, &entry, Rows::Evidence, 1)
            .unwrap_err()
            .kind(),
    );
    check(s.event_block(&node, &entry, 1).unwrap_err().kind());
}
