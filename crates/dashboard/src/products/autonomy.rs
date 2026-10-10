//! Subjects' autonomy over a branch, for the Autonomy widget: each subject's
//! level, overlord, autonomy score and the scores at which it moves down or up a
//! level, hour by hour.
//!
//! Every value is a recorded field, kept exact:
//!
//! - `Autonomy` is the subject's recorded status: `score_raw`, `lower_score`
//!   and `upper_score` are fixed5; `state` is the level's id, and
//!   `previous`/`next` are the levels below and above it, null at either end.
//!   The bounds use the level's and next level's `min_freedom_level` times
//!   the total score. A subject may pass its upper bound at the top level.
//! - `Subjects` names the overlord (overlord to subject relations), and its
//!   fixed5 `autonomy_ratio` is carried as recorded. Its role in the game is
//!   not established, so it is not interpreted.
//! - Levels are ordered by the content's autonomy states (`installed_content::autonomy`,
//!   by `min_freedom_level`) and named by their localisation. Without the game
//!   folders, or for a level the content does not define, the order falls back
//!   to the lowest move-down score recorded at that level, and the name is null.
//!
//! Both tables are hourly. A value holds for its hour only: runs join equal
//! consecutive recorded hours of one country identity and end at a gap or an
//! hour whose tables are incomplete.
//!
//! A row whose subject or overlord `CountryLinks` and `Countries` do not name
//! leaves its hour only partly known: the hour stays recorded, is listed in
//! `unidentified` and counted in a warning, so a country without a lane there is
//! never taken for one that was not a subject.

use super::{LoadContext, Product, country_groups::HourSpan, validate_tag};
use crate::{
    Source,
    catalogue::{Catalogue, View, catalogue},
    metadata::{link_slot, mapped_slot, validate_campaign},
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct Autonomy;

/// Branches kept per source.
const CACHED_BRANCHES: usize = 4;
/// Units per point of an autonomy score: fixed5, retaining five decimal places.
const SCORE_SCALE: u32 = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutonomyParams {
    pub branch_id: String,
    /// Every subject these countries had in the branch.
    pub overlords: Vec<String>,
    /// These countries, whenever they were subjects. Both selections empty asks
    /// for the whole branch, so frontend country lineage can resolve tag changes.
    pub subjects: Vec<String>,
}

/// A subject level, in level order (least free first).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyLevel {
    /// The autonomy state's id, as recorded.
    pub id: String,
    /// Its localised name; null without the game folders or a localisation.
    pub name: Option<String>,
    /// Whether the content's autonomy states place it in level order (define it with
    /// a plain `min_freedom_level`).
    pub defined: bool,
}

/// A subject's recorded autonomy held over consecutive hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Follows a gap, an hour without the tables or another country under the tag.
    pub starts_segment: bool,
    /// The overlord's tag; null when no relation names the subject this hour.
    pub overlord: Option<String>,
    /// Index into [`AutonomyReply::levels`].
    pub level: u32,
    /// In [`AutonomyReply::score_scale`] units.
    pub score_raw: String,
    /// The score below which it moves down a level.
    pub lower_raw: String,
    /// The score at which it moves up a level.
    pub upper_raw: String,
    /// No level below (`previous` is null) or above (`next` is null).
    pub lowest: bool,
    pub highest: bool,
    /// The relation's recorded `autonomy_ratio`, fixed5; null without a relation.
    pub ratio_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomySubject {
    pub tag: String,
    pub runs: Vec<AutonomyRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyReply {
    pub branch_id: String,
    /// Units per point of a score.
    pub score_scale: u32,
    /// Every level recorded in the branch, in level order.
    pub levels: Vec<AutonomyLevel>,
    /// The requested subjects in the order they first became subjects.
    pub subjects: Vec<AutonomySubject>,
    /// The hours recorded with both tables complete, in order: outside them nothing
    /// is known, so a country without a lane there is not recorded, not a non-subject.
    pub recorded: Vec<HourSpan>,
    /// The recorded hours, in order, with a subject or overlord the recording does
    /// not name: a country without a lane there may be one of them.
    pub unidentified: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

/// One subject's recorded state at an hour; `level` indexes the scan's levels.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sample {
    overlord: Option<String>,
    level: usize,
    score: i64,
    lower: i64,
    upper: i64,
    lowest: bool,
    highest: bool,
    ratio: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedAutonomy {
    /// Recorded level ids, in the order first seen.
    pub levels: Vec<String>,
    /// The lowest move-down score recorded at each level.
    pub lowest_lower: Vec<i64>,
    /// (tag, runs with scan level indices), in the order first seen.
    pub subjects: Vec<(String, Vec<AutonomyRun>)>,
    /// Hours where a subject had more than one overlord.
    pub shared_subjects: usize,
    /// The hours recorded with both tables complete, in order.
    pub recorded: Vec<HourSpan>,
    /// The recorded hours with a row whose subject or overlord is not named, in order.
    pub unidentified: Vec<HourSpan>,
}

/// Adds an hour to ascending spans, joining it to the last span when it follows it.
fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
        Some(span) if span.last_hour >= hour => {}
        _ => spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}

/// Scans one branch for every subject's autonomy.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedAutonomy> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Autonomy,
        TableId::Subjects,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Autonomy request cancelled");
    let mut result = ScannedAutonomy::default();
    let mut levels: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut by_tag: HashMap<String, usize> = HashMap::new();
    // Each subject's identity (slot) and sample at the last hour it was recorded.
    let mut last: Vec<Option<(i32, u32, Sample)>> = Vec::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let hour = frame.hour;
            let tables = decoded
                .complete::<tables::Countries>()
                .ok()
                .zip(decoded.complete::<tables::CountryLinks>().ok())
                .zip(decoded.complete::<tables::Autonomy>().ok())
                .zip(decoded.complete::<tables::Subjects>().ok());
            // An hour without the tables is not recorded: every run ends before it.
            let Some((((countries, links), autonomy), relations)) = tables else {
                continue;
            };
            mark(&mut result.recorded, hour);
            let tags: HashMap<u32, &[u8]> = countries
                .iter()
                .filter_map(|row| {
                    mapped_slot(row, links)
                        .ok()
                        .map(|slot| (slot, row.tag.as_slice()))
                })
                .collect();
            let mut overlords: HashMap<u32, (u32, i64)> = HashMap::new();
            let mut shared = BTreeSet::new();
            // A row naming a country the recording does not: this hour is known only in part.
            let mut unidentified = false;
            for relation in relations {
                let (Some(overlord), Some(subject)) = (
                    link_slot(links, relation.overlord),
                    link_slot(links, relation.subject),
                ) else {
                    unidentified = true;
                    continue;
                };
                unidentified |= !tags.contains_key(&overlord) || !tags.contains_key(&subject);
                if overlords
                    .insert(subject, (overlord, relation.autonomy_ratio))
                    .is_some()
                {
                    shared.insert(subject);
                }
            }
            result.shared_subjects += shared.len();
            for row in autonomy {
                let Some((slot, tag)) = link_slot(links, row.subject)
                    .and_then(|slot| tags.get(&slot).map(|tag| (slot, tag)))
                else {
                    unidentified = true;
                    continue;
                };
                let tag = String::from_utf8_lossy(tag).into_owned();
                let level = *levels.entry(row.state.clone()).or_insert_with(|| {
                    result
                        .levels
                        .push(String::from_utf8_lossy(&row.state).into_owned());
                    result.lowest_lower.push(row.lower_score);
                    result.levels.len() - 1
                });
                let lowest = &mut result.lowest_lower[level];
                *lowest = (*lowest).min(row.lower_score);
                let relation = overlords.get(&slot).filter(|_| !shared.contains(&slot));
                let sample = Sample {
                    overlord: relation.and_then(|(overlord, _)| {
                        tags.get(overlord)
                            .map(|tag| String::from_utf8_lossy(tag).into_owned())
                    }),
                    level,
                    score: row.score,
                    lower: row.lower_score,
                    upper: row.upper_score,
                    lowest: row.previous.is_none(),
                    highest: row.next.is_none(),
                    ratio: relation.map(|(_, ratio)| *ratio),
                };
                let at = *by_tag.entry(tag.clone()).or_insert_with(|| {
                    result.subjects.push((tag.clone(), Vec::new()));
                    last.push(None);
                    result.subjects.len() - 1
                });
                let previous = last[at].take();
                let continues = previous.as_ref().is_some_and(|(seen, seen_slot, _)| {
                    seen.checked_add(1) == Some(hour) && *seen_slot == slot
                });
                let subject_runs = &mut result.subjects[at].1;
                let same = continues
                    && previous
                        .as_ref()
                        .is_some_and(|(_, _, seen)| *seen == sample);
                if same && let Some(run) = subject_runs.last_mut() {
                    run.last_hour = hour;
                } else {
                    subject_runs.push(AutonomyRun {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment: !continues,
                        overlord: sample.overlord.clone(),
                        level: level as u32,
                        score_raw: sample.score.to_string(),
                        lower_raw: sample.lower.to_string(),
                        upper_raw: sample.upper.to_string(),
                        lowest: sample.lowest,
                        highest: sample.highest,
                        ratio_raw: sample.ratio.map(|ratio| ratio.to_string()),
                    });
                }
                last[at] = Some((hour, slot, sample));
            }
            if unidentified {
                mark(&mut result.unidentified, hour);
            }
        }
    }
    Ok(result)
}

/// The scan's levels in level order, named: the content's order (`states`) where
/// it places them, then the rest by their lowest recorded move-down score. `name`
/// localises a level's id. Returns the levels and, for each scan level, its place
/// among them.
pub fn order_levels(
    scanned: &ScannedAutonomy,
    states: Option<&installed_content::autonomy::AutonomyStates>,
    name: impl Fn(&str) -> Option<String>,
) -> (Vec<AutonomyLevel>, Vec<u32>) {
    let order: HashMap<&str, usize> = states
        .map(|states| {
            states
                .ordered()
                .into_iter()
                .filter(|state| state.min_freedom_level.is_some())
                .enumerate()
                .map(|(place, state)| (state.id.as_str(), place))
                .collect()
        })
        .unwrap_or_default();
    let mut indices: Vec<usize> = (0..scanned.levels.len()).collect();
    indices.sort_by_key(|&index| {
        let id = scanned.levels[index].as_str();
        (
            !order.contains_key(id),
            order.get(id).copied().unwrap_or(0),
            scanned.lowest_lower[index],
            index,
        )
    });
    let mut places = vec![0_u32; scanned.levels.len()];
    let levels = indices
        .iter()
        .enumerate()
        .map(|(place, &index)| {
            places[index] = place as u32;
            let id = scanned.levels[index].clone();
            AutonomyLevel {
                defined: order.contains_key(id.as_str()),
                name: name(&id),
                id,
            }
        })
        .collect();
    (levels, places)
}

pub(crate) fn level_name(catalogue: &Catalogue, id: &str) -> Option<String> {
    catalogue.localised(id, View::Plain)
}

/// Authored levels only: offline preparation has no scores to rank unplaced levels.
pub(crate) fn content_levels(
    states: &installed_content::autonomy::AutonomyStates,
    catalogue: &Catalogue,
) -> Vec<AutonomyLevel> {
    states
        .ordered()
        .into_iter()
        .map(|state| AutonomyLevel {
            id: state.id.clone(),
            name: level_name(catalogue, &state.id),
            defined: state.min_freedom_level.is_some(),
        })
        .collect()
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(u64, Arc<ScannedAutonomy>)>,
}

/// The content's autonomy states and the notes reading them left.
type States = (
    installed_content::autonomy::AutonomyStates,
    Vec<installed_content::Diagnostic>,
);
/// The states read for a catalogue, read once per catalogue (held, so a new one is
/// never mistaken for it).
type StatesCache = Option<(Arc<Catalogue>, Arc<States>)>;

/// The content's autonomy states, read once per catalogue.
fn content_states(source: &Source, catalogue: &Arc<Catalogue>) -> io::Result<Arc<States>> {
    let cache = source.state::<StatesCache>();
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, states)) = cache.as_ref()
        && Arc::ptr_eq(cached, catalogue)
    {
        return Ok(Arc::clone(states));
    }
    let states = Arc::new(installed_content::autonomy::read(&catalogue.content.layers)?);
    *cache = Some((Arc::clone(catalogue), Arc::clone(&states)));
    Ok(states)
}

impl Product for Autonomy {
    const KEY: &'static str = "autonomy";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AutonomyParams;
    type Reply = AutonomyReply;

    fn types() -> Vec<String> {
        vec![
            AutonomyParams::decl(),
            AutonomyLevel::decl(),
            AutonomyRun::decl(),
            AutonomySubject::decl(),
            AutonomyReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        params
            .overlords
            .iter()
            .chain(&params.subjects)
            .try_for_each(|tag| validate_tag(tag))
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch: u64 = params.branch_id.parse().expect("validated branch id");
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == branch)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != branch);
                cache.entries.push_back((branch, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_BRANCHES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = Vec::new();
        // Without the game folders the lanes still draw, ordered and named by the recording.
        let content = match context.content.as_ref() {
            Some(options) => match catalogue(source, options)
                .and_then(|catalogue| Ok((content_states(source, &catalogue)?, catalogue)))
            {
                Ok((states, catalogue)) => {
                    warnings.extend(states.1.iter().map(|(_, message)| message.clone()));
                    Some((states.0.clone(), catalogue))
                }
                Err(error) => {
                    warnings.push(format!("Subject level names and order could not be read from the game's files: {error}"));
                    None
                }
            },
            None => {
                warnings.push("Choose the game folders in Settings to see subject level names; levels are ordered by their recorded move-down scores.".into());
                None
            }
        };
        let (levels, places) =
            order_levels(&scanned, content.as_ref().map(|(states, _)| states), |id| {
                level_name(&content.as_ref()?.1, id)
            });
        if let Some((_, catalogue)) = &content {
            warnings.extend(catalogue.refusal_lines());
            let undefined: Vec<_> = levels
                .iter()
                .filter(|level| !level.defined)
                .map(|level| level.id.as_str())
                .collect();
            if !undefined.is_empty() {
                warnings.push(format!(
                    "Subject levels the game's autonomy states do not place: {}. They follow the others, by their recorded move-down scores.",
                    undefined.join(", ")
                ));
            }
        }
        if scanned.recorded.is_empty() {
            warnings.push(
                "Subjects' autonomy was not recorded in this branch (the Autonomy or Subjects table is missing at every hour).".into(),
            );
        }
        let unidentified: i64 = scanned
            .unidentified
            .iter()
            .map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1)
            .sum();
        if unidentified > 0 {
            warnings.push(format!(
                "Hours with a subject or overlord the recording does not name: {unidentified}. A country without a lane then may be one of them."
            ));
        }
        if scanned.shared_subjects > 0 {
            warnings.push(format!(
                "Hours where a subject had more than one recorded overlord: {}. No overlord is shown then.",
                scanned.shared_subjects
            ));
        }
        let subjects: HashSet<_> = params.subjects.iter().map(String::as_str).collect();
        let overlords: HashSet<_> = params.overlords.iter().map(String::as_str).collect();
        let wanted = |tag: &str, runs: &[AutonomyRun]| {
            (params.subjects.is_empty() && params.overlords.is_empty())
                || subjects.contains(tag)
                || runs.iter().any(|run| {
                    run.overlord
                        .as_ref()
                        .is_some_and(|overlord| overlords.contains(overlord.as_str()))
                })
        };
        let subjects = scanned
            .subjects
            .iter()
            .filter(|(tag, runs)| wanted(tag, runs))
            .map(|(tag, runs)| AutonomySubject {
                tag: tag.clone(),
                runs: runs
                    .iter()
                    .map(|run| AutonomyRun {
                        level: places[run.level as usize],
                        ..run.clone()
                    })
                    .collect(),
            })
            .collect();
        let reply = AutonomyReply {
            branch_id: params.branch_id.clone(),
            score_scale: SCORE_SCALE,
            levels,
            subjects,
            recorded: scanned.recorded.clone(),
            unidentified: scanned.unidentified.clone(),
            warnings,
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
