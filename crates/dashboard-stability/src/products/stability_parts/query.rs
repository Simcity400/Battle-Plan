//! Stability parts' replies (series family `stability-parts`), read from the family's series
//! publication ([`super::emit`]): a country's request expands to its parts of the measure
//! asked, the dating part first, each answered as a series of its own; the reply names every
//! asked country's parts in the viewer's language and carries their warnings.

use super::{
    Part,
    emit::{
        Catalogue, DECIMALS, Entry, MEASURES, NAMES, PART_NAMES, SUMMARY, Summary, names_actor,
        part_actor,
    },
    named_parts,
};
use crate::products::country_stability::query::measure_run;
use dashboard_base::{
    Source,
    prepared::{
        engine,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartSummary, MarkSummary},
    prepared_measure::query::{
        MeasureKeyReply, MeasureMarkReply, MeasureRead, MeasureState, MeasureStateReply,
        MeasureSummaryLine, MeasureSummaryReply,
    },
    products::{country_groups::HourSpan, politics_content::politics_content},
};
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot, codec::Binary, family::Directory, query::EventView,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Family {
    #[serde(rename = "stability-parts")]
    Parts { measure: String },
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
    #[serde(default)]
    #[allow(dead_code, reason = "The family's event pages are not served")]
    cursor: Option<String>,
    #[serde(default)]
    #[allow(dead_code, reason = "The family's event pages are not served")]
    page_size: Option<u32>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// The text of `key`'s latest event of `kind` through `last`.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    last: i32,
    kind: u64,
) -> io::Result<Option<Vec<u8>>> {
    let Some(event) = snapshot.latest_event(
        Selection {
            key,
            membership: None,
        },
        last,
        None,
    )?
    else {
        return Ok(None);
    };
    let malformed = || invalid("Prepared stability part names are not valid");
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind || event.fields.len != Cell::SIZE as u64 {
        return Err(malformed());
    }
    let Cell::Text(text) = snapshot.field(event.fields, 0)? else {
        return Err(malformed());
    };
    let length = usize::try_from(text.len).map_err(|_| malformed())?;
    let mut bytes = vec![0; length];
    snapshot.cell_bytes(text, 0, &mut bytes)?;
    Ok(Some(bytes))
}

/// `tag`'s names as the publication's last cut holds them: its summary and each part's latest
/// names; none before its parts were held.
pub(super) fn catalogue<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
) -> io::Result<Option<Catalogue>> {
    let Some(actor) = directory.actor(names_actor(tag)?.as_bytes()) else {
        return Ok(None);
    };
    let Some(key) = directory.key(actor, 0, None) else {
        return Ok(None);
    };
    let last = snapshot.publication().accepted.last;
    let Some(summary) = latest(snapshot, key, last, SUMMARY)? else {
        return Ok(None);
    };
    let summary: Summary = serde_json::from_slice(&summary)?;
    let mut parts: [BTreeMap<String, (Part, i32)>; 2] = Default::default();
    for (index, parts) in parts.iter_mut().enumerate() {
        for &(_, key) in directory.dimensions(actor, PART_NAMES + index as u64) {
            if let Some(entry) = latest(snapshot, key, last, NAMES)? {
                let entry: Entry = serde_json::from_slice(&entry)?;
                parts.insert(entry.id, (entry.part, entry.first));
            }
        }
    }
    Ok(Some(Catalogue {
        parts,
        dated: summary.dated,
        warnings: summary.warnings,
    }))
}

/// A part's key of `measure` and the hour it was first held.
struct Found {
    key: Option<u64>,
    first: Option<i32>,
}

/// Answers `product` for the `stability-parts` series family.
#[allow(clippy::too_many_arguments)]
pub fn query<P: PageRead>(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    product: &str,
    mut raw: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    raw["branchId"]
        .as_str()
        .and_then(|branch| branch.parse::<u64>().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid parts branch"))?;
    let index = match raw["family"]["measure"].as_str() {
        Some("stability") => 0,
        Some("warSupport") => 1,
        _ => return Err(dashboard_core::invalid("Unknown parts measure")),
    };
    if product == "series-summary" {
        raw["from"] = json!(raw["from"].as_f64().map(f64::floor));
        raw["to"] = json!(raw["to"].as_f64().map(f64::ceil));
    }
    let content = content
        .map(|options| politics_content(source, options))
        .transpose();
    let mut warnings = match &content {
        Ok(Some(content)) => content.warnings.clone(),
        Err(error) => vec![format!("Breakdown lines are not named: {error}")],
        _ => Vec::new(),
    };
    let content = content.ok().flatten();
    let field = if product == "series-state" {
        "reads"
    } else {
        "tags"
    };
    let requests = raw[field]
        .as_array()
        .ok_or_else(|| dashboard_core::invalid("Missing parts selection"))?;
    let mut expanded = Vec::new();
    let mut within = serde_json::Map::new();
    let mut metadata = BTreeMap::new();
    // Each expanded part's first hour of each measure, by the key a reply names it by.
    let mut firsts: HashMap<String, [Option<i32>; 2]> = HashMap::new();
    for request in requests {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let tag = request
            .as_str()
            .or_else(|| request["tag"].as_str())
            .ok_or_else(|| dashboard_core::invalid("Invalid parts tag"))?;
        dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
        let held = catalogue(snapshot, directory, tag)?.unwrap_or_default();
        warnings.extend(held.warnings.iter().cloned());
        let parts: BTreeMap<String, Part> = held.parts[index]
            .iter()
            .map(|(id, (part, _))| (id.clone(), part.clone()))
            .collect();
        let named = named_parts(&parts, &[], content.as_deref(), tag);
        for id in std::iter::once("").chain(named.parts.iter().map(|part| part.id.as_str())) {
            let key = part_actor(tag, id)?;
            let first = |measure: usize| {
                if id.is_empty() {
                    held.dated[measure]
                } else {
                    held.parts[measure].get(id).map(|(_, first)| *first)
                }
            };
            firsts.insert(key.clone(), [first(0), first(1)]);
            if let Some(spans) = raw["within"].get(tag) {
                within.insert(key.clone(), spans.clone());
            }
            expanded.push(if product == "series-state" {
                json!({"tag": key, "hour": request["hour"]})
            } else {
                json!(key)
            });
        }
        metadata.insert(tag.to_owned(), named);
    }
    raw[field] = Value::Array(expanded);
    raw["within"] = Value::Object(within);
    let params: SeriesParams = serde_json::from_value(raw)?;
    let Family::Parts { measure: name } = &params.family;
    let name = name.clone();
    let measure_of = |label: &str| MEASURES.iter().position(|measure| *measure == label);
    let found = |key: &str, measure: usize| -> Found {
        let handle = directory
            .actor(key.as_bytes())
            .and_then(|actor| directory.key(actor, measure as u64, None));
        Found {
            key: handle,
            first: firsts.get(key).and_then(|firsts| firsts[measure]),
        }
    };
    let last = snapshot.publication().accepted.last;
    let mut reply = match product {
        "series-state" => {
            let mut states = Vec::with_capacity(params.reads.len());
            for read in params.reads {
                dashboard_text::metadata::check_cancel(cancelled)?;
                dashboard_core::protocol::validate_tag(&read.tag)
                    .map_err(dashboard_core::invalid)?;
                let found = found(&read.tag, index);
                let run = match found.key {
                    Some(key) => measure_run(snapshot, key, read.hour)?,
                    None => None,
                };
                states.push(MeasureState {
                    approximate: false,
                    sample_hour: None,
                    experience: None,
                    segment_first_hour: None,
                    available: None,
                    read_hour: None,
                    recorded: Some(found.key.is_some()),
                    tag: read.tag,
                    hour: read.hour,
                    run,
                    drop: None,
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
                warnings: Vec::new(),
            })?
        }
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, params.tags.len())?;
            let mut lines = Vec::with_capacity(params.tags.len());
            for tag in params.tags {
                dashboard_text::metadata::check_cancel(cancelled)?;
                dashboard_core::protocol::validate_tag(&tag).map_err(dashboard_core::invalid)?;
                let found = found(&tag, index);
                let summary = match (found.key, found.first) {
                    (Some(key), Some(first)) => engine::summary(
                        snapshot,
                        key,
                        (first, last),
                        (from, to, width),
                        DECIMALS,
                        params.within.get(&tag).map(Vec::as_slice),
                    )?,
                    (Some(_), None) => {
                        return Err(invalid("A stability part's first hour is missing"));
                    }
                    (None, _) => ChartSummary {
                        from,
                        to,
                        width,
                        buckets: Vec::new(),
                        runs: Some(Vec::new()),
                        approximate: Vec::new(),
                    },
                };
                let marks = if params.with_marks {
                    Some(match found.key {
                        Some(key) => engine::clusters(snapshot, key, (from, to, width))?,
                        None => MarkSummary {
                            from,
                            to,
                            width,
                            clusters: Vec::new(),
                        },
                    })
                } else {
                    None
                };
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
                warnings: Vec::new(),
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
            let selected = if params.measures.is_empty() {
                vec![name.clone()]
            } else {
                params.measures.clone()
            };
            let mut keys = Vec::new();
            for tag in &params.tags {
                for label in &selected {
                    if let Some(key) = measure_of(label).and_then(|measure| found(tag, measure).key)
                    {
                        keys.push((key, params.within.get(tag).map(Vec::as_slice)));
                    }
                }
            }
            let mut reply = serde_json::to_value(MeasureKeyReply {
                hour: engine::navigate(snapshot, &keys, &navigation)?,
                approximate: false,
                coverage: None,
            })?;
            // Replies carry `states`, null where they hold none, as the row path's do.
            reply["states"] = Value::Null;
            return Ok(reply);
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            let tag = params.tag.ok_or_else(|| invalid("Missing event key"))?;
            dashboard_core::protocol::validate_tag(&tag).map_err(dashboard_core::invalid)?;
            let summary = match found(&tag, index).key {
                Some(key) => engine::clusters(snapshot, key, (from, to, width))?,
                None => MarkSummary {
                    from,
                    to,
                    width,
                    clusters: Vec::new(),
                },
            };
            serde_json::to_value(MeasureMarkReply {
                coverage: None,
                branch_id: params.branch_id,
                measure: name,
                tag,
                summary,
                warnings: Vec::new(),
            })?
        }
        _ => return Err(invalid("Unknown series query")),
    };
    if let Some(content) = &content {
        warnings.extend(content.catalogue.refusal_lines());
    }
    reply["parts"] = serde_json::to_value(metadata)?;
    reply["warnings"] = json!(warnings);
    // Replies carry `states`, null where they hold none, as the row path's do.
    if reply.get("states").is_none() {
        reply["states"] = Value::Null;
    }
    Ok(reply)
}
