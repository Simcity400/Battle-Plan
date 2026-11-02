//! Who was whose subject over one branch (the Subjects widget): each
//! overlord–subject relation as runs of consecutive recorded hours, with the
//! subject's level (its autonomy state) through each run and the engine's own
//! start hour of the relation.
//!
//! A relation counts while the viewer's political map counts it
//! (`active_subject` there): not cancelled, and before its recorded end when it
//! has one (an end not after its start is no end). Its start hour is the
//! engine's (`engine_native::csubject_relation::START_HOUR`), which the fresh-save
//! oracle compares with the save's start date for every relation that is not
//! cancelled (`compare_subjects` in the recorder), so a relation already running
//! when recording began is dated as the game dates it. Two runs of one pair with
//! different start hours are different relations.
//!
//! The level is the subject's current autonomy state (`Autonomy.state`),
//! not `Subjects.state`: that is the relation's
//! initial definition, which normal autonomy changes leave as it was (the
//! viewer's political map and `installed_content_country_names::country_names` read it the same way).
//! An hour whose Autonomy table is not complete, or that names no state for the
//! subject, has no known level. Level names are the content's localisation of
//! each state id, and their order the content's (`installed_content_core::autonomy`: least
//! free first), so a mod's levels work; a level the content does not define keeps
//! its id and sorts last.
//!
//! Hours whose Subjects table, or the country tables naming its countries, were
//! not recorded complete are unobserved: runs break there and nothing is carried
//! across. A relation naming a country the links cannot resolve leaves the hour's
//! list incomplete, so that hour is unobserved too, though the runs of the
//! relations it could name go on.
//!
//! Prepared on the segment driver: each segment's projector ([`facts`]) reads the relations,
//! the subjects' levels and the countries where they change, and the branch's emit ([`emit`])
//! steps the subject scan ([`scan`]) through every hour and publishes its relations on the
//! family's series publication, which the reply and the event feed's subject items read
//! ([`query`]). The hourly scan of the recording's rows ([`row`]) is kept behind
//! `test-support` as the parity oracle.

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    prepared::series::LateIndex,
};
use dashboard_base::{
    products::ContentOptions,
    products::LoadContext,
    products::Product,
    products::country_groups::{HourRun, HourSpan},
};
use installed_content_core::autonomy::AutonomyStates;
use replay_data::rows::SubjectRow;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicBool;
use std::{collections::BTreeSet, io, sync::Arc};
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod scan;

pub struct Subjects;

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
    /// While preparation has not reached the branch's end, the hours after its
    /// prefix whose relations and levels come from each block's keyframe
    /// ([`Scan::sample_tail`]): exact at the keyframe, approximate over the
    /// block, so a relation's end inside them is approximate (its start stays
    /// the game's recorded start). None once the branch is prepared, and from
    /// the recording product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approximate: Option<Vec<HourSpan>>,
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

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let scan = row::scanned_branch(source, branch, cancelled)?;
        scan.reply(source, &params.branch_id, context)
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
        for state in states.ordered().placed {
            if named.contains(state.id.as_str()) {
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
    let (states, warnings) = installed_content_core::autonomy::read(&catalogue.content.layers)?;
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
    holds(row.cancel, row.start_hour, row.end_hour, hour)
}

pub fn holds(cancel: u8, start_hour: i32, end_hour: i32, hour: i32) -> bool {
    cancel == 0 && (end_hour <= start_hour || hour < end_hour)
}

/// The recorded relation date when both identities are known then; otherwise
/// only a new relation witnessed after a complete preceding hour is an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectStart {
    pub subject: String,
    pub overlord: String,
    pub hour: i32,
    pub read_hour: i32,
    pub exact: bool,
}

/// What a branch's subject history counted, which its warnings report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Recorded country references no link resolved.
    pub unmapped: u64,
    /// Hours whose subject or country tables were not complete.
    pub incomplete: u64,
    /// Subject-hours with no recorded level, and those whose level id is not text.
    pub unleveled: u64,
    pub unreadable: u64,
    /// Subjects recorded with two levels in one hour.
    pub conflicting: u64,
    /// Blocks and reads of the recording that could not be read.
    pub unreadable_blocks: u64,
    pub stopped: u64,
    /// Whether some hour's relations were read complete with resolved countries.
    pub observed: bool,
}

/// The warnings `counts` give a reply.
pub fn warnings(counts: &Counts) -> Vec<String> {
    let mut warnings = Vec::new();
    if counts.unmapped > 0 {
        warnings.push(format!(
            "{} recorded country references had no country identity; relations naming them are left out.",
            counts.unmapped
        ));
    }
    if counts.incomplete > 0 {
        warnings.push(format!(
            "Subject tables were incomplete at {} hours; relations break there rather than carry over.",
            counts.incomplete
        ));
    }
    if counts.unleveled > 0 {
        warnings.push(format!(
            "A subject's level was not recorded for {} subject-hours; those stretches show no level.",
            counts.unleveled
        ));
    }
    if counts.unreadable > 0 {
        warnings.push(format!(
            "A subject's level was recorded under an id that is not text for {} subject-hours; those stretches show no level.",
            counts.unreadable
        ));
    }
    if counts.conflicting > 0 {
        warnings.push(format!(
            "{} subjects were recorded with two levels in one hour; their level is not known then.",
            counts.conflicting
        ));
    }
    if counts.unreadable_blocks > 0 || counts.stopped > 0 {
        warnings.push(format!(
            "Part of this branch could not be read ({} unreadable blocks, {} stopped reads); subject history is missing there.",
            counts.unreadable_blocks, counts.stopped
        ));
    }
    warnings
}

/// The reply of a branch's `runs` (sorted) and `unobserved` hours, with the warnings its history
/// gave (`scanned`), its levels named and ordered by the content `context` reads.
pub fn reply(
    source: &Source,
    branch_id: &str,
    context: &LoadContext,
    runs: Vec<SubjectRun>,
    unobserved: Vec<HourSpan>,
    scanned: &[String],
) -> io::Result<SubjectReply> {
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
    warnings.extend(scanned.iter().cloned());
    let named: BTreeSet<&str> = runs
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
        branch_id: branch_id.to_owned(),
        levels,
        runs,
        unobserved,
        warnings,
        approximate: None,
    })
}

/// Runs in the reply's order: by first hour, then overlord, subject and start hour.
pub fn sort_runs(runs: &mut [SubjectRun]) {
    runs.sort_by(|a, b| {
        (a.first_hour, &a.overlord, &a.subject, a.start_hour).cmp(&(
            b.first_hour,
            &b.overlord,
            &b.subject,
            b.start_hour,
        ))
    });
}

/// What the event feed reads of a branch's subjects: whether some hour's relations were
/// observed, the history's warnings, and each relation start under the key and ordinal the feed
/// pages it by: those read at their date under 0, the later ones under [`Feed::LATE`], each in
/// the order they were published, with the late ones indexed by date.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Feed {
    pub observed: bool,
    pub warnings: Vec<String>,
    pub on_time: Vec<SubjectStart>,
    pub late_starts: Vec<SubjectStart>,
    pub late: LateIndex,
}

impl Feed {
    /// The key of the starts read after their date.
    pub const LATE: u64 = 1 << 63;

    /// The feed of `starts` in the order their hours read them (read hours ascending); each
    /// hour's are published by date, subject and overlord.
    pub fn new(mut starts: Vec<SubjectStart>, observed: bool, warnings: Vec<String>) -> Self {
        for hour in starts.chunk_by_mut(|left, right| left.read_hour == right.read_hour) {
            hour.sort_by(|a, b| {
                (a.hour, &a.subject, &a.overlord).cmp(&(b.hour, &b.subject, &b.overlord))
            });
        }
        let mut feed = Self {
            observed,
            warnings,
            ..Self::default()
        };
        for start in starts {
            if start.read_hour > start.hour {
                let ordinal = feed.late_starts.len() as u64;
                feed.late
                    .insert(start.hour, start.read_hour, Self::LATE, ordinal);
                feed.late_starts.push(start);
            } else {
                feed.on_time.push(start);
            }
        }
        feed
    }

    /// The starts paged under `key`, by ordinal.
    pub fn keyed(&self, key: u64) -> &[SubjectStart] {
        match key {
            0 => &self.on_time,
            Self::LATE => &self.late_starts,
            _ => &[],
        }
    }

    pub fn bytes(&self) -> usize {
        let start = |start: &SubjectStart| {
            std::mem::size_of::<SubjectStart>() + start.subject.capacity() + start.overlord.capacity()
        };
        std::mem::size_of::<Self>()
            + self.on_time.iter().chain(&self.late_starts).map(start).sum::<usize>()
            + self.warnings.iter().map(String::capacity).sum::<usize>()
            + self.late_starts.len() * 2 * std::mem::size_of::<(i32, i32, u64, u64)>()
    }
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
