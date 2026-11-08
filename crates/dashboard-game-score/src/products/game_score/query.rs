//! Game score's replies, read from the family's series publication ([`super::emit`]): every
//! country's score at an hour, a part's chart summaries and keyboard navigation.

use super::emit::{DECIMALS, Detail, FAMILY, METADATA, Metadata, PART, PRESENT, kind};
use super::{
    GameScoreAmount, GameScoreCountry, GameScoreReply, GameScoreRun, GameScoreSeries,
    GameScoreValues, PARTS, Q, ReasonTable,
};
use dashboard_base::prepared::{engine, series};
use dashboard_core::protocol::validate_tag;
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot, codec::Binary, family::Directory, query::EventView,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

#[derive(Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreStateReply {
    pub reply: GameScoreReply,
    /// Native-holder continuity, including aliases, for the table/race row builder.
    pub stretches: BTreeMap<String, String>,
    /// Always false: every value is read from accepted preparation.
    pub approximate: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    kind: String,
    measure: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    hour: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<Read>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    key: Option<series::MeasureKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
}

/// The hour a `series-state` request reads, after checking it asks one.
pub fn state_hour(params: &Value) -> io::Result<i32> {
    let params: Params = serde_json::from_value(params.clone())?;
    match params.reads.as_slice() {
        [read] => Ok(read.hour),
        _ => Err(invalid("Score state requires one hour")),
    }
}

fn field_index(measure: Option<&str>) -> io::Result<usize> {
    PARTS
        .iter()
        .position(|part| Some(*part) == measure)
        .ok_or_else(|| invalid("Unknown score measure"))
}

/// The metadata the publication holds.
fn metadata(directory: &Directory) -> io::Result<Metadata> {
    match directory.scalar(METADATA) {
        Some(scalar) => Ok(serde_json::from_slice(
            scalar
                .text()
                .ok_or_else(|| invalid("Prepared score metadata is not valid"))?,
        )?),
        None => Ok(Metadata::default()),
    }
}

/// An actor's keys: PRESENT, then each part.
fn keys(directory: &Directory, actor: u64) -> Option<[u64; 7]> {
    let mut keys = [0; 7];
    for (measure, key) in keys.iter_mut().enumerate() {
        *key = directory.key(actor, measure as u64, None)?;
    }
    Some(keys)
}

/// The run of `key` holding `hour`, whole.
fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| run))
}

/// The detail an actor's PRESENT key holds by `hour`: its latest event then.
fn detail_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Detail> {
    let malformed = || invalid("Prepared score facts are not valid");
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            hour,
            None,
        )?
        .ok_or_else(|| invalid("Missing score facts"))?;
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind::DETAIL || event.fields.len != Cell::SIZE as u64 {
        return Err(malformed());
    }
    let Cell::Text(blob) = snapshot.field(event.fields, 0)? else {
        return Err(malformed());
    };
    let length = usize::try_from(blob.len).map_err(|_| malformed())?;
    let mut bytes = vec![0; length];
    if length != 0 {
        snapshot.cell_bytes(blob, 0, &mut bytes)?;
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// An actor's detail and score parts at `hour`, as one read of it.
fn values_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: [u64; 7],
    hour: i32,
    reasons: &mut ReasonTable,
    budget: &mut usize,
) -> io::Result<(Detail, GameScoreSeries)> {
    let detail = detail_at(snapshot, keys[PRESENT as usize], hour)?;
    let mut amounts = Vec::with_capacity(6);
    for index in 0..6 {
        let raw = run_at(snapshot, keys[PART as usize + index], hour)?
            .map(|run| i64::try_from(run.value))
            .transpose()
            .map_err(|_| invalid("Score value exceeds native width"))?;
        amounts.push(GameScoreAmount {
            raw,
            missing: detail.missing[index].clone(),
            read_each_hour: detail.read_each[index],
        });
    }
    let [army, navy, air, industry, land, total]: [GameScoreAmount; 6] = amounts
        .try_into()
        .map_err(|_| invalid("Score parts are not six"))?;
    let mut scores = GameScoreSeries::default();
    scores.push(
        hour,
        &GameScoreValues {
            army,
            navy,
            air,
            industry,
            land,
            total,
        },
        true,
        reasons,
        budget,
    )?;
    Ok((detail, scores))
}

/// A `game-score` series request (`series-state`, `series-summary` or `series-next-change`)
/// answered from `snapshot`, its publication pinned through `reach` where its coverage is
/// incomplete. A state read past `reach` is the caller's to refuse.
pub fn series<P: PageRead>(
    product: &str,
    params: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    reach: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params.clone())?;
    if params.family.kind != "game-score" {
        return Err(invalid("Unknown score family"));
    }
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid score branch"))?;
    if !params.tags.is_empty() {
        for tag in &params.tags {
            validate_tag(tag).map_err(dashboard_core::invalid)?;
        }
    }
    let mut selected: Vec<(String, u64)> = directory
        .actors
        .iter()
        .enumerate()
        .map(|(actor, held)| (String::from_utf8_lossy(&held.name).into_owned(), actor as u64))
        .filter(|(tag, _)| params.tags.is_empty() || params.tags.contains(tag))
        .collect();
    selected.sort_unstable();
    let metadata = metadata(directory)?;
    Ok(match product {
        "series-state" => {
            let [read] = params.reads.as_slice() else {
                return Err(invalid("Score state requires one hour"));
            };
            let hour = read.hour;
            let mut reasons = ReasonTable::default();
            let mut budget = 0;
            let mut countries = Vec::new();
            let mut stretches = BTreeMap::new();
            let family = keys(directory, FAMILY);
            let recorded = match family {
                Some(keys) => run_at(snapshot, keys[PRESENT as usize], hour)?.is_some(),
                None => false,
            };
            let multipliers = match family.filter(|_| recorded) {
                Some(keys) => values_at(snapshot, keys, hour, &mut reasons, &mut budget)?.1,
                None => GameScoreSeries::default(),
            };
            for (tag, actor) in selected {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let keys = keys(directory, actor).ok_or_else(|| invalid("Score country has no keys"))?;
                if run_at(snapshot, keys[PRESENT as usize], hour)?.is_none() {
                    continue;
                }
                let (detail, scores) = values_at(snapshot, keys, hour, &mut reasons, &mut budget)?;
                if let Some(stretch) = detail.stretch {
                    stretches.insert(tag.clone(), stretch);
                }
                let identity = detail
                    .identity
                    .ok_or_else(|| invalid("Score country identity is missing"))?;
                countries.push(GameScoreCountry {
                    tag,
                    scores,
                    identity: vec![GameScoreRun {
                        first_hour: hour,
                        last_hour: hour,
                        value: identity,
                    }],
                });
            }
            serde_json::to_value(GameScoreStateReply {
                reply: GameScoreReply {
                    branch_id: params.branch_id,
                    scale: Q,
                    reads: if recorded { vec![hour] } else { Vec::new() },
                    countries,
                    multipliers,
                    reasons: reasons.entries,
                    hints: metadata.hints,
                    warnings: metadata.content_warnings,
                    score_warnings: metadata.warnings,
                },
                stretches,
                approximate: false,
            })?
        }
        "series-summary" => {
            let index = field_index(params.family.measure.as_deref())?;
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, selected.len())?;
            let mut summaries = Vec::with_capacity(selected.len());
            for (tag, actor) in selected {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let key = keys(directory, actor)
                    .ok_or_else(|| invalid("Score country has no keys"))?[PART as usize + index];
                let summary = engine::summary(
                    snapshot,
                    key,
                    (i32::MIN, i32::MAX),
                    (from, to, width),
                    DECIMALS,
                    None,
                )?;
                summaries.push(json!({ "tag": tag, "summary": summary }));
            }
            json!({ "branchId": params.branch_id, "decimals": DECIMALS, "series": summaries,
                "warnings": metadata.content_warnings })
        }
        "series-next-change" => {
            let index = field_index(params.family.measure.as_deref())?;
            let mut navigation = series::Navigation {
                from: params.from,
                first_hour: params
                    .first_hour
                    .ok_or_else(|| invalid("Missing first score hour"))?,
                last_hour: params
                    .last_hour
                    .ok_or_else(|| invalid("Missing last score hour"))?,
                key: params
                    .key
                    .ok_or_else(|| invalid("Missing score navigation"))?,
            };
            // Nothing past the coverage is read.
            if let Some(reach) = reach.map(f64::from) {
                if reach < navigation.first_hour {
                    return Ok(json!({ "hour": null }));
                }
                navigation.last_hour = navigation.last_hour.min(reach);
            }
            let mut keys_of = Vec::with_capacity(selected.len());
            for (_, actor) in &selected {
                let key = keys(directory, *actor)
                    .ok_or_else(|| invalid("Score country has no keys"))?[PART as usize + index];
                keys_of.push((key, None));
            }
            json!({ "hour": engine::navigate(snapshot, &keys_of, &navigation)? })
        }
        _ => return Err(invalid("Unknown prepared score query")),
    })
}
