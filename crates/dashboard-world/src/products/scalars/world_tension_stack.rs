#[path = "world_tension_playback.rs"]
mod playback;
#[path = "world_tension_plot.rs"]
mod plot;
use playback::{EventIndex, EventWindow, Ranking, SlotIndex, Window};
use super::rows;
use crate::products::{group_members, groups::{self, MembershipRun}};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget}};
use dashboard_query::{
    identity::{Holder, Holders},
    log::{self, Cell},
    read,
    series::{
        MissingSpan, PAGE_LIMIT,
        ranked::{GroupKind, GroupMembership, GroupSelection},
    },
};
use dashboard_text::metadata::link_slot;
use engine_stability::{cthreat_source, cworld_threat};
use replay_columnar::{manifest::Status, summary::Source as ColumnSource};
use replay_lanes::lanes::CoveredValue;
use replay_data::rows::{TableId, TagHolderRow, ThreatEventRow, ThreatSourceRow, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool},
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
    pub top: Option<u32>,
    #[serde(default)]
    pub group: Option<GroupSelection>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub include_stack: Option<bool>,
    #[serde(default)]
    pub include_events: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionHolder {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub remembered: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionStackBand {
    pub slot: u32,
    pub holders: Vec<TensionHolder>,
    pub tag: Option<String>,
    pub remembered: bool,
    pub values: Vec<Option<f64>>,
    pub values_raw: Vec<Option<String>>,
    pub min_values: Vec<Option<f64>>,
    pub max_values: Vec<Option<f64>>,
    pub end_value: Option<f64>,
    pub end_raw: Option<String>,
    pub end_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionStackOther {
    pub count: u32,
    pub values: Vec<Option<f64>>,
    pub values_raw: Vec<Option<String>>,
    pub end_value: Option<f64>,
    pub end_raw: Option<String>,
    pub end_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionStackEvent {
    pub hour: i32,
    pub country: u32,
    pub slot: Option<u32>,
    pub second_country: u32,
    pub place: u32,
    pub tag: Option<String>,
    pub remembered: bool,
    pub reason: String,
    pub added_raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub marker_raw: Option<String>,
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
    pub capped_end_tension_raw: Option<String>,
    pub end_missing: Option<String>,
    pub end_sum_raw: Option<String>,
    pub downsampled: bool,
    pub refused: Option<String>,
    pub hours: Vec<i32>,
    pub total: Vec<Option<f64>>,
    pub total_raw: Vec<Option<String>>,
    pub sum_raw: Vec<Option<String>>,
    pub bands: Vec<TensionStackBand>,
    pub other: Option<TensionStackOther>,
    pub events: Vec<TensionStackEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub listing_missing: Option<String>,
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
    let mut places = HashMap::new();
    for source in sources {
        let place = places.entry((source.country, source.second_country, source.date, source.reason.as_slice())).or_insert(0);
        out.push((
            Identity { country: source.country, second: source.second_country, date: source.date, reason: source.reason.clone(), place: *place },
            source.final_raw,
            slot_of(source.country),
        ));
        *place += 1;
    }
    out
}

pub(super) fn listing_snapshot(frame: &replay_frame::frame::DecodedFrame, first: i32, last: i32) -> (Step, Option<MissingSpan>) {
    let sources = rows::<tables::ThreatSources>(frame);
    let links = rows::<tables::CountryLinks>(frame).ok();
    match sources {
        Ok(sources) => (Step::Stretch { first, last, listed: Some(listed(sources, |country| links.and_then(|links| link_slot(links, country)))) }, None),
        Err(reason) => (Step::Stretch { first, last, listed: None }, Some(MissingSpan { first_hour: first, last_hour: last, reason })),
    }
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
    country: u32,
    slot: Option<u32>,
    second_country: u32,
    place: u32,
    reason: String,
    added: Vec<(i32, i64)>,
    first_read: i32,
    last_read: Option<i32>,
    removed_at: Option<i32>,
}

pub(super) struct SlotNames(BTreeMap<u32, Vec<(i32, String, bool)>>);

impl SlotNames {
    pub(super) fn new(holders: &dashboard_query::identity::Holders) -> Self {
        let mut edges: BTreeMap<u32, Vec<i32>> = BTreeMap::new();
        for run in holders.tags.values().flatten() {
            if let dashboard_query::identity::Holder::Found { slot, .. } = run.holder {
                let changes = edges.entry(slot).or_default();
                changes.push(run.first);
                changes.extend(run.last.checked_add(1));
            }
        }
        Self(edges.into_iter().map(|(slot, mut edges)| {
            edges.sort_unstable();
            edges.dedup();
            let mut runs: Vec<(i32, String, bool)> = Vec::new();
            for edge in edges {
                if let Some(tag) = holders.slot_tag(slot, edge) {
                    if runs.last().is_none_or(|(_, last, former)| *former || last != tag) {
                        runs.push((edge, tag.to_owned(), false));
                    }
                } else if let Some((_, previous, false)) = runs.last() {
                    runs.push((edge, previous.clone(), true));
                }
            }
            (slot, runs)
        }).collect())
    }

    pub(super) fn slot(&self, slot: u32, hour: i32) -> Option<(&str, bool)> {
        let runs = self.0.get(&slot)?;
        runs[..runs.partition_point(|run| run.0 <= hour)].last().map(|(_, tag, former)| (tag.as_str(), *former))
    }

    pub(super) fn next_change(&self, slot: u32, hour: i32) -> Option<i32> {
        let runs = self.0.get(&slot)?;
        runs.get(runs.partition_point(|run| run.0 <= hour)).map(|run| run.0)
    }
}

struct BranchTension {
    names: SlotNames,
    tension: Vec<(i32, i32, i64)>,
    sum: Vec<(i32, i32, i64)>,
    slots: BTreeMap<u32, Vec<(i32, i32, Option<i64>)>>,
    aggregates_complete: Vec<(i32, i32)>,
    events: Vec<CachedEvent>,
    event_index: EventIndex,
    index: OnceLock<Arc<SlotIndex>>,
    views: Mutex<Vec<(WorldTensionStackParams, Arc<Mutex<Playback>>)>>,
    listing_missing: Vec<MissingSpan>,
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

fn push_slot_runs(target: &mut Vec<(i32, i32, Option<i64>)>, runs: &[(i32, i32, CoveredValue)], lo: i32, hi: i32) {
    for &(first, last, covered) in runs {
        let value = match covered {
            CoveredValue::Value(value) => Some(value),
            CoveredValue::Unvalued => None,
            CoveredValue::Uncovered => continue,
        };
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
    let mut listing_missing = unheld.clone();
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
            let (held, _) = read::covered_runs(recording, stretch.run, TableId::ThreatCountryAggregates, ColumnSource::Cell(contrib_col), None, lo, hi)?;
            for (key, runs) in held {
                if let Ok(slot) = u32::try_from(key) {
                    push_slot_runs(slots.entry(slot).or_default(), &runs, lo, hi);
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
    let listing_fields = ["country", "second_country", "date", "reason", "final_raw"].map(|field| dashboard_declare::field(TableId::ThreatSources, field)).into_iter().collect::<io::Result<Vec<_>>>()?;
    let link_fields = [dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?];
    let reads = [
        replay_lanes::at::Read { table: TableId::ThreatSources, columns: Some(&listing_fields), keys: replay_lanes::at::Keys::All },
        replay_lanes::at::Read { table: TableId::CountryLinks, columns: Some(&link_fields), keys: replay_lanes::at::Keys::All },
    ];
    let mut listings = replay_lanes::at::Cursor::new(recording, &reads)?;
    for stretch in &timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (valid, withheld) = valid_span(recording, TableId::ThreatSources, stretch.run, stretch.first, stretch.last);
        listing_missing.extend(withheld);
        let Some((lo, hi)) = valid else { continue };
        let statuses = read::statuses(recording, stretch.run, TableId::ThreatSources, lo, hi)?;
        let reasons = read::status_reasons(recording, stretch.run, TableId::ThreatSources, &statuses)?;
        for status in statuses {
            let first = status.first.max(lo);
            let last = status.last.min(hi);
            if first > last { continue; }
            while iter.next_if(|row| row.hour < first).is_some() {}
            if status.status != Status::Complete {
                let reason = match status.status {
                    Status::Unavailable { reason } => reasons.get(&reason).cloned().unwrap_or_else(|| "Threat source listing was not read.".into()),
                    Status::Truncated { .. } => "Threat source listing was incomplete at this hour.".into(),
                    Status::NotRecorded => "Threat source listing was not recorded.".into(),
                    Status::Complete => unreachable!(),
                };
                listing_missing.push(MissingSpan { first_hour: first, last_hour: last, reason });
                continue;
            }
            let frame = dashboard_query::cost::storage(|| listings.frame(stretch.run, first))?;
            let (step, missing) = listing_snapshot(&frame, first, last);
            steps.push(step);
            if let Some(span) = missing {
                listing_missing.push(span);
                continue;
            }
            while let Some(row) = iter.next_if(|row| row.hour <= last) {
                if row.hour > first {
                    if let Some(step) = row_step(&row) {
                        steps.push(step);
                    } else {
                        unreadable_hours.push(row.hour);
                    }
                }
            }
        }
    }
    unreadable_hours.sort_unstable();
    let (joined, unmatched_hours) = join(steps);
    let mut events: Vec<CachedEvent> = joined
        .into_iter()
        .map(|event| CachedEvent {
            hour: event.identity.date,
            country: event.identity.country,
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
    events.sort_by_key(|event| (event.first_read, event.hour));
    warnings.sort_by_key(|(hour, _)| *hour);
    let built = Arc::new(BranchTension {
        names: SlotNames::new(&query.holders),
        event_index: EventIndex::new(&events),
        index: OnceLock::new(),
        views: Default::default(),
        tension,
        sum,
        slots,
        aggregates_complete,
        events,
        listing_missing: merge_missing(listing_missing),
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

pub(super) struct StackInput<'a> {
    pub tension: &'a [(i32, i32, i64)],
    pub sum: &'a [(i32, i32, i64)],
    pub slots: &'a BTreeMap<u32, Vec<(i32, i32, Option<i64>)>>,
    pub aggregates_complete: &'a [(i32, i32)],
    pub tension_missing: &'a [MissingSpan],
    pub aggregates_missing: &'a [MissingSpan],
}

const TENSION_UNKNOWN: &str = "World tension was not recorded at this hour.";
const CONTRIBUTION_UNKNOWN: &str = "This country's threat contribution was not recorded at this hour.";
const SUM_UNKNOWN: &str = "The sum of threat contributions was not recorded at this hour.";
const NO_SHARE: &str = "Threat contributions sum to zero or less at this hour, so no share is defined.";
const NOT_MEMBER: &str = "Not a member of the selected group at this hour.";
const REST_UNKNOWN: &str = "A shown country's contribution was not recorded at this hour, so the rest is not known.";

fn reason_at(spans: &[MissingSpan], hour: i32) -> Option<&str> {
    spans.get(spans.partition_point(|span| span.last_hour < hour)).filter(|span| span.first_hour <= hour).map(|span| span.reason.as_str())
}

pub(super) struct DatedGroupFilter {
    edges: Vec<i32>,
    unknown: Vec<(i32, i32)>,
    runs_by_slot: HashMap<u32, Vec<(i32, i32)>>,
}

impl DatedGroupFilter {
    pub(super) fn new(members: &[MembershipRun], membership: GroupMembership, holders: &Holders) -> Self {
        let mut runs_by_slot = HashMap::<u32, Vec<(i32, i32)>>::new();
        for member in members {
            for held in holders.within(&member.tag, member.first_hour, member.last_hour) {
                if let Holder::Found { slot, .. } = held.holder {
                    let last = match membership {
                        GroupMembership::Hour => held.last,
                        GroupMembership::Ever => i32::MAX,
                    };
                    runs_by_slot.entry(slot).or_default().push((held.first, last));
                }
            }
        }
        for runs in runs_by_slot.values_mut() {
            runs.sort_unstable();
            for (first, last) in std::mem::take(runs) {
                push_span(runs, first, last);
            }
        }
        let mut edges = Vec::new();
        for run in members {
            edges.push(run.first_hour);
            if matches!(membership, GroupMembership::Hour) && let Some(after) = run.last_hour.checked_add(1) {
                edges.push(after);
            }
        }
        edges.sort_unstable();
        edges.dedup();
        Self { runs_by_slot, edges, unknown: Vec::new() }
    }

    fn spans(&self, slot: u32, first: i32, last: i32) -> &[(i32, i32)] {
        let Some(runs) = self.runs_by_slot.get(&slot) else { return &[] };
        &runs[runs.partition_point(|span| span.1 < first)..runs.partition_point(|span| span.0 <= last)]
    }

}

struct Sample {
    hour: i32,
    total: Option<f64>,
    total_raw: Option<i128>,
    sum_raw: Option<i64>,
    bands: Vec<Option<f64>>,
    band_raws: Vec<Option<i64>>,
    band_min: Vec<Option<f64>>,
    band_max: Vec<Option<f64>>,
    other: Option<f64>,
    other_raw: Option<i128>,
}

#[derive(Default)]
struct Playback {
    countries: Option<Arc<HashSet<String>>>,
    index: Option<Arc<SlotIndex>>,
    ranking: Ranking,
    plot: plot::Plot,
    events: EventWindow,
    group: Option<Result<Arc<DatedGroupFilter>, String>>,
}

#[derive(Default)]
pub(super) struct StackOutput {
    top_slots: Option<HashSet<u32>>,
    ranking_missing: Option<String>,
    refused: Option<String>,
    pub end_hour: Option<i32>,
    pub capped_end_tension: Option<f64>,
    pub capped_end_tension_raw: Option<String>,
    pub end_missing: Option<String>,
    pub end_sum_raw: Option<String>,
    pub downsampled: bool,
    pub hours: Vec<i32>,
    pub total: Vec<Option<f64>>,
    pub total_raw: Vec<Option<String>>,
    pub sum_raw: Vec<Option<String>>,
    pub bands: Vec<TensionStackBand>,
    pub other: Option<TensionStackOther>,
    pub next_change_hour: Option<i32>,
}

#[cfg(test)]
#[expect(clippy::too_many_arguments, reason = "world tension stack selection, bounds, and callbacks")]
pub(super) fn compute_stack(
    input: &StackInput<'_>,
    first: i32,
    last: i32,
    layers: usize,
    mode: TensionValueMode,
    filter: Option<&HashSet<&str>>,
    group_filter: Option<&DatedGroupFilter>,
    top_limit: Option<usize>,
    max_points: impl Into<Option<usize>>,
    slot_tag: impl Fn(u32, i32) -> Option<(String, bool)>,
    mut slot_next_change: impl FnMut(u32, i32) -> Option<i32>,
) -> StackOutput {
    let mut playback = Playback::default();
    playback.index = Some(Arc::new(SlotIndex::new(input, group_filter, None)));
    let next_name = input.slots.keys().filter_map(|&slot| slot_next_change(slot, last)).min();
    let selected = |tag: &str| filter.is_some_and(|tags| tags.contains(tag));
    let wanted = filter.map(|_| &selected as &dyn Fn(&str) -> bool);
    let mut out = compute_window(&mut playback, input, first, last, layers, mode, wanted, group_filter, top_limit,
        max_points.into(), slot_tag, slot_next_change);
    if let Some(next) = next_name { out.next_change_hour = Some(out.next_change_hour.map_or(next, |hour| hour.min(next))); }
    out
}

#[expect(clippy::too_many_arguments, reason = "world tension stack selection, bounds, and callbacks")]
fn compute_window(
    playback: &mut Playback,
    input: &StackInput<'_>,
    first: i32,
    last: i32,
    layers: usize,
    mode: TensionValueMode,
    filter: Option<&dyn Fn(&str) -> bool>,
    group_filter: Option<&DatedGroupFilter>,
    top_limit: Option<usize>,
    max_points: Option<usize>,
    slot_tag: impl Fn(u32, i32) -> Option<(String, bool)>,
    mut slot_next_change: impl FnMut(u32, i32) -> Option<i32>,
) -> StackOutput {
    let tension_scale = (cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR) as f64;
    let mode_missing = match mode {
        TensionValueMode::Share => input.tension_missing,
        TensionValueMode::Recorded => input.aggregates_missing,
    };
    let mut next_change: Option<i32> = next_run_change(input.tension, last);
    let bump = |best: &mut Option<i32>, candidate: Option<i32>| {
        if let Some(hour) = candidate {
            *best = Some(best.map_or(hour, |held| held.min(hour)));
        }
    };
    bump(&mut next_change, next_run_change(input.sum, last));
    bump(&mut next_change, next_span_change(input.aggregates_complete, last));
    let index = playback.index.as_ref().unwrap();
    bump(&mut next_change, index.next(last));
    let rank_missing = (top_limit.is_some() && mode == TensionValueMode::Share).then_some(input.aggregates_missing);
    for spans in Some(mode_missing).into_iter().chain(rank_missing) {
        if let Some(span) = spans.get(spans.partition_point(|span| span.last_hour < last)) {
            bump(&mut next_change, if span.first_hour > last { Some(span.first_hour) } else { span.last_hour.checked_add(1) });
        }
    }
    let last_complete = is_complete(input.aggregates_complete, last);
    let complete = input.aggregates_complete;
    let at = complete.partition_point(|span| span.0 <= last);
    let witnessed = complete[..at].last().map(|span| span.1.min(last)).filter(|&hour| hour >= first);
    let rank_hour = if top_limit.is_some() {
        Some(last)
    } else {
        witnessed.or_else(|| filter.is_some().then_some(last))
    };
    let rank_complete = rank_hour.is_some_and(|hour| is_complete(complete, hour));
    let capped_end_raw_val = lookup_run(input.tension, last);
    let capped_end_tension = capped_end_raw_val.map(|raw| raw as f64 / tension_scale);
    let capped_end_tension_raw = capped_end_raw_val.map(|raw| raw.to_string());
    playback.ranking.update(index, input, Window { first, last, rank: rank_hour, complete: rank_complete }, group_filter, filter, top_limit.is_some(), &slot_tag);
    if top_limit.is_some() && (!last_complete || playback.ranking.unknown()) {
        let reason = reason_at(input.aggregates_missing, last).unwrap_or(CONTRIBUTION_UNKNOWN);
        return StackOutput { refused: Some(reason.to_owned()), next_change_hour: next_change, ..Default::default() };
    }
    let ranked = playback.ranking.top(top_limit.unwrap_or(layers));
    let top_slots = top_limit.map(|_| ranked.iter().copied().collect());
    let Some(max_points) = max_points else {
        return StackOutput { top_slots, next_change_hour: next_change, ..Default::default() };
    };
    let top = ranked;
    let (has_other, other_count) = if top_limit.is_some() || group_filter.is_some() { (false, 0) } else { playback.ranking.other(&top) };
    let (samples, downsampled, end_sample) = playback.plot.read(input, index, &top, group_filter, mode, has_other, first, last, max_points);
    let member_at = |slot: u32, hour: i32| group_filter.is_none_or(|gf| !gf.spans(slot, hour, hour).is_empty());
    let end_tension = lookup_run(input.tension, last);
    let end_sum = lookup_run(input.sum, last);
    let world_reason = || reason_at(input.tension_missing, last).unwrap_or(TENSION_UNKNOWN).to_owned();
    let share_reason = || match (mode, end_tension, end_sum) {
        (TensionValueMode::Recorded, ..) => None,
        (TensionValueMode::Share, None, _) => Some(world_reason()),
        (TensionValueMode::Share, _, None) => Some(SUM_UNKNOWN.to_owned()),
        (TensionValueMode::Share, Some(t), Some(s)) if s > 0 || (t == 0 && s == 0) => None,
        (TensionValueMode::Share, ..) => Some(NO_SHARE.to_owned()),
    };
    let end_complete = is_complete(input.aggregates_complete, last);
    let band_missing: Vec<Option<String>> = top
        .iter()
        .zip(&end_sample.bands)
        .map(|(&slot, value)| {
            if value.is_some() {
                None
            } else if !member_at(slot, last) {
                Some(NOT_MEMBER.to_owned())
            } else if lookup_slot(input.slots.get(&slot).map(Vec::as_slice), end_complete, last).is_none() {
                Some(reason_at(input.aggregates_missing, last).unwrap_or(CONTRIBUTION_UNKNOWN).to_owned())
            } else {
                Some(share_reason().unwrap_or_else(|| NO_SHARE.to_owned()))
            }
        })
        .collect();
    let other_missing = (has_other && end_sample.other.is_none()).then(|| {
        if end_sum.is_none() {
            SUM_UNKNOWN.to_owned()
        } else {
            share_reason().unwrap_or_else(|| REST_UNKNOWN.to_owned())
        }
    });
    let end_hour = (end_sample.total.is_some()
        || end_sample.bands.iter().any(Option::is_some)
        || end_sample.other.is_some())
        .then_some(last);
    let mut hours = Vec::with_capacity(samples.len());
    let mut total = Vec::with_capacity(samples.len());
    let mut total_raw = Vec::with_capacity(samples.len());
    let mut sum_raw = Vec::with_capacity(samples.len());
    let mut band_series: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(samples.len()); top.len()];
    let mut band_raw_series: Vec<Vec<Option<String>>> = vec![Vec::with_capacity(samples.len()); top.len()];
    let mut band_mins: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(if downsampled { samples.len() } else { 0 }); top.len()];
    let mut band_maxs: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(if downsampled { samples.len() } else { 0 }); top.len()];
    let mut other_series: Vec<Option<f64>> = Vec::with_capacity(if has_other { samples.len() } else { 0 });
    let mut other_raw_series: Vec<Option<String>> = Vec::with_capacity(if has_other { samples.len() } else { 0 });
    for sample in samples {
        hours.push(sample.hour);
        total.push(sample.total);
        total_raw.push(sample.total_raw.map(|v| v.to_string()));
        sum_raw.push(sample.sum_raw.map(|v| v.to_string()));
        for (i, val) in sample.bands.into_iter().enumerate() {
            band_series[i].push(val);
        }
        for (i, raw) in sample.band_raws.into_iter().enumerate() {
            band_raw_series[i].push(raw.map(|v| v.to_string()));
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
            other_raw_series.push(sample.other_raw.map(|v| v.to_string()));
        }
    }
    let bands = top.into_iter().enumerate()
        .map(|(i, slot)| {
            let resolved = slot_tag(slot, last);
            let end_raw = end_sample
                .band_raws
                .get(i)
                .copied()
                .flatten()
                .map(|v| v.to_string());
            let mut holders = Vec::new();
            let mut first_hour = first;
            loop {
                let next = slot_next_change(slot, first_hour).filter(|&at| first_hour < at && at <= last);
                let last_hour = next.map_or(last, |at| at - 1);
                if let Some((tag, remembered)) = slot_tag(slot, first_hour) {
                    holders.push(TensionHolder { first_hour, last_hour, tag, remembered });
                }
                let Some(at) = next else { break };
                first_hour = at;
            }
            TensionStackBand {
                holders,
                slot,
                tag: resolved.as_ref().map(|(tag, _)| tag.clone()),
                remembered: resolved.as_ref().is_some_and(|(_, remembered)| *remembered),
                values: std::mem::take(&mut band_series[i]),
                values_raw: std::mem::take(&mut band_raw_series[i]),
                min_values: std::mem::take(&mut band_mins[i]),
                max_values: std::mem::take(&mut band_maxs[i]),
                end_value: end_sample.bands.get(i).copied().flatten(),
                end_raw,
                end_missing: band_missing[i].clone(),
            }
        })
        .collect();
    let other = has_other.then(|| TensionStackOther {
        count: other_count,
        values: other_series,
        values_raw: other_raw_series,
        end_value: end_sample.other,
        end_raw: end_sample.other_raw.map(|v| v.to_string()),
        end_missing: other_missing,
    });
    StackOutput {
        top_slots,
        ranking_missing: rank_hour.is_none().then(|| reason_at(input.aggregates_missing, last).unwrap_or("Country threat contributions were not recorded at this hour.").to_owned()),
        refused: None,
        end_hour,
        capped_end_tension,
        capped_end_tension_raw,
        end_missing: end_tension.is_none().then(world_reason),
        end_sum_raw: end_sample.sum_raw.map(|v| v.to_string()),
        downsampled,
        hours,
        total,
        total_raw,
        sum_raw,
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End, layers, values, countries, top, group, width, includeStack, includeEvents];
    type Reply = WorldTensionStackReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            TensionValueMode::decl(),
            WorldTensionStackParams::decl(),
            TensionHolder::decl(),
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
        if let Some(top) = params.top
            && (top == 0 || top > 1500)
        {
            return Err("Invalid world tension top count");
        }
        if let Some(countries) = &params.countries {
            for tag in countries {
                dashboard_base::products::validate_tag(tag)?;
            }
        }
        if let Some(group) = &params.group {
            match group.kind {
                GroupKind::Humans => {}
                GroupKind::Faction => {
                    if !group.id.as_deref().is_some_and(|id| !id.is_empty()) {
                        return Err("A faction group names its id");
                    }
                }
                GroupKind::War => {
                    let Some(tag) = group.tag.as_deref() else {
                        return Err("A war group names its country");
                    };
                    dashboard_base::products::validate_tag(tag)?;
                }
            }
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
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
                capped_end_tension_raw: None,
                end_missing: None,
                end_sum_raw: None,
                downsampled: false,
                refused: None,
                hours: Vec::new(),
                total: Vec::new(),
                total_raw: Vec::new(),
                sum_raw: Vec::new(),
                bands: Vec::new(),
                other: None,
                events: Vec::new(),
                listing_missing: None,
                missing: vec![MissingSpan { first_hour: params.first_hour, last_hour: params.last_hour, reason: "This time is outside the recording.".into() }],
                next_change_hour,
                warnings,
            };
            check_reply_budget(&reply)?;
            return Ok(reply);
        }
        let mut next_change_hour: Option<i32> = None;
        let bump = |best: &mut Option<i32>, candidate: Option<i32>| {
            if let Some(hour) = candidate {
                *best = Some(best.map_or(hour, |held| held.min(hour)));
            }
        };
        let view = {
            let mut views = held.views.lock().unwrap_or_else(|error| error.into_inner());
            let at = views.iter().position(|(key, _)| key.layers == params.layers && key.top == params.top
                && key.values == params.values && key.include_stack == params.include_stack && key.include_events == params.include_events
                && key.group == params.group && key.countries == params.countries);
            let entry = at.map(|at| views.remove(at)).unwrap_or_else(|| {
                let countries = params.countries.as_ref().map(|tags| Arc::new(tags.iter().cloned().collect()));
                (params.clone(), Arc::new(Mutex::new(Playback { countries, ..Playback::default() })))
            });
            let view = Arc::clone(&entry.1);
            views.push(entry);
            if views.len() > 8 { views.remove(0); }
            view
        };
        let mut playback = view.lock().unwrap_or_else(|error| error.into_inner());
        let group_filter = if let Some(group) = &params.group {
            if playback.group.is_none() {
                let groups = groups::shared(source, &query, context, cancelled)?;
                playback.group = Some(group_members::runs(&groups, group)?.map(|runs| {
                    let mut filter = DatedGroupFilter::new(&runs.members, group.membership, &query.holders);
                    let mut spans: Vec<_> = runs.unknown.iter().map(|span| (span.first_hour, span.last_hour)).collect();
                    spans.sort_unstable();
                    for (first, last) in spans { push_span(&mut filter.unknown, first, last); }
                    Arc::new(filter)
                }));
            }
            let (filter, refusal) = match playback.group.as_ref().unwrap() {
                Ok(filter) => {
                    let since = match group.membership { GroupMembership::Hour => last, GroupMembership::Ever => first };
                    let at = filter.unknown.partition_point(|span| span.1 < since);
                    let unknown = filter.unknown.get(at).is_some_and(|span| span.0 <= last);
                    bump(&mut next_change_hour, next_span_change(&filter.unknown, last));
                    let edge = filter.edges.partition_point(|&hour| hour <= last);
                    bump(&mut next_change_hour, filter.edges.get(edge).copied());
                    (Some(Arc::clone(filter)), unknown.then(|| group_members::UNKNOWN.to_owned()))
                }
                Err(reason) => (None, Some(reason.clone())),
            };
            if let Some(reason) = refusal {
                let warnings = dated_warnings(&held, last, &mut next_change_hour);
                let reply = WorldTensionStackReply {
                    branch_id: params.branch_id.clone(),
                    tension_scale,
                    contribution_scale,
                    ceiling_raw,
                    end_hour: None,
                    capped_end_tension: None,
                    capped_end_tension_raw: None,
                    end_missing: None,
                    end_sum_raw: None,
                    downsampled: false,
                    refused: Some(reason),
                    hours: Vec::new(),
                    total: Vec::new(),
                    total_raw: Vec::new(),
                    sum_raw: Vec::new(),
                    bands: Vec::new(),
                    other: None,
                    events: Vec::new(),
                    listing_missing: None,
                    missing: Vec::new(),
                    next_change_hour,
                    warnings,
                };
                check_reply_budget(&reply)?;
                return Ok(reply);
            }
            filter
        } else {
            None
        };
        let top_limit = params.top.map(|n| n as usize);
        let countries = playback.countries.clone().filter(|_| group_filter.is_none() && top_limit.is_none());
        let selected = |tag: &str| countries.as_ref().is_some_and(|tags| tags.contains(tag));
        let filter = countries.as_ref().map(|_| &selected as &dyn Fn(&str) -> bool);
        let include_stack = params.include_stack.unwrap_or(true);
        let include_events = params.include_events.unwrap_or(true);
        let max_points = params.width.unwrap_or(1600).clamp(120, 2400) as usize;
        let named = &held.names;
        let mode_missing = match params.values {
            TensionValueMode::Share => &held.tension_missing,
            TensionValueMode::Recorded => &held.aggregates_missing,
        };
        let computed = if include_stack || top_limit.is_some() {
            let input = StackInput {
                tension: &held.tension,
                sum: &held.sum,
                slots: &held.slots,
                aggregates_complete: &held.aggregates_complete,
                tension_missing: &held.tension_missing,
                aggregates_missing: &held.aggregates_missing,
            };
            if playback.index.is_none() {
                playback.index = Some(if let Some(group) = group_filter.as_deref() {
                    Arc::new(SlotIndex::new(&input, Some(group), Some(named)))
                } else {
                    held.index.get_or_init(|| Arc::new(SlotIndex::new(&input, None, Some(named)))).clone()
                });
            }
            let out = compute_window(
                &mut playback, &input,
                first,
                last,
                params.layers as usize,
                params.values,
                filter,
                group_filter.as_deref(),
                top_limit,
                include_stack.then_some(max_points),
                |slot, hour| named.slot(slot, hour).map(|(tag, remembered)| (tag.to_owned(), remembered)),
                |slot, hour| named.next_change(slot, hour),
            );
            bump(&mut next_change_hour, out.next_change_hour);
            out
        } else {
            StackOutput::default()
        };
        let mut events: Vec<TensionStackEvent> = Vec::new();
        let mut listing_missing = None;
        if include_events && computed.refused.is_none() {
            let at = held.listing_missing.partition_point(|span| span.last_hour < last);
            if let Some(span) = held.listing_missing.get(at) {
                if span.first_hour <= last {
                    listing_missing = Some(span.reason.clone());
                    bump(&mut next_change_hour, span.last_hour.checked_add(1));
                } else {
                    bump(&mut next_change_hour, Some(span.first_hour));
                }
            }
            bump(&mut next_change_hour, held.event_index.next(last));
            playback.events.update(&held.event_index, &held.events, first, last, |event| {
                let at = event.added.partition_point(|&(hour, _)| hour <= last);
                let &(_, added_raw) = event.added[..at].last()?;
                let ev_hour = event.first_read.min(last);
                let resolved = event.slot.and_then(|slot| named.slot(slot, ev_hour));
                if let Some(gf) = &group_filter {
                    if !event.slot.is_some_and(|slot| !gf.spans(slot, ev_hour, ev_hour).is_empty()) {
                        return None;
                    }
                } else if let Some(wanted) = filter
                    && !resolved.is_some_and(|(tag, _)| wanted(tag))
                {
                    return None;
                }
                let last_read = event
                    .removed_at
                    .is_some_and(|at| at <= last)
                    .then_some(event.last_read)
                    .flatten();
                Some(TensionStackEvent {
                    hour: event.hour,
                    country: event.country,
                    slot: event.slot,
                    second_country: event.second_country,
                    place: event.place,
                    tag: resolved.map(|(tag, _)| tag.to_owned()),
                    remembered: resolved.is_some_and(|(_, remembered)| remembered),
                    reason: event.reason.clone(),
                    added_raw: added_raw.to_string(),
                    marker_raw: event.added[..event.added.partition_point(|&(read, _)| read <= event.hour)].last().map(|(_, raw)| raw.to_string()),
                    first_read: event.first_read,
                    last_read,
                })
            });
            events = playback.events.shown(computed.top_slots.as_ref());
        }
        let missing: Vec<MissingSpan> = if include_stack {
            mode_missing[mode_missing.partition_point(|span| span.last_hour < first)..mode_missing.partition_point(|span| span.first_hour <= last)]
                .iter()
                .filter_map(|span| {
                    let lo = span.first_hour.max(first);
                    let hi = span.last_hour.min(last);
                    (lo <= hi).then(|| MissingSpan { first_hour: lo, last_hour: hi, reason: span.reason.clone() })
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut warnings = dated_warnings(&held, last, &mut next_change_hour);
        warnings.extend(computed.ranking_missing);
        let reply = WorldTensionStackReply {
            branch_id: params.branch_id.clone(),
            tension_scale,
            contribution_scale,
            ceiling_raw,
            end_hour: computed.end_hour,
            capped_end_tension: computed.capped_end_tension,
            capped_end_tension_raw: computed.capped_end_tension_raw,
            end_missing: computed.end_missing,
            end_sum_raw: computed.end_sum_raw,
            downsampled: computed.downsampled,
            refused: computed.refused,
            hours: computed.hours,
            total: computed.total,
            total_raw: computed.total_raw,
            sum_raw: computed.sum_raw,
            bands: computed.bands,
            other: computed.other,
            events,
            listing_missing,
            missing,
            next_change_hour,
            warnings,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
