//! Who was whose subject over one branch (the Subjects widget): each
//! overlord–subject relation as runs of consecutive recorded hours, with the
//! subject's level (its autonomy state) through each run and the engine's own
//! start hour of the relation.
//!
//! A relation counts while the viewer's political map counts it
//! (`active_subject` there): not cancelled, and before its recorded end when it
//! has one (an end not after its start is no end). Its start hour is the
//! engine's (`engine::csubject_relation::START_HOUR`), which the fresh-save
//! oracle compares with the save's start date for every relation that is not
//! cancelled (`compare_subjects` in the recorder), so a relation already running
//! when recording began is dated as the game dates it. Two runs of one pair with
//! different start hours are different relations.
//!
//! The level is the subject's current autonomy state (`Autonomy.state`),
//! not `Subjects.state`: that is the relation's
//! initial definition, which normal autonomy changes leave as it was (the
//! viewer's political map and `installed_content::country_names` read it the same way).
//! An hour whose Autonomy table is not complete, or that names no state for the
//! subject, has no known level. Level names are the content's localisation of
//! each state id, and their order the content's (`installed_content::autonomy`: least
//! free first), so a mod's levels work; a level the content does not define keeps
//! its id and sorts last.
//!
//! Hours whose Subjects table, or the country tables naming its countries, were
//! not recorded complete are unobserved: runs break there and nothing is carried
//! across. A relation naming a country the links cannot resolve leaves the hour's
//! list incomplete, so that hour is unobserved too, though the runs of the
//! relations it could name go on.

use super::{
    ContentOptions, LoadContext, Product,
    country_groups::{HourRun, HourSpan, extend, mark_unobserved},
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{LinkTags, branch_bounds, validate_campaign},
};
use installed_content::autonomy::AutonomyStates;
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{SubjectRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct Subjects;

/// Branches kept per source, so tracks on different branches do not evict each other.
const CACHED_BRANCHES: usize = 4;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubjectParams {
    pub branch_id: String,
}

/// A subject level (autonomy state) the reply names.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubjectLevel {
    /// The autonomy state's id, as recorded.
    pub token: String,
    /// The game's name for it; null when the localisation has none or is not read.
    pub name: Option<String>,
    /// Whether the content defines the state and orders it (by its
    /// `min_freedom_level`); a level it does not is listed after those it does.
    pub ordered: bool,
}

/// Consecutive hours of a run with one level; `level` is null where it was not recorded.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LevelStep {
    pub first_hour: i32,
    pub last_hour: i32,
    pub level: Option<String>,
}

/// One overlord–subject relation over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubjectRun {
    pub overlord: String,
    pub subject: String,
    /// The hour the relation began, as the engine dates it (may be before the branch).
    pub start_hour: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    /// The subject's level through the run, in hour order, covering every hour of it.
    pub steps: Vec<LevelStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubjectReply {
    pub branch_id: String,
    /// Every level the runs name, least free first in the content's order.
    pub levels: Vec<SubjectLevel>,
    /// Every run, by first hour, then overlord and subject.
    pub runs: Vec<SubjectRun>,
    /// Hours inside the branch whose relations were not observed: a run that ends
    /// or starts next to one of these is a gap, not an end or a start.
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

impl Product for Subjects {
    const KEY: &'static str = "subjects";
    const CONTENT_DEPENDENT: bool = true;
    type Params = SubjectParams;
    type Reply = SubjectReply;

    fn types() -> Vec<String> {
        vec![
            SubjectParams::decl(),
            SubjectLevel::decl(),
            LevelStep::decl(),
            SubjectRun::decl(),
            SubjectReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid subjects branch")
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let scan = scanned_branch(source, branch, cancelled)?;
        let mut warnings = Vec::new();
        let content = match context.content.as_ref() {
            Some(options) => match subject_content(source, options) {
                Ok(content) => {
                    warnings.extend(content.warnings.iter().cloned());
                    Some(content)
                }
                Err(error) => {
                    warnings.push(format!(
                        "Subject levels show by their ids, in no known order: {error}"
                    ));
                    None
                }
            },
            None => {
                warnings.push(
                    "Subject levels show by their ids, in no known order, until the game folders are chosen."
                        .into(),
                );
                None
            }
        };
        warnings.extend(scan.warnings.iter().cloned());
        let named: BTreeSet<&str> = scan
            .runs
            .iter()
            .flat_map(|run| run.steps.iter().filter_map(|step| step.level.as_deref()))
            .collect();
        let levels = levels(
            &named,
            content.as_ref().map(|content| &content.states),
            content.as_ref().map(|content| &*content.catalogue),
        );
        if let Some(content) = &content {
            warnings.extend(content.catalogue.refusal_lines());
        }
        Ok(SubjectReply {
            branch_id: params.branch_id.clone(),
            levels,
            runs: scan.runs.clone(),
            unobserved: scan.unobserved.clone(),
            warnings,
        })
    }
}

/// The levels `named`, in the content's order (least free first), then those the
/// content does not order, by id. Each is named by the localisation of its id.
pub fn levels(
    named: &BTreeSet<&str>,
    states: Option<&AutonomyStates>,
    catalogue: Option<&Catalogue>,
) -> Vec<SubjectLevel> {
    let name = |token: &str| {
        catalogue.and_then(|catalogue| catalogue.localised(token, catalogue::View::Plain))
    };
    let mut result = Vec::new();
    let mut placed = BTreeSet::new();
    if let Some(states) = states {
        for state in states.ordered() {
            if state.min_freedom_level.is_some() && named.contains(state.id.as_str()) {
                placed.insert(state.id.as_str());
                result.push(SubjectLevel {
                    token: state.id.clone(),
                    name: name(&state.id),
                    ordered: true,
                });
            }
        }
    }
    for token in named.iter().filter(|token| !placed.contains(*token)) {
        result.push(SubjectLevel {
            token: (*token).to_owned(),
            name: name(token),
            ordered: false,
        });
    }
    result
}

/// The autonomy states of the source's content, read once per catalogue.
struct SubjectContent {
    catalogue: Arc<Catalogue>,
    states: AutonomyStates,
    warnings: Vec<String>,
}

fn subject_content(source: &Source, options: &ContentOptions) -> io::Result<Arc<SubjectContent>> {
    let catalogue = catalogue::catalogue(source, options)?;
    let state = source.state::<Option<Arc<SubjectContent>>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = state.as_ref()
        && Arc::ptr_eq(&cached.catalogue, &catalogue)
    {
        return Ok(Arc::clone(cached));
    }
    let (states, warnings) = installed_content::autonomy::read(&catalogue.content.layers)?;
    let loaded = Arc::new(SubjectContent {
        catalogue,
        states,
        warnings: warnings.into_iter().map(|(_, message)| message).collect(),
    });
    *state = Some(Arc::clone(&loaded));
    Ok(loaded)
}

impl HourRun for SubjectRun {
    fn last_hour(&self) -> i32 {
        self.last_hour
    }
    fn set_last_hour(&mut self, hour: i32) {
        self.last_hour = hour;
    }
}

/// Whether a recorded relation holds at `hour`, as the viewer's political map
/// reads it (`active_subject`): not cancelled, and before its end when the end
/// is after its start (the engine writes an end only then).
pub fn active(row: &SubjectRow, hour: i32) -> bool {
    row.cancel == 0 && (row.end_hour <= row.start_hour || hour < row.end_hour)
}

pub(crate) fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Scan>> {
    let state = source.state::<Vec<(u64, Arc<Scan>)>>();
    let cached = {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .iter()
            .position(|(cached, _)| *cached == branch)
            .map(|index| {
                let cached = state.remove(index);
                let scan = Arc::clone(&cached.1);
                state.insert(0, cached);
                scan
            })
    };
    // Scan outside the lock, so requests for other branches are not held up.
    let scan = match cached {
        Some(scan) => scan,
        None => {
            let scan = Arc::new(scan_branch(&source.replay(), branch, cancelled)?);
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            state.retain(|(cached, _)| *cached != branch);
            state.insert(0, (branch, Arc::clone(&scan)));
            state.truncate(CACHED_BRANCHES);
            scan
        }
    };
    Ok(scan)
}

/// One branch's subject history, before levels are named.
pub struct Scan {
    pub(crate) runs: Vec<SubjectRun>,
    pub(crate) unobserved: Vec<HourSpan>,
    pub(crate) warnings: Vec<String>,
    pub(crate) starts: Vec<SubjectStart>,
    pub(crate) observed: bool,
}

/// The recorded relation date when both identities are known then; otherwise
/// only a new relation witnessed after a complete preceding hour is an event.
#[derive(Clone, Debug)]
pub(crate) struct SubjectStart {
    pub subject: String,
    pub overlord: String,
    pub hour: i32,
    pub read_hour: i32,
    pub exact: bool,
}

#[cfg(test)]
impl Scan {
    pub fn runs(&self) -> &[SubjectRun] {
        &self.runs
    }
    pub fn unobserved(&self) -> &[HourSpan] {
        &self.unobserved
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn scan_branch(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Subjects,
        TableId::Autonomy,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Subjects request cancelled");
    let mut runs: Vec<SubjectRun> = Vec::new();
    let mut open: BTreeMap<(String, String, i32), usize> = BTreeMap::new();
    let mut unobserved = Vec::new();
    let mut tags = LinkTags::default();
    let mut unmapped = BTreeSet::new();
    let mut incomplete = 0_u32;
    let mut unleveled = 0_u32;
    let mut unreadable = 0_u32;
    let mut conflicting = BTreeSet::new();
    let mut unreadable_blocks = 0_u32;
    let mut stopped = 0_u32;
    let mut starts = Vec::new();
    let mut seen_starts = BTreeSet::new();
    let mut named_since: BTreeMap<u32, (String, i32)> = BTreeMap::new();
    let mut continuous_since = None;
    let mut first_observed = None;
    let mut previous_complete = None;
    let mut previous_identities = BTreeMap::new();
    let mut identities = BTreeMap::new();
    let mut identities_changed = false;
    let mut identity_since = BTreeMap::<u32, (u32, i32)>::new();
    let mut observed = false;
    // Hours before the first readable frame and after the last are unobserved too.
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start));
    let mut previous_hour: Option<i32> = bounds.and_then(|(first, _)| first.checked_sub(1));
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    unreadable_blocks += 1;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        stopped += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        stopped += 1;
                        break;
                    }
                    let hour = frame.hour;
                    let first_observed = *first_observed.get_or_insert(hour);
                    if let Some(previous) = previous_hour
                        && hour > previous + 1
                    {
                        mark_unobserved(&mut unobserved, previous + 1, hour - 1);
                        continuous_since = None;
                    }
                    previous_hour = Some(hour);
                    let Ok(rebuilt) = tags.update(decoded) else {
                        incomplete += 1;
                        mark_unobserved(&mut unobserved, hour, hour);
                        continuous_since = None;
                        previous_complete = None;
                        continue;
                    };
                    if rebuilt {
                        identities_changed = true;
                        identities = decoded
                            .complete::<tables::Countries>()?
                            .iter()
                            .map(|country| (country.canonical_slot, country.id))
                            .collect();
                        identity_since.retain(|slot, (id, _)| identities.get(slot) == Some(id));
                        for (&slot, &id) in &identities {
                            identity_since.entry(slot).or_insert((id, hour));
                        }
                        named_since.retain(|raw, (tag, _)| tags.get(*raw) == Some(tag));
                        for (raw, tag) in tags.iter() {
                            named_since
                                .entry(*raw)
                                .or_insert_with(|| (tag.clone(), hour));
                        }
                    }
                    let Ok(rows) = decoded.complete::<tables::Subjects>() else {
                        incomplete += 1;
                        mark_unobserved(&mut unobserved, hour, hour);
                        continuous_since = None;
                        previous_complete = None;
                        continue;
                    };
                    observed = true;
                    let links = decoded.complete::<tables::CountryLinks>()?;
                    let since = *continuous_since.get_or_insert(hour);
                    // Each subject's current level this hour, by tag; none when the table is not
                    // complete. A subject the rows give two levels has no known level.
                    let levels: Option<HashMap<&String, Option<&[u8]>>> =
                        decoded.complete::<tables::Autonomy>().ok().map(|autonomy| {
                            let mut levels: HashMap<&String, Option<&[u8]>> = HashMap::new();
                            for row in autonomy.iter() {
                                let Some(tag) = tags.get(row.subject) else {
                                    continue;
                                };
                                let level = levels.entry(tag).or_insert(Some(row.state.as_slice()));
                                if *level != Some(row.state.as_slice()) {
                                    conflicting.insert(tag.clone());
                                    *level = None;
                                }
                            }
                            levels
                        });
                    let mut named_all = true;
                    for row in rows.iter().filter(|row| active(row, hour)) {
                        let (Some(overlord), Some(subject)) =
                            (tags.get(row.overlord), tags.get(row.subject))
                        else {
                            for raw in [row.overlord, row.subject] {
                                if tags.get(raw).is_none() {
                                    unmapped.insert(raw);
                                }
                            }
                            named_all = false;
                            continue;
                        };
                        // A level whose recorded id is not text (or is empty) is refused, and counted.
                        let level = match levels
                            .as_ref()
                            .and_then(|levels| levels.get(subject).copied().flatten())
                        {
                            Some(bytes) => match std::str::from_utf8(bytes) {
                                Ok(token) if !token.is_empty() => Some(token.to_owned()),
                                _ => {
                                    unreadable += 1;
                                    None
                                }
                            },
                            None => None,
                        };
                        if level.is_none() {
                            unleveled += 1;
                        }
                        let key = (overlord.clone(), subject.clone(), row.start_hour);
                        let started =
                            extend(&mut open, &mut runs, key.clone(), hour, || SubjectRun {
                                overlord: overlord.clone(),
                                subject: subject.clone(),
                                start_hour: row.start_hour,
                                first_hour: hour,
                                last_hour: hour,
                                steps: Vec::new(),
                            });
                        if started {
                            let identity_key = [row.overlord, row.subject].map(|raw| {
                                crate::metadata::link_slot(links, raw)
                                    .and_then(|slot| identities.get(&slot).map(|id| (slot, *id)))
                            });
                            let identity_key = identity_key[0]
                                .zip(identity_key[1])
                                .map(|(overlord, subject)| (overlord, subject, row.start_hour));
                            let new_identity =
                                identity_key.is_some_and(|identity| seen_starts.insert(identity));
                            if new_identity && row.start_hour > first_observed {
                                // Relations serialize start_date; names are exact only while both raw
                                // links keep their tags. A start at or before the first observed frame is
                                // an initial snapshot, even if first published later.
                                let date_named =
                                    [row.overlord, row.subject].into_iter().all(|raw| {
                                        named_since.get(&raw).is_some_and(|(_, from)| {
                                            since.max(*from) <= row.start_hour
                                        })
                                    }) && identity_key.is_some_and(|(overlord, subject, _)| {
                                        [overlord, subject].into_iter().all(|(slot, id)| {
                                            identity_since.get(&slot).is_some_and(|(seen, from)| {
                                                *seen == id && *from <= row.start_hour
                                            })
                                        })
                                    });
                                let exact = date_named && row.start_hour <= hour;
                                let witnessed =
                                    previous_complete.is_some_and(|previous: i32| {
                                        previous.checked_add(1) == Some(hour)
                                    }) && identity_key.is_some_and(|(overlord, subject, _)| {
                                        [overlord, subject].into_iter().all(|(slot, id)| {
                                            previous_identities.get(&slot) == Some(&id)
                                        })
                                    });
                                if exact || witnessed {
                                    starts.push(SubjectStart {
                                        subject: subject.clone(),
                                        overlord: overlord.clone(),
                                        hour: if exact { row.start_hour } else { hour },
                                        read_hour: hour,
                                        exact,
                                    });
                                }
                            }
                        }
                        let run = &mut runs[open[&key]];
                        match run.steps.last_mut() {
                            Some(step) if step.last_hour == hour => {}
                            Some(step)
                                if step.level == level
                                    && step.last_hour + 1 == hour
                                    && !started =>
                            {
                                step.last_hour = hour;
                            }
                            _ => {
                                run.steps.push(LevelStep {
                                    first_hour: hour,
                                    last_hour: hour,
                                    level,
                                });
                            }
                        }
                    }
                    if !named_all {
                        mark_unobserved(&mut unobserved, hour, hour);
                    }
                    previous_complete = named_all.then_some(hour);
                    if identities_changed {
                        previous_identities.clone_from(&identities);
                        identities_changed = false;
                    }
                }
            }
            Ok(())
        },
    )?;
    if let (Some((first, last)), previous) = (bounds, previous_hour) {
        let from = previous.map_or(first, |previous| previous.saturating_add(1));
        if from <= last {
            mark_unobserved(&mut unobserved, from, last);
        }
    }
    runs.sort_by(|a, b| {
        (a.first_hour, &a.overlord, &a.subject, a.start_hour).cmp(&(
            b.first_hour,
            &b.overlord,
            &b.subject,
            b.start_hour,
        ))
    });
    let mut warnings = Vec::new();
    if !unmapped.is_empty() {
        warnings.push(format!(
            "{} recorded country references had no country identity; relations naming them are left out.",
            unmapped.len()
        ));
    }
    if incomplete > 0 {
        warnings.push(format!(
            "Subject tables were incomplete at {incomplete} hours; relations break there rather than carry over."
        ));
    }
    if unleveled > 0 {
        warnings.push(format!(
            "A subject's level was not recorded for {unleveled} subject-hours; those stretches show no level."
        ));
    }
    if unreadable > 0 {
        warnings.push(format!(
            "A subject's level was recorded under an id that is not text for {unreadable} subject-hours; those stretches show no level."
        ));
    }
    if !conflicting.is_empty() {
        warnings.push(format!(
            "{} subjects were recorded with two levels in one hour; their level is not known then.",
            conflicting.len()
        ));
    }
    if unreadable_blocks > 0 || stopped > 0 {
        warnings.push(format!(
            "Part of this branch could not be read ({unreadable_blocks} unreadable blocks, {stopped} stopped reads); subject history is missing there."
        ));
    }
    Ok(Scan {
        runs,
        unobserved,
        warnings,
        starts,
        observed,
    })
}

#[cfg(test)]
mod tests;
