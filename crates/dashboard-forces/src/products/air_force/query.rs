//! Air force size's replies, read from the family's series publication ([`super::emit`]): the
//! chart summaries of the selected or ranked tags with their runs and parts, their runs at
//! hours, keyboard navigation, and the scoreboard's cells.

use super::{
    AirForceMeasure, AirForceParams, AirForceSize, Counts, RawRun, ScannedAir,
    emit::{
        COUNTS, FAMILY, MIX, MIX_KIND, PART_KEYS, PARTS, POSITIONS, SPELLED, UNDISPLAYABLE, VALUES,
        channel,
    },
    present,
};
use dashboard_base::{
    prepared::{engine, series},
    prepared_chart::ChartSummary,
    products::{
        Product,
        country_groups::{HourSpan, mark_unobserved},
        country_history::{self, RecordedHistories},
    },
    ranking::for_each_piece,
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot, codec::Binary, family::Directory, query::EventView,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// What the publication's census counted: its unreadable, missing, refused and remainder hours,
/// and each tag that is not UTF-8 with its hours.
pub fn scanned(directory: &Directory) -> ScannedAir {
    let count = |scalar: &dashboard_series::family::Scalar| {
        scalar
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0)
    };
    let counts = directory
        .scalar(COUNTS)
        .map_or(&[][..], |scalar| scalar.list());
    let at = |index: usize| counts.get(index).map_or(0, count);
    ScannedAir {
        unreadable_owners: [at(0), at(1)],
        equipment_missing: at(2),
        equipment_refused: at(3),
        equipment_remainders: at(4),
        undisplayable_tags: directory
            .scalar(UNDISPLAYABLE)
            .map_or(&[][..], |scalar| scalar.list())
            .iter()
            .filter_map(|pair| match pair.list() {
                [tag, hours] => Some((tag.text()?.to_vec(), count(hours))),
                _ => None,
            })
            .collect(),
        ..ScannedAir::default()
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
        .ok_or_else(|| invalid("A prepared air rank names an unknown country"))
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

/// The compositions' spelled parts, read once per request.
#[derive(Default)]
pub(super) struct Spelled(HashMap<u64, String>);

impl Spelled {
    fn get(&mut self, directory: &Directory, id: u64) -> io::Result<&str> {
        if !self.0.contains_key(&id) {
            let text = directory
                .scalar(SPELLED + id)
                .and_then(|scalar| scalar.text())
                .ok_or_else(|| invalid("A prepared air composition names an unknown part"))?;
            self.0
                .insert(id, String::from_utf8_lossy(text).into_owned());
        }
        Ok(self.0[&id].as_str())
    }
}

/// The counts a composition's side holds, as the row census spells them.
fn side<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    spelled: &mut Spelled,
    cell: Cell,
) -> io::Result<Option<Counts>> {
    let malformed = || invalid("Prepared air composition is not valid");
    let Cell::List(fields) = cell else {
        return match cell {
            Cell::Null => Ok(None),
            _ => Err(malformed()),
        };
    };
    if fields.len != 4 * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let integer = |cell: Cell| match cell {
        Cell::Integer(value) => i64::try_from(value).map_err(|_| malformed()),
        _ => Err(malformed()),
    };
    let unsigned = |cell: Cell| match cell {
        Cell::Unsigned(value) => Ok(value),
        _ => Err(malformed()),
    };
    let mut counts = Counts {
        planes: integer(snapshot.field(fields, 0)?)?,
        wings: integer(snapshot.field(fields, 1)?)?,
        ..Counts::default()
    };
    let Cell::List(parts) = snapshot.field(fields, 2)? else {
        return Err(malformed());
    };
    for at in 0..parts.len / Cell::SIZE as u64 / 2 {
        let part = unsigned(snapshot.field(parts, at * 2)?)?;
        let amount = match snapshot.field(parts, at * 2 + 1)? {
            Cell::Null => None,
            cell => Some(integer(cell)?),
        };
        counts
            .planes_by_token
            .insert(spelled.get(directory, part)?.to_owned(), amount);
    }
    let Cell::List(signatures) = snapshot.field(fields, 3)? else {
        return Err(malformed());
    };
    for at in 0..signatures.len / Cell::SIZE as u64 / 2 {
        let Cell::List(held) = snapshot.field(signatures, at * 2)? else {
            return Err(malformed());
        };
        let mut tokens = Vec::with_capacity((held.len / Cell::SIZE as u64) as usize);
        for part in 0..held.len / Cell::SIZE as u64 {
            let part = unsigned(snapshot.field(held, part)?)?;
            tokens.push(spelled.get(directory, part)?.to_owned());
        }
        tokens.sort_unstable();
        tokens.dedup();
        let count = integer(snapshot.field(signatures, at * 2 + 1)?)?;
        counts.wings_by_tokens.insert(tokens, count);
    }
    Ok(Some(counts))
}

/// The run `run` of composition `key` as the row census holds it: unknown under both
/// ownerships for version 0, else its event's counts.
pub(super) fn raw_run<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    spelled: &mut Spelled,
    key: u64,
    run: &dashboard_series::Run,
) -> io::Result<RawRun> {
    let mut raw = RawRun {
        first_hour: run.span.first,
        last_hour: run.span.last,
        starts_segment: run.break_before,
        roster: None,
        effective: None,
    };
    if run.value == 0 {
        return Ok(raw);
    }
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            run.span.first,
            None,
        )?
        .ok_or_else(|| invalid("Missing prepared air composition"))?;
    let EventView::Family(event) = event else {
        return Err(invalid("Missing prepared air composition"));
    };
    if event.kind != MIX_KIND || event.fields.len != 2 * Cell::SIZE as u64 {
        return Err(invalid("Prepared air composition is not valid"));
    }
    let roster = snapshot.field(event.fields, 0)?;
    let effective = snapshot.field(event.fields, 1)?;
    raw.roster = side(snapshot, directory, spelled, roster)?.map(Arc::new);
    raw.effective = side(snapshot, directory, spelled, effective)?.map(Arc::new);
    Ok(raw)
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

/// A chart of `key` over `view`, its bins laid from `bounds`, within `spans`; no bins, and
/// nothing read, for empty bounds.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    spans: &[HourSpan],
) -> io::Result<ChartSummary> {
    if bounds.0 <= bounds.1 {
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

fn dates(summary: &ChartSummary, dates: &mut BTreeSet<i32>) {
    for bucket in summary.buckets.iter().flatten() {
        dates.extend([
            bucket.first_hour,
            bucket.last_hour,
            bucket.min_hour,
            bucket.max_hour,
        ]);
    }
}

/// A series request of the `air-force-size` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from `snapshot`; nothing past `reach`, the coverage the
/// publication shares with the identity history while it is partial, is read.
#[allow(clippy::too_many_arguments)]
pub fn series<P: PageRead>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    reach: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let through = |last: i32| reach.map_or(last, |reach| last.min(reach));
    let reads: Vec<dashboard_base::prepared_measure::query::MeasureRead> =
        if product == "series-state" {
            serde_json::from_value(params["reads"].clone())?
        } else {
            Vec::new()
        };
    let read_tags: Vec<String> = reads.iter().map(|read| read.tag.clone()).collect();
    let tags: Vec<String> = if product == "series-state" {
        read_tags.clone()
    } else {
        serde_json::from_value(params.get("tags").cloned().unwrap_or_else(|| json!([])))?
    };
    let selection: AirForceParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": tags, "top": params["family"]["top"],
        "measure": params["family"]["measure"], "ownership": params["family"]["ownership"],
    }))?;
    AirForceSize::validate(&selection).map_err(dashboard_core::invalid)?;
    let index = channel(
        selection.ownership.index(),
        selection.measure == AirForceMeasure::Wings,
    );
    let value_key = |tag: &str| {
        actor(directory, tag).and_then(|actor| directory.key(actor, VALUES + index as u64, None))
    };
    let mix_key =
        |tag: &str| actor(directory, tag).and_then(|actor| directory.key(actor, MIX, None));
    // The history's tags the family named, which ranks answer for.
    let named = history
        .countries
        .keys()
        .filter(|tag| actor(directory, tag).is_some())
        .count();
    let positions: Vec<u64> = directory
        .dimensions(FAMILY, POSITIONS + index as u64)
        .iter()
        .map(|&(_, key)| key)
        .collect();
    let ranked = |top: u32| &positions[..(top as usize).min(named).min(positions.len())];
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    if product == "series-next-change" {
        let mut navigation: series::Navigation = serde_json::from_value(params.clone())?;
        if let Some(reach) = reach.map(f64::from) {
            if reach < navigation.first_hour {
                return Ok(json!({ "hour": null }));
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
                    values.extend(value_key(&tag(directory, run.value)?));
                }
            }
            keys.extend(values.into_iter().map(|key| (key, None)));
        } else {
            // An alias's changes are the selection's only while a selected native country
            // holds it.
            for (tag, spans) in
                country_history::selected_spans(history, &selection.tags, &within, cancelled)?
            {
                if let Some(key) = value_key(&tag) {
                    keys.push((key, Some(spans)));
                }
            }
        }
        let keys: Vec<(u64, Option<&[HourSpan]>)> = keys
            .iter()
            .map(|(key, spans)| (*key, spans.as_deref()))
            .collect();
        return Ok(json!({ "hour": engine::navigate(snapshot, &keys, &navigation)? }));
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
    let mut membership: Option<BTreeMap<String, Vec<HourSpan>>> =
        selection.top.map(|_| BTreeMap::new());
    if let Some(top) = selection.top {
        for &key in ranked(top) {
            for run in engine::runs(snapshot, key, first, through(last))? {
                let tag = tag(directory, run.value)?;
                if !history.countries.contains_key(&tag) {
                    return Err(invalid("Unknown air rank key"));
                }
                membership
                    .as_mut()
                    .expect("ranked membership")
                    .entry(tag)
                    .or_default()
                    .push(HourSpan {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                    });
            }
        }
    }
    if let Some(members) = &mut membership {
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
        || country_history::selected_aliases(history, &selection.tags),
        |members| members.keys().cloned().collect(),
    );
    if product == "series-state" {
        wanted = read_tags;
        wanted.sort();
        wanted.dedup();
    }
    let selected = country_history::selected_identities(history, &selection.tags);
    let dense =
        product != "series-state" && i64::from(last) - i64::from(first) + 1 > i64::from(width);
    let part_name = |id: u64| -> io::Result<String> {
        directory
            .scalar(PART_KEYS + id)
            .and_then(|scalar| scalar.text())
            .map(|text| String::from_utf8_lossy(text).into_owned())
            .ok_or_else(|| invalid("A prepared air part names no key"))
    };
    let mut spelled = Spelled::default();
    // The page's lines read and presented from nothing: exactly those tags, none expanded again.
    let mut answer = |mut page: Option<&mut series::LinePage>| -> io::Result<Value> {
        let mut scanned = scanned(directory);
        let (mut display, mut shown) = (Vec::new(), Vec::new());
        let lines = match page.as_deref() {
            Some(page) => page.lines(wanted.len()),
            None => 0..wanted.len(),
        };
        for at in lines {
            let tag = &wanted[at];
            dashboard_text::metadata::check_cancel(cancelled)?;
            let (Some(key), Some(mix)) = (value_key(tag), mix_key(tag)) else {
                continue;
            };
            let country = history.countries.get(tag);
            let mut spans: Vec<HourSpan> = if let Some(members) = &membership {
                members.get(tag).cloned().unwrap_or_default()
            } else {
                country
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
            let line = summary(snapshot, key, (first, through(last)), view, &spans)?;
            let mut hours = BTreeSet::new();
            let mut parts = Vec::new();
            if dense {
                let actor = actor(directory, tag).expect("named air tag");
                let mut keyed = Vec::new();
                for &(part, key) in directory.dimensions(actor, PARTS + index as u64) {
                    keyed.push((part_name(part)?, key));
                }
                keyed.sort();
                for (name, key) in keyed {
                    let part = summary(snapshot, key, (first, through(last)), view, &spans)?;
                    dates(&part, &mut hours);
                    parts.push(json!({ "key": name, "summary": part }));
                }
                dates(&line, &mut hours);
            }
            let paged = match page.as_deref_mut() {
                Some(page) => {
                    let line = json!({ "tag": tag, "summary": line, "parts": parts });
                    if !page.admit(at, &line)? {
                        break;
                    }
                    Some(line)
                }
                None => None,
            };
            let runs: Vec<dashboard_series::Run> = if dense || product == "series-state" {
                if product == "series-state" {
                    hours.extend(
                        reads
                            .iter()
                            .filter(|read| &read.tag == tag && read.hour == through(read.hour))
                            .map(|read| read.hour),
                    );
                }
                let mut runs = BTreeMap::new();
                for &hour in &hours {
                    if let Some(run) = run_at(snapshot, Some(mix), hour)? {
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
                let mut runs = engine::runs(snapshot, mix, first, through(last))?;
                // A run the window clips starts no segment at the window's first hour.
                if let Some(head) = runs.first_mut()
                    && head.break_before
                    && head.span.first == first
                    && run_at(snapshot, Some(mix), first)?.is_some_and(|run| run.span.first < first)
                {
                    head.break_before = false;
                }
                runs
            };
            let mut bounded = Vec::new();
            for run in runs {
                let value = raw_run(snapshot, directory, &mut spelled, mix, &run)?;
                let bounds = if product == "series-state" {
                    (run.span.first, run.span.last)
                } else {
                    (run.span.first.max(first), run.span.last.min(through(last)))
                };
                if bounds.0 <= bounds.1 {
                    bounded.push((bounds, run.break_before, value));
                }
            }
            // An alias's other holders and hours outside `within` are not the selection.
            let mut raw = Vec::new();
            for_each_piece(
                &bounded,
                &spans,
                |&(bounds, _, _)| bounds,
                cancelled,
                |(bounds, starts, value), first_hour, last_hour| {
                    raw.push(RawRun {
                        first_hour,
                        last_hour,
                        starts_segment: *starts && first_hour == bounds.0,
                        ..value.clone()
                    });
                    Ok(())
                },
            )?;
            scanned.series.insert(tag.clone(), raw);
            scanned.segments.insert(
                tag.clone(),
                country
                    .into_iter()
                    .flat_map(|country| country.segments.iter())
                    .filter(|span| span.first_hour <= last && span.last_hour >= first)
                    .cloned()
                    .collect(),
            );
            if let Some(line) = paged {
                display.push(line);
                shown.push(tag.clone());
            }
        }
        let mut request = selection.clone();
        let members = if page.is_some() {
            request.tags = shown;
            membership.as_ref().map(|members| {
                request
                    .tags
                    .iter()
                    .filter_map(|tag| Some((tag.clone(), members.get(tag)?.clone())))
                    .collect()
            })
        } else {
            // A state read answers each read tag's aliases too.
            request.tags = if request.top.is_none() {
                country_history::selected_aliases(history, &wanted)
            } else {
                wanted.clone()
            };
            membership.clone()
        };
        let reply = present(
            source,
            &request,
            context,
            cancelled,
            &scanned,
            Some(history),
            members,
        )?;
        let mut reply = serde_json::to_value(reply)?;
        if page.is_some() {
            reply["summaries"] = json!(display);
        }
        Ok(reply)
    };
    if product == "series-state" {
        let reply = answer(None)?;
        dashboard_base::products::check_reply_budget(&reply)?;
        return Ok(reply);
    }
    series::summary_budget(from, to, width, 1)?;
    series::paged(
        params["cursor"].as_str(),
        dashboard_base::products::MAX_PRODUCT_REPLY_BYTES,
        |page| answer(Some(page)),
    )
}

/// Each tag's roster planes at `hour`, and whether its census was read then; none where it held
/// no value.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    hour: i32,
) -> io::Result<Vec<(Option<i128>, bool)>> {
    let planes = VALUES + channel(0, false) as u64;
    tags.iter()
        .map(|tag| {
            let Some(actor) = actor(directory, tag) else {
                return Ok((None, false));
            };
            let total = run_at(snapshot, directory.key(actor, planes, None), hour)?;
            let read = run_at(snapshot, directory.key(actor, MIX, None), hour)?;
            Ok((total.map(|run| run.value), read.is_some()))
        })
        .collect()
}
