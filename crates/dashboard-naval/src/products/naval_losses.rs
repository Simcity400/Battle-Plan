//! Country naval tallies from the shared sinkings scan. Baselines are not
//! lifetime totals: counts start at each table's first complete read. Missing
//! hours stay unknown; short breaks recover, while possible retention loss taints
//! running totals and only the calendar months where unseen records could prune.

use super::sinkings;
use dashboard_base::{
    catalogue,
    products::{
        MAX_PRODUCT_REPLY_BYTES, Product,
        country_groups::HourSpan,
        country_history::{self, CountryIdentityRun, Identity, RecordedHistories},
    },
    ranking,
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag, transport::json_len};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

use sinkings::{month, month_start};

pub struct NavalLosses;
pub mod prepared;
pub mod served;

const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NavalKind {
    Ships,
    Convoys,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NavalAttribution {
    Lost,
    SunkBy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NavalPeriod {
    Running,
    Month,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalLossesParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    pub kind: NavalKind,
    pub attribution: NavalAttribution,
    pub period: NavalPeriod,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalLossRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    #[ts(type = "number | null")]
    pub ships: Option<i64>,
    #[ts(type = "number | null")]
    pub convoys: Option<i64>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalLossSeries {
    pub tag: String,
    pub runs: Vec<NavalLossRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalLossesReply {
    pub branch_id: String,
    pub available: bool,
    pub first_read: Option<i32>,
    pub ship_first_read: Option<i32>,
    pub convoy_first_read: Option<i32>,
    pub ship_last_read: Option<i32>,
    pub convoy_last_read: Option<i32>,
    pub ship_reads: Vec<i32>,
    pub convoy_reads: Vec<i32>,
    pub series: Vec<NavalLossSeries>,
    pub warnings: Vec<String>,
    /// The first hour of each stretch after hours the recording lost: every tally restarts
    /// there, counting only what is read after them. Absent without any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<i32>>")]
    pub restarts: Vec<i32>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Events {
    ships: Vec<(i32, i64)>,
    ship_months: BTreeMap<i32, Vec<(i32, i64)>>,
    convoys: Vec<(i32, i32, i64)>,
    convoy_running: Vec<(i32, i64)>,
    convoy_months: BTreeMap<i32, Vec<(i32, i64)>>,
    unknown_convoys: Option<i32>,
    unknown_convoy_months: BTreeMap<i32, i32>,
    unknown_ship_months: BTreeMap<i32, i32>,
    unknown_ships: Vec<i32>,
    // Prepared continuation uses the same uncertainty discoveries as the oracle.
    ship_unknown_points: Vec<(i32, i32)>,
    convoy_unknown_points: Vec<(i32, i32)>,
    convoy_running_unknown: Vec<i32>,
    baseline_unknown: Option<i32>,
}

fn baseline(scanned: &sinkings::Scanned, kind: NavalKind) -> Option<i32> {
    match kind {
        NavalKind::Ships => scanned.stretch_ship_read,
        NavalKind::Convoys => scanned.stretch_convoy_read,
        NavalKind::Both => scanned
            .stretch_ship_read
            .zip(scanned.stretch_convoy_read)
            .map(|(s, c)| s.max(c)),
    }
}

fn prefix(points: &mut [(i32, i64)]) -> io::Result<()> {
    points.sort_unstable_by_key(|p| p.0);
    let mut total = 0_i64;
    for (_, amount) in points {
        total = total
            .checked_add(*amount)
            .ok_or_else(|| io::Error::other("Naval losses count exceeds i64 arithmetic"))?;
        *amount = total;
    }
    Ok(())
}
fn at(points: &[(i32, i64)], hour: i32) -> i64 {
    let end = points.partition_point(|p| p.0 <= hour);
    end.checked_sub(1).map_or(0, |i| points[i].1)
}

fn intersects(spans: &[HourSpan], first: i32, last: i32) -> bool {
    let end = spans.partition_point(|s| s.first_hour <= last);
    end.checked_sub(1)
        .is_some_and(|i| spans[i].last_hour >= first)
}

fn lost(spans: &[sinkings::HistoryLoss], first: i32, last: i32, hour: i32) -> bool {
    let end = spans.partition_point(|span| span.known_at <= hour);
    let end = spans[..end].partition_point(|span| span.first_hour <= last);
    end.checked_sub(1)
        .is_some_and(|i| spans[i].last_hour >= first)
}

fn known(
    first: Option<i32>,
    unread: &[HourSpan],
    loss: &[sinkings::HistoryLoss],
    hour: i32,
    period: NavalPeriod,
) -> bool {
    let Some(first) = first.filter(|&first| hour >= first) else {
        return false;
    };
    if intersects(unread, hour, hour) {
        return false;
    }
    let from = match period {
        NavalPeriod::Running => first,
        NavalPeriod::Month => month(hour).and_then(month_start).unwrap_or(first),
    };
    !lost(loss, from, hour, hour)
}

fn value(
    events: &Events,
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    hour: i32,
    loss: (&[sinkings::HistoryLoss], &[sinkings::HistoryLoss]),
) -> (Option<i64>, Option<i64>) {
    let bucket = month(hour);
    let first = baseline(scanned, params.kind);
    let ships = (known(
        if params.period == NavalPeriod::Running {
            first
        } else {
            scanned.stretch_ship_read
        },
        &scanned.ship_unread,
        loss.0,
        hour,
        params.period,
    ) && match params.period {
        NavalPeriod::Running => {
            let end = events.unknown_ships.partition_point(|&h| h <= hour);
            end.checked_sub(1)
                .is_none_or(|i| first.is_some_and(|first| events.unknown_ships[i] < first))
        }
        NavalPeriod::Month => bucket
            .and_then(|m| events.unknown_ship_months.get(&m))
            .is_none_or(|&read| read > hour),
    })
    .then(|| amounts(events, scanned, params, hour).0);
    let convoys = (known(
        if params.period == NavalPeriod::Running {
            first
        } else {
            scanned.stretch_convoy_read
        },
        &scanned.convoy_unread,
        loss.1,
        hour,
        params.period,
    ) && match params.period {
        NavalPeriod::Running => events.unknown_convoys.is_none_or(|h| h > hour),
        NavalPeriod::Month => bucket
            .and_then(|m| events.unknown_convoy_months.get(&m))
            .is_none_or(|&read| read > hour),
    })
    .then(|| amounts(events, scanned, params, hour).1);
    (ships, convoys)
}

fn amounts(
    events: &Events,
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    hour: i32,
) -> (i64, i64) {
    let bucket = month(hour);
    let first = baseline(scanned, params.kind);
    let ships = {
        if params.period == NavalPeriod::Running {
            at(&events.ships, hour) - at(&events.ships, first.unwrap_or(hour).saturating_sub(1))
        } else {
            bucket
                .and_then(|m| events.ship_months.get(&m))
                .map_or(0, |points| at(points, hour))
        }
    };
    let convoys = {
        if params.period == NavalPeriod::Running {
            at(&events.convoy_running, hour)
        } else {
            // The loss month is retained, but the bucket's amount is known only
            // from its supplying read. Never backfill a later read.
            bucket
                .and_then(|m| events.convoy_months.get(&m))
                .map_or(0, |points| at(points, hour))
        }
    };
    (ships, convoys)
}

fn note_unknown_month(months: &mut BTreeMap<i32, i32>, month: i32, read: i32) {
    months
        .entry(month)
        .and_modify(|first| *first = (*first).min(read))
        .or_insert(read);
}

#[cfg(test)]
fn reply_with_retention(
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    cancelled: &AtomicBool,
    safe: (bool, bool),
    loss: (&[sinkings::HistoryLoss], &[sinkings::HistoryLoss]),
    retention_warnings: Vec<String>,
) -> io::Result<NavalLossesReply> {
    reply_with_histories(
        scanned,
        params,
        cancelled,
        safe,
        loss,
        retention_warnings,
        None,
    )
}

struct Tallies {
    events: BTreeMap<Identity, Events>,
    warnings: Vec<String>,
    unresolved: usize,
    baseline_crossings: usize,
    error: Option<String>,
}
fn collect(
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    cancelled: &AtomicBool,
    retention_warnings: Vec<String>,
    events: BTreeMap<Identity, Events>,
) -> io::Result<Tallies> {
    collect_parts(
        scanned,
        scanned.ships.iter(),
        scanned.convoys.iter(),
        Some(uncertainties(scanned, params)),
        params,
        cancelled,
        retention_warnings,
        events,
    )
}
#[allow(clippy::too_many_arguments)]
fn collect_parts<'a>(
    scanned: &sinkings::Scanned,
    ships: impl Iterator<Item = &'a sinkings::ShipEvent>,
    convoys: impl Iterator<Item = &'a sinkings::ConvoyEvent>,
    uncertainty: Option<Uncertainty<'_>>,
    params: &NavalLossesParams,
    cancelled: &AtomicBool,
    retention_warnings: Vec<String>,
    mut events: BTreeMap<Identity, Events>,
) -> io::Result<Tallies> {
    // A native country retains its amounts across tag changes. A tag reused
    // by another native country never inherits its preceding holder's tally.
    let mut unresolved = 0usize;
    let mut error = None;
    let mut baseline_crossings = 0usize;
    let first_read = baseline(scanned, params.kind);
    for event in ships {
        #[cfg(test)]
        sinkings::work::WORK.with(|work| work.borrow_mut().events += 1);
        dashboard_text::metadata::check_cancel(cancelled)?;
        if params.kind == NavalKind::Convoys {
            continue;
        }
        if params.attribution == NavalAttribution::SunkBy
            && (event.entry.cause_record != 0 || event.entry.assisted_kill != 0)
        {
            continue;
        }
        let Some(first) = (if params.period == NavalPeriod::Month {
            scanned.stretch_ship_read
        } else {
            first_read
        }) else {
            continue;
        };
        // Positive retention is measured in whole years, so the first read's
        // retained vector covers its entire current month.
        let from = if params.period == NavalPeriod::Month {
            month(first).and_then(month_start).unwrap_or(first)
        } else {
            first
        };
        if event.entry.sink_hour < from {
            continue;
        }
        let (tag, as_of) = match params.attribution {
            NavalAttribution::Lost => (event.victim.as_ref(), event.victim_as_of),
            NavalAttribution::SunkBy => (event.attacker.as_ref(), event.attacker_as_of),
        };
        let raw = match params.attribution {
            NavalAttribution::Lost => event.entry.victim_country,
            NavalAttribution::SunkBy => event.entry.attacker_country,
        };
        let identity =
            tag.and_then(|tag| scanned.identity(tag, as_of.unwrap_or(event.entry.sink_hour)));
        if let Some(identity) = identity {
            let values = events.entry(identity).or_default();
            values.ships.push((event.read_hour, 1));
            if let Some(bucket) = month(event.entry.sink_hour) {
                values
                    .ship_months
                    .entry(bucket)
                    .or_default()
                    .push((event.read_hour, 1));
            }
        } else {
            if raw != 0
                && scanned
                    .country_candidates
                    .get(&raw)
                    .is_none_or(BTreeSet::is_empty)
            {
                error.get_or_insert_with(|| format!("Naval losses cannot identify possible holders of ship country reference {raw}"));
            }
            unresolved += 1;
        }
    }
    if let Some((uncertain, _)) = uncertainty
        && params.kind != NavalKind::Convoys
    {
        for (identity, hours) in uncertain {
            let values = events.entry(*identity).or_default();
            for &(sink, read) in hours {
                if first_read.is_some_and(|first| sink >= first) {
                    values.unknown_ships.push(read);
                }
                if let Some(bucket) = month(sink) {
                    values.ship_unknown_points.push((bucket, read));
                    note_unknown_month(&mut values.unknown_ship_months, bucket, read);
                }
            }
        }
    }
    for event in convoys {
        #[cfg(test)]
        sinkings::work::WORK.with(|work| work.borrow_mut().events += 1);
        dashboard_text::metadata::check_cancel(cancelled)?;
        if params.kind == NavalKind::Ships {
            continue;
        }
        let convoy_baseline = if params.period == NavalPeriod::Month {
            scanned.stretch_convoy_read
        } else {
            first_read
        };
        if params.period == NavalPeriod::Running
            && (event.baseline || convoy_baseline.is_none_or(|first| event.read_hour <= first))
        {
            continue;
        }
        // The two table baselines can differ. One unsplittable bucket delta
        // crossing the later baseline still cannot supply a since-baseline total.
        let crosses_baseline = params.kind == NavalKind::Both
            && params.period == NavalPeriod::Running
            && first_read.is_some_and(|first| {
                event.read_hour > first && event.previous_read.is_some_and(|read| read < first)
            });
        baseline_crossings += usize::from(crosses_baseline);
        let tag = match params.attribution {
            NavalAttribution::Lost => event.owner.as_ref(),
            NavalAttribution::SunkBy => event.killer.as_ref(),
        };
        if let Some(identity) = tag.and_then(|tag| scanned.identity(tag, event.read_hour)) {
            let values = events.entry(identity).or_default();
            values
                .convoys
                .push((event.read_hour, event.month, event.count));
            if crosses_baseline {
                values.baseline_unknown = Some(
                    values
                        .baseline_unknown
                        .map_or(event.read_hour, |h| h.min(event.read_hour)),
                );
                values.unknown_convoys = Some(
                    values
                        .unknown_convoys
                        .map_or(event.read_hour, |h| h.min(event.read_hour)),
                );
            }
        } else {
            let raw = if params.attribution == NavalAttribution::Lost {
                event.owner_raw
            } else {
                event.killer_raw
            };
            if raw != 0
                && scanned
                    .country_candidates
                    .get(&raw)
                    .is_none_or(BTreeSet::is_empty)
            {
                error.get_or_insert_with(|| format!("Naval losses cannot identify possible holders of convoy country reference {raw}"));
            }
            unresolved += 1;
        }
    }
    if let Some((_, uncertain)) = uncertainty
        && params.kind != NavalKind::Ships
    {
        for (identity, points) in uncertain {
            let values = events.entry(*identity).or_default();
            for &(bucket, read) in points {
                values.convoy_unknown_points.push((bucket, read));
                if first_read.is_some_and(|first| read > first) {
                    values.convoy_running_unknown.push(read);
                    values.unknown_convoys =
                        Some(values.unknown_convoys.map_or(read, |first| first.min(read)));
                }
                note_unknown_month(&mut values.unknown_convoy_months, bucket, read);
            }
        }
    }
    // The oracle refuses unidentified operands before doing arithmetic. Keep
    // that error precedence while preparation retains the pending attribution.
    let prefix = |points: &mut Vec<(i32, i64)>| {
        prefix(points).map_err(|failure| {
            error
                .as_ref()
                .map_or(failure, |message| io::Error::other(message.clone()))
        })
    };
    for values in events.values_mut() {
        for &(hour, bucket, count) in &values.convoys {
            values.convoy_running.push((hour, count));
            values
                .convoy_months
                .entry(bucket)
                .or_default()
                .push((hour, count));
        }
        values.unknown_ships.sort_unstable();
        prefix(&mut values.ships)?;
        for points in values.ship_months.values_mut() {
            prefix(points)?;
        }
        prefix(&mut values.convoy_running)?;
        for points in values.convoy_months.values_mut() {
            prefix(points)?;
        }
    }
    let warnings = tally_warnings(scanned, retention_warnings, unresolved, baseline_crossings);
    Ok(Tallies {
        events,
        warnings,
        unresolved,
        baseline_crossings,
        error,
    })
}

type Uncertainty<'a> = (
    &'a BTreeMap<Identity, Vec<(i32, i32)>>,
    &'a BTreeMap<Identity, Vec<(i32, i32)>>,
);
fn uncertainties<'a>(
    scanned: &'a sinkings::Scanned,
    params: &NavalLossesParams,
) -> Uncertainty<'a> {
    if params.attribution == NavalAttribution::Lost {
        (&scanned.uncertain_victims, &scanned.uncertain_convoy_owners)
    } else {
        (
            &scanned.uncertain_attackers,
            &scanned.uncertain_convoy_killers,
        )
    }
}

fn tally_warnings(
    scanned: &sinkings::Scanned,
    retention: Vec<String>,
    unresolved: usize,
    baseline_crossings: usize,
) -> Vec<String> {
    let mut warnings = scanned.warnings.clone();
    warnings.extend(retention);
    if unresolved > 0 {
        warnings.push(format!("{unresolved} naval changes lack a country identity; only identified possible holders have unknown tallies."));
    }
    if baseline_crossings > 0 {
        warnings.push(format!("{baseline_crossings} convoy changes straddle the common running baseline; the affected convoy total is unknown."));
    }
    warnings
}

fn reply_with_histories(
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    cancelled: &AtomicBool,
    safe: (bool, bool),
    loss: (&[sinkings::HistoryLoss], &[sinkings::HistoryLoss]),
    retention_warnings: Vec<String>,
    histories: Option<&RecordedHistories>,
) -> io::Result<NavalLossesReply> {
    let Tallies {
        events,
        warnings,
        error,
        ..
    } = collect(
        scanned,
        params,
        cancelled,
        retention_warnings,
        BTreeMap::new(),
    )?;
    if let Some(error) = error {
        return Err(io::Error::other(error));
    }
    let mut bounds = BTreeSet::from([scanned.first_hour, scanned.last_hour.saturating_add(1)]);
    for spans in [&scanned.ship_unread, &scanned.convoy_unread] {
        for span in spans {
            bounds.insert(span.first_hour);
            bounds.insert(span.last_hour.saturating_add(1));
        }
    }
    for hour in [scanned.first_ship_read, scanned.first_convoy_read]
        .into_iter()
        .flatten()
    {
        bounds.insert(hour);
    }
    if params.period == NavalPeriod::Month
        && let (Some(mut m), Some(last)) = (month(scanned.first_hour), month(scanned.last_hour))
    {
        while m <= last {
            if let Some(h) = month_start(m) {
                bounds.insert(h.max(scanned.first_hour));
            }
            m += 1;
        }
    }
    let selected_ids = selected_ids(scanned, params, histories);
    let mut series = BTreeMap::new();
    for (tag, segments) in &scanned.segments {
        let identities = histories.and_then(|histories| histories.countries.get(tag));
        let selected_spans: Option<Vec<_>> = selected_ids.as_ref().map(|selected| {
            if histories.is_some() {
                identities
                    .into_iter()
                    .flat_map(|country| country.identities.iter())
                    .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect()
            } else {
                scanned
                    .country_identities
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .filter(|span| selected.contains(&span.identity))
                    .map(|span| HourSpan {
                        first_hour: span.first_hour,
                        last_hour: span.last_hour,
                    })
                    .collect()
            }
        });
        if selected_spans.as_ref().is_some_and(Vec::is_empty) {
            continue;
        }
        let segments = selected_spans.as_deref().unwrap_or(segments);
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Naval losses cancelled",
            ));
        }
        let empty = Events::default();
        let mut tag_bounds = bounds.clone();
        let identities: BTreeSet<_> = scanned
            .country_identities
            .get(tag)
            .into_iter()
            .flatten()
            .map(|span| span.identity)
            .collect();
        for identity in identities {
            if let Some(events) = events.get(&identity) {
                tag_bounds.extend(events.ships.iter().map(|p| p.0));
                tag_bounds.extend(events.unknown_ships.iter().copied());
                tag_bounds.extend(events.ship_months.values().flatten().map(|p| p.0));
                tag_bounds.extend(events.unknown_ship_months.values().copied());
                tag_bounds.extend(events.unknown_convoy_months.values().copied());
                tag_bounds.extend(events.unknown_convoys);
                tag_bounds.extend(events.convoys.iter().map(|p| p.0));
            }
        }
        for span in segments {
            tag_bounds.insert(span.first_hour);
            tag_bounds.insert(span.last_hour.saturating_add(1));
        }
        let tag_bounds: Vec<_> = tag_bounds.into_iter().collect();
        let mut runs: Vec<NavalLossRun> = Vec::new();
        for pair in tag_bounds.windows(2) {
            let first = pair[0].max(scanned.first_hour);
            let last = (pair[1] - 1).min(scanned.last_hour);
            let end = segments.partition_point(|span| span.first_hour <= first);
            if first > last
                || end
                    .checked_sub(1)
                    .is_none_or(|at| segments[at].last_hour < first)
            {
                continue;
            }
            let identity = scanned.identity(tag, first);
            let events = identity
                .and_then(|identity| events.get(&identity))
                .unwrap_or(&empty);
            let (mut ships, mut convoys) = value(events, scanned, params, first, loss);
            if identity.is_none() {
                ships = None;
                convoys = None;
            }
            if !safe.0 {
                ships = None;
            }
            if !safe.1 {
                convoys = None;
            }
            // JavaScript Number representation, not a recording-size cap.
            let amount = match params.kind {
                NavalKind::Ships => ships,
                NavalKind::Convoys => convoys,
                NavalKind::Both => ships.zip(convoys).and_then(|(s, c)| s.checked_add(c)),
            };
            if [ships, convoys, amount]
                .into_iter()
                .flatten()
                .any(|n| !(-JS_MAX_SAFE_INTEGER..=JS_MAX_SAFE_INTEGER).contains(&n))
            {
                return Err(io::Error::other(
                    "Naval losses count exceeds JavaScript MAX_SAFE_INTEGER",
                ));
            }
            let starts_segment = segments
                .binary_search_by_key(&first, |span| span.first_hour)
                .is_ok();
            if let Some(run) = runs.last_mut()
                && !starts_segment
                && run.last_hour.checked_add(1) == Some(first)
                && run.ships == ships
                && run.convoys == convoys
            {
                run.last_hour = last;
            } else {
                let starts_segment = starts_segment
                    || runs
                        .last()
                        .is_none_or(|r| r.last_hour.checked_add(1) != Some(first));
                runs.push(NavalLossRun {
                    first_hour: first,
                    last_hour: last,
                    starts_segment,
                    ships,
                    convoys,
                });
            }
        }
        series.insert(tag.clone(), runs);
    }
    project_reply(scanned, params, cancelled, warnings, histories, series)
}

fn selected_ids(
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    histories: Option<&RecordedHistories>,
) -> Option<BTreeSet<Identity>> {
    params.top.is_none().then(|| {
        histories.map_or_else(
            || {
                params
                    .tags
                    .iter()
                    .filter_map(|tag| scanned.country_identities.get(tag))
                    .flatten()
                    .map(|span| span.identity)
                    .collect()
            },
            |histories| country_history::selected_identities(histories, &params.tags),
        )
    })
}

fn project_reply(
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    cancelled: &AtomicBool,
    warnings: Vec<String>,
    histories: Option<&RecordedHistories>,
    series: BTreeMap<String, Vec<NavalLossRun>>,
) -> io::Result<NavalLossesReply> {
    let first_read = baseline(scanned, params.kind);
    let selected_ids = selected_ids(scanned, params, histories);
    let memberships = params
        .top
        .map(|top| {
            ranking::top_membership(
                &series,
                top as usize,
                |r| (r.first_hour, r.last_hour),
                |r| match params.kind {
                    NavalKind::Ships => r.ships,
                    NavalKind::Convoys => r.convoys,
                    NavalKind::Both => r.ships.zip(r.convoys).and_then(|(s, c)| s.checked_add(c)),
                },
                cancelled,
            )
        })
        .transpose()?;
    let mut reply = NavalLossesReply {
        branch_id: params.branch_id.clone(),
        available: first_read.is_some(),
        first_read,
        ship_first_read: scanned.first_ship_read,
        convoy_first_read: scanned.first_convoy_read,
        ship_last_read: scanned.last_ship_read,
        convoy_last_read: scanned.last_convoy_read,
        ship_reads: Vec::new(),
        convoy_reads: Vec::new(),
        series: Vec::new(),
        restarts: Vec::new(),
        warnings,
    };
    let budget_error = || io::Error::other("Naval losses reply exceeds the transport budget");
    let mut bytes = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(budget_error)?;
    for reads in [&scanned.ship_reads, &scanned.convoy_reads] {
        bytes += json_len(reads, MAX_PRODUCT_REPLY_BYTES)
            .ok_or_else(budget_error)?
            .saturating_sub(2);
        if bytes > MAX_PRODUCT_REPLY_BYTES {
            return Err(budget_error());
        }
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SeriesMeasure<'a> {
        tag: &'a str,
        runs: &'a [NavalLossRun],
        segments: &'a [HourSpan],
        membership: Option<&'a [HourSpan]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identities: Option<&'a [&'a CountryIdentityRun]>,
    }
    let mut projected = Vec::new();
    for (tag, runs) in series {
        let membership = memberships.as_ref().and_then(|m| m.get(&tag)).cloned();
        if memberships.is_some() && membership.is_none() {
            continue;
        }
        let runs = if let Some(spans) = &membership {
            let mut clipped = Vec::new();
            ranking::for_each_piece(
                &runs,
                spans,
                |r| (r.first_hour, r.last_hour),
                cancelled,
                |run, first, last| {
                    let mut run = run.clone();
                    run.first_hour = first;
                    run.last_hour = last;
                    clipped.push(run);
                    Ok(())
                },
            )?;
            clipped
        } else {
            runs
        };
        // Native identity is independent of sinks. Manual roots contribute only
        // their selected holders' alias intervals, never every holder of an alias.
        let identities = histories.map(|histories| {
            histories
                .countries
                .get(&tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| {
                    selected_ids.as_ref().is_none_or(|selected| {
                        selected.contains(&(identity.slot, identity.country_id))
                    })
                })
                .collect::<Vec<_>>()
        });
        let segments: Vec<_> = identities.as_ref().map_or_else(
            || {
                scanned
                    .country_identities
                    .get(&tag)
                    .into_iter()
                    .flatten()
                    .filter(|span| {
                        selected_ids
                            .as_ref()
                            .is_none_or(|selected| selected.contains(&span.identity))
                    })
                    .map(|span| HourSpan {
                        first_hour: span.first_hour,
                        last_hour: span.last_hour,
                    })
                    .collect()
            },
            |identities| {
                identities
                    .iter()
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect()
            },
        );
        bytes += usize::from(!projected.is_empty());
        bytes += json_len(
            &SeriesMeasure {
                tag: &tag,
                runs: &runs,
                segments: &segments,
                membership: membership.as_deref(),
                identities: identities.as_deref(),
            },
            MAX_PRODUCT_REPLY_BYTES.saturating_sub(bytes),
        )
        .ok_or_else(budget_error)?;
        projected.push((tag, runs, segments, membership, identities));
    }
    for (tag, runs, segments, membership, identities) in projected {
        reply.series.push(NavalLossSeries {
            tag,
            runs,
            segments,
            membership,
            identities: identities.map(|identities| identities.into_iter().cloned().collect()),
        });
    }
    reply.ship_reads.clone_from(&scanned.ship_reads);
    reply.convoy_reads.clone_from(&scanned.convoy_reads);
    if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "Naval losses reply exceeds the transport budget",
        ));
    }
    Ok(reply)
}

pub fn retention(
    scanned: &sinkings::Scanned,
    catalogue: Result<&catalogue::Catalogue, String>,
) -> sinkings::Retention {
    match catalogue {
        Ok(cat) => sinkings::retention(scanned, Some(cat)),
        Err(error) => sinkings::Retention {
            ships: false,
            convoys: false,
            ship_loss: Vec::new(),
            convoy_loss: Vec::new(),
            warnings: vec![format!(
                "Naval retention defines are unavailable: {error}; completeness of country tallies is unknown."
            )],
        },
    }
}

impl Product for NavalLosses {
    const KEY: &'static str = "naval-losses";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = NavalLossesParams;
    type Reply = NavalLossesReply;
    fn types() -> Vec<String> {
        vec![
            NavalKind::decl(),
            NavalAttribution::decl(),
            NavalPeriod::decl(),
            NavalLossesParams::decl(),
            NavalLossRun::decl(),
            NavalLossSeries::decl(),
            NavalLossesReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err()
            || params.top == Some(0)
            || (params.top.is_some() && !params.tags.is_empty())
        {
            return Err("Invalid naval loss selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scanned = sinkings::cached(
            source,
            params.branch_id.parse().expect("validated branch"),
            cancelled,
        )?;
        let cat = context
            .content()
            .and_then(|content| catalogue::catalogue(source, content));
        let retention = retention(&scanned, cat.as_deref().map_err(ToString::to_string));
        reply_with_histories(
            &scanned,
            params,
            cancelled,
            (retention.ships, retention.convoys),
            (&retention.ship_loss, &retention.convoy_loss),
            retention.warnings,
            Some(&scanned.histories),
        )
    }
}

#[cfg(test)]
fn reply(
    scanned: &sinkings::Scanned,
    params: &NavalLossesParams,
    cancelled: &AtomicBool,
) -> io::Result<NavalLossesReply> {
    reply_with_retention(
        scanned,
        params,
        cancelled,
        (true, true),
        (&[], &[]),
        Vec::new(),
    )
}

#[cfg(test)]
mod tests;
