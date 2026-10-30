//! A country's division equipment history read back from the family's series publication:
//! its values' runs within the window, under the names the UI reads them by, with the
//! operands those runs name resolved in the shown language and art.

use super::emit::{
    ARTWORKS, DESIGN, DESIGNS, DIVISIONS, FAMILY, KINDS, KNOWN, NAMINGS, NOTE, NOTES, PAIRS, POOL,
    REFUSED, SHORT, SIDE, STATE, STOCK, STOCK_DESIGN, TEMPLATE, TYPE, UNKNOWN, VARIANT,
};
use super::{
    DivisionEquipmentHistory, DivisionEquipmentHistoryParams, DivisionEquipmentHistoryReply,
    HistoryIcon, HistoryKind, HistoryName, HistoryRefusal, HistoryWarnings, KeptReason, answered,
    at,
};
use crate::products::army_content::{self, Refusals, TemplateArtwork};
use dashboard_base::{
    products::{Product, country_groups::HourSpan, equipment::RawVariant},
    source::Source,
};
use dashboard_core::config::{ContentOptions, LoadContext};
use dashboard_keyed::query::{KeyedRun, KeyedSeries, KeyedWindow};
use dashboard_series::{PageRead, Snapshot, family::Directory};
use dashboard_text::{catalogue::Catalogue, template_facts::TemplateNaming};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

const MEASURES: [&str; 7] = [
    "held", "needed", "filled", "missing", "stock", "short", "unknown",
];
const PARTS: [&str; 11] = [
    "divisions",
    "unknown",
    "typesShort",
    "manpower",
    "required",
    "hp",
    "hpMax",
    "needed",
    "held",
    "naming",
    "artwork",
];

/// The operands a window's runs name, read from the publication's catalogue.
#[derive(Default)]
struct Operands {
    kinds: BTreeMap<usize, Option<String>>,
    namings: BTreeMap<usize, Result<TemplateNaming, KeptReason>>,
    artworks: BTreeMap<usize, Result<TemplateArtwork, KeptReason>>,
    designs: BTreeMap<usize, RawVariant>,
    notes: BTreeMap<usize, KeptReason>,
}

/// The catalogued operand `number` of `range`, as JSON.
fn json<T: serde::de::DeserializeOwned>(
    directory: &Directory,
    range: u64,
    number: usize,
) -> io::Result<T> {
    let text = directory
        .scalar(range + number as u64)
        .and_then(|scalar| scalar.text())
        .ok_or_else(|| invalid("Unknown division equipment history operand"))?;
    Ok(serde_json::from_slice(text)?)
}

/// `key`'s runs over `from..=to`, a run going on across a cut kept as one.
fn runs_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    (from, to): (i32, i32),
) -> io::Result<Vec<(i32, i32, i128, bool)>> {
    let mut runs: Vec<(i32, i32, i128, bool)> = Vec::new();
    for run in dashboard_base::prepared::engine::runs(snapshot, key, from, to)? {
        let (first, last) = (run.span.first, run.span.last);
        match runs.last_mut() {
            Some(held)
                if held.1.checked_add(1) == Some(first)
                    && held.2 == run.value
                    && !run.break_before =>
            {
                held.1 = last;
            }
            _ => runs.push((first, last, run.value, run.break_before)),
        }
    }
    Ok(runs)
}

fn series(name: String, runs: Vec<(i32, i32, i128, bool)>) -> Option<KeyedSeries> {
    (!runs.is_empty()).then(|| KeyedSeries {
        name,
        runs: runs
            .into_iter()
            .map(|(first, last, value, starts)| -> KeyedRun {
                (first, last, value.to_string(), starts)
            })
            .collect(),
    })
}

/// The branch's recorded first and last hours.
fn bounds(source: &Source, branch: u64) -> io::Result<(i32, i32)> {
    source
        .with_metadata(|metadata| {
            metadata
                .branches
                .iter()
                .find(|item| item.id.parse::<u64>().ok() == Some(branch))
                .map(|item| (item.first_hour, item.last_hour))
        })
        .ok_or_else(|| dashboard_core::invalid("Unknown branch"))
}

/// One country's history for one attribution within the request's window.
pub fn reply<P: PageRead>(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &serde_json::Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<DivisionEquipmentHistoryReply> {
    let p: DivisionEquipmentHistoryParams = serde_json::from_value(params.clone())?;
    DivisionEquipmentHistory::validate(&p).map_err(dashboard_core::invalid)?;
    let branch = p
        .branch_id
        .parse::<u64>()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let (first, last) = bounds(source, branch)?;
    let window = (
        p.first_hour.map_or(first, |hour| hour.max(first)),
        p.last_hour.map_or(last, |hour| hour.min(last)),
    );
    let context = LoadContext {
        content: content.cloned(),
    };
    let mut base = Refusals::default();
    let cat = base.keep(army_content::content(source, &context)).0;
    if let Some(cat) = &cat {
        let (_, notes) =
            dashboard_land_battles::products::division_log::division_name_groups(source, cat);
        for note in notes {
            base.note(note);
        }
    }
    let mut reply = DivisionEquipmentHistoryReply {
        branch_id: p.branch_id.clone(),
        tag: p.tag.clone(),
        count_for: p.count_for,
        window: dashboard_keyed::query::empty(window.0, window.1),
        kinds: Vec::new(),
        namings: BTreeMap::new(),
        artworks: BTreeMap::new(),
        designs: BTreeMap::new(),
        hours: Vec::new(),
        absent: Vec::new(),
        served: Vec::new(),
        warnings: Vec::new(),
    };
    if window.0 <= window.1 {
        let operands = read(
            snapshot,
            directory,
            &p,
            window,
            &mut reply.window,
            cancelled,
        )?;
        let drawn = name_operands(source, &operands, cat.as_ref(), &mut reply)?;
        reply.hours = hours(&operands, &reply, &base, &drawn)?;
    }
    reply.absent = base.lines();
    Ok(reply)
}

/// The window's series and known hours, and the operands they name.
fn read<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    p: &DivisionEquipmentHistoryParams,
    window: (i32, i32),
    into: &mut KeyedWindow,
    cancelled: &AtomicBool,
) -> io::Result<Operands> {
    let mut operands = Operands::default();
    if let Some(key) = directory.key(FAMILY, KNOWN, None) {
        for (first, last, _, _) in runs_of(snapshot, key, window)? {
            match into.known.last_mut() {
                Some(span) if span.last_hour.checked_add(1) == Some(first) => {
                    span.last_hour = last;
                }
                _ => into.known.push(HourSpan {
                    first_hour: first,
                    last_hour: last,
                }),
            }
        }
    }
    for (measure, name) in [(POOL, "*|pool"), (STOCK, "*|stock")] {
        if let Some(key) = directory.key(FAMILY, measure, None) {
            into.series
                .extend(series(name.to_owned(), runs_of(snapshot, key, window)?));
        }
    }
    let Some(actor) = directory.actor(p.tag.as_bytes()) else {
        return Ok(operands);
    };
    let side = match p.count_for {
        dashboard_forces::products::army_size::ArmyAttribution::Owner => 0,
        dashboard_forces::products::army_size::ArmyAttribution::Host => 1,
    };
    let base = side * SIDE;
    for (code, name) in [
        (STATE, "state"),
        (DIVISIONS, "divisions"),
        (SHORT, "short"),
        (UNKNOWN, "unknown"),
        (REFUSED, "refused"),
    ] {
        if let Some(key) = directory.key(actor, base + code, None) {
            into.series
                .extend(series(name.to_owned(), runs_of(snapshot, key, window)?));
        }
    }
    let mut kinds = BTreeSet::new();
    for (at, measure) in MEASURES.iter().enumerate() {
        for &(kind, key) in directory.dimensions(actor, base + TYPE + at as u64) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if let Some(held) = series(
                format!("type|{kind}|{measure}"),
                runs_of(snapshot, key, window)?,
            ) {
                kinds.insert(kind as usize);
                into.series.push(held);
            }
        }
    }
    let mut designs = BTreeSet::new();
    for (code, part) in [(VARIANT, "variant"), (DESIGN, "design")] {
        for &(pair, key) in directory.dimensions(actor, base + code) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let runs = runs_of(snapshot, key, window)?;
            if runs.is_empty() {
                continue;
            }
            let pair = directory
                .scalar(PAIRS + pair)
                .ok_or_else(|| invalid("Unknown division equipment design"))?;
            let [kind, handle] = pair.list() else {
                return Err(invalid("Damaged division equipment design"));
            };
            let field = |scalar: &dashboard_series::family::Scalar| {
                scalar
                    .integer()
                    .and_then(|value| u64::try_from(value).ok())
                    .ok_or_else(|| invalid("Damaged division equipment design"))
            };
            let (kind, handle) = (field(kind)?, field(handle)?);
            kinds.insert(kind as usize);
            if code == DESIGN {
                designs.extend(runs.iter().map(|run| run.2 as usize));
            }
            into.series
                .extend(series(format!("type|{kind}|{part}|{handle}"), runs));
        }
    }
    let mut namings = BTreeSet::new();
    let mut artworks = BTreeSet::new();
    for (at, part) in PARTS.iter().enumerate() {
        for &(template, key) in directory.dimensions(actor, base + TEMPLATE + at as u64) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let runs = runs_of(snapshot, key, window)?;
            match *part {
                "naming" => namings.extend(runs.iter().map(|run| run.2 as usize)),
                "artwork" => artworks.extend(runs.iter().map(|run| run.2 as usize)),
                _ => {}
            }
            into.series
                .extend(series(format!("template|{template}|{part}"), runs));
        }
    }
    let mut notes = BTreeSet::new();
    for &(note, key) in directory.dimensions(actor, base + NOTE) {
        if let Some(held) = series(format!("note|{note}"), runs_of(snapshot, key, window)?) {
            notes.insert(note as usize);
            into.series.push(held);
        }
    }
    for &(handle, key) in directory.dimensions(actor, base + STOCK_DESIGN) {
        let runs = runs_of(snapshot, key, window)?;
        designs.extend(runs.iter().map(|run| run.2 as usize));
        into.series
            .extend(series(format!("stockDesign|{handle}"), runs));
    }
    for kind in kinds {
        let key = match directory.scalar(KINDS + kind as u64) {
            Some(dashboard_series::family::Scalar::Null) => None,
            Some(scalar) => Some(
                std::str::from_utf8(
                    scalar
                        .text()
                        .ok_or_else(|| invalid("Damaged equipment type"))?,
                )
                .map_err(|_| invalid("Damaged equipment type"))?
                .to_owned(),
            ),
            None => return Err(invalid("Unknown equipment type")),
        };
        operands.kinds.insert(kind, key);
    }
    for naming in namings {
        operands
            .namings
            .insert(naming, json(directory, NAMINGS, naming)?);
    }
    for artwork in artworks {
        operands
            .artworks
            .insert(artwork, json(directory, ARTWORKS, artwork)?);
    }
    for design in designs {
        operands
            .designs
            .insert(design, json(directory, DESIGNS, design)?);
    }
    for note in notes {
        operands.notes.insert(note, json(directory, NOTES, note)?);
    }
    Ok(operands)
}

/// The window's operands named in the shown language and art, with each icon's fallbacks.
fn name_operands(
    source: &Source,
    operands: &Operands,
    cat: Option<&Arc<Catalogue>>,
    reply: &mut DivisionEquipmentHistoryReply,
) -> io::Result<BTreeMap<String, BTreeSet<String>>> {
    let mut drawn = BTreeMap::new();
    let mut kinds = BTreeSet::new();
    let number = |text: &str| {
        text.parse::<usize>()
            .map_err(|_| io::Error::other("Invalid history operand"))
    };
    for series in &reply.window.series {
        let parts: Vec<&str> = series.name.split('|').collect();
        if let ["type", kind, ..] = parts.as_slice() {
            kinds.insert(number(kind)?);
        }
        let ids = series.runs.iter().map(|run| number(&run.2));
        match parts.as_slice() {
            ["template", _, "naming"] => {
                for id in ids {
                    let id = id?;
                    let naming = operands
                        .namings
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown template naming"))?;
                    reply.namings.entry(id.to_string()).or_insert_with(|| {
                        let (name, name_missing) =
                            answered(naming, |naming| army_content::template_name_of(naming, cat));
                        HistoryName { name, name_missing }
                    });
                }
            }
            ["template", _, "artwork"] => {
                for id in ids {
                    let id = id?;
                    let artwork = operands
                        .artworks
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown template artwork"))?;
                    if !reply.artworks.contains_key(&id.to_string()) {
                        // An hour names only the fallbacks of the icons it shows.
                        let ((icon, icon_missing), own) =
                            dashboard_text::catalogue::capture_isolated(|| {
                                answered(artwork, |artwork| {
                                    army_content::template_icon_of(source, artwork, cat)
                                })
                            });
                        reply
                            .artworks
                            .insert(id.to_string(), HistoryIcon { icon, icon_missing });
                        drawn.insert(id.to_string(), own.fallbacks().clone());
                    }
                }
            }
            ["type", _, "design", _] | ["stockDesign", _] => {
                for id in ids {
                    let id = id?;
                    let raw = operands
                        .designs
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown design operands"))?;
                    reply
                        .designs
                        .entry(id.to_string())
                        .or_insert_with(|| army_content::design_name(raw, cat));
                }
            }
            _ => {}
        }
    }
    // Types in the reply's key order, the unclassified first.
    let mut listed: Vec<(Option<String>, usize)> = kinds
        .into_iter()
        .map(|id| {
            operands
                .kinds
                .get(&id)
                .cloned()
                .map(|key| (key, id))
                .ok_or_else(|| io::Error::other("Unknown equipment type"))
        })
        .collect::<io::Result<_>>()?;
    listed.sort();
    for (archetype, id) in listed {
        let (name, name_missing) = match &archetype {
            Some(key) => keep(army_content::localised(cat, key)),
            None => (None, Some("Equipment archetype is unclassified".into())),
        };
        reply.kinds.push(HistoryKind {
            id: id.to_string(),
            archetype,
            name,
            name_missing,
        });
    }
    Ok(drawn)
}

fn keep<T>(value: Result<T, String>) -> (Option<T>, Option<String>) {
    match value {
        Ok(value) => (Some(value), None),
        Err(reason) => (None, Some(reason)),
    }
}

/// Each present hour's refusals, naming ones counted again as the reached product counts them.
fn hours(
    operands: &Operands,
    reply: &DivisionEquipmentHistoryReply,
    base: &Refusals,
    drawn: &BTreeMap<String, BTreeSet<String>>,
) -> io::Result<Vec<HistoryWarnings>> {
    let window = &reply.window;
    let Some(state) = window.series.iter().find(|series| series.name == "state") else {
        return Ok(Vec::new());
    };
    let read: Vec<&KeyedSeries> = window
        .series
        .iter()
        .filter(|series| {
            let parts: Vec<&str> = series.name.split('|').collect();
            matches!(
                parts.as_slice(),
                ["state"]
                    | ["note", _]
                    | ["type", _, "held"]
                    | ["type", _, "design", _]
                    | ["template", _, "naming" | "artwork"]
                    | ["stockDesign", _]
            )
        })
        .collect();
    let mut cuts = BTreeSet::new();
    for series in &read {
        // A holding's warnings follow its presence, not its amount: runs that touch are one.
        let held = series.name.ends_with("|held");
        let touch = |before: &(i32, i32, String, bool), after: &(i32, i32, String, bool)| {
            held && before.1.checked_add(1) == Some(after.0)
        };
        for (index, run) in series.runs.iter().enumerate() {
            if index == 0 || !touch(&series.runs[index - 1], run) {
                cuts.insert(run.0);
            }
            let joined = series.runs.get(index + 1).is_some_and(|next| touch(run, next));
            if !joined && let Some(after) = run.1.checked_add(1) {
                cuts.insert(after);
            }
        }
    }
    let ends: Vec<i32> = cuts.iter().copied().collect();
    let mut spans: Vec<HistoryWarnings> = Vec::new();
    for (index, &start) in ends.iter().enumerate() {
        let Some(&next) = ends.get(index + 1) else {
            break;
        };
        if at(&state.runs, start).is_none() {
            continue;
        }
        let mut refusals = BTreeMap::<KeptReason, usize>::new();
        let mut count = |reason: KeptReason, times: usize| {
            if times > 0 {
                *refusals.entry(reason).or_default() += times;
            }
        };
        for (reason, times) in base.clone().counted() {
            count(KeptReason::literal(reason), times);
        }
        let mut fallbacks = BTreeSet::new();
        for series in &read {
            let Some(value) = at(&series.runs, start) else {
                continue;
            };
            let parts: Vec<&str> = series.name.split('|').collect();
            let number = || {
                value
                    .parse::<usize>()
                    .map_err(|_| io::Error::other("Invalid history operand"))
            };
            match parts.as_slice() {
                ["note", note] => {
                    let id = note
                        .parse::<usize>()
                        .map_err(|_| io::Error::other("Invalid history note"))?;
                    let text = operands
                        .notes
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown history note"))?;
                    count(text.clone(), number()?);
                }
                ["type", kind, "held"] => {
                    if let Some(found) = reply.kinds.iter().find(|listed| listed.id == *kind)
                        && found.archetype.is_some()
                        && let Some(missing) = &found.name_missing
                    {
                        count(KeptReason::literal(missing.clone()), 1);
                    }
                }
                ["template", _, "naming"] => {
                    if let Some(missing) = reply
                        .namings
                        .get(&number()?.to_string())
                        .and_then(|name| name.name_missing.as_ref())
                    {
                        count(missing.clone(), 1);
                    }
                }
                ["template", _, "artwork"] => {
                    let id = number()?.to_string();
                    if let Some(missing) = reply
                        .artworks
                        .get(&id)
                        .and_then(|icon| icon.icon_missing.as_ref())
                    {
                        count(missing.clone(), 1);
                    }
                    fallbacks.extend(drawn.get(&id).into_iter().flatten().cloned());
                }
                ["type", _, "design", _] | ["stockDesign", _] => {
                    if reply
                        .designs
                        .get(&number()?.to_string())
                        .is_some_and(Option::is_none)
                    {
                        count(KeptReason::literal(army_content::UNNAMED_DESIGN), 1);
                    }
                }
                _ => {}
            }
        }
        let refusals: Vec<HistoryRefusal> = refusals
            .into_iter()
            .map(|(reason, times)| HistoryRefusal { reason, times })
            .collect();
        let fallback = installed_content_core::content::overlay_refusal_line(&fallbacks);
        match spans.last_mut() {
            Some(span)
                if (&span.refusals, &span.fallback) == (&refusals, &fallback)
                    && span.last_hour.checked_add(1) == Some(start) =>
            {
                span.last_hour = next - 1
            }
            _ => spans.push(HistoryWarnings {
                first_hour: start,
                last_hour: next - 1,
                refusals,
                fallback,
            }),
        }
    }
    Ok(spans)
}
