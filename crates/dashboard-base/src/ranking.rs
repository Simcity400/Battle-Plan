
use crate::span::HourSpan;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn for_each_piece<R>(
    runs: &[R],
    spans: &[HourSpan],
    span: impl Fn(&R) -> (i32, i32),
    cancelled: &AtomicBool,
    mut piece: impl FnMut(&R, i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The ranking was cancelled",
        ));
    }
    let (mut run_index, mut span_index) = (0, 0);
    let mut steps = 0usize;
    while let (Some(run), Some(selected)) = (runs.get(run_index), spans.get(span_index)) {
        steps += 1;
        if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        let (first, last) = span(run);
        let first_hour = first.max(selected.first_hour);
        let last_hour = last.min(selected.last_hour);
        if first_hour <= last_hour {
            piece(run, first_hour, last_hour)?;
        }
        if last < selected.last_hour {
            run_index += 1;
        } else {
            span_index += 1;
        }
    }
    Ok(())
}

pub fn joined_spans(mut spans: Vec<HourSpan>, first: i32, last: i32) -> Vec<HourSpan> {
    spans.sort_unstable_by_key(|span| (span.first_hour, span.last_hour));
    let mut joined: Vec<HourSpan> = Vec::new();
    for span in spans {
        let (first_hour, last_hour) = (span.first_hour.max(first), span.last_hour.min(last));
        if first_hour > last_hour {
            continue;
        }
        match joined.last_mut() {
            Some(held) if i64::from(first_hour) <= i64::from(held.last_hour) + 1 => {
                held.last_hour = held.last_hour.max(last_hour);
            }
            _ => joined.push(HourSpan {
                first_hour,
                last_hour,
            }),
        }
    }
    joined
}

pub fn within_spans(
    spans: &[HourSpan],
    within: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<Vec<HourSpan>> {
    let mut clipped = Vec::new();
    for_each_piece(
        spans,
        within,
        |span| (span.first_hour, span.last_hour),
        cancelled,
        |_, first_hour, last_hour| {
            clipped.push(HourSpan {
                first_hour,
                last_hour,
            });
            Ok(())
        },
    )?;
    Ok(clipped)
}

pub fn top_membership<R, N: Into<i128>>(
    series: &BTreeMap<String, Vec<R>>,
    count: usize,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<N>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    membership(
        series,
        count,
        span,
        |run| value(run).map(Into::into),
        true,
        false,
        cancelled,
        None,
        false,
    )
}

pub fn top_membership_exact<R>(
    series: &BTreeMap<String, Vec<R>>,
    count: usize,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    descending: bool,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    membership(
        series, count, span, value, descending, true, cancelled, None, false,
    )
}

pub fn rank_positions<R>(
    series: &BTreeMap<String, Vec<R>>,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    cancelled: &AtomicBool,
    mut position: impl FnMut(usize, &str, i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    membership(
        series,
        usize::MAX,
        span,
        value,
        true,
        false,
        cancelled,
        Some(&mut position),
        false,
    )?;
    Ok(())
}

pub fn rank_runs<R>(
    series: &BTreeMap<String, Vec<R>>,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    cancelled: &AtomicBool,
    mut position: impl FnMut(usize, &str, i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    RankPlan::new(series, span, cancelled)?.runs(value, cancelled, &mut position)
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum RankValue {
    Ascending(i128),
    Descending(Reverse<i128>),
    Unknown,
}
impl RankValue {
    fn new(amount: Option<i128>, descending: bool) -> Self {
        match amount {
            Some(amount) if descending => Self::Descending(Reverse(amount)),
            Some(amount) => Self::Ascending(amount),
            None => Self::Unknown,
        }
    }
}
pub fn compare(left: Option<i128>, right: Option<i128>, descending: bool) -> std::cmp::Ordering {
    RankValue::new(left, descending).cmp(&RankValue::new(right, descending))
}

struct RankEvent {
    hour: i64,
    starts: bool,
    index: usize,
}
type RankPosition<'a> = dyn FnMut(usize, &str, i32, i32) -> io::Result<()> + 'a;

fn membership<R>(
    series: &BTreeMap<String, Vec<R>>,
    count: usize,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    descending: bool,
    include_unknown: bool,
    cancelled: &AtomicBool,
    position: Option<&mut RankPosition<'_>>,
    coalesce: bool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The ranking was cancelled",
        ));
    }
    if count == 0 {
        return Ok(BTreeMap::new());
    }
    RankPlan::new(series, span, cancelled)?.membership(
        count,
        value,
        descending,
        include_unknown,
        cancelled,
        position,
        coalesce,
    )
}

pub struct RankPlan<'a, R> {
    tags: Vec<(&'a String, &'a Vec<R>)>,
    events: Vec<RankEvent>,
    steps: usize,
}
impl<'a, R> RankPlan<'a, R> {
    pub fn new(
        series: &'a BTreeMap<String, Vec<R>>,
        span: impl Fn(&R) -> (i32, i32),
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        let tags: Vec<(&String, &Vec<R>)> = series.iter().collect();
        let mut events = Vec::new();
        let mut steps = 0usize;
        for (index, runs) in series.values().enumerate() {
            for run in runs {
                steps += 1;
                if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "The ranking was cancelled",
                    ));
                }
                let (first, last) = span(run);
                events.push(RankEvent {
                    hour: i64::from(first),
                    starts: true,
                    index,
                });
                events.push(RankEvent {
                    hour: i64::from(last) + 1,
                    starts: false,
                    index,
                });
            }
        }
        events.sort_unstable_by_key(|event| (event.hour, event.starts));
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        Ok(Self {
            tags,
            events,
            steps,
        })
    }
    pub fn runs(
        &self,
        value: impl Fn(&R) -> Option<i128>,
        cancelled: &AtomicBool,
        mut position: impl FnMut(usize, &str, i32, i32) -> io::Result<()>,
    ) -> io::Result<()> {
        self.membership(
            usize::MAX,
            value,
            true,
            false,
            cancelled,
            Some(&mut position),
            true,
        )?;
        Ok(())
    }
    fn membership(
        &self,
        count: usize,
        value: impl Fn(&R) -> Option<i128>,
        descending: bool,
        include_unknown: bool,
        cancelled: &AtomicBool,
        mut position: Option<&mut RankPosition<'_>>,
        coalesce: bool,
    ) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        let tags = &self.tags;
        let events = &self.events;
        let mut steps = self.steps;
        let mut spans: Vec<Vec<HourSpan>> = vec![Vec::new(); tags.len()];
        let mut cursors = vec![0usize; tags.len()];
        let mut active = vec![None; tags.len()];
        let mut ranked = BTreeSet::new();
        let mut open = coalesce.then(|| vec![None::<(usize, i32, i32)>; tags.len()]);
        let mut event_index = 0;
        while let Some(event) = events.get(event_index) {
            let hour = event.hour;
            let boundary = event_index;
            while let Some(event) = events.get(event_index).filter(|event| event.hour == hour) {
                steps += 1;
                if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "The ranking was cancelled",
                    ));
                }
                if event.starts {
                    let amount = value(&tags[event.index].1[cursors[event.index]]);
                    let amount = (amount.is_some() || include_unknown)
                        .then(|| RankValue::new(amount, descending));
                    active[event.index] = amount;
                    if let Some(amount) = amount {
                        ranked.insert((amount, event.index));
                    }
                } else {
                    if let Some(amount) = active[event.index].take() {
                        ranked.remove(&(amount, event.index));
                    }
                    cursors[event.index] += 1;
                }
                event_index += 1;
            }
            if let (Some(open), Some(position)) = (&mut open, &mut position) {
                for event in &events[boundary..event_index] {
                    if active[event.index].is_none()
                        && let Some((rank, first, last)) = open[event.index].take()
                    {
                        position(rank, tags[event.index].0, first, last)?;
                    }
                }
            }
            let Some(next) = events.get(event_index) else {
                break;
            };
            let (start, end) = (hour as i32, (next.hour - 1) as i32);
            for (rank, (_, index)) in ranked.iter().take(count).enumerate() {
                steps += 1;
                if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "The ranking was cancelled",
                    ));
                }
                if let Some(position) = &mut position {
                    if let Some(open) = &mut open {
                        match &mut open[*index] {
                            Some((held, _, last))
                                if *held == rank && last.checked_add(1) == Some(start) =>
                            {
                                *last = end;
                            }
                            slot => {
                                if let Some((held, first, last)) = slot.take() {
                                    position(held, tags[*index].0, first, last)?;
                                }
                                *slot = Some((rank, start, end));
                            }
                        }
                    } else {
                        position(rank, tags[*index].0, start, end)?;
                    }
                    continue;
                }
                let list = &mut spans[*index];
                match list.last_mut() {
                    Some(span) if span.last_hour.checked_add(1) == Some(start) => {
                        span.last_hour = end
                    }
                    _ => list.push(HourSpan {
                        first_hour: start,
                        last_hour: end,
                    }),
                }
            }
        }
        if let (Some(open), Some(position)) = (open, &mut position) {
            for (index, held) in open.into_iter().enumerate() {
                if let Some((rank, first, last)) = held {
                    position(rank, tags[index].0, first, last)?;
                }
            }
        }
        Ok(spans
            .into_iter()
            .enumerate()
            .filter(|(_, spans)| !spans.is_empty())
            .map(|(index, spans)| (tags[index].0.clone(), spans))
            .collect())
    }
}

pub fn for_each_optional_piece<R>(
    runs: &[R],
    spans: Option<&[HourSpan]>,
    bounds: impl Fn(&R) -> (i32, i32),
    cancelled: &AtomicBool,
    mut piece: impl FnMut(&R, i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The ranking was cancelled",
        ));
    }
    if let Some(spans) = spans {
        return for_each_piece(runs, spans, bounds, cancelled, piece);
    }
    for (index, run) in runs.iter().enumerate() {
        if index.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        let (first, last) = bounds(run);
        piece(run, first, last)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
