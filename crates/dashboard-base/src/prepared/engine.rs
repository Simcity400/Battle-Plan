//! Replies a family answers from its series-engine publication ([`dashboard_series`]), in the
//! shapes this crate's prepared series reader gave them: chart summaries with their hourly
//! runs, points, clipped runs, keyboard navigation and event marks. The engine's differential
//! test holds both readers to the same answers on the same streams
//! (`series::engine_parity`).

use super::series::{MeasureKey, Navigation};
use crate::{
    prepared_chart::{ChartBucket, ChartRun, ChartSummary, MarkCluster, MarkSummary},
    products::country_groups::HourSpan,
};
use dashboard_series::{
    self as engine, EventQuery, PageRead, Selection, Snapshot, Viewport, query::Tail,
    summary::Summary,
};
use std::io;

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// `spans` as engine spans.
pub fn spans(spans: &[HourSpan]) -> io::Result<Vec<engine::Span>> {
    spans
        .iter()
        .map(|span| engine::Span::new(span.first_hour, span.last_hour))
        .collect()
}

/// A summary as the reader's chart bucket; a contribution past 128 bits is refused.
pub fn bucket(summary: Summary) -> io::Result<ChartBucket> {
    let total = summary.absolute_total.0;
    if total[2] != 0 || total[3] != 0 {
        return Err(invalid("A chart contribution exceeds 128 bits"));
    }
    Ok(ChartBucket {
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
    })
}

/// `key`'s chart over `view`, its bins laid from `bounds` (the key's first hour and the
/// publication's last), clipped to `within`; hourly views list their original runs.
pub fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    decimals: u8,
    within: Option<&[HourSpan]>,
) -> io::Result<ChartSummary> {
    let (from, to, width) = view;
    let membership = within.map(spans).transpose()?;
    let selection = Selection {
        key,
        membership: membership.as_deref(),
    };
    let bounds = engine::Span::new(bounds.0, bounds.1)?;
    let viewport = Viewport { from, to, width };
    let mut buckets = Vec::new();
    let mut cursor = None;
    let hourly = loop {
        let page = snapshot.summary_tiles(
            &[selection],
            bounds,
            viewport,
            Tail::EMPTY,
            cursor,
            BUDGET,
            |_, bin| {
                buckets.push(bin.exact.map(bucket).transpose()?);
                Ok(())
            },
        )?;
        cursor = page.page.next;
        if cursor.is_none() {
            break page.hourly;
        }
    };
    let runs = if hourly {
        let scale = 10_f64.powi(i32::from(decimals));
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page =
                snapshot.hourly_runs(selection, bounds, viewport, cursor, BUDGET, |run| {
                    runs.push(ChartRun {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                        value: run.value as f64 / scale,
                        break_before: run.break_before,
                    });
                    Ok(())
                })?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        Some(runs)
    } else {
        None
    };
    Ok(ChartSummary {
        from,
        to,
        width,
        buckets,
        runs,
        approximate: Vec::new(),
    })
}

/// The run holding `hour`, unclipped, with its evidence; None where the key holds no value.
pub fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<(engine::Run, engine::Evidence)>> {
    Ok(match snapshot.value_at(key, hour)? {
        engine::Observation::Value { run, evidence } => Some((run, evidence)),
        _ => None,
    })
}

/// [`run_at`]'s value at `hour`, with the hours [`run_at`] reads the same there: those its run
/// and evidence both hold within the accepted hours; `hour` alone where it holds no value.
pub fn held_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<(Option<i128>, (i32, i32))> {
    let accepted = snapshot.publication().accepted;
    Ok(match run_at(snapshot, key, hour)? {
        Some((run, evidence)) => (
            Some(run.value),
            (
                run.span
                    .first
                    .max(evidence.span.first)
                    .max(accepted.first)
                    .min(hour),
                run.span
                    .last
                    .min(evidence.span.last)
                    .min(accepted.last)
                    .max(hour),
            ),
        ),
        None => (None, (hour, hour)),
    })
}

/// `key`'s runs over `from..=to`, clipped to it.
pub fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    from: i32,
    to: i32,
) -> io::Result<Vec<engine::Run>> {
    let mut runs = Vec::new();
    if from > to {
        return Ok(runs);
    }
    let mut cursor = None;
    loop {
        let page = snapshot.run_page(
            Selection {
                key,
                membership: None,
            },
            engine::Span::new(from, to)?,
            cursor,
            BUDGET,
            |run| {
                runs.push(run);
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() {
            return Ok(runs);
        }
    }
}

/// One keyboard walk over `keys`, each within its membership, with the reached-hour fallback.
pub fn navigate<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &[(u64, Option<&[HourSpan]>)],
    navigation: &Navigation,
) -> io::Result<Option<f64>> {
    navigation.origin()?;
    let memberships = keys
        .iter()
        .map(|(_, within)| within.map(spans).transpose())
        .collect::<io::Result<Vec<_>>>()?;
    let selections: Vec<_> = keys
        .iter()
        .zip(&memberships)
        .map(|(&(key, _), membership)| Selection {
            key,
            membership: membership.as_deref(),
        })
        .collect();
    let direction = match navigation.key {
        MeasureKey::ArrowRight => engine::Direction::Forward,
        MeasureKey::ArrowLeft => engine::Direction::Backward,
        MeasureKey::Home => engine::Direction::Home,
        MeasureKey::End => engine::Direction::End,
    };
    snapshot.navigate_window(
        &selections,
        navigation.from,
        direction,
        (navigation.first_hour, navigation.last_hour),
    )
}

/// The marks of `key`'s events over the viewport.
pub fn clusters<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    (from, to, width): (f64, f64, u32),
) -> io::Result<MarkSummary> {
    let selection = [Selection {
        key,
        membership: None,
    }];
    let mut clusters = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.event_clusters(
            EventQuery {
                selection: &selection,
                native: engine::Span {
                    first: i32::MIN,
                    last: i32::MAX,
                },
                as_of: None,
                order: engine::EventOrder::Native,
            },
            Viewport { from, to, width },
            cursor,
            BUDGET,
            |cluster| {
                clusters.push(MarkCluster {
                    first_hour: cluster
                        .first_hour
                        .ok_or_else(|| invalid("A mark cluster without its first hour"))?,
                    last_hour: cluster
                        .last_hour
                        .ok_or_else(|| invalid("A mark cluster without its last hour"))?,
                    first_ordinal: cluster.first_ordinal.to_string(),
                    count: cluster.count.to_string(),
                });
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(MarkSummary {
        from,
        to,
        width,
        clusters,
    })
}
