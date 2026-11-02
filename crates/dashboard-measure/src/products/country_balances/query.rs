//! Country balances' replies (series family `country-measure`), read from the family's series
//! publication ([`super::emit`]): values with their holder at hours and the drop there, charts
//! from each tag's first hour with the drops as marks, keyboard navigation, pages of drops in
//! hour order, and the scoreboard's cells.

use super::emit::{DECIMALS, DROP, MEASURES, measure};
use dashboard_base::{
    prepared::{
        engine,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartSummary, MarkSummary},
    prepared_measure::{
        MeasureRun,
        query::{
            MeasureDropReply, MeasureEvent, MeasureKeyReply, MeasureMarkReply, MeasureRead,
            MeasureState, MeasureStateReply, MeasureSummaryLine, MeasureSummaryReply,
        },
    },
    products::{
        country_groups::HourSpan,
        country_measure::{BalanceDrop, Measure},
    },
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, codec::Binary,
    family::Directory, query::EventView,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};

/// Bytes one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 20;

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Family {
    #[serde(rename = "country-measure")]
    Balances { measure: Measure },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeriesParams {
    branch_id: String,
    family: Family,
    #[serde(default)]
    measures: Vec<String>,
    #[serde(default)]
    within: BTreeMap<String, Vec<HourSpan>>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<MeasureRead>,
    tag: Option<String>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    key: Option<MeasureKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
    #[serde(default)]
    with_marks: bool,
    cursor: Option<String>,
    page_size: Option<u32>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// `measure`'s name, key measure and noun.
fn named(measure: Measure) -> (&'static str, u64, &'static str) {
    match measure {
        Measure::PoliticalPower => ("politicalPower", MEASURES[0], "political power"),
        Measure::CommandPower => ("commandPower", MEASURES[1], "command power"),
    }
}

/// A reply's warnings: each tag with no key, or a key never valued, has no recorded balance in
/// this branch.
struct Warnings<'d> {
    directory: &'d Directory,
    measure: u64,
    noun: &'static str,
    checked: HashMap<String, bool>,
    lines: Vec<String>,
}

impl Warnings<'_> {
    fn warn<P: PageRead>(&mut self, snapshot: &mut Snapshot<'_, P>, tag: &str) -> io::Result<()> {
        let recorded = match self.checked.get(tag) {
            Some(&recorded) => recorded,
            None => {
                let recorded = match key(self.directory, tag, self.measure) {
                    Some((key, _)) => valued(snapshot, key)?,
                    None => false,
                };
                self.checked.insert(tag.to_owned(), recorded);
                recorded
            }
        };
        let line = format!("{tag}: no recorded {} in this branch", self.noun);
        if !recorded && !self.lines.contains(&line) {
            self.lines.push(line);
        }
        Ok(())
    }
}

/// A tag's key of `measure`, and its tag's first hour.
fn key(directory: &Directory, tag: &str, measure: u64) -> Option<(u64, i32)> {
    let actor = directory.actor(tag.as_bytes())?;
    Some((
        directory.key(actor, measure, None)?,
        directory.actors[actor as usize].first_hour,
    ))
}

fn every(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

/// Whether `key` ever held a value.
fn valued<P: PageRead>(snapshot: &mut Snapshot<'_, P>, key: u64) -> io::Result<bool> {
    let page = snapshot.run_page(
        every(key),
        Span::new(i32::MIN, i32::MAX)?,
        None,
        dashboard_series::Run::SIZE,
        |_| Ok(()),
    )?;
    Ok(page.returned > 0)
}

/// The run of `key` at `hour` with the holder its evidence names.
fn measure_run<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<MeasureRun>> {
    let Some((run, evidence)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let holder = evidence
        .holder
        .ok_or_else(|| invalid("Missing measure holder segment"))?;
    Ok(Some(MeasureRun {
        first_hour: run.span.first,
        last_hour: run.span.last,
        value_raw: run.value.to_string(),
        slot: (holder >> 32) as u32,
        country_id: holder as u32,
        starts_segment: run.break_before,
        segment_first_hour: evidence.identity_first.unwrap_or(evidence.span.first),
        read_hour: evidence.supplying_read,
    }))
}

/// The drop an event holds.
fn drop_of<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: EventView) -> io::Result<BalanceDrop> {
    let EventView::Family(event) = event else {
        return Err(invalid("A balance event is not a drop"));
    };
    if event.kind != DROP || event.fields.len != 2 * Cell::SIZE as u64 {
        return Err(invalid("A balance event is not a drop"));
    }
    let mut amount = |at| match snapshot.field(event.fields, at)? {
        Cell::Integer(value) => Ok(value),
        _ => Err(invalid("A balance drop amount is not an integer")),
    };
    let before = amount(0)?;
    let after = amount(1)?;
    Ok(BalanceDrop {
        hour: event.native_hour,
        before_raw: before.to_string(),
        after_raw: after.to_string(),
    })
}

fn events_through<'a>(selection: &'a [Selection<'a>], last: i32) -> EventQuery<'a> {
    EventQuery {
        selection,
        native: Span {
            first: i32::MIN,
            last,
        },
        as_of: None,
        order: EventOrder::Native,
    }
}

/// The selected keys' drops through `hour`.
fn count_through<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    selection: &[Selection<'_>],
    hour: i32,
) -> io::Result<u64> {
    if selection.is_empty() {
        return Ok(0);
    }
    snapshot.event_count(events_through(selection, hour))
}

/// The drop of `key` at `hour`, if it fell then.
fn drop_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<BalanceDrop>> {
    let selection = [every(key)];
    let mut found = None;
    snapshot.event_page(
        EventQuery {
            selection: &selection,
            native: Span::new(hour, hour)?,
            as_of: None,
            order: EventOrder::Native,
        },
        None,
        BUDGET,
        |event| {
            found = found.or(Some(event));
            Ok(())
        },
    )?;
    found.map(|event| drop_of(snapshot, event)).transpose()
}

/// The page of the keys' drops from merged ordinal `cursor` within `from..=to`, in hour, key and
/// per-key order: `(first ordinal, total, rows, next cursor)`. Ordinals count from the branch's
/// first drop; a cursor before `from`'s first drop starts there.
#[allow(clippy::type_complexity)]
fn drops_page<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &BTreeMap<u64, String>,
    (from, to): (i32, i32),
    cursor: u64,
    page_size: u32,
    cancelled: &AtomicBool,
) -> io::Result<(u64, u64, Vec<MeasureEvent>, Option<u64>)> {
    if from > to {
        return Err(invalid("invalid event page"));
    }
    let selection: Vec<_> = keys.keys().map(|&key| every(key)).collect();
    let base = match from.checked_sub(1) {
        Some(before) => count_through(snapshot, &selection, before)?,
        None => 0,
    };
    let last = count_through(snapshot, &selection, to)?;
    let first = cursor.max(base);
    let end = first.saturating_add(u64::from(page_size)).min(last);
    let mut rows = Vec::new();
    if first < end {
        // The first row's hour: the least through which more than `first` drops fell.
        let (mut low, mut high) = (i64::from(from), i64::from(to));
        while low < high {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let middle = low + (high - low) / 2;
            if count_through(snapshot, &selection, middle as i32)? > first {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        let hour = low as i32;
        let mut ordinals = HashMap::with_capacity(keys.len());
        let mut before = 0_u64;
        for &key in keys.keys() {
            let held = match hour.checked_sub(1) {
                Some(last) => count_through(snapshot, &[every(key)], last)?,
                None => 0,
            };
            before += held;
            ordinals.insert(key, held);
        }
        let mut skip = first - before;
        let mut wanted = end - first;
        let query = EventQuery {
            selection: &selection,
            native: Span::new(hour, to)?,
            as_of: None,
            order: EventOrder::Native,
        };
        let mut page = None;
        while wanted > 0 {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let mut events = Vec::new();
            let result = snapshot.event_page(query, page, BUDGET, |event| {
                events.push(event);
                Ok(())
            })?;
            for event in events {
                let ordinal = ordinals
                    .get_mut(&event.key())
                    .ok_or_else(|| invalid("A balance drop of no requested key"))?;
                let at = *ordinal;
                *ordinal += 1;
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                if wanted == 0 {
                    break;
                }
                wanted -= 1;
                let drop = drop_of(snapshot, event)?;
                rows.push(MeasureEvent {
                    tag: keys[&event.key()].clone(),
                    hour: drop.hour,
                    ordinal: at.to_string(),
                    row: serde_json::to_value(drop)?,
                });
            }
            page = result.next;
            if page.is_none() {
                break;
            }
        }
        if wanted > 0 {
            return Err(invalid("Balance drop counts disagree with their rows"));
        }
    }
    Ok((first, last - base, rows, (end < last).then_some(end)))
}

/// Each tag's `measure_name` at `hour`, for the scoreboard; None where the tag holds no value.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    measure_name: &str,
    hour: i32,
) -> io::Result<Vec<Option<i128>>> {
    let measure = measure(measure_name).ok_or_else(|| invalid("Unknown balance measure"))?;
    tags.iter()
        .map(|tag| match key(directory, tag, measure) {
            Some((key, _)) => Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| run.value)),
            None => Ok(None),
        })
        .collect()
}

/// Answers `product` for the `country-measure` series family.
pub fn query<P: PageRead>(
    product: &str,
    params: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: SeriesParams = serde_json::from_value(params)?;
    let Family::Balances { measure: chosen } = params.family;
    let (name, measure, noun) = named(chosen);
    let name = name.to_owned();
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid series branch"))?;
    let last = snapshot.publication().accepted.last;
    let mut warnings = Warnings {
        directory,
        measure,
        noun,
        checked: HashMap::new(),
        lines: Vec::new(),
    };
    let empty_marks = |from, to, width| MarkSummary {
        from,
        to,
        width,
        clusters: Vec::new(),
    };
    Ok(match product {
        "series-state" => {
            let mut states = Vec::with_capacity(params.reads.len());
            for read in params.reads {
                dashboard_text::metadata::check_cancel(cancelled)?;
                dashboard_core::protocol::validate_tag(&read.tag).map_err(invalid)?;
                let (run, drop) = match key(directory, &read.tag, measure) {
                    Some((key, _)) => (
                        measure_run(snapshot, key, read.hour)?,
                        drop_at(snapshot, key, read.hour)?,
                    ),
                    None => (None, None),
                };
                warnings.warn(snapshot, &read.tag)?;
                states.push(MeasureState {
                    approximate: false,
                    sample_hour: None,
                    experience: None,
                    segment_first_hour: None,
                    available: None,
                    read_hour: run.as_ref().and_then(|run| run.read_hour),
                    recorded: None,
                    tag: read.tag,
                    hour: read.hour,
                    run,
                    drop,
                });
            }
            serde_json::to_value(MeasureStateReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                decimals: DECIMALS,
                scale: None,
                day_hours: None,
                states,
                warnings: warnings.lines,
            })?
        }
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, params.tags.len())?;
            let mut lines = Vec::with_capacity(params.tags.len());
            for tag in params.tags {
                dashboard_text::metadata::check_cancel(cancelled)?;
                dashboard_core::protocol::validate_tag(&tag).map_err(invalid)?;
                let found = key(directory, &tag, measure);
                let summary = match found {
                    Some((key, first)) => engine::summary(
                        snapshot,
                        key,
                        (first, last),
                        (from, to, width),
                        DECIMALS,
                        params.within.get(&tag).map(Vec::as_slice),
                    )?,
                    None => ChartSummary {
                        from,
                        to,
                        width,
                        buckets: Vec::new(),
                        runs: Some(Vec::new()),
                        approximate: Vec::new(),
                    },
                };
                let marks = if params.with_marks {
                    Some(match found {
                        Some((key, _)) => engine::clusters(snapshot, key, (from, to, width))?,
                        None => empty_marks(from, to, width),
                    })
                } else {
                    None
                };
                warnings.warn(snapshot, &tag)?;
                lines.push(MeasureSummaryLine {
                    tag,
                    summary,
                    marks,
                });
            }
            serde_json::to_value(MeasureSummaryReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                decimals: DECIMALS,
                series: lines,
                warnings: warnings.lines,
            })?
        }
        "series-next-change" => {
            let (first_hour, last_hour) = series::range(params.first_hour, params.last_hour)?;
            let navigation = Navigation {
                from: params.from,
                first_hour,
                last_hour,
                key: params
                    .key
                    .ok_or_else(|| invalid("Missing navigation direction"))?,
            };
            // Only the family's own balance moves; another measure named has no changes here.
            let navigates =
                params.measures.is_empty() || params.measures.iter().any(|other| *other == name);
            let mut keys = Vec::new();
            for tag in &params.tags {
                if let Some((key, _)) = key(directory, tag, measure).filter(|_| navigates) {
                    keys.push((key, params.within.get(tag).map(Vec::as_slice)));
                }
            }
            serde_json::to_value(MeasureKeyReply {
                hour: engine::navigate(snapshot, &keys, &navigation)?,
                approximate: false,
                coverage: None,
            })?
        }
        "events-page" => {
            let (from, to, cursor, page_size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                params.page_size,
            )?;
            let mut keys = BTreeMap::new();
            for tag in params.tags {
                dashboard_core::protocol::validate_tag(&tag).map_err(invalid)?;
                warnings.warn(snapshot, &tag)?;
                if let Some((key, _)) = key(directory, &tag, measure) {
                    keys.insert(key, tag);
                }
            }
            let (first, total, rows, next) =
                drops_page(snapshot, &keys, (from, to), cursor, page_size, cancelled)?;
            serde_json::to_value(MeasureDropReply {
                coverage: None,
                first_ordinal: first.to_string(),
                total: total.to_string(),
                rows,
                warnings: warnings.lines,
                cursor: next.map(|next| next.to_string()),
            })?
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            let tag = params.tag.ok_or_else(|| invalid("Missing event key"))?;
            dashboard_core::protocol::validate_tag(&tag).map_err(invalid)?;
            let summary = match key(directory, &tag, measure) {
                Some((key, _)) => engine::clusters(snapshot, key, (from, to, width))?,
                None => empty_marks(from, to, width),
            };
            warnings.warn(snapshot, &tag)?;
            serde_json::to_value(MeasureMarkReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                tag,
                summary,
                warnings: warnings.lines,
            })?
        }
        _ => return Err(invalid("Unknown series query")),
    })
}
