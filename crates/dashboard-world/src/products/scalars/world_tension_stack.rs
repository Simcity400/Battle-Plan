use super::{Named, rows};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget}};
use dashboard_query::{
    log::{self, Cell},
    read,
    series::{MissingSpan, PAGE_LIMIT},
};
use dashboard_text::metadata::link_slot;
use engine_stability::{cthreat_source, cworld_threat};
use replay_columnar::{manifest::Status, summary::Source as ColumnSource};
use replay_data::rows::{TableId, TagHolderRow, ThreatEventRow, ThreatSourceRow, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct WorldTensionStack;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum TensionValueMode {
    #[default]
    Share,
    Recorded,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldTensionStackParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub layers: u32,
    #[serde(default)]
    pub values: TensionValueMode,
    #[serde(default)]
    pub countries: Option<Vec<String>>,
    #[serde(default)]
    pub width: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionStackBand {
    pub slot: u32,
    pub tag: Option<String>,
    pub remembered: bool,
    pub values: Vec<Option<f64>>,
    pub min_values: Vec<Option<f64>>,
    pub max_values: Vec<Option<f64>>,
    pub end_value: Option<f64>,
    pub end_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionStackOther {
    pub count: u32,
    pub values: Vec<Option<f64>>,
    pub end_value: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionStackEvent {
    pub hour: i32,
    pub slot: Option<u32>,
    pub second_country: u32,
    pub place: u32,
    pub tag: Option<String>,
    pub remembered: bool,
    pub reason: String,
    pub added_raw: String,
    pub first_read: i32,
    pub last_read: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorldTensionStackReply {
    pub branch_id: String,
    pub tension_scale: u32,
    pub contribution_scale: u32,
    pub ceiling_raw: String,
    pub end_hour: Option<i32>,
    pub capped_end_tension: Option<f64>,
    pub downsampled: bool,
    pub hours: Vec<i32>,
    pub total: Vec<Option<f64>>,
    pub bands: Vec<TensionStackBand>,
    pub other: Option<TensionStackOther>,
    pub events: Vec<TensionStackEvent>,
    pub missing: Vec<MissingSpan>,
    pub next_change_hour: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WorldTensionStackReply);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Identity {
    pub country: u32,
    pub second: u32,
    pub date: i32,
    pub reason: Vec<u8>,
    pub place: u32,
}

pub(super) enum Step {
    Stretch { first: i32, last: i32, listed: Option<Vec<(Identity, i64, Option<u32>)>> },
    Row { hour: i32, kind: u8, identity: Identity, final_raw: i64, slot: Option<u32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Joined {
    pub identity: Identity,
    pub slot: Option<u32>,
    pub added: Vec<(i32, i64)>,
    pub first_read: i32,
    pub last_read: Option<i32>,
    pub removed_at: Option<i32>,
}

pub(super) fn join(steps: impl IntoIterator<Item = Step>) -> (Vec<Joined>, Vec<i32>) {
    let mut events: Vec<Joined> = Vec::new();
    let mut last_seen: Vec<i32> = Vec::new();
    let mut open: HashMap<Identity, usize> = HashMap::new();
    let mut stretch_last: Option<i32> = None;
    let mut unmatched_hours: Vec<i32> = Vec::new();
    let added = |event: &mut Joined, hour: i32, value: i64| {
        if event.added.last().is_none_or(|&(_, last)| last != value) {
            event.added.push((hour, value));
        }
    };
    for step in steps {
        match step {
            Step::Stretch { first, last, listed: Some(listed) } => {
                let now: HashSet<&Identity> = listed.iter().map(|(identity, ..)| identity).collect();
                open.retain(|identity, at| {
                    let kept = now.contains(identity);
                    if !kept {
                        let seen = stretch_last.map_or(last_seen[*at], |s| s.max(last_seen[*at]));
                        events[*at].last_read = Some(seen);
                        events[*at].removed_at = Some(first);
                    }
                    kept
                });
                for (identity, value, slot) in listed {
                    match open.get(&identity) {
                        Some(&at) => {
                            added(&mut events[at], first, value);
                            last_seen[at] = last_seen[at].max(first);
                        }
                        None => {
                            open.insert(identity.clone(), events.len());
                            events.push(Joined {
                                identity,
                                slot,
                                added: vec![(first, value)],
                                first_read: first,
                                last_read: None,
                                removed_at: None,
                            });
                            last_seen.push(first);
                        }
                    }
                }
                stretch_last = Some(last);
            }
            Step::Stretch { listed: None, .. } => {}
            Step::Row { hour, kind, identity, final_raw, slot } => match (kind, open.get(&identity).copied()) {
                (ThreatEventRow::DROPPED, Some(at)) => {
                    events[at].last_read = Some(hour - 1);
                    events[at].removed_at = Some(hour);
                    open.remove(&identity);
                }
                (ThreatEventRow::DROPPED, None) => unmatched_hours.push(hour),
                (_, Some(at)) => {
                    added(&mut events[at], hour, final_raw);
                    last_seen[at] = last_seen[at].max(hour);
                }
                (kind, None) => {
                    if kind == ThreatEventRow::CHANGED {
                        unmatched_hours.push(hour);
                    }
                    open.insert(identity.clone(), events.len());
                    events.push(Joined {
                        identity,
                        slot,
                        added: vec![(hour, final_raw)],
                        first_read: hour,
                        last_read: None,
                        removed_at: None,
                    });
                    last_seen.push(hour);
                }
            },
        }
    }
    unmatched_hours.sort_unstable();
    (events, unmatched_hours)
}

fn listed(sources: &[ThreatSourceRow], slot_of: impl Fn(u32) -> Option<u32>) -> Vec<(Identity, i64, Option<u32>)> {
    let mut out: Vec<(Identity, i64, Option<u32>)> = Vec::with_capacity(sources.len());
    for source in sources {
        let place = out
            .iter()
            .filter(|(held, ..)| held.country == source.country && held.second == source.second_country && held.date == source.date && held.reason == source.reason)
            .count() as u32;
        out.push((
            Identity { country: source.country, second: source.second_country, date: source.date, reason: source.reason.clone(), place },
            source.final_raw,
            slot_of(source.country),
        ));
    }
    out
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column)? {
        Cell::Number(value) => Some(*value),
        _ => None,
    }
}

fn bytes(cells: &[Cell], column: usize) -> Option<Vec<u8>> {
    match cells.get(column)? {
        Cell::Text(text) => Some(text.clone().into_bytes()),
        Cell::List(bytes) => Some(bytes.clone()),
        _ => None,
    }
}

fn row_step(row: &log::Row) -> Option<Step> {
    let slot = (row.key >> 64) as u32;
    let field = |name: &str| dashboard_declare::field(TableId::ThreatEvents, name).ok();
    let cells = &row.cells;
    Some(Step::Row {
        hour: row.hour,
        kind: (row.key >> 96) as u8,
        identity: Identity {
            country: u32::try_from(number(cells, field("country")?)?).ok()?,
            second: u32::try_from(number(cells, field("second_country")?)?).ok()?,
            date: i32::try_from(number(cells, field("date")?)?).ok()?,
            reason: bytes(cells, field("reason")?)?,
            place: u32::try_from(number(cells, field("place")?)?).ok()?,
        },
        final_raw: number(cells, field("final_raw")?)?,
        slot: (slot != TagHolderRow::NONE).then_some(slot),
    })
}

struct CachedEvent {
    hour: i32,
    slot: Option<u32>,
    second_country: u32,
    place: u32,
    reason: String,
    added: Vec<(i32, i64)>,
    first_read: i32,
    last_read: Option<i32>,
    removed_at: Option<i32>,
}

struct BranchTension {
    tension: Vec<(i32, i32, i64)>,
    sum: Vec<(i32, i32, i64)>,
    slots: BTreeMap<u32, Vec<(i32, i32, Option<i64>)>>,
    aggregates_complete: Vec<(i32, i32)>,
    events: Vec<CachedEvent>,
    tension_missing: Vec<MissingSpan>,
    aggregates_missing: Vec<MissingSpan>,
    warnings: Vec<(i32, String)>,
    unreadable_hours: Vec<i32>,
    unmatched_hours: Vec<i32>,
}

#[derive(Default)]
struct Cache {
    branches: BTreeMap<u64, Arc<BranchTension>>,
    gates: BTreeMap<u64, Arc<Mutex<()>>>,
}

pub(crate) fn forget(source: &Source) {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.branches.clear();
    cache.gates.clear();
}

fn valid_span(recording: &replay_recording::columnar_frames::Recording, table: TableId, run: u32, first: i32, last: i32) -> (Option<(i32, i32)>, Option<MissingSpan>) {
    match recording.table_coverage(table.index(), run) {
        replay_recording::columnar_frames::TableCoverage::Valid => (Some((first, last)), None),
        replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => (None, Some(MissingSpan { first_hour: first, last_hour: last, reason: reason.to_owned() })),
        replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, reason } => {
            let valid = recording.run(run).map_or(i32::MAX, |held| held.first_hour.saturating_add(ordinal as i32));
            let missing = (first < valid).then(|| MissingSpan { first_hour: first, last_hour: last.min(valid - 1), reason: reason.to_owned() });
            let kept = (valid <= last).then(|| (first.max(valid), last));
            (kept, missing)
        }
    }
}

fn push_runs(target: &mut Vec<(i32, i32, i64)>, runs: &[(i32, i32, Option<i64>)], lo: i32, hi: i32) {
    for &(first, last, value) in runs {
        let Some(value) = value else { continue };
        let (first, last) = (first.max(lo), last.min(hi));
        if first > last {
            continue;
        }
        if let Some(prev) = target.last_mut()
            && prev.2 == value
            && i64::from(prev.1) + 1 == i64::from(first)
        {
            prev.1 = last;
        } else {
            target.push((first, last, value));
        }
    }
}

fn push_opt_runs(target: &mut Vec<(i32, i32, Option<i64>)>, runs: &[(i32, i32, Option<i64>)], lo: i32, hi: i32) {
    for &(first, last, value) in runs {
        let (first, last) = (first.max(lo), last.min(hi));
        if first > last {
            continue;
        }
        if let Some(prev) = target.last_mut()
            && prev.2 == value
            && i64::from(prev.1) + 1 == i64::from(first)
        {
            prev.1 = last;
        } else {
            target.push((first, last, value));
        }
    }
}

fn push_span(target: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    if first > last {
        return;
    }
    if let Some(prev) = target.last_mut()
        && i64::from(prev.1) + 1 >= i64::from(first)
    {
        prev.1 = prev.1.max(last);
    } else {
        target.push((first, last));
    }
}

fn merge_missing(mut spans: Vec<MissingSpan>) -> Vec<MissingSpan> {
    spans.sort_by_key(|span| (span.first_hour, span.last_hour));
    let mut merged: Vec<MissingSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.first_hour > span.last_hour {
            continue;
        }
        if let Some(prev) = merged.last_mut()
            && i64::from(span.first_hour) <= i64::from(prev.last_hour) + 1
            && prev.reason == span.reason
        {
            prev.last_hour = prev.last_hour.max(span.last_hour);
        } else {
            merged.push(span);
        }
    }
    merged
}

fn build_branch(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<BranchTension>> {
    let state = source.state::<Cache>();
    let gate = {
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(held) = cache.branches.get(&branch) {
            return Ok(Arc::clone(held));
        }
        cache.gates.entry(branch).or_default().clone()
    };
    let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(held) = state.lock().unwrap_or_else(|error| error.into_inner()).branches.get(&branch) {
        return Ok(Arc::clone(held));
    }
    let query = dashboard_base::queries::query(source, branch)?;
    let timeline = &query.timeline;
    let recording = query.replay.recording();
    let tension_col = dashboard_declare::field(TableId::WorldTension, "total_raw")?;
    let sum_col = dashboard_declare::field(TableId::ThreatTotals, "sum")?;
    let contrib_col = dashboard_declare::field(TableId::ThreatCountryAggregates, "total_raw")?;
    let mut tension = Vec::new();
    let mut sum = Vec::new();
    let mut slots: BTreeMap<u32, Vec<(i32, i32, Option<i64>)>> = BTreeMap::new();
    let mut aggregates_complete: Vec<(i32, i32)> = Vec::new();
    let unheld: Vec<MissingSpan> = timeline
        .unheld
        .iter()
        .map(|gap| MissingSpan { first_hour: gap.first, last_hour: gap.last, reason: gap.reason.clone() })
        .collect();
    let mut tension_missing = unheld.clone();
    let mut aggregates_missing = unheld;
    let mut warnings = Vec::new();
    for stretch in &timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (t_span, t_miss) = valid_span(recording, TableId::WorldTension, stretch.run, stretch.first, stretch.last);
        if let Some(span) = t_miss {
            tension_missing.push(span);
        }
        if let Some((lo, hi)) = t_span {
            let statuses = read::statuses(recording, stretch.run, TableId::WorldTension, lo, hi)?;
            let reasons = read::status_reasons(recording, stretch.run, TableId::WorldTension, &statuses)?;
            for status_run in &statuses {
                let (s_lo, s_hi) = (status_run.first.max(lo), status_run.last.min(hi));
                if s_lo > s_hi || matches!(status_run.status, Status::Complete) {
                    continue;
                }
                let reason = match status_run.status {
                    Status::Unavailable { reason } => reasons.get(&reason).cloned().unwrap_or_else(|| "World tension was not recorded at this hour.".into()),
                    Status::NotRecorded => "World tension was not recorded in this recording.".into(),
                    Status::Truncated { .. } => "World tension was incomplete at this hour.".into(),
                    Status::Complete => continue,
                };
                tension_missing.push(MissingSpan { first_hour: s_lo, last_hour: s_hi, reason });
            }
            if let Some(runs) = read::runs(recording, stretch.run, TableId::WorldTension, ColumnSource::Cell(tension_col), Some(&[0]), lo, hi)?.get(&0) {
                push_runs(&mut tension, runs, lo, hi);
            }
        }
        if let (Some((lo, hi)), _) = valid_span(recording, TableId::ThreatTotals, stretch.run, stretch.first, stretch.last)
            && let Some(runs) = read::runs(recording, stretch.run, TableId::ThreatTotals, ColumnSource::Cell(sum_col), Some(&[0]), lo, hi)?.get(&0)
        {
            push_runs(&mut sum, runs, lo, hi);
        }
        let (a_span, a_miss) = valid_span(recording, TableId::ThreatCountryAggregates, stretch.run, stretch.first, stretch.last);
        if let Some(span) = a_miss {
            aggregates_missing.push(span);
        }
        if let Some((lo, hi)) = a_span {
            let statuses = read::statuses(recording, stretch.run, TableId::ThreatCountryAggregates, lo, hi)?;
            let reasons = read::status_reasons(recording, stretch.run, TableId::ThreatCountryAggregates, &statuses)?;
            for status_run in &statuses {
                let (s_lo, s_hi) = (status_run.first.max(lo), status_run.last.min(hi));
                if s_lo > s_hi {
                    continue;
                }
                match status_run.status {
                    Status::Complete => push_span(&mut aggregates_complete, s_lo, s_hi),
                    Status::Unavailable { reason } => {
                        let text = reasons.get(&reason).cloned().unwrap_or_else(|| "Country threat contributions were not recorded at this hour.".into());
                        aggregates_missing.push(MissingSpan { first_hour: s_lo, last_hour: s_hi, reason: text });
                    }
                    Status::NotRecorded => aggregates_missing.push(MissingSpan {
                        first_hour: s_lo,
                        last_hour: s_hi,
                        reason: "Country threat contributions were not recorded in this recording.".into(),
                    }),
                    Status::Truncated { .. } => aggregates_missing.push(MissingSpan {
                        first_hour: s_lo,
                        last_hour: s_hi,
                        reason: "Country threat contributions were incomplete at this hour.".into(),
                    }),
                }
            }
            let held = read::runs(recording, stretch.run, TableId::ThreatCountryAggregates, ColumnSource::Cell(contrib_col), None, lo, hi)?;
            for (key, runs) in held {
                if let Ok(slot) = u32::try_from(key) {
                    push_opt_runs(slots.entry(slot).or_default(), &runs, lo, hi);
                }
            }
        }
    }
    let declared = dashboard_declare::log("threat-events").ok_or_else(|| dashboard_core::invalid("The threat events log is not declared"))?;
    let reader = log::Reader::new(&query, declared)?;
    let selection = reader.select(&[], timeline.first, timeline.last)?;
    let mut log_rows: Vec<log::Row> = Vec::new();
    let mut cursor = Some(0);
    while let Some(at) = cursor {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.page(&selection.patterns, selection.groups.len(), timeline.first, timeline.last, at, PAGE_LIMIT)?;
        cursor = page.cursor();
        log_rows.extend(page.rows.into_iter().map(|paged| paged.row));
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut iter = log_rows.into_iter().peekable();
    let mut unreadable_hours: Vec<i32> = Vec::new();
    for stretch in &timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let reads = [
            replay_lanes::at::Read { table: TableId::ThreatSources, columns: None, keys: replay_lanes::at::Keys::All },
            replay_lanes::at::Read { table: TableId::CountryLinks, columns: None, keys: replay_lanes::at::Keys::All },
        ];
        let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, stretch.run, stretch.first, &reads))?;
        let listed = match rows::<tables::ThreatSources>(&frame) {
            Ok(sources) => {
                let links = rows::<tables::CountryLinks>(&frame).ok();
                Some(listed(sources, |country| links.and_then(|links| link_slot(links, country))))
            }
            Err(error) => {
                warnings.push((stretch.first, format!("Threat sources listed at hour {} are not known: {error}", stretch.first)));
                None
            }
        };
        while let Some(row) = iter.next_if(|row| row.hour < stretch.first) {
            steps.extend(row_step(&row).or_else(|| {
                unreadable_hours.push(row.hour);
                None
            }));
        }
        steps.push(Step::Stretch { first: stretch.first, last: stretch.last, listed });
        while let Some(row) = iter.next_if(|row| row.hour <= stretch.last) {
            steps.extend(row_step(&row).or_else(|| {
                unreadable_hours.push(row.hour);
                None
            }));
        }
    }
    unreadable_hours.sort_unstable();
    let (joined, unmatched_hours) = join(steps);
    let events: Vec<CachedEvent> = joined
        .into_iter()
        .map(|event| CachedEvent {
            hour: event.identity.date,
            slot: event.slot,
            second_country: event.identity.second,
            place: event.identity.place,
            reason: dashboard_text::catalogue::display_text(&event.identity.reason).unwrap_or_else(|why| {
                let raw = String::from_utf8_lossy(&event.identity.reason).into_owned();
                warnings.push((event.first_read, format!("Threat reason {raw:?} {why}")));
                raw
            }),
            added: event.added,
            first_read: event.first_read,
            last_read: event.last_read,
            removed_at: event.removed_at,
        })
        .collect();
    warnings.sort_by_key(|(hour, _)| *hour);
    let built = Arc::new(BranchTension {
        tension,
        sum,
        slots,
        aggregates_complete,
        events,
        tension_missing: merge_missing(tension_missing),
        aggregates_missing: merge_missing(aggregates_missing),
        warnings,
        unreadable_hours,
        unmatched_hours,
    });
    state.lock().unwrap_or_else(|error| error.into_inner()).branches.insert(branch, Arc::clone(&built));
    Ok(built)
}

fn lookup_run(runs: &[(i32, i32, i64)], hour: i32) -> Option<i64> {
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at).filter(|run| run.0 <= hour && hour <= run.1).map(|run| run.2)
}

fn next_run_change(runs: &[(i32, i32, i64)], after: i32) -> Option<i32> {
    let at = runs.partition_point(|run| run.1 < after);
    match runs.get(at) {
        Some(&(first, last, _)) if first <= after && after <= last => last.checked_add(1).filter(|&next| next > after),
        Some(&(first, _, _)) => Some(first),
        None => None,
    }
}

fn is_complete(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.1 < hour);
    spans.get(at).is_some_and(|span| span.0 <= hour && hour <= span.1)
}

fn lookup_slot(runs: Option<&[(i32, i32, Option<i64>)]>, complete: bool, hour: i32) -> Option<i64> {
    if let Some(runs) = runs {
        let at = runs.partition_point(|run| run.1 < hour);
        if let Some(&(first, last, value)) = runs.get(at)
            && first <= hour
            && hour <= last
        {
            return value;
        }
    }
    complete.then_some(0)
}

fn next_opt_run_change(runs: &[(i32, i32, Option<i64>)], after: i32) -> Option<i32> {
    let at = runs.partition_point(|run| run.1 < after);
    match runs.get(at) {
        Some(&(first, last, _)) if first <= after && after <= last => last.checked_add(1).filter(|&next| next > after),
        Some(&(first, _, _)) => Some(first),
        None => None,
    }
}

fn next_span_change(spans: &[(i32, i32)], after: i32) -> Option<i32> {
    let at = spans.partition_point(|span| span.1 < after);
    match spans.get(at) {
        Some(&(first, last)) if first <= after && after <= last => last.checked_add(1).filter(|&next| next > after),
        Some(&(first, _)) => Some(first),
        None => None,
    }
}

fn collect_change_hours(runs: &[(i32, i32, i64)], first: i32, last: i32, out: &mut Vec<i32>) {
    let start = runs.partition_point(|run| run.1 < first);
    for &(r_first, r_last, _) in &runs[start..] {
        if r_first > last {
            break;
        }
        out.push(r_first.max(first));
        if r_last < last {
            out.push(r_last + 1);
        }
    }
}

fn collect_opt_change_hours(runs: &[(i32, i32, Option<i64>)], first: i32, last: i32, out: &mut Vec<i32>) {
    let start = runs.partition_point(|run| run.1 < first);
    for &(r_first, r_last, _) in &runs[start..] {
        if r_first > last {
            break;
        }
        out.push(r_first.max(first));
        if r_last < last {
            out.push(r_last + 1);
        }
    }
}

fn collect_span_change_hours(spans: &[(i32, i32)], first: i32, last: i32, out: &mut Vec<i32>) {
    let start = spans.partition_point(|span| span.1 < first);
    for &(s_first, s_last) in &spans[start..] {
        if s_first > last {
            break;
        }
        out.push(s_first.max(first));
        if s_last < last {
            out.push(s_last + 1);
        }
    }
}

pub(super) struct StackInput<'a> {
    pub tension: &'a [(i32, i32, i64)],
    pub sum: &'a [(i32, i32, i64)],
    pub slots: &'a BTreeMap<u32, Vec<(i32, i32, Option<i64>)>>,
    pub aggregates_complete: &'a [(i32, i32)],
    pub missing: &'a [MissingSpan],
}

pub(super) struct StackOutput {
    pub end_hour: Option<i32>,
    pub capped_end_tension: Option<f64>,
    pub downsampled: bool,
    pub hours: Vec<i32>,
    pub total: Vec<Option<f64>>,
    pub bands: Vec<TensionStackBand>,
    pub other: Option<TensionStackOther>,
    pub next_change_hour: Option<i32>,
}

pub(super) fn compute_stack(
    input: &StackInput<'_>,
    first: i32,
    last: i32,
    layers: usize,
    mode: TensionValueMode,
    filter: Option<&HashSet<&str>>,
    max_points: usize,
    mut slot_tag: impl FnMut(u32, i32) -> Option<(String, bool)>,
    mut slot_next_change: impl FnMut(u32, i32) -> Option<i32>,
) -> StackOutput {
    let tension_scale = (cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR) as f64;
    let contribution_scale = cthreat_source::FIXED_POINT_SCALE as f64;
    let mut next_change: Option<i32> = next_run_change(input.tension, last);
    let bump = |best: &mut Option<i32>, candidate: Option<i32>| {
        if let Some(hour) = candidate {
            *best = Some(best.map_or(hour, |held| held.min(hour)));
        }
    };
    bump(&mut next_change, next_run_change(input.sum, last));
    bump(&mut next_change, next_span_change(input.aggregates_complete, last));
    for (&slot, runs) in input.slots {
        bump(&mut next_change, next_opt_run_change(runs, last));
        bump(&mut next_change, slot_next_change(slot, last));
    }
    for span in input.missing {
        if span.first_hour > last {
            bump(&mut next_change, Some(span.first_hour));
            break;
        } else if span.last_hour >= last {
            bump(&mut next_change, span.last_hour.checked_add(1));
        }
    }
    let last_complete = is_complete(input.aggregates_complete, last);
    let has_contrib_at_last = last_complete
        || input.slots.values().any(|runs| lookup_slot(Some(runs), false, last).is_some());
    let rank_hour = if has_contrib_at_last {
        last
    } else {
        let mut latest: Option<i32> = None;
        let pos = input.aggregates_complete.partition_point(|span| span.0 <= last);
        if let Some(&(s_first, s_last)) = input.aggregates_complete[..pos].last() {
            let hi = s_last.min(last);
            if hi >= first && s_first <= last {
                latest = Some(hi);
            }
        }
        for runs in input.slots.values() {
            let pos = runs.partition_point(|run| run.0 <= last);
            if let Some(hi) = runs[..pos].iter().rev().find_map(|&(r_first, r_last, val)| {
                let hi = r_last.min(last);
                (val.is_some() && hi >= first && r_first <= last).then_some(hi)
            }) {
                latest = Some(latest.map_or(hi, |held| held.max(hi)));
            }
        }
        latest.unwrap_or(last)
    };
    let rank_complete = is_complete(input.aggregates_complete, rank_hour);
    let capped_end_tension = lookup_run(input.tension, last).map(|raw| raw as f64 / tension_scale);
    struct RankedSlot {
        slot: u32,
        end_abs: u64,
        end_signed: i64,
        peak_abs: u64,
        resolved: Option<(String, bool)>,
    }
    let mut ranked: Vec<RankedSlot> = Vec::new();
    for (&slot, runs) in input.slots {
        let resolved = slot_tag(slot, last);
        if let Some(wanted) = filter {
            let keep = resolved.as_ref().is_some_and(|(tag, _)| wanted.contains(tag.as_str()));
            if !keep {
                continue;
            }
        }
        let end_opt = lookup_slot(Some(runs), rank_complete, rank_hour);
        let end_signed = end_opt.unwrap_or(0);
        let end_abs = end_signed.unsigned_abs();
        let mut peak_abs = end_abs;
        let start = runs.partition_point(|run| run.1 < first);
        for &(r_first, _, value) in &runs[start..] {
            if r_first > last {
                break;
            }
            if let Some(val) = value {
                peak_abs = peak_abs.max(val.unsigned_abs());
            }
        }
        if end_abs > 0 || peak_abs > 0 {
            ranked.push(RankedSlot { slot, end_abs, end_signed, peak_abs, resolved });
        }
    }
    ranked.sort_by(|a, b| {
        b.end_abs
            .cmp(&a.end_abs)
            .then_with(|| b.end_signed.cmp(&a.end_signed))
            .then_with(|| b.peak_abs.cmp(&a.peak_abs))
            .then_with(|| a.slot.cmp(&b.slot))
    });
    let kept_count = ranked.len().min(layers);
    let top = &ranked[..kept_count];
    let tail = &ranked[kept_count..];
    let other_count = if filter.is_some() { 0 } else { tail.iter().filter(|entry| entry.end_abs > 0).count() as u32 };
    let has_other = filter.is_none() && !tail.is_empty();
    let mut raw_hours = Vec::new();
    raw_hours.push(first);
    collect_change_hours(input.tension, first, last, &mut raw_hours);
    collect_change_hours(input.sum, first, last, &mut raw_hours);
    collect_span_change_hours(input.aggregates_complete, first, last, &mut raw_hours);
    for runs in input.slots.values() {
        collect_opt_change_hours(runs, first, last, &mut raw_hours);
    }
    for span in input.missing {
        if span.last_hour < first || span.first_hour > last {
            continue;
        }
        raw_hours.push(span.first_hour.max(first));
        if span.last_hour < last {
            raw_hours.push(span.last_hour + 1);
        }
    }
    raw_hours.sort_unstable();
    raw_hours.dedup();
    struct Sample {
        hour: i32,
        total: Option<f64>,
        bands: Vec<Option<f64>>,
        band_min: Vec<Option<f64>>,
        band_max: Vec<Option<f64>>,
        other: Option<f64>,
    }
    let make_sample = |hour: i32, total: Option<f64>, bands: Vec<Option<f64>>, other: Option<f64>| Sample {
        hour,
        total,
        band_min: bands.clone(),
        band_max: bands.clone(),
        bands,
        other,
    };
    let evaluate = |hour: i32| -> Sample {
        let tension_raw = lookup_run(input.tension, hour);
        let sum_raw = lookup_run(input.sum, hour);
        let agg_complete = is_complete(input.aggregates_complete, hour);
        let top_raws: Vec<Option<i64>> = top
            .iter()
            .map(|entry| lookup_slot(input.slots.get(&entry.slot).map(Vec::as_slice), agg_complete, hour))
            .collect();
        let all_top_known = top_raws.iter().all(Option::is_some);
        let top_sum_raw: Option<i128> = all_top_known.then(|| top_raws.iter().flatten().map(|&raw| i128::from(raw)).sum());
        match mode {
            TensionValueMode::Share => {
                let total_pct = tension_raw.map(|raw| raw as f64 / tension_scale);
                let (Some(t_raw), Some(s_val)) = (tension_raw, sum_raw) else {
                    return make_sample(hour, total_pct, vec![None; top.len()], None);
                };
                if s_val > 0 {
                    let denom = s_val as f64 * tension_scale;
                    let band_vals: Vec<Option<f64>> = top_raws
                        .iter()
                        .map(|raw| raw.map(|v| ((i128::from(v) * i128::from(t_raw)) as f64) / denom))
                        .collect();
                    let other_val = if has_other {
                        top_sum_raw.map(|top_sum| (((i128::from(s_val) - top_sum) * i128::from(t_raw)) as f64) / denom)
                    } else {
                        None
                    };
                    make_sample(hour, total_pct, band_vals, other_val)
                } else if t_raw == 0 && s_val == 0 {
                    let band_vals: Vec<Option<f64>> = top_raws
                        .iter()
                        .map(|raw| raw.and_then(|v| (v == 0).then_some(0.0)))
                        .collect();
                    let other_val = if has_other {
                        (all_top_known && top_raws.iter().all(|v| *v == Some(0))).then_some(0.0)
                    } else {
                        None
                    };
                    make_sample(hour, total_pct, band_vals, other_val)
                } else {
                    make_sample(hour, total_pct, vec![None; top.len()], None)
                }
            }
            TensionValueMode::Recorded => {
                let band_vals: Vec<Option<f64>> = top_raws
                    .iter()
                    .map(|raw| raw.map(|v| v as f64 / contribution_scale))
                    .collect();
                if filter.is_none() {
                    let total_val = sum_raw.map(|s| s as f64 / contribution_scale);
                    let other_val = if has_other {
                        sum_raw
                            .zip(top_sum_raw)
                            .map(|(s, top_sum)| (i128::from(s) - top_sum) as f64 / contribution_scale)
                    } else {
                        None
                    };
                    make_sample(hour, total_val, band_vals, other_val)
                } else {
                    let total_val = top_sum_raw.map(|s| s as f64 / contribution_scale);
                    make_sample(hour, total_val, band_vals, None)
                }
            }
        }
    };
    let mut samples: Vec<Sample> = Vec::with_capacity(raw_hours.len());
    for hour in raw_hours {
        let sample = evaluate(hour);
        let same = samples.last().is_some_and(|prev| {
            prev.total == sample.total && prev.other == sample.other && prev.bands == sample.bands
        });
        if !same {
            samples.push(sample);
        }
    }
    let mut downsampled = false;
    if samples.len() > max_points && max_points >= 4 {
        downsampled = true;
        let span = (i64::from(last) - i64::from(first)).max(1);
        let step = (span / max_points as i64).max(1);
        let mut reduced: Vec<Sample> = Vec::with_capacity(max_points + 2);
        let total_len = samples.len();
        let mut last_bucket = i64::MIN;
        let mut prev_nulls: Vec<bool> = Vec::new();
        for (idx, sample) in samples.into_iter().enumerate() {
            let mut cur_nulls = Vec::with_capacity(sample.bands.len() + 2);
            cur_nulls.push(sample.total.is_none());
            cur_nulls.push(sample.other.is_none());
            cur_nulls.extend(sample.bands.iter().map(Option::is_none));
            let bucket = (i64::from(sample.hour) - i64::from(first)) / step;
            let keep = idx == 0 || idx + 1 == total_len || cur_nulls != prev_nulls || bucket != last_bucket;
            if keep {
                reduced.push(sample);
                last_bucket = bucket;
            } else if let Some(last_kept) = reduced.last_mut() {
                last_kept.total = sample.total;
                last_kept.other = sample.other;
                for i in 0..last_kept.bands.len() {
                    let val = sample.bands[i];
                    last_kept.bands[i] = val;
                    if let Some(v) = val {
                        last_kept.band_min[i] = Some(last_kept.band_min[i].map_or(v, |m| m.min(v)));
                        last_kept.band_max[i] = Some(last_kept.band_max[i].map_or(v, |m| m.max(v)));
                    }
                }
            }
            prev_nulls = cur_nulls;
        }
        samples = reduced;
    }
    let end_sample = evaluate(last);
    let end_hour = (end_sample.total.is_some()
        || end_sample.bands.iter().any(Option::is_some)
        || end_sample.other.is_some())
        .then_some(last);
    let mut hours = Vec::with_capacity(samples.len());
    let mut total = Vec::with_capacity(samples.len());
    let mut band_series: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(samples.len()); top.len()];
    let mut band_mins: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(if downsampled { samples.len() } else { 0 }); top.len()];
    let mut band_maxs: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(if downsampled { samples.len() } else { 0 }); top.len()];
    let mut other_series: Vec<Option<f64>> = Vec::with_capacity(if has_other { samples.len() } else { 0 });
    for sample in samples {
        hours.push(sample.hour);
        total.push(sample.total);
        for (i, val) in sample.bands.into_iter().enumerate() {
            band_series[i].push(val);
        }
        if downsampled {
            for (i, val) in sample.band_min.into_iter().enumerate() {
                band_mins[i].push(val);
            }
            for (i, val) in sample.band_max.into_iter().enumerate() {
                band_maxs[i].push(val);
            }
        }
        if has_other {
            other_series.push(sample.other);
        }
    }
    let bands = top
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let end_raw = lookup_slot(input.slots.get(&entry.slot).map(Vec::as_slice), last_complete, last)
                .map(|v| v.to_string());
            TensionStackBand {
                slot: entry.slot,
                tag: entry.resolved.as_ref().map(|(tag, _)| tag.clone()),
                remembered: entry.resolved.as_ref().is_some_and(|(_, remembered)| *remembered),
                values: std::mem::take(&mut band_series[i]),
                min_values: std::mem::take(&mut band_mins[i]),
                max_values: std::mem::take(&mut band_maxs[i]),
                end_value: end_sample.bands.get(i).copied().flatten(),
                end_raw,
            }
        })
        .collect();
    let other = has_other.then(|| TensionStackOther {
        count: other_count,
        values: other_series,
        end_value: end_sample.other,
    });
    StackOutput {
        end_hour,
        capped_end_tension,
        downsampled,
        hours,
        total,
        bands,
        other,
        next_change_hour: next_change,
    }
}

pub(super) fn filter_dated_warnings(
    warnings: &[(i32, String)],
    unreadable_hours: &[i32],
    unmatched_hours: &[i32],
    last: i32,
    next_change_hour: &mut Option<i32>,
) -> Vec<String> {
    let mut out = Vec::new();
    let bump = |best: &mut Option<i32>, hour: i32| {
        *best = Some(best.map_or(hour, |held| held.min(hour)));
    };
    for (hour, text) in warnings {
        if *hour <= last {
            out.push(text.clone());
        } else {
            bump(next_change_hour, *hour);
            break;
        }
    }
    let unreadable = unreadable_hours.partition_point(|&h| h <= last);
    if unreadable > 0 {
        out.push(format!("{unreadable} threat event rows hold a cell that was not recorded and are not shown"));
    }
    if let Some(&hour) = unreadable_hours.get(unreadable) {
        bump(next_change_hour, hour);
    }
    let unmatched = unmatched_hours.partition_point(|&h| h <= last);
    if unmatched > 0 {
        out.push(format!("{unmatched} threat event rows change or drop a source no read listed"));
    }
    if let Some(&hour) = unmatched_hours.get(unmatched) {
        bump(next_change_hour, hour);
    }
    out
}

fn dated_warnings(held: &BranchTension, last: i32, next_change_hour: &mut Option<i32>) -> Vec<String> {
    filter_dated_warnings(&held.warnings, &held.unreadable_hours, &held.unmatched_hours, last, next_change_hour)
}

impl Product for WorldTensionStack {
    const KEY: &'static str = "world-tension-stack";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = WorldTensionStackParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End, layers, values, countries, width];
    type Reply = WorldTensionStackReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            TensionValueMode::decl(),
            WorldTensionStackParams::decl(),
            TensionStackBand::decl(),
            TensionStackOther::decl(),
            TensionStackEvent::decl(),
            WorldTensionStackReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)?;
        if params.first_hour > params.last_hour {
            return Err("Invalid world tension time range");
        }
        if params.layers == 0 || params.layers > 1500 {
            return Err("Invalid world tension layer count");
        }
        if let Some(countries) = &params.countries {
            for tag in countries {
                dashboard_base::products::validate_tag(tag)?;
            }
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let held = build_branch(source, branch, cancelled)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let timeline = &query.timeline;
        let tension_scale = (cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR) as u32;
        let contribution_scale = cthreat_source::FIXED_POINT_SCALE as u32;
        let ceiling_raw = cworld_threat::CLAMP_CEILING.to_string();
        let first = params.first_hour.max(timeline.first);
        let last = params.last_hour.min(timeline.last);
        if first > last {
            let mut next_change_hour = (params.last_hour < timeline.first).then_some(timeline.first);
            let warnings = dated_warnings(&held, params.last_hour, &mut next_change_hour);
            let reply = WorldTensionStackReply {
                branch_id: params.branch_id.clone(),
                tension_scale,
                contribution_scale,
                ceiling_raw,
                end_hour: None,
                capped_end_tension: None,
                downsampled: false,
                hours: Vec::new(),
                total: Vec::new(),
                bands: Vec::new(),
                other: None,
                events: Vec::new(),
                missing: vec![MissingSpan { first_hour: params.first_hour, last_hour: params.last_hour, reason: "This time is outside the recording.".into() }],
                next_change_hour,
                warnings,
            };
            check_reply_budget(&reply)?;
            return Ok(reply);
        }
        let filter: Option<HashSet<&str>> = params
            .countries
            .as_ref()
            .filter(|list| !list.is_empty())
            .map(|list| list.iter().map(String::as_str).collect());
        let max_points = params.width.unwrap_or(1600).clamp(120, 2400) as usize;
        let named = Named::new(&query.holders, last);
        let mode_missing = match params.values {
            TensionValueMode::Share => &held.tension_missing,
            TensionValueMode::Recorded => &held.aggregates_missing,
        };
        let input = StackInput {
            tension: &held.tension,
            sum: &held.sum,
            slots: &held.slots,
            aggregates_complete: &held.aggregates_complete,
            missing: mode_missing,
        };
        let computed = compute_stack(
            &input,
            first,
            last,
            params.layers as usize,
            params.values,
            filter.as_ref(),
            max_points,
            |slot, hour| named.slot(slot, hour).map(|(tag, remembered)| (tag.to_owned(), remembered)),
            |slot, hour| named.next_change(slot, hour),
        );
        let mut next_change_hour = computed.next_change_hour;
        let mut events: Vec<TensionStackEvent> = Vec::new();
        for event in &held.events {
            if event.first_read > last {
                next_change_hour = Some(next_change_hour.map_or(event.first_read, |h| h.min(event.first_read)));
                continue;
            }
            if let Some(&(later_hour, _)) = event.added.iter().find(|&&(h, _)| h > last) {
                next_change_hour = Some(next_change_hour.map_or(later_hour, |h| h.min(later_hour)));
            }
            if event.hour < first || event.hour > last {
                continue;
            }
            let Some(&(_, added_raw)) = event.added.iter().rev().find(|&&(h, _)| h <= last) else {
                continue;
            };
            let resolved = event.slot.and_then(|slot| named.slot(slot, event.first_read.min(last)));
            if let Some(wanted) = &filter
                && !resolved.is_some_and(|(tag, _)| wanted.contains(tag))
            {
                continue;
            }
            if let Some(removed_at) = event.removed_at
                && removed_at > last
            {
                next_change_hour = Some(next_change_hour.map_or(removed_at, |h| h.min(removed_at)));
            }
            let last_read = event
                .removed_at
                .is_some_and(|at| at <= last)
                .then_some(event.last_read)
                .flatten();
            events.push(TensionStackEvent {
                hour: event.hour,
                slot: event.slot,
                second_country: event.second_country,
                place: event.place,
                tag: resolved.map(|(tag, _)| tag.to_owned()),
                remembered: resolved.is_some_and(|(_, remembered)| remembered),
                reason: event.reason.clone(),
                added_raw: added_raw.to_string(),
                first_read: event.first_read,
                last_read,
            });
        }
        events.sort_by(|a, b| a.hour.cmp(&b.hour).then_with(|| a.first_read.cmp(&b.first_read)));
        let missing: Vec<MissingSpan> = mode_missing
            .iter()
            .filter_map(|span| {
                let lo = span.first_hour.max(first);
                let hi = span.last_hour.min(last);
                (lo <= hi).then(|| MissingSpan { first_hour: lo, last_hour: hi, reason: span.reason.clone() })
            })
            .collect();
        let warnings = dated_warnings(&held, last, &mut next_change_hour);
        let reply = WorldTensionStackReply {
            branch_id: params.branch_id.clone(),
            tension_scale,
            contribution_scale,
            ceiling_raw,
            end_hour: computed.end_hour,
            capped_end_tension: computed.capped_end_tension,
            downsampled: computed.downsampled,
            hours: computed.hours,
            total: computed.total,
            bands: computed.bands,
            other: computed.other,
            events,
            missing,
            next_change_hour,
            warnings,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
