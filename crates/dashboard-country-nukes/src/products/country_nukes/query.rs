//! Country nukes' replies, read from the family's series publication ([`super::emit`]): the
//! chart summaries of the selected or ranked tags with their stockpiles, their stockpiles at
//! hours, keyboard navigation, the scoreboard's cells and the event feed's increases.

use super::{
    CountryNukes, NukeCount, NukeRun, NukeTypeInfo, NukesParams, ScannedNukes, history_membership,
    native_type_info, present, type_info,
    emit::{
        COUNTS, EMPTY, FAMILY, INCREASES, MIX, PARTS, POSITIONS, READS, REFUSALS, REASONS,
        REFUSED, TOKENS, TOTAL, TYPES, kind,
    },
};
use dashboard_base::{
    Source,
    prepared::{
        coverage::Coverage,
        engine,
        series::{self, Navigation},
    },
    prepared_chart::ChartSummary,
    products::{
        LoadContext, Product,
        country_groups::{HourSpan, mark_unobserved},
        country_history::{RecordedHistories, selected_identities},
    },
    ranking::for_each_piece,
};
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// A family scalar of `[Unsigned key, value]` pairs.
fn pairs(directory: &Directory, id: u64) -> impl Iterator<Item = (u64, &Scalar)> {
    directory
        .scalar(id)
        .map_or(&[][..], Scalar::list)
        .iter()
        .filter_map(|pair| match pair.list() {
            [key, value] => Some((u64::try_from(key.integer()?).ok()?, value)),
            _ => None,
        })
}

/// What the publication's reads found: the ordinals read, each native type's spelling, each
/// refused ordinal's rows, and the unresolved reads and unknown country-hours.
pub fn scanned(directory: &Directory) -> ScannedNukes {
    let count = |scalar: &Scalar| {
        scalar
            .integer()
            .and_then(|count| usize::try_from(count).ok())
    };
    let counts = directory.scalar(COUNTS).map_or(&[][..], Scalar::list);
    ScannedNukes {
        types: directory
            .scalar(TYPES)
            .map_or(&[][..], Scalar::list)
            .iter()
            .filter_map(|ordinal| u32::try_from(ordinal.integer()?).ok())
            .collect(),
        tokens: pairs(directory, TOKENS)
            .filter_map(|(ordinal, spelling)| {
                Some((
                    u32::try_from(ordinal).ok()?,
                    String::from_utf8(spelling.text()?.to_vec()).ok()?,
                ))
            })
            .collect(),
        refused: pairs(directory, REFUSED)
            .filter_map(|(ordinal, rows)| Some((u32::try_from(ordinal).ok()?, count(rows)?)))
            .collect(),
        unreadable: counts.first().and_then(count).unwrap_or(0),
        unknown_hours: counts.get(1).and_then(count).unwrap_or(0) as u64,
        ..ScannedNukes::default()
    }
}

/// The tag actor of `tag`, when the family ever named it.
fn actor(directory: &Directory, tag: &str) -> Option<u64> {
    directory.actor(tag.as_bytes())
}

/// The tag `actor` names.
fn tag(directory: &Directory, actor: i128) -> io::Result<String> {
    u64::try_from(actor)
        .ok()
        .and_then(|actor| directory.actors.get(actor as usize))
        .map(|actor| String::from_utf8_lossy(&actor.name).into_owned())
        .ok_or_else(|| invalid("A prepared nuclear rank names an unknown country"))
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

/// The complete reads' schedule, and whether each read's stockpiles were empty.
pub(super) struct Reads {
    schedule: Option<u64>,
    empty: Option<u64>,
}

impl Reads {
    pub(super) fn of(directory: &Directory) -> Self {
        Self {
            schedule: directory.key(FAMILY, READS, None),
            empty: directory.key(FAMILY, EMPTY, None),
        }
    }

    /// The last read by `hour` of the schedule run holding `at`: a run reads at its first hour
    /// and every period after (a period of 0 at its first hour alone).
    fn by<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        at: i32,
        hour: i32,
    ) -> io::Result<Option<i32>> {
        let Some(run) = run_at(snapshot, self.schedule, at)? else {
            return Ok(None);
        };
        let period = i32::try_from(run.value)
            .ok()
            .filter(|period| *period >= 0)
            .ok_or_else(|| invalid("Prepared nuclear read schedule is not valid"))?;
        let first = run.span.first;
        Ok((first <= hour).then(|| match period {
            0 => first,
            period => first + (run.span.last.min(hour) - first) / period * period,
        }))
    }

    /// Whether the read at `hour`'s stockpiles were empty.
    fn empty<P: PageRead>(&self, snapshot: &mut Snapshot<'_, P>, hour: i32) -> io::Result<bool> {
        Ok(run_at(snapshot, self.empty, hour)?.is_some_and(|run| run.value == 1))
    }

    /// The latest complete read by `hour`, and whether its stockpiles were empty.
    fn latest<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        hour: i32,
    ) -> io::Result<Option<(i32, bool)>> {
        let mut read = self.by(snapshot, hour, hour)?;
        if read.is_none()
            && let Some(key) = self.schedule
        {
            // Past a read due that was missed: the run before ends ahead of the hour.
            let accepted = snapshot.publication().accepted;
            let selection = Selection {
                key,
                membership: None,
            };
            if let Some(stop) = snapshot.next_change(selection, hour, false, accepted)? {
                for at in [stop.saturating_sub(1), stop] {
                    if let Some(found) = self.by(snapshot, at, hour)? {
                        read = Some(read.map_or(found, |held: i32| held.max(found)));
                    }
                }
            }
        }
        match read {
            Some(read) => Ok(Some((read, self.empty(snapshot, read)?))),
            None => Ok(None),
        }
    }

    /// Every complete read of `first..=last`, and whether its stockpiles were empty.
    pub(super) fn within<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        first: i32,
        last: i32,
    ) -> io::Result<Vec<(i32, bool)>> {
        let Some(key) = self.schedule.filter(|_| first <= last) else {
            return Ok(Vec::new());
        };
        let mut reads = Vec::new();
        for clipped in engine::runs(snapshot, key, first, last)? {
            // A run clipped at the window's start reads from its own first hour.
            let run = if clipped.span.first == first {
                run_at(snapshot, Some(key), first)?.unwrap_or(clipped)
            } else {
                clipped
            };
            let period = i32::try_from(run.value)
                .ok()
                .filter(|period| *period >= 0)
                .ok_or_else(|| invalid("Prepared nuclear read schedule is not valid"))?;
            let end = run.span.last.min(last);
            let mut read = run.span.first;
            if period == 0 {
                if read >= first && read <= end {
                    reads.push(read);
                }
                continue;
            }
            if read < first {
                read += (first - read + period - 1) / period * period;
            }
            while read <= end {
                reads.push(read);
                read += period;
            }
        }
        let empty = match self.empty {
            Some(key) if !reads.is_empty() => engine::runs(snapshot, key, first, last)?,
            _ => Vec::new(),
        };
        let mut runs = empty.iter().peekable();
        Ok(reads
            .into_iter()
            .map(|read| {
                while runs.next_if(|run| run.span.last < read).is_some() {}
                let flag = runs
                    .peek()
                    .is_some_and(|run| run.span.first <= read && run.value == 1);
                (read, flag)
            })
            .collect())
    }
}

/// The composition a [`kind::MIX`] event holds: each type's count, and the total.
pub(super) fn mix_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: EventView,
) -> io::Result<(Vec<NukeCount>, Option<i32>)> {
    let malformed = || invalid("Prepared nuclear composition is not valid");
    let EventView::Family(event) = event else {
        return Err(invalid("Missing prepared nuclear composition"));
    };
    let cells = event.fields.len / Cell::SIZE as u64;
    if event.kind != kind::MIX || cells == 0 {
        return Err(malformed());
    }
    let mut counts = Vec::with_capacity(cells as usize - 1);
    for at in 0..cells - 1 {
        let Cell::List(pair) = snapshot.field(event.fields, at)? else {
            return Err(malformed());
        };
        let (Cell::Unsigned(ordinal), Cell::Integer(value)) =
            (snapshot.field(pair, 0)?, snapshot.field(pair, 1)?)
        else {
            return Err(malformed());
        };
        counts.push(NukeCount {
            ordinal: u32::try_from(ordinal).map_err(|_| malformed())?,
            value: i32::try_from(value).map_err(|_| malformed())?,
        });
    }
    let total = match snapshot.field(event.fields, cells - 1)? {
        Cell::Null => None,
        Cell::Integer(total) => Some(i32::try_from(total).map_err(|_| malformed())?),
        _ => return Err(malformed()),
    };
    Ok((counts, total))
}

/// `key`'s composition at `hour`: its latest event by then.
fn composition<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<(Vec<NukeCount>, Option<i32>)> {
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            hour,
            None,
        )?
        .ok_or_else(|| invalid("Missing prepared nuclear composition"))?;
    mix_of(snapshot, event)
}

/// A chart of `key` over `view`, its bins laid from `bounds`, within `spans`; every bin
/// unknown where the family never named the key; no bins, and nothing read, for empty bounds.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    spans: &[HourSpan],
) -> io::Result<ChartSummary> {
    if let Some(key) = key.filter(|_| bounds.0 <= bounds.1) {
        return engine::summary(snapshot, key, bounds, view, 0, Some(spans));
    }
    let (exact, bins) = series::summary_bins(bounds, view)?;
    Ok(ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets: vec![None; bins.len()],
        runs: exact.then(Vec::new),
        approximate: Vec::new(),
    })
}

/// The hours `spans` hold inside `within`.
fn clipped(
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

/// A series request of the `country-nukes` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from `snapshot`, the publication pinned through `accepted`, the
/// coverage it shares with the identity history; pending before there is any.
#[allow(clippy::too_many_arguments)]
pub fn series<P: PageRead>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    accepted: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let reads: Vec<dashboard_base::prepared_measure::query::MeasureRead> =
        if product == "series-state" {
            serde_json::from_value(params["reads"].clone())?
        } else {
            Vec::new()
        };
    let read_tags: Vec<_> = reads.iter().map(|read| read.tag.clone()).collect();
    let selection: NukesParams = serde_json::from_value(json!({
        "branchId": params["branchId"],
        "tags": if product == "series-state" {
            json!(read_tags)
        } else {
            params.get("tags").cloned().unwrap_or_else(|| json!([]))
        },
        "top": params["family"]["top"], "originHour": params["family"]["originHour"],
    }))?;
    CountryNukes::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch: u64 = selection
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid nuclear branch"))?;
    let Some(shared) = accepted.clone() else {
        return Ok(Value::Null);
    };
    // The stockpiles may be published past the identities: nothing past their shared coverage is
    // read.
    let reach = (!shared.complete)
        .then(|| shared.last_hour.min(snapshot.publication().accepted.last));
    let through = |last: i32| reach.map_or(last, |reach| last.min(reach));
    let coverage = dashboard_base::prepared::prefix::extent(source, branch, accepted)?;
    let mut scanned = scanned(directory);
    let schedule = Reads::of(directory);
    // The history's tags the family named, which ranks and keys answer for.
    let named = history
        .countries
        .keys()
        .filter(|tag| actor(directory, tag).is_some())
        .count();
    let positions: Vec<u64> = directory
        .dimensions(FAMILY, POSITIONS)
        .iter()
        .map(|&(_, key)| key)
        .collect();
    let ranked = |top: u32| &positions[..(top as usize).min(named).min(positions.len())];
    let total_key = |tag: &str| actor(directory, tag).and_then(|actor| directory.key(actor, TOTAL, None));
    let mix_key = |tag: &str| actor(directory, tag).and_then(|actor| directory.key(actor, MIX, None));
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    let identity_membership = if selection.top.is_none() {
        Some(history_membership(&selection, history)?)
    } else {
        None
    };
    let mut navigation_membership = identity_membership.clone();
    if let Some(members) = &mut navigation_membership {
        for (tag, spans) in members {
            if let Some(within) = within.get(tag) {
                *spans = clipped(spans, within, cancelled)?;
            }
        }
    }
    if product == "series-next-change" {
        let mut navigation: Navigation = serde_json::from_value(params.clone())?;
        if let Some(reach) = reach.map(f64::from) {
            if reach < navigation.first_hour {
                return Ok(json!({ "hour": null, "coverage": coverage, "approximate": false }));
            }
            navigation.last_hour = navigation.last_hour.min(reach);
        }
        let mut keys: Vec<(u64, Option<Vec<HourSpan>>)> = Vec::new();
        if let Some(top) = selection.top {
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut values = Vec::new();
            for &key in ranked(top) {
                keys.push((key, None));
                if let Some(run) = run_at(snapshot, Some(key), hour)? {
                    let tag = tag(directory, run.value)?;
                    values.extend(total_key(&tag));
                    values.extend(mix_key(&tag));
                }
            }
            keys.extend(values.into_iter().map(|key| (key, None)));
        } else {
            for (tag, spans) in navigation_membership.iter().flatten() {
                // Composition changes stay navigable where the total and rank do not change.
                for key in [total_key(tag), mix_key(tag)].into_iter().flatten() {
                    keys.push((key, Some(spans.clone())));
                }
            }
        }
        let keys: Vec<(u64, Option<&[HourSpan]>)> = keys
            .iter()
            .map(|(key, spans)| (*key, spans.as_deref()))
            .collect();
        let hour = engine::navigate(snapshot, &keys, &navigation)?;
        return Ok(json!({ "hour": hour, "coverage": coverage, "approximate": false }));
    }
    let (from, to, width) = if product == "series-state" {
        (
            f64::from(reads.iter().map(|read| read.hour).min().unwrap_or(0)),
            f64::from(reads.iter().map(|read| read.hour).max().unwrap_or(0)),
            1,
        )
    } else {
        series::viewport(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["width"]
                .as_u64()
                .and_then(|width| u32::try_from(width).ok()),
        )?
    };
    let (first, last) = (from.floor() as i32, to.ceil() as i32);
    let mut membership = if selection.top.is_some() {
        Some(BTreeMap::new())
    } else {
        identity_membership
    };
    if let (Some(top), Some(members)) = (selection.top, &mut membership) {
        for &key in ranked(top) {
            for run in engine::runs(snapshot, key, first, through(last))? {
                let tag = tag(directory, run.value)?;
                if !history.countries.contains_key(&tag) {
                    return Err(invalid("Unknown nuclear rank key"));
                }
                members.entry(tag).or_default().push(HourSpan {
                    first_hour: run.span.first,
                    last_hour: run.span.last,
                });
            }
        }
        for spans in members.values_mut() {
            spans.sort_by_key(|span| span.first_hour);
            let mut merged = Vec::new();
            for span in std::mem::take(spans) {
                mark_unobserved(&mut merged, span.first_hour, span.last_hour);
            }
            *spans = merged;
        }
    }
    let mut wanted: Vec<String> = membership.as_ref().map_or_else(
        || selection.tags.clone(),
        |members| members.keys().cloned().collect(),
    );
    if product == "series-state" {
        wanted = read_tags;
        wanted.sort();
        wanted.dedup();
    }
    let selected = selected_identities(history, &selection.tags);
    let dense =
        product != "series-state" && i64::from(last) - i64::from(first) + 1 > i64::from(width);
    let mut display = Vec::new();
    let mut supplying = BTreeMap::new();
    for tag in &wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let country = actor(directory, tag);
        let total = country.and_then(|actor| directory.key(actor, TOTAL, None));
        let mix = country.and_then(|actor| directory.key(actor, MIX, None));
        let mut spans: Vec<_> = if let Some(members) = &membership {
            members.get(tag).cloned().unwrap_or_default()
        } else {
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect()
        };
        if let Some(within) = within.get(tag) {
            spans = clipped(&spans, within, cancelled)?;
        }
        let view = (from, to, width);
        let line = summary(snapshot, total, (first, through(last)), view, &spans)?;
        let mut dates = BTreeSet::new();
        let mut parts = Vec::new();
        if dense {
            for ordinal in &scanned.types {
                let key = country
                    .and_then(|actor| directory.key(actor, PARTS, Some(u64::from(*ordinal))));
                let part = summary(snapshot, key, (first, through(last)), view, &spans)?;
                for bucket in part.buckets.iter().flatten() {
                    dates.extend([
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.max_hour,
                    ]);
                }
                parts.push(json!({ "key": ordinal.to_string(), "summary": part }));
            }
            for bucket in line.buckets.iter().flatten() {
                dates.extend([
                    bucket.first_hour,
                    bucket.last_hour,
                    bucket.min_hour,
                    bucket.max_hour,
                ]);
            }
        }
        let runs: Vec<dashboard_series::Run> = if dense || product == "series-state" {
            if product == "series-state" {
                dates.extend(
                    reads
                        .iter()
                        .filter(|read| &read.tag == tag && read.hour == through(read.hour))
                        .map(|read| read.hour),
                );
            }
            let mut runs = BTreeMap::new();
            for &hour in &dates {
                if let Some(run) = run_at(snapshot, mix, hour)? {
                    if product == "series-state" {
                        let starts = run.break_before && run.span.first == hour;
                        runs.insert(
                            hour,
                            dashboard_series::Run {
                                span: dashboard_series::Span::new(hour, hour)?,
                                break_before: starts,
                                ..run
                            },
                        );
                    } else {
                        runs.insert(run.span.first, run);
                    }
                }
            }
            runs.into_values().collect()
        } else {
            let mut runs = match mix {
                Some(key) => engine::runs(snapshot, key, first, through(last))?,
                None => Vec::new(),
            };
            // A run the window clips starts no segment at the window's first hour.
            if let Some(head) = runs.first_mut()
                && head.break_before
                && head.span.first == first
                && run_at(snapshot, mix, first)?.is_some_and(|run| run.span.first < first)
            {
                head.break_before = false;
            }
            runs
        };
        let mut raw = Vec::new();
        for run in runs {
            let key = mix.ok_or_else(|| invalid("Missing prepared nuclear composition"))?;
            let (counts, total) = composition(snapshot, key, run.span.first)?;
            let (first_hour, last_hour) = if product == "series-state" {
                (run.span.first, run.span.last)
            } else {
                (run.span.first.max(first), run.span.last.min(through(last)))
            };
            if first_hour <= last_hour {
                raw.push(NukeRun {
                    first_hour,
                    last_hour,
                    starts_segment: run.break_before,
                    counts,
                    total,
                });
            }
        }
        // Supplying dates stay bound to the compositions that witnessed them.
        for run in &raw {
            for hour in [run.first_hour, run.last_hour] {
                if let Some((at, empty)) = schedule.latest(snapshot, hour)? {
                    supplying.insert(at, empty);
                }
            }
        }
        raw.sort_by_key(|run| run.first_hour);
        scanned.series.insert(tag.clone(), raw);
        scanned.segments.insert(
            tag.clone(),
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.segments.iter())
                .filter(|span| span.first_hour <= last && span.last_hour >= first)
                .cloned()
                .collect(),
        );
        if product != "series-state" {
            series::summary_budget(from, to, width, display.len() + 1)?;
            display.push(json!({ "tag": tag, "summary": line, "parts": parts }));
        }
    }
    if !dense && product != "series-state" {
        supplying.extend(schedule.within(snapshot, first, through(last))?);
    }
    scanned.read_hours = supplying.keys().copied().collect();
    scanned.empty_read_hours = supplying
        .iter()
        .filter_map(|(&hour, &empty)| empty.then_some(hour))
        .collect();
    let mut params = selection;
    params.tags = wanted;
    let reply = present(
        source,
        &params,
        context,
        cancelled,
        &scanned,
        Some(history),
        membership,
    )?;
    let mut reply = serde_json::to_value(reply)?;
    reply["summaries"] = json!(display);
    reply["coverage"] = json!(coverage);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// Each tag's total at `hour`, and whether its stockpile was read then (its composition known);
/// none where it held no value.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    hour: i32,
) -> io::Result<Vec<(Option<i128>, bool)>> {
    tags.iter()
        .map(|tag| {
            let Some(actor) = actor(directory, tag) else {
                return Ok((None, false));
            };
            let total = run_at(snapshot, directory.key(actor, TOTAL, None), hour)?;
            let read = run_at(snapshot, directory.key(actor, MIX, None), hour)?;
            Ok((total.map(|run| run.value), read.is_some()))
        })
        .collect()
}

/// A stockpile increase, as the event feed reads it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Increase {
    pub tag: String,
    pub slot: u32,
    pub country_id: u32,
    pub amount: u32,
    pub hour: i32,
}

/// What the event feed reads of the publication: every increase in hour order, then in tag
/// order, whether a complete read was ever recorded by each hour (its first), the ordinals
/// refused and every type's recorded spelling, and each refusal's hours by reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Feed {
    pub increases: Vec<Increase>,
    pub first_read: Option<i32>,
    pub refused: BTreeMap<u32, usize>,
    pub tokens: BTreeMap<u32, String>,
    pub types: BTreeSet<u32>,
    pub refusals: BTreeMap<String, Vec<HourSpan>>,
}

impl Feed {
    /// Every type the game names or a read recorded, by ordinal, spelled as recorded.
    pub fn types(&self) -> Vec<NukeTypeInfo> {
        let scanned = ScannedNukes {
            types: self.types.clone(),
            tokens: self.tokens.clone(),
            ..ScannedNukes::default()
        };
        native_type_info(&scanned, |_| None)
            .into_iter()
            .chain(type_info(&scanned, |_| None))
            .map(|info| (info.ordinal, info))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .increases
                .iter()
                .map(|increase| std::mem::size_of::<Increase>() + increase.tag.capacity())
                .sum::<usize>()
            + self
                .refusals
                .iter()
                .map(|(reason, spans)| {
                    64 + reason.capacity() + spans.capacity() * std::mem::size_of::<HourSpan>()
                })
                .sum::<usize>()
            + self.tokens.values().map(|text| 64 + text.capacity()).sum::<usize>()
    }
}

/// The event feed's view of the publication.
pub fn feed<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<Feed> {
    let malformed = || invalid("A prepared nuclear increase is not valid");
    let accepted = snapshot.publication().accepted;
    let mut increases = Vec::new();
    if let Some(key) = directory.key(FAMILY, INCREASES, None) {
        let selection = [Selection {
            key,
            membership: None,
        }];
        let mut cursor = None;
        loop {
            let mut events = Vec::new();
            let page = snapshot.event_page(
                dashboard_series::EventQuery {
                    selection: &selection,
                    native: accepted,
                    as_of: None,
                    order: dashboard_series::EventOrder::Native,
                },
                cursor,
                1 << 20,
                |event| {
                    events.push(event);
                    Ok(())
                },
            )?;
            for event in events {
                let EventView::Family(event) = event else {
                    return Err(malformed());
                };
                if event.kind != kind::INCREASE || event.fields.len != 4 * Cell::SIZE as u64 {
                    return Err(malformed());
                }
                let mut field = |at| match snapshot.field(event.fields, at)? {
                    Cell::Unsigned(value) => Ok(value),
                    _ => Err(malformed()),
                };
                let (actor, slot, id, amount) = (field(0)?, field(1)?, field(2)?, field(3)?);
                increases.push(Increase {
                    tag: tag(directory, i128::from(actor))?,
                    slot: u32::try_from(slot).map_err(|_| malformed())?,
                    country_id: u32::try_from(id).map_err(|_| malformed())?,
                    amount: u32::try_from(amount).map_err(|_| malformed())?,
                    hour: event.native_hour,
                });
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
    }
    let first_read = match directory.key(FAMILY, READS, None) {
        Some(key) => engine::runs(snapshot, key, accepted.first, accepted.last)?
            .first()
            .map(|run| run.span.first),
        None => None,
    };
    let mut refusals = BTreeMap::new();
    for &(id, key) in directory.dimensions(FAMILY, REFUSALS) {
        let reason = directory
            .scalar(REASONS + id)
            .and_then(Scalar::text)
            .ok_or_else(|| invalid("A prepared nuclear refusal names no reason"))?;
        let spans: Vec<HourSpan> = engine::runs(snapshot, key, accepted.first, accepted.last)?
            .into_iter()
            .map(|run| HourSpan {
                first_hour: run.span.first,
                last_hour: run.span.last,
            })
            .collect();
        if !spans.is_empty() {
            refusals.insert(String::from_utf8_lossy(reason).into_owned(), spans);
        }
    }
    let scanned = scanned(directory);
    Ok(Feed {
        increases,
        first_read,
        refused: scanned.refused,
        tokens: scanned.tokens,
        types: scanned.types,
        refusals,
    })
}
