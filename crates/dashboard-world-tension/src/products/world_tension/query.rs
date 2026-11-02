//! World tension's replies, read from the family's series publication ([`super::emit`]): the
//! chart summaries of world tension, the contributions' sum and the selected contributors, the
//! reads and contributions at the hours asked, the threat events, and keyboard navigation over
//! reads and events.

use super::{
    ScannedTension, TensionAdded, TensionCountry, TensionEvent, TensionRead,
    emit::{
        COUNTS, DECIMALS, EVENTS, FAMILY, READ, SHARE, SUM, UNNAMED, VALUE, WORLD, ZERO, contributor,
        kind,
    },
};
use dashboard_base::{
    Source,
    prepared::{
        engine,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartRun, ChartSummary},
    products::{
        BranchParams, country_groups::HourSpan, country_history::RecordedHistories,
        country_measure::FixedRun,
    },
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot,
    codec::Binary,
    family::{Directory, Scalar},
    query::{Alias, EventView},
};
use engine_stability::cworld_threat;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
    sync::atomic::AtomicBool,
};

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Requested {
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    reads: Vec<Read>,
    #[serde(default)]
    within: BTreeMap<String, Vec<(String, i32, i32, bool)>>,
    #[serde(default)]
    share: bool,
}

#[derive(Deserialize)]
struct Read {
    hour: i32,
}

/// A contributor as the publication catalogued it.
struct Contributor {
    actor: u64,
    tag: String,
    slot: u32,
    first_hour: i32,
}

/// What every reply of one publication reads whole: the threat events, in reply order, the
/// hours a read holds, and the reads' counts.
#[derive(Clone, Debug, Default)]
pub struct Published {
    pub events: Vec<TensionEvent>,
    pub coverage: Vec<HourSpan>,
    pub counts: [u64; 5],
}

impl Published {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.coverage.capacity() * std::mem::size_of::<HourSpan>()
            + self
                .events
                .iter()
                .map(|event| {
                    std::mem::size_of::<TensionEvent>()
                        + event.reason.capacity()
                        + event.tag.as_ref().map_or(0, String::capacity)
                        + event
                            .added
                            .iter()
                            .map(|added| std::mem::size_of::<TensionAdded>() + added.added_raw.capacity())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// The run of `key` holding `hour`, whole.
fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(match key {
        Some(key) => engine::run_at(snapshot, key, hour)?.map(|(run, _)| run),
        None => None,
    })
}

fn family_key(directory: &Directory, measure: u64) -> Option<u64> {
    directory.key(FAMILY, measure, None)
}

/// The text of a text or null event field.
fn text<P: PageRead>(snapshot: &mut Snapshot<'_, P>, cell: Cell) -> io::Result<Option<String>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Text(blob) => {
            let length = usize::try_from(blob.len).map_err(|_| invalid("Prepared tension text is not valid"))?;
            let mut bytes = vec![0; length];
            snapshot.cell_bytes(blob, 0, &mut bytes)?;
            String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| invalid("Prepared tension text is not valid"))
        }
        _ => Err(invalid("Prepared tension text is not valid")),
    }
}

/// The threat events, the read hours and the counts the publication pinned in `snapshot` holds.
pub fn published<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<Published> {
    let malformed = || invalid("A prepared threat event is not valid");
    let accepted = snapshot.publication().accepted;
    let coverage: Vec<HourSpan> = match family_key(directory, ZERO) {
        Some(key) => engine::runs(snapshot, key, accepted.first, accepted.last)?
            .into_iter()
            .map(|run| HourSpan {
                first_hour: run.span.first,
                last_hour: run.span.last,
            })
            .collect(),
        None => Vec::new(),
    };
    // Each listed event was last read by the last read.
    let last_read = match coverage.last() {
        Some(span) => run_at(snapshot, family_key(directory, READ), span.last_hour)?
            .map(|run| i32::try_from(run.value).map_err(|_| malformed()))
            .transpose()?,
        None => None,
    };
    let mut events: Vec<TensionEvent> = Vec::new();
    let mut listed: Vec<bool> = Vec::new();
    if let Some(key) = family_key(directory, EVENTS) {
        let selection = [Selection {
            key,
            membership: None,
        }];
        let mut cursor = None;
        loop {
            let mut page = Vec::new();
            let next = snapshot.event_page(
                EventQuery {
                    selection: &selection,
                    native: accepted,
                    as_of: None,
                    order: EventOrder::Native,
                },
                cursor,
                1 << 20,
                |event| {
                    page.push(event);
                    Ok(())
                },
            )?;
            for event in page {
                let EventView::Family(event) = event else {
                    return Err(malformed());
                };
                let cells = event.fields.len / Cell::SIZE as u64;
                let mut fields = Vec::with_capacity(cells as usize);
                for at in 0..cells {
                    fields.push(snapshot.field(event.fields, at)?);
                }
                let id = match fields.first() {
                    Some(Cell::Unsigned(id)) => usize::try_from(*id).map_err(|_| malformed())?,
                    _ => return Err(malformed()),
                };
                let hour = event.native_hour;
                let integer = |cell: Option<&Cell>| match cell {
                    Some(Cell::Integer(value)) => Ok(*value),
                    _ => Err(malformed()),
                };
                if event.kind == kind::SOURCE {
                    if id != events.len() || fields.len() != 5 {
                        return Err(malformed());
                    }
                    let date = i32::try_from(integer(fields.get(1))?).map_err(|_| malformed())?;
                    let reason = text(snapshot, fields[2])?.ok_or_else(malformed)?;
                    let tag = text(snapshot, fields[3])?;
                    events.push(TensionEvent {
                        hour: date,
                        tag,
                        reason,
                        added: vec![TensionAdded {
                            hour,
                            added_raw: integer(fields.get(4))?.to_string(),
                        }],
                        first_read: hour,
                        last_read: hour,
                    });
                    listed.push(true);
                    continue;
                }
                let held = events.get_mut(id).ok_or_else(malformed)?;
                match event.kind {
                    kind::ADDED => held.added.push(TensionAdded {
                        hour,
                        added_raw: integer(fields.get(1))?.to_string(),
                    }),
                    kind::NAMED => {
                        held.tag = Some(text(snapshot, *fields.get(1).ok_or_else(malformed)?)?.ok_or_else(malformed)?)
                    }
                    kind::DROPPED => {
                        held.last_read = i32::try_from(integer(fields.get(1))?).map_err(|_| malformed())?;
                        listed[id] = false;
                    }
                    kind::RESUMED => listed[id] = true,
                    _ => return Err(malformed()),
                }
            }
            cursor = next.next;
            if cursor.is_none() {
                break;
            }
        }
    }
    for (event, listed) in events.iter_mut().zip(listed) {
        if listed {
            event.last_read = last_read.ok_or_else(malformed)?;
        }
    }
    events.sort_by_key(|event| (event.hour, event.first_read));
    let counts = directory.scalar(COUNTS).map_or(&[][..], Scalar::list);
    let mut held = [0; 5];
    for (held, count) in held.iter_mut().zip(counts) {
        *held = count
            .integer()
            .and_then(|count| u64::try_from(count).ok())
            .ok_or_else(|| invalid("Prepared tension counts are not valid"))?;
    }
    Ok(Published {
        events,
        coverage,
        counts: held,
    })
}

/// The read holding `hour`.
pub(super) fn read_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hour: i32,
) -> io::Result<Option<TensionRead>> {
    let Some(run) = run_at(snapshot, family_key(directory, READ), hour)? else {
        return Ok(None);
    };
    let mut values = [0_i128; 3];
    for (value, measure) in values.iter_mut().zip([WORLD, SUM, UNNAMED]) {
        *value = run_at(snapshot, family_key(directory, measure), hour)?
            .ok_or_else(|| invalid("Missing tension value at a known read"))?
            .value;
    }
    Ok(Some(TensionRead {
        first_hour: i32::try_from(run.value).map_err(|_| invalid("Invalid tension read hour"))?,
        last_hour: run.span.last,
        starts_segment: run.break_before,
        tension_raw: (values[0] / i128::from(cworld_threat::SCALAR_DIVISOR)).to_string(),
        sum_raw: values[1].to_string(),
        unnamed_raw: Some(values[2].to_string()),
        preparing: Vec::new(),
    }))
}

/// A series key names a contributor by its catalogue key, or by its recorded tag and slot.
fn country_index(contributors: &[Contributor], key: &str) -> io::Result<Option<usize>> {
    if key.starts_with('[') {
        let (tag, slot): (String, u32) = serde_json::from_str(key)?;
        Ok(contributors
            .iter()
            .position(|country| country.tag == tag && country.slot == slot))
    } else {
        let index = key
            .parse::<usize>()
            .map_err(|_| invalid("Invalid tension series key"))?;
        if index >= contributors.len() {
            return Err(invalid("Unknown tension series key"));
        }
        Ok(Some(index))
    }
}

/// `spans` of keys (key, first, last) as one chart over `view`, each key read only within its
/// span; an empty chart where no span meets the view.
fn summary_spans<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    spans: &[(u64, i32, i32)],
    bounds: (i32, i32),
    view: (f64, f64, u32),
) -> io::Result<ChartSummary> {
    series::viewport(Some(view.0), Some(view.1), Some(view.2))?;
    if spans.iter().any(|span| span.1 > span.2)
        || spans.windows(2).any(|pair| pair[0].2 >= pair[1].1)
    {
        return Err(invalid("Invalid dated series spans"));
    }
    let aliases = spans
        .iter()
        .filter(|span| f64::from(span.2) >= view.0.floor() && f64::from(span.1) <= view.1.floor())
        .map(|&(key, first, last)| {
            Ok(Alias {
                key,
                span: dashboard_series::Span::new(first, last)?,
                boundary: None,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let (from, to, width) = view;
    if aliases.is_empty() {
        return Ok(ChartSummary {
            from,
            to,
            width,
            buckets: Vec::new(),
            runs: Some(Vec::new()),
            approximate: Vec::new(),
        });
    }
    let bounds = dashboard_series::Span::new(bounds.0, bounds.1)?;
    let viewport = dashboard_series::Viewport { from, to, width };
    let mut buckets = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.summary_aliases(&aliases, bounds, viewport, cursor, BUDGET, |_, bin| {
            buckets.push(bin.exact.map(engine::bucket).transpose()?);
            Ok(())
        })?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    let (exact, _) = series::summary_bins((bounds.first, bounds.last), view)?;
    let runs = if exact {
        let scale = 10_f64.powi(i32::from(DECIMALS));
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot.hourly_alias_runs(&aliases, bounds, viewport, cursor, BUDGET, |run| {
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

/// A series request of the `world-tension` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from `snapshot` and what `published` read of it whole.
#[allow(clippy::too_many_arguments)]
pub fn series<P: PageRead>(
    source: &Source,
    product: &str,
    value: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    published: &Published,
    history: &RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: BranchParams = serde_json::from_value(json!({"branchId":value["branchId"]}))?;
    params.validate().map_err(dashboard_core::invalid)?;
    let requested: Requested = serde_json::from_value(value.clone())?;
    let reached_hours: Vec<_> = requested.reads.iter().map(|read| read.hour).collect();
    let navigation = if product == "series-next-change" {
        Some(serde_json::from_value::<Navigation>(value.clone())?)
    } else {
        None
    };
    let (from, to, width) = if product == "series-summary" {
        series::viewport(requested.from, requested.to, requested.width)?
    } else if let Some(navigation) = &navigation {
        (navigation.first_hour, navigation.last_hour, 1)
    } else {
        let hour = f64::from(reached_hours.first().copied().unwrap_or(0));
        (hour, hour, 1)
    };
    series::summary_budget(from, to, width, requested.within.len() + 2)?;
    let share = requested.share;
    let branch = params.branch();
    let bounds = source
        .with_metadata(|metadata| {
            metadata
                .branches
                .iter()
                .find(|item| item.id.parse::<u64>().ok() == Some(branch))
                .map(|item| (item.first_hour, item.last_hour))
        })
        .ok_or_else(|| invalid("Unknown tension branch"))?;
    let contributors = directory
        .actors
        .iter()
        .enumerate()
        .map(|(actor, held)| {
            let (slot, tag) = contributor(&held.name)
                .ok_or_else(|| invalid("A prepared tension contributor is not valid"))?;
            Ok(Contributor {
                actor: actor as u64,
                tag: tag.to_owned(),
                slot,
                first_hour: held.first_hour,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let reached: BTreeSet<_> = reached_hours.iter().copied().collect();
    let mut hours = reached.clone();
    if !requested.within.is_empty()
        && to.floor().min(f64::from(bounds.1)) - from.floor().max(f64::from(bounds.0)) + 1.0
            < 2.0 * f64::from(width)
        && let Some(key) = family_key(directory, READ)
    {
        hours.extend(
            engine::runs(snapshot, key, from.floor() as i32, to.floor() as i32)?
                .into_iter()
                .map(|run| run.span.first),
        );
    }
    let mut reads = Vec::new();
    for &hour in &hours {
        if let Some(read) = read_at(snapshot, directory, hour)? {
            reads.push(read);
        }
    }
    reads.sort_by_key(|read| read.first_hour);
    reads.dedup();
    let mut selected_keys = HashSet::new();
    for (key, _, _, _) in requested.within.values().flatten() {
        if let Some(index) = country_index(&contributors, key)? {
            selected_keys.insert(index);
        }
    }
    let mut order: Vec<usize> = (0..contributors.len()).collect();
    order.sort_by(|&a, &b| {
        (&contributors[a].tag, contributors[a].slot).cmp(&(&contributors[b].tag, contributors[b].slot))
    });
    let first_contributions = order.iter().map(|&at| contributors[at].first_hour).collect();
    let mut countries = Vec::with_capacity(order.len());
    for &index in &order {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let country = &contributors[index];
        let key = directory.key(country.actor, VALUE, None);
        let mut runs = Vec::new();
        let mut windows: Vec<(i32, i32)> = Vec::new();
        let selected = selected_keys.contains(&index);
        for &hour in if selected { &hours } else { &reached } {
            if hour < country.first_hour || hour > bounds.1 {
                continue;
            }
            match windows.last_mut() {
                Some((_, last)) if last.checked_add(1) == Some(hour) => *last = hour,
                _ => windows.push((hour, hour)),
            }
        }
        if let Some(key) = key {
            for (first, last) in windows {
                // Each window read once; a run it clips is read whole where it goes on past it.
                let mut values = engine::runs(snapshot, key, first, last)?;
                if let Some(value) = values.first_mut()
                    && value.span.first == first
                    && first > country.first_hour
                    && let Some(canonical) = run_at(snapshot, Some(key), first)?
                {
                    *value = canonical;
                }
                if let Some(value) = values.last_mut()
                    && value.span.last == last
                    && last < bounds.1
                    && let Some(canonical) = run_at(snapshot, Some(key), last)?
                {
                    *value = canonical;
                }
                runs.extend(values.into_iter().filter(|run| run.value != 0).map(|run| FixedRun {
                    first_hour: run.span.first,
                    last_hour: run.span.last,
                    value_raw: run.value.to_string(),
                    starts_segment: run.break_before,
                }));
            }
        }
        runs.sort_by_key(|run| run.first_hour);
        runs.dedup();
        countries.push(TensionCountry {
            tag: country.tag.clone(),
            slot: country.slot,
            runs,
            identities: None,
        });
    }
    let mut summaries = BTreeMap::new();
    let view = (from, to, width);
    let world = family_key(directory, WORLD);
    let sum = family_key(directory, SUM);
    let zero = family_key(directory, ZERO);
    if product == "series-summary" {
        let mut chart = |key: Option<u64>| match key {
            Some(key) => engine::summary(snapshot, key, bounds, view, DECIMALS, None),
            None => summary_spans(snapshot, &[], bounds, view),
        };
        summaries.insert("world".to_owned(), chart(world)?);
        summaries.insert("sum".to_owned(), chart(if share { world } else { sum })?);
    }
    for (id, selected_spans) in &requested.within {
        if selected_spans.iter().any(|(key, _, _, _)| {
            country_index(&contributors, key).is_ok_and(|index| index.is_none())
        }) {
            continue;
        }
        let Some(zero) = zero else {
            if product == "series-summary" {
                summaries.insert(id.clone(), summary_spans(snapshot, &[], bounds, view)?);
            }
            continue;
        };
        let mut spans = Vec::new();
        let mut first = i64::from(bounds.0);
        for (tag, span_first, span_last, _) in selected_spans {
            let Some(index) = country_index(&contributors, tag)? else {
                continue;
            };
            let country = &contributors[index];
            let lo = (*span_first).max(bounds.0);
            let hi = (*span_last).min(bounds.1);
            if lo <= hi {
                if first < i64::from(lo) {
                    spans.push((zero, first as i32, lo - 1));
                }
                let known = lo.max(country.first_hour);
                if lo < known {
                    spans.push((zero, lo, hi.min(known - 1)));
                }
                if known <= hi {
                    let key = directory
                        .key(country.actor, if share { SHARE } else { VALUE }, None)
                        .ok_or_else(|| invalid("A prepared tension contributor has no series"))?;
                    spans.push((key, known, hi));
                }
                first = i64::from(hi) + 1;
            }
        }
        if first <= i64::from(bounds.1) {
            spans.push((zero, first as i32, bounds.1));
        }
        if product == "series-summary" {
            summaries.insert(id.clone(), summary_spans(snapshot, &spans, bounds, view)?);
        }
    }
    if let Some(navigation) = &navigation {
        let origin = navigation.origin()?;
        let forward = matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight);
        let read = match family_key(directory, READ) {
            Some(key) => engine::navigate(
                snapshot,
                &[(key, Some(published.coverage.as_slice()))],
                &Navigation {
                    from: Some(origin),
                    first_hour: from,
                    last_hour: to,
                    key: if forward {
                        MeasureKey::ArrowRight
                    } else {
                        MeasureKey::ArrowLeft
                    },
                },
            )?,
            None => None,
        }
        .filter(|hour| if forward { *hour > origin } else { *hour < origin })
        .map(|hour| hour as i32);
        let event = published
            .events
            .iter()
            .map(|event| event.hour)
            .filter(|hour| {
                if forward {
                    f64::from(*hour) > origin
                } else {
                    f64::from(*hour) < origin
                }
            })
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
        let hour = [read, event]
            .into_iter()
            .flatten()
            .filter(|hour| from <= f64::from(*hour) && f64::from(*hour) <= to)
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
        return Ok(json!({ "hour": hour }));
    }
    let [read_count, unnamed_reads, partial_sources, aggregate_mismatches, tension_mismatches] =
        published.counts.map(|count| count as usize);
    let scanned = ScannedTension {
        reads,
        countries,
        events: published.events.clone(),
        aggregate_mismatches,
        tension_mismatches,
        partial_sources,
        unnamed_reads,
        first_contributions,
        read_count: read_count as u64,
    };
    let reply = scanned.reply(&params, history)?;
    let mut value = serde_json::to_value(reply)?;
    value["catalogue"] = json!(
        order
            .iter()
            .map(|&at| {
                let country = &contributors[at];
                json!({"key":country.actor.to_string(),"tag":country.tag,"slot":country.slot,"firstHour":country.first_hour})
            })
            .collect::<Vec<_>>()
    );
    value["summaries"] = serde_json::to_value(summaries)?;
    value["exactHours"] = serde_json::to_value(reached_hours)?;
    value["available"] = json!(read_count > 0);
    Ok(value)
}

/// The reply before the family publishes the branch: nothing read.
pub fn unpublished(params: &Value) -> io::Result<Value> {
    let params: BranchParams = serde_json::from_value(json!({"branchId":params["branchId"]}))?;
    params.validate().map_err(dashboard_core::invalid)?;
    let mut data = serde_json::to_value(ScannedTension::default().reply(&params, &RecordedHistories::default())?)?;
    data["available"] = json!(false);
    data["catalogue"] = json!([]);
    data["summaries"] = json!({});
    data["exactHours"] = json!([]);
    Ok(data)
}
