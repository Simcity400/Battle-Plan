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

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
};
use dashboard_base::{
    products::ContentOptions,
    products::LoadContext,
    products::Product,
    products::country_groups::{HourRun, HourSpan, extend, mark_unobserved},
};
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
use dashboard_text::metadata::{LinkTags, branch_bounds};
use installed_content_core::autonomy::AutonomyStates;
use replay::container::Replay;
use replay_data::{rows::SubjectRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::Ordering;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct Subjects;

/// Branches kept per source, so tracks on different branches do not evict each other.
#[cfg(any(test, feature = "test-support"))]
pub const CACHED_BRANCHES: usize = 4;

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
        let scan = scanned_branch(source, branch, cancelled)?;
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

/// `value` after `hours` additions of `rate`, each made as `+=` makes it: a
/// sum that cannot overflow at once, otherwise one addition at a time, so an
/// overflow behaves exactly as it does hour by hour.
fn repeated(value: u32, rate: u32, hours: u64) -> u32 {
    u32::try_from(hours)
        .ok()
        .and_then(|hours| rate.checked_mul(hours))
        .and_then(|added| value.checked_add(added))
        .unwrap_or_else(|| (0..hours).fold(value, |value, _| value + rate))
}

#[cfg(any(test, feature = "test-support"))]
pub fn scanned_branch(
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
type RelationStart = ((u32, u32), (u32, u32), i32);
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Scan {
    pub(crate) runs: Vec<SubjectRun>,
    pub(crate) unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
    pub starts: Vec<SubjectStart>,
    pub observed: bool,
    #[serde(with = "dashboard_core::map_pairs")]
    open: BTreeMap<(String, String, i32), usize>,
    tags: LinkTags,
    unmapped: BTreeSet<u32>,
    incomplete: u32,
    unleveled: u32,
    unreadable: u32,
    conflicting: BTreeSet<String>,
    unreadable_blocks: u32,
    stopped: u32,
    seen_starts: BTreeSet<RelationStart>,
    named_since: BTreeMap<u32, (String, i32)>,
    continuous_since: Option<i32>,
    first_observed: Option<i32>,
    previous_complete: Option<i32>,
    previous_identities: BTreeMap<u32, u32>,
    identities: BTreeMap<u32, u32>,
    identities_changed: bool,
    identity_since: BTreeMap<u32, (u32, i32)>,
    previous_hour: Option<i32>,
    carried: bool,
    diagnostic_rate: (u32, u32, u32),
    unobserved_here: bool,
    next_expiry: Option<i32>,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    level_inputs: Option<Vec<(u32, Vec<u8>)>>,
    #[serde(skip)]
    relation_inputs: Option<Arc<Vec<RelationOperands>>>,
    #[serde(skip)]
    resolved_levels: Option<Arc<HashMap<String, Option<Vec<u8>>>>>,
    #[serde(skip)]
    link_inputs: HashMap<u32, u32>,
    #[serde(skip)]
    links_known: bool,
    #[serde(skip)]
    relations_by_tag: BTreeMap<String, Vec<usize>>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

/// Current relation operands only; levels and holder identities rebind separately.
#[derive(Clone)]
struct RelationOperands {
    overlord: u32,
    subject: u32,
    start_hour: i32,
    end_hour: i32,
    cancel: u8,
}
impl RelationOperands {
    fn active(&self, hour: i32) -> bool {
        holds(self.cancel, self.start_hour, self.end_hour, hour)
    }
}

/// The recorded relation date when both identities are known then; otherwise
/// only a new relation witnessed after a complete preceding hour is an event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubjectStart {
    pub subject: String,
    pub overlord: String,
    pub hour: i32,
    pub read_hour: i32,
    pub exact: bool,
}

struct RelationSample {
    overlord: String,
    subject: String,
    start_hour: i32,
    level: Option<String>,
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

impl Scan {
    pub fn prepare_changes_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
        events: &mut dashboard_base::prepared::series::Accumulator,
        late: &mut dashboard_base::prepared::series::LateIndex,
    ) -> io::Result<()> {
        let countries_changed = changes.rows_changed[TableId::Countries.index()]
            && self.country_keys.as_ref().is_none_or(|keys| {
                frame
                    .complete::<tables::Countries>()
                    .map_or(true, |rows| !keys.same(rows))
            });
        let mut levels_changed = false;
        if changes.rows_changed[TableId::Autonomy.index()] {
            let levels = frame.complete::<tables::Autonomy>().ok().map(|rows| {
                #[cfg(test)]
                {
                    self.work.row_visits += rows.len();
                }
                rows.iter()
                    .map(|row| (row.subject, row.state.clone()))
                    .collect::<Vec<_>>()
            });
            levels_changed = self.level_inputs != levels;
            self.level_inputs = levels;
        }
        let changed = changes.rows_changed[TableId::Subjects.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
            || countries_changed
            || levels_changed
            || self.next_expiry.is_some_and(|hour| hour <= frame.hour)
            || self.previous_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
        if !changed {
            self.advance_unchanged(frame.hour, frame.hour, events);
            return Ok(());
        }
        let identities_changed = countries_changed
            || changes.rows_changed[TableId::CountryLinks.index()]
            || self.country_keys.is_none();
        if identities_changed {
            self.refresh_holders(
                frame,
                changes.rows_changed[TableId::Countries.index()],
                changes.rows_changed[TableId::CountryLinks.index()],
            );
        }
        let expiry = self.next_expiry.is_some_and(|hour| hour <= frame.hour);
        let relations_changed =
            changes.rows_changed[TableId::Subjects.index()] || self.relation_inputs.is_none();
        let partial = !identities_changed
            && !relations_changed
            && !expiry
            && self.previous_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        if !partial {
            self.seal();
        }
        let before = (self.incomplete, self.unleveled, self.unreadable);
        let first = self.starts.len();
        self.observe_inputs(
            frame,
            identities_changed,
            relations_changed,
            levels_changed || identities_changed,
            partial,
        )?;
        self.publish_starts(first, frame.hour, events, late)?;
        self.diagnostic_rate = (
            self.incomplete - before.0,
            self.unleveled - before.1,
            self.unreadable - before.2,
        );
        self.carried = self.diagnostic_rate.0 == 0;
        self.unobserved_here = self
            .unobserved
            .last()
            .is_some_and(|span| span.last_hour == frame.hour);
        if relations_changed || expiry {
            self.next_expiry = self.relation_inputs.as_ref().and_then(|rows| {
                rows.iter()
                    .filter(|row| {
                        row.cancel == 0
                            && row.end_hour > row.start_hour
                            && row.end_hour > frame.hour
                    })
                    .map(|row| row.end_hour)
                    .min()
            });
        }
        Ok(())
    }
    /// The step of an hour whose relations, levels, holders and continuity
    /// are the hour before's and at which no relation ends, taken for every
    /// hour `first..=last` at once: each hour adds the diagnostic rates of the
    /// last observed hour (as that many `+=` would, overflow included), an
    /// unobserved hour stays unobserved, and the frontier advances. Open runs
    /// are sealed later, at a change, a gap, publication or a read.
    fn advance_unchanged(
        &mut self,
        first: i32,
        last: i32,
        events: &mut dashboard_base::prepared::series::Accumulator,
    ) {
        let hours = u64::try_from(i64::from(last) - i64::from(first) + 1).unwrap_or(0);
        self.incomplete = repeated(self.incomplete, self.diagnostic_rate.0, hours);
        self.unleveled = repeated(self.unleveled, self.diagnostic_rate.1, hours);
        self.unreadable = repeated(self.unreadable, self.diagnostic_rate.2, hours);
        if self.unobserved_here {
            mark_unobserved(&mut self.unobserved, first, last);
        }
        self.previous_hour = Some(last);
        if self.previous_complete.is_some() {
            self.previous_complete = Some(last);
        }
        events.through(first);
        events.through(last);
    }
    /// Hours `first..=last` of a block read by its changes, at which none of
    /// the fields this scan reads (relation keys, starts, ends and cancel
    /// bytes, autonomy states, country keys and links) changed: the unchanged
    /// step for each, at once, when they continue the last observed hour and
    /// no relation ends among them. False, with nothing changed, otherwise:
    /// the caller then observes them on their frames.
    pub fn advance_quiet(
        &mut self,
        first: i32,
        last: i32,
        events: &mut dashboard_base::prepared::series::Accumulator,
    ) -> bool {
        if last < first
            || self.previous_hour.and_then(|hour| hour.checked_add(1)) != Some(first)
            || self.next_expiry.is_some_and(|hour| hour <= last)
        {
            return false;
        }
        self.advance_unchanged(first, last, events);
        true
    }
    pub fn seal(&mut self) {
        if !self.carried {
            return;
        }
        let Some(hour) = self.previous_hour else {
            return;
        };
        for &index in self.open.values() {
            let run = &mut self.runs[index];
            if run.last_hour < hour {
                run.last_hour = hour;
                if let Some(step) = run.steps.last_mut() {
                    step.last_hour = hour;
                }
            }
        }
    }
    /// Publish the very same witnessed starts retained by the subject scan.
    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare_changes(
        &mut self,
        frame: &DecodedFrame,
        events: &mut dashboard_base::prepared::series::Accumulator,
        late: &mut dashboard_base::prepared::series::LateIndex,
    ) -> io::Result<()> {
        let first = self.starts.len();
        self.observe(frame)?;
        self.publish_starts(first, frame.hour, events, late)
    }
    fn publish_starts(
        &self,
        first: usize,
        hour: i32,
        events: &mut dashboard_base::prepared::series::Accumulator,
        late: &mut dashboard_base::prepared::series::LateIndex,
    ) -> io::Result<()> {
        events.through(hour);
        let mut starts: Vec<_> = self.starts[first..].iter().collect();
        starts.sort_by(|a, b| {
            (a.hour, &a.subject, &a.overlord).cmp(&(b.hour, &b.subject, &b.overlord))
        });
        for event in starts {
            let key = u64::from(event.read_hour > event.hour) << 63;
            let ordinal = events.state(key).map_or(0, |state| state.events);
            events.event(key, event.read_hour, event)?;
            late.insert(event.hour, event.read_hour, key, ordinal);
        }
        Ok(())
    }
    pub fn new(replay: &Replay, branch: u64) -> Self {
        Self {
            previous_hour: replay
                .branches
                .iter()
                .find(|start| start.id == branch)
                .and_then(|start| branch_bounds(replay, start).0.checked_sub(1)),
            ..Self::default()
        }
    }
    pub(crate) fn observe(&mut self, decoded: &DecodedFrame) -> io::Result<()> {
        self.refresh_holders(decoded, true, true);
        self.level_inputs = decoded.complete::<tables::Autonomy>().ok().map(|rows| {
            rows.iter()
                .map(|row| (row.subject, row.state.clone()))
                .collect()
        });
        self.observe_inputs(decoded, true, true, true, false)
    }
    fn refresh_holders(&mut self, frame: &DecodedFrame, countries: bool, links: bool) {
        if countries {
            self.country_keys = frame
                .complete::<tables::Countries>()
                .ok()
                .map(dashboard_text::metadata::CountryKeys::read);
            #[cfg(test)]
            {
                self.work.row_visits += frame
                    .complete::<tables::Countries>()
                    .map_or(0, |rows| rows.len());
            }
        }
        if links {
            let inputs = frame.complete::<tables::CountryLinks>();
            self.links_known = inputs.is_ok();
            self.link_inputs = inputs
                .map(dashboard_text::metadata::link_slots)
                .unwrap_or_default();
            #[cfg(test)]
            {
                self.work.row_visits += self.link_inputs.len();
            }
        }
    }
    fn observe_inputs(
        &mut self,
        decoded: &DecodedFrame,
        identities_changed: bool,
        relations_changed: bool,
        levels_changed: bool,
        partial: bool,
    ) -> io::Result<()> {
        let hour = decoded.hour;
        let old_frontier = self.previous_hour;
        let old_levels = self.resolved_levels.clone();
        let old_complete = self.previous_complete.is_some();
        let first = *self.first_observed.get_or_insert(hour);
        if let Some(previous) = self.previous_hour
            && hour > previous + 1
        {
            mark_unobserved(&mut self.unobserved, previous + 1, hour - 1);
            self.continuous_since = None;
        }
        self.previous_hour = Some(hour);
        let updated = if identities_changed {
            self.tags.update_projected(
                self.country_keys.as_ref(),
                self.links_known.then_some(&self.link_inputs),
            )
        } else {
            decoded
                .complete::<tables::Countries>()
                .and_then(|_| decoded.complete::<tables::CountryLinks>())
                .map(|_| false)
        };
        let Ok(rebuilt) = updated else {
            self.incomplete += 1;
            mark_unobserved(&mut self.unobserved, hour, hour);
            self.continuous_since = None;
            self.previous_complete = None;
            return Ok(());
        };
        if rebuilt || identities_changed {
            self.identities_changed = true;
            self.identities = self
                .country_keys
                .as_ref()
                .expect("known country operands")
                .iter()
                .map(|(id, slot, _)| (slot, id))
                .collect();
            self.identity_since
                .retain(|slot, (id, _)| self.identities.get(slot) == Some(id));
            for (&slot, &id) in &self.identities {
                self.identity_since.entry(slot).or_insert((id, hour));
            }
            self.named_since
                .retain(|raw, (tag, _)| self.tags.get(*raw) == Some(tag));
            for (raw, tag) in self.tags.iter() {
                self.named_since
                    .entry(*raw)
                    .or_insert_with(|| (tag.clone(), hour));
            }
        }
        if relations_changed {
            self.relation_inputs = decoded.complete::<tables::Subjects>().ok().map(|rows| {
                #[cfg(test)]
                {
                    self.work.row_visits += rows.len();
                }
                Arc::new(
                    rows.iter()
                        .map(|row| RelationOperands {
                            overlord: row.overlord,
                            subject: row.subject,
                            start_hour: row.start_hour,
                            end_hour: row.end_hour,
                            cancel: row.cancel,
                        })
                        .collect(),
                )
            });
        }
        if relations_changed || identities_changed {
            self.relations_by_tag.clear();
            if let Some(rows) = &self.relation_inputs {
                for (index, row) in rows.iter().enumerate() {
                    if let Some(tag) = self.tags.get(row.subject) {
                        self.relations_by_tag
                            .entry(tag.clone())
                            .or_default()
                            .push(index);
                    }
                }
            }
        }
        let Some(rows) = self.relation_inputs.clone() else {
            self.incomplete += 1;
            mark_unobserved(&mut self.unobserved, hour, hour);
            self.continuous_since = None;
            self.previous_complete = None;
            return Ok(());
        };
        self.observed = true;
        let since = *self.continuous_since.get_or_insert(hour);
        // Each subject's current level this hour, by tag; none when the table is not
        // complete. A subject the rows give two levels has no known level.
        if levels_changed {
            self.resolved_levels = self.level_inputs.as_ref().map(|autonomy| {
                let mut levels: HashMap<String, Option<Vec<u8>>> = HashMap::new();
                for (subject, state) in autonomy {
                    let Some(tag) = self.tags.get(*subject) else {
                        continue;
                    };
                    let level = levels
                        .entry(tag.clone())
                        .or_insert_with(|| Some(state.clone()));
                    if level.as_deref() != Some(state.as_slice()) {
                        self.conflicting.insert(tag.clone());
                        *level = None;
                    }
                }
                Arc::new(levels)
            });
        }
        let levels = self.resolved_levels.clone();
        let selected: Vec<usize> = if partial {
            let tags: BTreeSet<_> = old_levels
                .as_ref()
                .into_iter()
                .flat_map(|levels| levels.keys())
                .chain(levels.as_ref().into_iter().flat_map(|levels| levels.keys()))
                .collect();
            let changed: Vec<_> = tags
                .into_iter()
                .filter(|tag| {
                    old_levels
                        .as_ref()
                        .and_then(|levels| levels.get(*tag))
                        .and_then(|value| value.as_deref())
                        != levels
                            .as_ref()
                            .and_then(|levels| levels.get(*tag))
                            .and_then(|value| value.as_deref())
                })
                .collect();
            changed
                .into_iter()
                .flat_map(|tag| {
                    self.relations_by_tag
                        .get(tag)
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect()
        } else {
            (0..rows.len()).collect()
        };
        if partial {
            self.incomplete += self.diagnostic_rate.0;
            self.unleveled += self.diagnostic_rate.1;
            self.unreadable += self.diagnostic_rate.2;
        }
        let mut named_all = !partial || old_complete;
        for row in selected
            .into_iter()
            .map(|index| &rows[index])
            .filter(|row| row.active(hour))
        {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            if partial
                && let (Some(overlord), Some(subject)) =
                    (self.tags.get(row.overlord), self.tags.get(row.subject))
            {
                let previous = old_levels
                    .as_ref()
                    .and_then(|levels| levels.get(subject))
                    .and_then(|level| level.as_deref());
                let unreadable = previous
                    .is_some_and(|bytes| std::str::from_utf8(bytes).ok().is_none_or(str::is_empty));
                self.unleveled -= u32::from(previous.is_none() || unreadable);
                self.unreadable -= u32::from(unreadable);
                if self.carried
                    && let Some(frontier) = old_frontier
                    && let Some(&index) =
                        self.open
                            .get(&(overlord.clone(), subject.clone(), row.start_hour))
                {
                    let run = &mut self.runs[index];
                    run.last_hour = frontier;
                    if let Some(step) = run.steps.last_mut() {
                        step.last_hour = frontier;
                    }
                }
            }
            let (Some(overlord), Some(subject)) =
                (self.tags.get(row.overlord), self.tags.get(row.subject))
            else {
                for raw in [row.overlord, row.subject] {
                    if self.tags.get(raw).is_none() {
                        self.unmapped.insert(raw);
                    }
                }
                named_all = false;
                continue;
            };
            // A level whose recorded id is not text (or is empty) is refused, and counted.
            let level = match levels
                .as_ref()
                .and_then(|levels| levels.get(subject).and_then(|level| level.as_deref()))
            {
                Some(bytes) => match std::str::from_utf8(bytes) {
                    Ok(token) if !token.is_empty() => Some(token.to_owned()),
                    _ => {
                        self.unreadable += 1;
                        None
                    }
                },
                None => None,
            };
            if level.is_none() {
                self.unleveled += 1;
            }
            let key = (overlord.clone(), subject.clone(), row.start_hour);
            let started = extend(&mut self.open, &mut self.runs, key.clone(), hour, || {
                SubjectRun {
                    overlord: overlord.clone(),
                    subject: subject.clone(),
                    start_hour: row.start_hour,
                    first_hour: hour,
                    last_hour: hour,
                    steps: Vec::new(),
                }
            });
            if started {
                let identity_key = [row.overlord, row.subject].map(|raw| {
                    self.link_inputs
                        .get(&raw)
                        .copied()
                        .and_then(|slot| self.identities.get(&slot).map(|id| (slot, *id)))
                });
                let identity_key = identity_key[0]
                    .zip(identity_key[1])
                    .map(|(overlord, subject)| (overlord, subject, row.start_hour));
                let new_identity =
                    identity_key.is_some_and(|identity| self.seen_starts.insert(identity));
                if new_identity && row.start_hour > first {
                    // Relations serialize start_date; names are exact only while both raw
                    // links keep their tags. A start at or before the first observed frame is
                    // an initial snapshot, even if first published later.
                    let date_named = [row.overlord, row.subject].into_iter().all(|raw| {
                        self.named_since
                            .get(&raw)
                            .is_some_and(|(_, from)| since.max(*from) <= row.start_hour)
                    }) && identity_key.is_some_and(|(overlord, subject, _)| {
                        [overlord, subject].into_iter().all(|(slot, id)| {
                            self.identity_since
                                .get(&slot)
                                .is_some_and(|(seen, from)| *seen == id && *from <= row.start_hour)
                        })
                    });
                    let exact = date_named && row.start_hour <= hour;
                    let witnessed = self
                        .previous_complete
                        .is_some_and(|previous: i32| previous.checked_add(1) == Some(hour))
                        && identity_key.is_some_and(|(overlord, subject, _)| {
                            [overlord, subject]
                                .into_iter()
                                .all(|(slot, id)| self.previous_identities.get(&slot) == Some(&id))
                        });
                    if exact || witnessed {
                        self.starts.push(SubjectStart {
                            subject: subject.clone(),
                            overlord: overlord.clone(),
                            hour: if exact { row.start_hour } else { hour },
                            read_hour: hour,
                            exact,
                        });
                    }
                }
            }
            let run = &mut self.runs[self.open[&key]];
            match run.steps.last_mut() {
                Some(step) if step.last_hour == hour => {}
                Some(step) if step.level == level && step.last_hour + 1 == hour && !started => {
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
            mark_unobserved(&mut self.unobserved, hour, hour);
        }
        self.previous_complete = named_all.then_some(hour);
        if self.identities_changed {
            self.previous_identities.clone_from(&self.identities);
            self.identities_changed = false;
        }
        if !partial {
            self.open
                .retain(|_, index| self.runs[*index].last_hour == hour);
        }
        Ok(())
    }
    pub fn finish(&self, replay: &Replay, branch: u64) -> Self {
        let mut projection = self.clone();
        projection.seal();
        projection.carried = false;
        projection.settle(replay, branch);
        projection
    }
    fn settle(&mut self, replay: &Replay, branch: u64) {
        let bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        if let (Some((first, last)), previous) = (bounds, self.previous_hour) {
            let from = previous.map_or(first, |previous| previous.saturating_add(1));
            if from <= last {
                mark_unobserved(&mut self.unobserved, from, last);
            }
        }
        self.runs.sort_by(|a, b| {
            (a.first_hour, &a.overlord, &a.subject, a.start_hour).cmp(&(
                b.first_hour,
                &b.overlord,
                &b.subject,
                b.start_hour,
            ))
        });
        self.warnings = self.change_warnings();
    }
    pub fn change_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.unmapped.is_empty() {
            warnings.push(format!(
            "{} recorded country references had no country identity; relations naming them are left out.",
            self.unmapped.len()
        ));
        }
        if self.incomplete > 0 {
            warnings.push(format!(
            "Subject tables were incomplete at {} hours; relations break there rather than carry over.", self.incomplete
        ));
        }
        if self.unleveled > 0 {
            warnings.push(format!(
            "A subject's level was not recorded for {} subject-hours; those stretches show no level.", self.unleveled
        ));
        }
        if self.unreadable > 0 {
            warnings.push(format!(
            "A subject's level was recorded under an id that is not text for {} subject-hours; those stretches show no level.", self.unreadable
        ));
        }
        if !self.conflicting.is_empty() {
            warnings.push(format!(
            "{} subjects were recorded with two levels in one hour; their level is not known then.",
            self.conflicting.len()
        ));
        }
        if self.unreadable_blocks > 0 || self.stopped > 0 {
            warnings.push(format!(
            "Part of this branch could not be read ({} unreadable blocks, {} stopped reads); subject history is missing there.", self.unreadable_blocks, self.stopped
        ));
        }

        warnings
    }
    pub fn reply(
        &self,
        source: &Source,
        branch_id: &str,
        context: &LoadContext,
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
        warnings.extend(self.warnings.iter().cloned());
        let named: BTreeSet<&str> = self
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
            branch_id: branch_id.to_owned(),
            levels,
            runs: {
                let mut runs = self.runs.clone();
                if self.carried
                    && let Some(hour) = self.previous_hour
                {
                    for &index in self.open.values() {
                        runs[index].last_hour = hour;
                        if let Some(step) = runs[index].steps.last_mut() {
                            step.last_hour = hour;
                        }
                    }
                }
                runs
            },
            unobserved: self.unobserved.clone(),
            warnings,
            approximate: None,
        })
    }

    /// The relations of the hours after `after` (a prepared prefix's last
    /// hour) sampled at each later block's keyframe, joined onto this scan's
    /// runs: each relation active at a keyframe, with the subject's level
    /// there, stands for the block's hours after the prefix. A relation
    /// reaching the prefix's end, or the hour before a sampled block, goes on
    /// when the keyframe still holds it with the same start; runs break at
    /// recorded gaps. A keyframe whose relations this scan would not observe
    /// (an incomplete Subjects, Countries or CountryLinks table, or a
    /// relation naming an unresolved country) samples nothing, so its block
    /// stays pending. Returns the sampled hours: the reply's approximate spans.
    pub fn sample_tail(
        &mut self,
        source: &Source,
        branch: u64,
        after: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<HourSpan>> {
        type Key = (String, String, i32);
        let key = |run: &SubjectRun| (run.overlord.clone(), run.subject.clone(), run.start_hour);
        // The run each relation continues, by the last hour it reaches.
        let mut open: BTreeMap<Key, usize> = self
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.last_hour == after)
            .map(|(index, run)| (key(run), index))
            .collect();
        let runs = &mut self.runs;
        let approximate = dashboard_base::prepared::query::keyframes::after(
            source,
            "subjects:keyframe",
            branch,
            after,
            None,
            prepared::TABLES,
            cancelled,
            &Self::at_keyframe,
            &|runs: &Vec<RelationSample>| {
                std::mem::size_of::<Vec<RelationSample>>()
                    + runs.capacity() * std::mem::size_of::<RelationSample>()
                    + runs
                        .iter()
                        .map(|run| {
                            run.overlord.capacity()
                                + run.subject.capacity()
                                + run.level.as_ref().map_or(0, String::capacity)
                        })
                        .sum::<usize>()
            },
            |spans, sampled| {
                let mut next = BTreeMap::new();
                for run in sampled {
                    let relation = (run.overlord.clone(), run.subject.clone(), run.start_hour);
                    let level = run.level.clone();
                    let mut current = open.get(&relation).copied();
                    for span in spans {
                        let index = match current {
                            Some(index)
                                if runs[index].last_hour.checked_add(1)
                                    == Some(span.first_hour) =>
                            {
                                index
                            }
                            _ => {
                                runs.push(SubjectRun {
                                    overlord: run.overlord.clone(),
                                    subject: run.subject.clone(),
                                    start_hour: run.start_hour,
                                    first_hour: span.first_hour,
                                    last_hour: span.first_hour,
                                    steps: Vec::new(),
                                });
                                runs.len() - 1
                            }
                        };
                        let joined = &mut runs[index];
                        match joined.steps.last_mut() {
                            Some(step)
                                if step.level == level
                                    && step.last_hour.checked_add(1) == Some(span.first_hour) =>
                            {
                                step.last_hour = span.last_hour;
                            }
                            _ => joined.steps.push(LevelStep {
                                first_hour: span.first_hour,
                                last_hour: span.last_hour,
                                level: level.clone(),
                            }),
                        }
                        joined.last_hour = span.last_hour;
                        current = Some(index);
                    }
                    if let Some(index) = current {
                        next.insert(relation, index);
                    }
                }
                open = next;
                Ok(())
            },
        )?;
        self.runs.sort_by(|a, b| {
            (a.first_hour, &a.overlord, &a.subject, a.start_hour).cmp(&(
                b.first_hour,
                &b.overlord,
                &b.subject,
                b.start_hour,
            ))
        });
        Ok(approximate)
    }

    /// The relations a keyframe holds as this scan observes an hour alone:
    /// each active relation's one-hour run, with the subject's level at the
    /// hour. None when the scan would leave the hour unobserved.
    fn at_keyframe(frame: &DecodedFrame) -> io::Result<Option<Vec<RelationSample>>> {
        let mut alone = Scan {
            previous_hour: frame.hour.checked_sub(1),
            ..Scan::default()
        };
        alone.observe(frame)?;
        let observed = !alone
            .unobserved
            .iter()
            .any(|span| span.first_hour <= frame.hour && frame.hour <= span.last_hour);
        Ok(observed.then(|| {
            let mut samples: Vec<_> = alone
                .runs
                .into_iter()
                .map(|run| RelationSample {
                    overlord: run.overlord,
                    subject: run.subject,
                    start_hour: run.start_hour,
                    level: run.steps.into_iter().next().and_then(|step| step.level),
                })
                .collect();
            samples.shrink_to_fit();
            samples
        }))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn scan_branch(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Subjects,
        TableId::Autonomy,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Subjects request cancelled");
    let mut scan = Scan::new(replay, branch);
    dashboard_core::blocks::scan_blocks(
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
                    scan.unreadable_blocks += 1;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        scan.stopped += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        scan.stopped += 1;
                        break;
                    }
                    scan.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.finish(replay, branch))
}

pub mod prepared;

#[cfg(test)]
mod tests;
