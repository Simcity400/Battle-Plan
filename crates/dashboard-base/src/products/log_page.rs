use super::occurrences::OccurrenceCount;
use dashboard_follow::panel::{ExactReader, Panel, ProductRead, ProductReader};
use dashboard_query::{Query, log::{self, BucketCache, FacetFilter}, series::{MissingSpan, PAGE_LIMIT, hour_of}};
use replay_columnar::manifest::Status;
use replay_recording::columnar_frames::{Damage, DamageScope, TableCoverage};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const GROUP_UNOBSERVED_REASON: &str = "Group membership is unavailable at this time.";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn declared(name: &str) -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::log(name).ok_or_else(|| invalid(format!("No declared log {name}")))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LogCell {
    Number { value: String },
    Text { value: String },
    List { bytes: String },
    Missing,
    NotRecorded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogPageRow {
    pub hour: i32,
    #[doc = " The row's key, as a decimal string."]
    pub key: String,
    #[doc = " Its ordinal among the asked rows from the timeline's first hour."]
    pub ordinal: String,
    #[doc = " The country whose filter it matched, when a facet is filtered by countries."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub country: Option<dashboard_query::identity::Country>,
    #[doc = " Its ordinal among its country's rows from the timeline's first hour, with a country."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub own_ordinal: Option<String>,
    #[doc = " By the reply's `columns`."]
    pub cells: Vec<LogCell>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogFacetCountAsk {
    pub key: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogTailParams {
    pub branch_id: String,
    pub log: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    #[serde(default)]
    pub from: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    #[serde(default)]
    pub count_facets: Vec<LogFacetCountAsk>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogTailReply {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub base_ordinal: String,
    pub first_ordinal: String,
    pub offset: String,
    pub total: String,
    pub rows: Vec<LogPageRow>,
    pub counts: Vec<OccurrenceCount>,
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

pub trait TailPanelReply: Clone + Serialize + Send + Sync + 'static {
    fn row_count(&self) -> usize;
    fn missing(&self) -> &[MissingSpan];
    fn set_missing(&mut self, missing: Vec<MissingSpan>);
}

impl TailPanelReply for LogTailReply {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn missing(&self) -> &[MissingSpan] {
        &self.missing
    }

    fn set_missing(&mut self, missing: Vec<MissingSpan>) {
        self.missing = missing;
    }
}

crate::reply_warnings!(LogTailReply);

type BlockCoverageKey = (u64, u64, [u8; 16], u32, u32, usize);

#[derive(Default)]
pub struct CoverageCache {
    by_block: HashMap<BlockCoverageKey, Arc<Vec<MissingSpan>>>,
}

impl CoverageCache {
    pub fn clear(&mut self) {
        self.by_block.clear();
    }

    fn get(&self, key: &BlockCoverageKey) -> Option<Arc<Vec<MissingSpan>>> {
        self.by_block.get(key).map(Arc::clone)
    }

    fn insert(&mut self, key: BlockCoverageKey, spans: Arc<Vec<MissingSpan>>) {
        self.by_block.insert(key, spans);
    }
}

#[derive(Clone)]
pub struct LogCaches {
    pub buckets: Arc<Mutex<BucketCache>>,
    pub coverage: Arc<Mutex<CoverageCache>>,
}

fn unheld(query: &Query<'_>, from: i32, to: i32) -> Vec<MissingSpan> {
    query
        .timeline
        .unheld
        .iter()
        .filter(|gap| gap.last >= from && gap.first <= to)
        .map(|gap| MissingSpan { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: gap.reason.clone() })
        .collect()
}

fn push_coalesced(out: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: String) {
    match out.last_mut() {
        Some(held) if held.reason == reason && held.last_hour + 1 == first_hour => held.last_hour = last_hour,
        _ => out.push(MissingSpan { first_hour, last_hour, reason }),
    }
}

fn unread_run_range(
    recording: &replay_recording::columnar_frames::Recording,
    run: u32,
    log: &dashboard_declare::Log,
    from: i32,
    to: i32,
) -> io::Result<Vec<MissingSpan>> {
    let mut out: Vec<MissingSpan> = Vec::new();
    for status_run in dashboard_query::read::statuses(recording, run, log.table, from, to)? {
        let reason = match status_run.status {
            Status::Complete => continue,
            Status::NotRecorded => format!("The {} log was not recorded here", log.name),
            Status::Truncated { missing } => format!("The {} log was read in part here ({missing} rows not read)", log.name),
            Status::Unavailable { reason } => {
                let held = dashboard_query::cost::storage(|| replay_lanes::lanes::at_keys(recording, run, log.table, status_run.first, None, replay_lanes::lanes::Keys::These(&[])))?;
                held.strings.get(reason).map_or_else(|| format!("The {} log was not available here", log.name), |text| String::from_utf8_lossy(text).into_owned())
            }
        };
        push_coalesced(&mut out, status_run.first, status_run.last, reason);
    }
    Ok(out)
}

fn unread(query: &Query<'_>, log: &dashboard_declare::Log, from: i32, to: i32, cache: Option<&Mutex<CoverageCache>>) -> io::Result<Vec<MissingSpan>> {
    let recording = query.replay.recording();
    let index = log.table.index();
    let rev = recording.revision();
    let mut out: Vec<MissingSpan> = Vec::new();
    for stretch in query.timeline.within(from, to) {
        let Some(cache) = cache else {
            for span in unread_run_range(recording, stretch.run, log, stretch.first, stretch.last)? {
                push_coalesced(&mut out, span.first_hour, span.last_hour, span.reason);
            }
            continue;
        };
        let valid_from = if recording.predates(index).is_some() {
            None
        } else {
            match recording.table_coverage(index, stretch.run) {
                TableCoverage::Valid => None,
                TableCoverage::WithheldAll(reason) => {
                    return Err(Damage::wrap(DamageScope::Withheld { table: index as u32 }, invalid(reason.to_owned())));
                }
                TableCoverage::WithheldBefore { ordinal, reason } => {
                    let valid = recording.run(stretch.run)?.first_hour + ordinal as i32;
                    if stretch.last < valid {
                        return Err(Damage::wrap(DamageScope::Withheld { table: index as u32 }, invalid(reason.to_owned())));
                    }
                    Some(valid)
                }
            }
        };
        let held = recording.run(stretch.run)?;
        let s_from = stretch.first.max(held.first_hour).max(valid_from.unwrap_or(i32::MIN));
        let s_to = stretch.last.min(held.last_hour());
        let (Some(first_ord), Some(last_ord)) = (held.ordinal(s_from), held.ordinal(s_to)) else {
            continue;
        };
        if first_ord > last_ord {
            continue;
        }
        let first_block = held.place(first_ord).0;
        let last_block = held.place(last_ord).0;
        for block in first_block..=last_block {
            let block_first = held.block_first_hour(block);
            let block_len = held.block(block as usize).1 as i32;
            let block_last = block_first + block_len - 1;
            let whole_block = valid_from.is_none_or(|valid| valid <= block_first);
            let key = (rev.number, rev.length, rev.root, stretch.run, block, index);
            let block_spans = if whole_block {
                if let Some(hit) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
                    hit
                } else {
                    let spans = Arc::new(unread_run_range(recording, stretch.run, log, block_first, block_last)?);
                    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(key, Arc::clone(&spans));
                    spans
                }
            } else {
                let b_from = block_first.max(valid_from.unwrap_or(i32::MIN));
                Arc::new(unread_run_range(recording, stretch.run, log, b_from, block_last)?)
            };
            for span in block_spans.iter() {
                if span.first_hour > s_to {
                    break;
                }
                if span.last_hour < s_from {
                    continue;
                }
                push_coalesced(&mut out, span.first_hour.max(s_from), span.last_hour.min(s_to), span.reason.clone());
            }
        }
    }
    Ok(out)
}

pub fn append_unobserved(out: &mut Vec<MissingSpan>, facets: &[FacetFilter], from: i32, to: i32) {
    if from > to {
        return;
    }
    let mut added = false;
    for filter in facets {
        for span in &filter.unobserved {
            let first_hour = span.first_hour.max(from);
            let last_hour = span.last_hour.min(to);
            if first_hour <= last_hour {
                out.push(MissingSpan { first_hour, last_hour, reason: GROUP_UNOBSERVED_REASON.to_owned() });
                added = true;
            }
        }
    }
    if added {
        out.sort_by_key(|span| (span.first_hour, span.last_hour));
        out.dedup();
    }
}

fn unknown_chosen(holders: &dashboard_query::identity::Holders, facets: &[FacetFilter], from: i32, to: i32) -> Vec<MissingSpan> {
    let mut out = Vec::new();
    if from > to {
        return out;
    }
    let mut chosen: Vec<(i32, i32)> = Vec::new();
    for filter in facets {
        if !filter.countries.is_empty() {
            chosen.push((from, to));
        }
        chosen.extend(filter.membership.iter().map(|span| (span.first_hour.max(from), span.last_hour.min(to))).filter(|(first, last)| first <= last));
    }
    if chosen.is_empty() {
        return out;
    }
    chosen.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::with_capacity(chosen.len());
    for (first, last) in chosen {
        match merged.last_mut() {
            Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
            _ => merged.push((first, last)),
        }
    }
    let chosen = merged;
    for (first, last, reason) in holders.chosen_not_known(from, to) {
        for &(lo, hi) in &chosen {
            let (first_hour, last_hour) = (first.max(lo), last.min(hi));
            if first_hour <= last_hour {
                out.push(MissingSpan { first_hour, last_hour, reason: reason.clone() });
            }
        }
    }
    out.sort_by_key(|span| (span.first_hour, span.last_hour));
    out.dedup();
    out
}

fn append_unknown_holders(out: &mut Vec<MissingSpan>, holders: &dashboard_query::identity::Holders, facets: &[FacetFilter], from: i32, to: i32) {
    let spans = unknown_chosen(holders, facets, from, to);
    if !spans.is_empty() {
        out.extend(spans);
        out.sort_by_key(|span| (span.first_hour, span.last_hour));
        out.dedup();
    }
}

fn count_not_known(holders: &dashboard_query::identity::Holders, ask: &LogFacetCountAsk, from: i32, to: i32) -> Option<String> {
    unknown_chosen(holders, &ask.facets, from, to).into_iter().next().map(|span| span.reason)
}

pub fn missing(query: &Query<'_>, log: &dashboard_declare::Log, from: i32, to: i32) -> io::Result<Vec<MissingSpan>> {
    missing_with(query, log, from, to, None)
}

pub fn missing_with(query: &Query<'_>, log: &dashboard_declare::Log, from: i32, to: i32, cache: Option<&Mutex<CoverageCache>>) -> io::Result<Vec<MissingSpan>> {
    if from > to {
        return Ok(Vec::new());
    }
    let mut out = unheld(query, from, to);
    out.extend(unread(query, log, from, to, cache)?);
    out.sort_by_key(|span| (span.first_hour, span.last_hour));
    Ok(out)
}

fn reader_for<'q, 'a>(query: &'q Query<'a>, log: &'static dashboard_declare::Log, caches: Option<&LogCaches>) -> io::Result<log::Reader<'q, 'a>> {
    match caches {
        Some(caches) => log::Reader::with_cache(query, log, Arc::clone(&caches.buckets)),
        None => log::Reader::new(query, log),
    }
}

pub fn tail<R>(
    query: &Query<'_>,
    log_name: &str,
    facets: &[FacetFilter],
    from: Option<f64>,
    to: i32,
    offset: u64,
    limit: u32,
    count_facets: &[LogFacetCountAsk],
    map: impl FnMut(&log::Paged, Option<&dashboard_query::identity::Country>) -> R,
) -> io::Result<(log::NewestPage, Vec<R>, Vec<OccurrenceCount>, Vec<MissingSpan>)> {
    tail_with(query, log_name, facets, from, to, offset, limit, count_facets, None, map)
}

pub fn tail_with<R>(
    query: &Query<'_>,
    log_name: &str,
    facets: &[FacetFilter],
    from: Option<f64>,
    to: i32,
    offset: u64,
    limit: u32,
    count_facets: &[LogFacetCountAsk],
    caches: Option<&LogCaches>,
    mut map: impl FnMut(&log::Paged, Option<&dashboard_query::identity::Country>) -> R,
) -> io::Result<(log::NewestPage, Vec<R>, Vec<OccurrenceCount>, Vec<MissingSpan>)> {
    let log = declared(log_name)?;
    let reader = reader_for(query, log, caches)?;
    let from = from.map(|from| hour_of(Some(from), "from")).transpose()?.unwrap_or(query.timeline.first);
    if !(1..=PAGE_LIMIT).contains(&limit) {
        return Err(invalid("Invalid tail limit"));
    }
    if from > to {
        let totals = count_facets.iter().map(|ask| OccurrenceCount { key: ask.key.clone(), total: "0".into(), missing: None }).collect();
        return Ok((log::NewestPage { base: 0, offset: 0, total: 0, rows: Vec::new() }, Vec::new(), totals, Vec::new()));
    }
    let selection = reader.select(facets, query.timeline.first, to)?;
    let page = reader.newest_page_with(&selection.patterns, selection.groups.len(), from, to, offset, limit, selection.overlap)?;
    let rows = page.rows.iter().map(|paged| map(paged, selection.groups[paged.group].as_ref())).collect();
    let mut totals = Vec::with_capacity(count_facets.len());
    for ask in count_facets {
        let count_selection = reader.select(&ask.facets, from, to)?;
        let count = reader.total_with(&count_selection.patterns, from, to, count_selection.overlap)?;
        totals.push(OccurrenceCount { key: ask.key.clone(), total: count.to_string(), missing: count_not_known(&query.holders, ask, from, to) });
    }
    let mut gaps = missing_with(query, log, from, to, caches.map(|c| c.coverage.as_ref()))?;
    append_unobserved(&mut gaps, facets, from, to);
    append_unknown_holders(&mut gaps, &query.holders, facets, from, to);
    Ok((page, rows, totals, gaps))
}

pub fn quiet_tail_missing(
    query: &Query<'_>,
    log_name: &str,
    caches: &LogCaches,
    selection: &log::Selection,
    count_selections: &[log::Selection],
    facets: &[FacetFilter],
    count_facets: &[LogFacetCountAsk],
    from: Option<f64>,
    prev_hour: i32,
    hour: i32,
) -> io::Result<Option<Vec<MissingSpan>>> {
    if hour <= prev_hour {
        return Ok(None);
    }
    let log = declared(log_name)?;
    let from = from.map(|from| hour_of(Some(from), "from")).transpose()?.unwrap_or(query.timeline.first);
    if hour < from {
        return Ok(Some(Vec::new()));
    }
    let reader = reader_for(query, log, Some(caches))?;
    let check_from = (prev_hour + 1).max(from);
    if reader.has_any_in(selection, check_from, hour)? {
        return Ok(None);
    }
    for cs in count_selections {
        if reader.has_any_in(cs, check_from, hour)? {
            return Ok(None);
        }
    }
    if count_facets.iter().any(|ask| count_not_known(&query.holders, ask, check_from, hour).is_some()) {
        return Ok(None);
    }
    let mut gaps = missing_with(query, log, from, hour, Some(caches.coverage.as_ref()))?;
    append_unobserved(&mut gaps, facets, from, hour);
    append_unknown_holders(&mut gaps, &query.holders, facets, from, hour);
    Ok(Some(gaps))
}

struct TailSeed<R> {
    hour: i32,
    reply: R,
    value: Arc<dashboard_follow::panel::PanelData>,
    rows: u64,
}

pub fn dated_tail_panel<R, F, Q>(
    executable: Option<engine_executable::executable_tables::Tables>,
    quiet: Q,
    read: F,
) -> Box<dyn Panel>
where
    R: TailPanelReply,
    Q: Fn(i32, i32, &AtomicBool) -> io::Result<Option<Vec<MissingSpan>>> + Send + Sync + 'static,
    F: Fn(i32, &AtomicBool) -> io::Result<R> + Send + Sync + 'static,
{
    let read = Arc::new(read);
    let step_read = Arc::clone(&read);
    let held: Arc<Mutex<Option<TailSeed<R>>>> = Arc::new(Mutex::new(None));
    let advance: ProductReader = Arc::new(move |hour, seed, cancelled, _step| {
        if seed {
            let guard = held.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(prev) = guard.as_ref() {
                if hour == prev.hour {
                    return ProductRead { value: Arc::clone(&prev.value), rows: prev.rows, fresh: false, pending: false, live: None, lanes: Vec::new() };
                }
                if hour > prev.hour {
                    let prev_hour = prev.hour;
                    let prev_reply = prev.reply.clone();
                    let prev_value = Arc::clone(&prev.value);
                    let prev_rows = prev.rows;
                    drop(guard);
                    let checked = engine_executable::executable_tables::scope(executable, || quiet(prev_hour, hour, cancelled));
                    match checked {
                        Ok(Some(gaps)) => {
                            if gaps == prev_reply.missing() {
                                let mut guard = held.lock().unwrap_or_else(PoisonError::into_inner);
                                if let Some(slot) = guard.as_mut()
                                    && slot.hour == prev_hour
                                {
                                    slot.hour = hour;
                                }
                                return ProductRead { value: prev_value, rows: prev_rows, fresh: false, pending: false, live: None, lanes: Vec::new() };
                            }
                            let mut next_reply = prev_reply;
                            next_reply.set_missing(gaps);
                            let value = Arc::new(serde_json::to_value(&next_reply).map_err(|error| error.to_string()));
                            *held.lock().unwrap_or_else(PoisonError::into_inner) = Some(TailSeed {
                                hour,
                                reply: next_reply,
                                value: Arc::clone(&value),
                                rows: prev_rows,
                            });
                            return ProductRead { value, rows: prev_rows, fresh: true, pending: false, live: None, lanes: Vec::new() };
                        }
                        Ok(None) => {}
                        Err(error) => {
                            return ProductRead { value: Arc::new(Err(error.to_string())), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() };
                        }
                    }
                }
            }
        }
        let made = engine_executable::executable_tables::scope(executable, || step_read(hour, cancelled));
        let (rows, value) = match made {
            Ok(reply) => {
                let count = reply.row_count() as u64;
                let value = Arc::new(serde_json::to_value(&reply).map_err(|error| error.to_string()));
                *held.lock().unwrap_or_else(PoisonError::into_inner) = Some(TailSeed {
                    hour,
                    reply,
                    value: Arc::clone(&value),
                    rows: count,
                });
                (count, value)
            }
            Err(error) => (0, Arc::new(Err(error.to_string()))),
        };
        ProductRead { value, rows, fresh: true, pending: false, live: None, lanes: Vec::new() }
    });
    let cold: ExactReader = Arc::new(move |hour, cancelled| {
        let reply = engine_executable::executable_tables::scope(executable, || read(hour, cancelled)).map_err(|error| error.to_string())?;
        serde_json::to_value(reply).map_err(|error| error.to_string())
    });
    super::at_hour::product_panel(advance, cold)
}

#[cfg(test)]
#[test]
fn a_country_filter_over_hours_whose_holders_are_not_known_names_them_a_gap_never_an_empty_count() {
    use dashboard_query::identity::{Country, Holders};
    let reason = "Countries was not complete at these hours";
    let holders = Holders::new(Default::default(), vec![(110, 120, reason.to_owned())]);
    let by_country = FacetFilter { name: "country".into(), values: Vec::new(), countries: vec![Country::from("GER")], membership: Vec::new(), unobserved: Vec::new(), deduplicate: false };
    let mut gaps = Vec::new();
    append_unknown_holders(&mut gaps, &holders, std::slice::from_ref(&by_country), 100, 115);
    assert_eq!(gaps, vec![MissingSpan { first_hour: 110, last_hour: 115, reason: format!("Who holds the chosen countries is not known: {reason}") }]);
    let unfiltered = FacetFilter { countries: Vec::new(), ..by_country };
    let mut none = Vec::new();
    append_unknown_holders(&mut none, &holders, std::slice::from_ref(&unfiltered), 100, 115);
    assert!(none.is_empty(), "without a country filter nothing depends on who holds what");
    let member = FacetFilter {
        membership: vec![dashboard_query::log::CountrySpan { country: Country::from("GER"), first_hour: 114, last_hour: 130 }],
        ..unfiltered
    };
    let mut dated = Vec::new();
    append_unknown_holders(&mut dated, &holders, std::slice::from_ref(&member), 100, 130);
    assert_eq!(dated, vec![MissingSpan { first_hour: 114, last_hour: 120, reason: format!("Who holds the chosen countries is not known: {reason}") }], "a dated membership names its countries only within its span");
    let members = FacetFilter {
        membership: vec![
            dashboard_query::log::CountrySpan { country: Country::from("GER"), first_hour: 110, last_hour: 118 },
            dashboard_query::log::CountrySpan { country: Country::from("ITA"), first_hour: 114, last_hour: 120 },
        ],
        ..member.clone()
    };
    let mut overlapping = Vec::new();
    append_unknown_holders(&mut overlapping, &holders, std::slice::from_ref(&members), 100, 130);
    assert_eq!(overlapping, vec![MissingSpan { first_hour: 110, last_hour: 120, reason: format!("Who holds the chosen countries is not known: {reason}") }], "overlapping memberships make one gap for one outage");
    let counted = LogFacetCountAsk { key: "ger".into(), facets: vec![FacetFilter { countries: vec![Country::from("GER")], ..FacetFilter { name: "country".into(), values: Vec::new(), countries: Vec::new(), membership: Vec::new(), unobserved: Vec::new(), deduplicate: false } }] };
    assert_eq!(count_not_known(&holders, &counted, 100, 115), Some(format!("Who holds the chosen countries is not known: {reason}")), "a count of a chosen country carries its own reason");
    assert_eq!(count_not_known(&holders, &counted, 121, 130), None, "outside the unknown hours it is a number");
}
