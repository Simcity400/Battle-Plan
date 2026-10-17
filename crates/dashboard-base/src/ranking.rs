//! The top N countries at every hour, for every product that ranks countries by
//! a value held over runs of hours (the race widgets: Factories, Equipment
//! produced). The product ranks here, so a reply carries only the countries ever
//! among the top N, each with the hours it was.

use crate::products::country_groups::HourSpan;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Visit the intersection of ordered, nonoverlapping runs and spans in one
/// linear pass. A clipped piece retains its original supplying run; gaps and
/// read dates remain the caller's recorded facts. The visitor can refuse a
/// budget before copying the piece.
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

/// The hours each country was among the top N, by tag: at every hour a value can
/// change (a run starting or ending in any series), every country with a known
/// value then is ranked (highest first, ties by tag), so between two such hours
/// the top N stay the same. A country without a run at an hour, or whose value
/// there is unknown (`value` gives none), is not ranked there, never as zero.
///
/// Each series' runs are in hour order and do not overlap; `span` gives a run's
/// first and last hour.
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

/// Exact table ranking in either direction. An active run with an unknown value
/// sorts after all known values; no run means the country is absent. Tags break
/// ties in recorded UTF-8 order. Callers supply identity spans independently of
/// their measure's coverage, so missing data never becomes a recorded zero.
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

/// Publish rank positions from the same sweep and comparison used by top membership.
/// Callbacks retain every input run boundary, in hour order and then rank
/// order: callers select native instances at the interval's first hour, and
/// an unseeded series batch takes its first hour from its first callback.
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

/// Publish maximal adjacent spans of the same country at the same rank.
/// Unlike `rank_positions`, input boundaries that leave positions unchanged
/// make no callback. Callers must seed their batch bounds independently and
/// use country keys that do not depend on a span's first hour.
pub fn rank_runs<R>(
    series: &BTreeMap<String, Vec<R>>,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    cancelled: &AtomicBool,
    mut position: impl FnMut(usize, &str, i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    RankPlan::new(series, span, cancelled)?.runs(value, cancelled, &mut position)
}

// Reverse compares directly, so descending values retain both i128 extrema.
// Every set uses one direction; either known variant precedes Unknown.
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
/// The same exact, unknown-last comparison for a table at one recorded hour.
pub fn compare(left: Option<i128>, right: Option<i128>, descending: bool) -> std::cmp::Ordering {
    RankValue::new(left, descending).cmp(&RankValue::new(right, descending))
}

struct RankEvent {
    hour: i64,
    starts: bool,
    index: usize,
}
type RankPosition<'a> = dyn FnMut(usize, &str, i32, i32) -> io::Result<()> + 'a;

#[allow(
    clippy::too_many_arguments,
    reason = "The shared sweep also supplies prepared rank positions"
)]
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

/// One sorted boundary plan shared by several ranking channels over the
/// same runs. Values and rank state remain each channel's own.
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
        // Ends precede starts at the same boundary, including adjacent equal runs.
        // i64 keeps the one-past-i32::MAX boundary distinct from recorded hours.
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
    #[allow(
        clippy::too_many_arguments,
        reason = "The shared sweep supplies membership and prepared positions"
    )]
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
            // Changed countries update the set; its first N supply membership.
            // The index breaks equal-value ties by the BTreeMap's UTF-8 tag order.
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

/// A measure without selected spans keeps each recorded run intact; otherwise
/// use the shared clipped walk, so every product has the same gap boundaries.
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
