//! Subjects' autonomy over a branch, for the Autonomy widget: each subject's
//! level, overlord, autonomy score and the scores at which it moves down or up a
//! level, hour by hour. Prepared as the `autonomy` series family ([`facts`],
//! [`emit`]) and answered from its publication ([`query`]).
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
//! - Levels are ordered by the content's autonomy states (`installed_content_core::autonomy`,
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

use dashboard_base::{
    Source,
    catalogue::{Catalogue, View, catalogue},
    products::{LoadContext, Product, country_groups::HourSpan, validate_tag},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;

pub struct Autonomy;

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

/// What a reply reads of a branch's scan: the recorded level ids in the order first seen with
/// the lowest move-down score recorded at each, each subject's runs in the order first seen
/// (their levels index the scan's levels), the subject-hours with more than one overlord, the
/// hours recorded with every table complete and those naming a country the recording does not,
/// and the subjects each overlord ever held.
#[derive(Default)]
pub struct ScannedAutonomy {
    pub levels: Vec<String>,
    pub lowest_lower: Vec<i64>,
    pub subjects: Vec<(String, Vec<AutonomyRun>)>,
    pub shared_subjects: usize,
    pub recorded: Vec<HourSpan>,
    pub unidentified: Vec<HourSpan>,
    overlord_subjects: HashMap<String, BTreeSet<String>>,
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

/// The scan's levels in level order, named: the content's order (`states`) where
/// it places them, then the rest by their lowest recorded move-down score. `name`
/// localises a level's id. Returns the levels and, for each scan level, its place
/// among them.
pub fn order_levels(
    scanned: &ScannedAutonomy,
    states: Option<&installed_content_core::autonomy::AutonomyStates>,
    name: impl Fn(&str) -> Option<String>,
) -> (Vec<AutonomyLevel>, Vec<u32>) {
    let order: HashMap<&str, usize> = states
        .map(|states| {
            states
                .ordered()
                .placed
                .into_iter()
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
pub fn content_levels(
    states: &installed_content_core::autonomy::AutonomyStates,
    catalogue: &Catalogue,
) -> Vec<AutonomyLevel> {
    let order = states.ordered();
    let level = |state: &installed_content_core::autonomy::AutonomyState, defined| AutonomyLevel {
        id: state.id.clone(),
        name: level_name(catalogue, &state.id),
        defined,
    };
    order
        .placed
        .into_iter()
        .map(|state| level(state, true))
        .chain(order.unplaced.into_iter().map(|state| level(state, false)))
        .collect()
}

/// The content's autonomy states and the notes reading them left.
type States = (
    installed_content_core::autonomy::AutonomyStates,
    Vec<installed_content_core::Diagnostic>,
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
    let states = Arc::new(installed_content_core::autonomy::read(&catalogue.content.layers)?);
    *cache = Some((Arc::clone(catalogue), Arc::clone(&states)));
    Ok(states)
}

impl Product for Autonomy {
    const KEY: &'static str = "autonomy";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<Self::Reply> {
        row::load(source, params, context, cancelled)
    }
}

impl ScannedAutonomy {
    fn reply(
        &self,
        source: &Source,
        params: &AutonomyParams,
        context: &LoadContext,
    ) -> io::Result<(AutonomyReply, Vec<u32>)> {
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
            order_levels(self, content.as_ref().map(|(states, _)| states), |id| {
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
        if self.recorded.is_empty() {
            warnings.push(
                "Subjects' autonomy was not recorded in this branch (the Autonomy or Subjects table is missing at every hour).".into(),
            );
        }
        let unidentified: i64 = self
            .unidentified
            .iter()
            .map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1)
            .sum();
        if unidentified > 0 {
            warnings.push(format!(
                "Hours with a subject or overlord the recording does not name: {unidentified}. A country without a lane then may be one of them."
            ));
        }
        if self.shared_subjects > 0 {
            warnings.push(format!(
                "Hours where a subject had more than one recorded overlord: {}. No overlord is shown then.",
                self.shared_subjects
            ));
        }
        let wanted = selection(params, &self.overlord_subjects);
        let subjects = self
            .subjects
            .iter()
            .filter(|(tag, _)| wanted(tag))
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
            recorded: self.recorded.clone(),
            unidentified: self.unidentified.clone(),
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok((reply, places))
    }
}

fn selection<'a>(
    params: &'a AutonomyParams,
    held: &'a HashMap<String, BTreeSet<String>>,
) -> impl Fn(&str) -> bool + 'a {
    let mut subjects: HashSet<_> = params.subjects.iter().map(String::as_str).collect();
    for overlord in &params.overlords {
        if let Some(tags) = held.get(overlord) {
            subjects.extend(tags.iter().map(String::as_str));
        }
    }
    move |tag| (params.subjects.is_empty() && params.overlords.is_empty()) || subjects.contains(tag)
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
