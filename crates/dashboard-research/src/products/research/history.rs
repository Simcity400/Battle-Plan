use super::{AssignmentCells, OBSERVATION_NOTE, Presentation, ResearchCoverage, ResearchDefinitionText, ResearchEntry, ResearchOutcome, all_columns, assignment_keys, content, coverage, coverage_keys, last_read, presentation, spans};
use dashboard_base::{ResearchFolder, ResearchTechnologyFolder, Source};
use dashboard_base::products::{Country, LoadContext, Product, check_reply_budget, validate_country};
use dashboard_core::dto::Artwork;
use dashboard_query::{Query, cost, identity::Holder};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::TableId;
use replay_lanes::lanes::{self, History, Keys};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchHistoryParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    #[serde(default)]
    pub folders: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BoundKind {
    Exact,
    By,
    HeldBefore,
    LastSeen,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchBound {
    pub kind: BoundKind,
    pub hour: i32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchOrigin {
    pub slot: u32,
    pub start: ResearchBound,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchItem {
    pub token: String,
    pub database_index: u32,
    pub first_observed_hour: i32,
    pub outcome: ResearchOutcome,
    pub pieces: Vec<ResearchCoverage>,
    pub start: ResearchBound,
    pub end: Option<ResearchBound>,
    pub origin: Option<ResearchOrigin>,
    pub took_days: Option<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotAt {
    pub index: u32,
    pub availability: Vec<ResearchCoverage>,
    pub items: Vec<ResearchItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchHistoryReply {
    pub hour: i32,
    pub next_change: Option<i32>,
    pub unrecorded: Option<String>,
    pub partial: Option<String>,
    pub completed: u32,
    pub coverage: Vec<ResearchCoverage>,
    pub slots: Vec<ResearchSlotAt>,
    pub artwork: Vec<Artwork>,
    pub folders: Vec<ResearchFolder>,
    pub technology_folders: Vec<ResearchTechnologyFolder>,
    pub definitions: Vec<ResearchDefinitionText>,
    pub unnamed: String,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ResearchHistoryReply);

pub(crate) const NOT_RECORDED: &str = "Research was not recorded by this date";
const BEFORE_BRANCH: &str = "This date is before the branch begins";
pub(crate) const UNREAD: &str = "Research was not read for part of this campaign, so more technologies may have been completed";
const NO_FOLDERS: &str = "Research folders are unavailable, so technologies cannot be counted by folder";
const UNLOCALISED: &str = "The recorded localisation has no name for this technology";

pub(crate) type TechKey = (String, u32, u32, i32, i32);

pub(crate) struct Assignment {
    pub slot: u32,
    pub revisions: Vec<(i32, ResearchEntry)>,
    pub stretch_last: i32,
    pub last_read: Option<i32>,
}

impl Assignment {
    fn at(&self, hour: i32) -> Option<&ResearchEntry> {
        let at = self.revisions.partition_point(|(step, _)| *step <= hour);
        self.revisions[..at].last().map(|(_, entry)| entry).filter(|entry| entry.first_observed_hour <= hour)
    }
}

#[derive(Default)]
pub(crate) struct Index {
    pub runs: Vec<(i32, i32)>,
    pub gaps: Vec<(i32, i32)>,
    pub coverage: Vec<ResearchCoverage>,
    pub availability: BTreeMap<u32, Vec<ResearchCoverage>>,
    pub assignments: Vec<Assignment>,
    pub unreadable: Vec<i32>,
    pub unresolved: Vec<(i32, i32)>,
    pub boundaries: Vec<i32>,
    pub journeys: HashMap<TechKey, Vec<usize>>,
}

pub(crate) fn revisions(history: &History, unreadable: &mut Vec<i32>) -> io::Result<BTreeMap<(u32, i32, i32), Vec<(i32, ResearchEntry)>>> {
    let cells = AssignmentCells::of(history)?;
    let mut held: BTreeMap<(u32, i32, i32), Vec<(i32, ResearchEntry)>> = BTreeMap::new();
    for (key, steps) in &history.keys {
        let index = (*key >> 16) as u32;
        for step in steps {
            let Some(row) = step.cells.as_deref() else { continue };
            let Some(entry) = cells.entry(history, row) else {
                unreadable.push(step.hour);
                continue;
            };
            held.entry((index, entry.first_observed_hour, entry.continuity_start_hour)).or_default().push((step.hour, entry));
        }
    }
    for versions in held.values_mut() {
        versions.sort_by_key(|(hour, _)| *hour);
        let mut kept: Vec<(i32, ResearchEntry)> = Vec::with_capacity(versions.len());
        for (hour, entry) in versions.drain(..) {
            match kept.last_mut() {
                Some(last) if last.0 == hour => last.1 = entry,
                Some(last) if last.1 == entry => {}
                _ => kept.push((hour, entry)),
            }
        }
        *versions = kept;
    }
    Ok(held)
}

impl Index {
    pub(crate) fn finish(mut self) -> Self {
        self.assignments.sort_by_key(|assignment| (assignment.slot, assignment.revisions[0].1.first_observed_hour));
        self.unreadable.sort_unstable();
        let mut hours = Vec::new();
        let mut edges = |spans: &[ResearchCoverage]| {
            for span in spans {
                hours.push(span.first_hour);
                hours.push(span.last_hour.saturating_add(1));
            }
        };
        edges(&self.coverage);
        for spans in self.availability.values() {
            edges(spans);
        }
        for &(first, last) in self.runs.iter().chain(&self.unresolved).chain(&self.gaps) {
            hours.push(first);
            hours.push(last.saturating_add(1));
        }
        for (at, assignment) in self.assignments.iter().enumerate() {
            hours.extend(assignment.revisions.iter().map(|(hour, entry)| (*hour).max(entry.first_observed_hour)));
            hours.push(assignment.stretch_last.saturating_add(1));
            let first = &assignment.revisions[0].1;
            let key = (first.token.clone(), first.database_index, first.technology_handle, first.level, first.continuity_start_hour);
            self.journeys.entry(key).or_default().push(at);
        }
        hours.extend(&self.unreadable);
        hours.sort_unstable();
        hours.dedup();
        self.boundaries = hours;
        for journey in self.journeys.values_mut() {
            journey.sort_by_key(|&at| (self.assignments[at].revisions[0].1.first_observed_hour, self.assignments[at].slot));
        }
        self
    }

    fn next_change(&self, hour: i32) -> Option<i32> {
        self.boundaries.get(self.boundaries.partition_point(|&at| at <= hour)).copied()
    }

    fn entry_at(&self, at: usize, hour: i32) -> Option<ResearchEntry> {
        let assignment = &self.assignments[at];
        let mut entry = assignment.at(hour)?.clone();
        if entry.outcome == ResearchOutcome::Active && hour > assignment.stretch_last {
            entry.outcome = ResearchOutcome::Unknown;
            entry.last_observed_hour = assignment.last_read.unwrap_or(entry.last_observed_hour).max(entry.first_observed_hour);
            entry.ended_by_hour = None;
        }
        Some(entry)
    }

    fn origin(&self, at: usize, entry: &ResearchEntry, hour: i32, coverage: &[ResearchCoverage]) -> Option<ResearchOrigin> {
        let key = (entry.token.clone(), entry.database_index, entry.technology_handle, entry.level, entry.continuity_start_hour);
        let journey = self.journeys.get(&key)?;
        let mut place = journey.iter().position(|&held| held == at)?;
        let mut after = entry.clone();
        while place > 0 {
            let before_at = journey[place - 1];
            let Some(before) = self.entry_at(before_at, hour) else { break };
            if before.outcome != ResearchOutcome::Switched || before.ended_by_hour.is_none_or(|end| end > after.first_observed_hour) {
                break;
            }
            place -= 1;
            after = before;
        }
        (journey[place] != at).then(|| ResearchOrigin { slot: self.assignments[journey[place]].slot, start: start_bound(&after, coverage) })
    }

    fn unread_through(&self, hour: i32, coverage: &[ResearchCoverage]) -> bool {
        let held: i64 = self
            .runs
            .iter()
            .map(|&(first, last)| (first, last.min(hour)))
            .filter(|(first, last)| first <= last)
            .map(|(first, last)| i64::from(last) - i64::from(first) + 1)
            .sum();
        let read: i64 = coverage.iter().map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1).sum();
        read < held || self.gaps.iter().any(|(first, _)| *first <= hour)
    }
}

fn day(hour: i32) -> i32 {
    hour.div_euclid(24)
}

fn start_bound(entry: &ResearchEntry, coverage: &[ResearchCoverage]) -> ResearchBound {
    let hour = entry.first_observed_hour;
    let kind = match entry.started_after_hour {
        Some(after) if day(after.saturating_add(1)) == day(hour) => BoundKind::Exact,
        Some(_) => BoundKind::By,
        None if coverage.iter().any(|span| span.first_hour == hour) => BoundKind::HeldBefore,
        None => BoundKind::By,
    };
    ResearchBound { kind, hour }
}

fn end_bound(entry: &ResearchEntry) -> Option<ResearchBound> {
    match (entry.outcome, entry.ended_by_hour) {
        (ResearchOutcome::Active, _) => None,
        (ResearchOutcome::Completed | ResearchOutcome::Switched, Some(end)) => {
            let kind = if day(entry.last_observed_hour.saturating_add(1)) == day(end) { BoundKind::Exact } else { BoundKind::By };
            Some(ResearchBound { kind, hour: end })
        }
        _ => Some(ResearchBound { kind: BoundKind::LastSeen, hour: entry.last_observed_hour }),
    }
}

fn clip(spans: &[ResearchCoverage], hour: i32) -> Vec<ResearchCoverage> {
    spans.iter().filter(|span| span.first_hour <= hour).map(|span| ResearchCoverage { first_hour: span.first_hour, last_hour: span.last_hour.min(hour) }).collect()
}

fn within(first: i32, last: i32, spans: &[ResearchCoverage]) -> Vec<ResearchCoverage> {
    let from = spans.partition_point(|span| span.last_hour < first);
    spans[from..]
        .iter()
        .take_while(|span| span.first_hour <= last)
        .map(|span| ResearchCoverage { first_hour: span.first_hour.max(first), last_hour: span.last_hour.min(last) })
        .collect()
}

pub(crate) fn pieces(first: i32, last: i32, coverage: &[ResearchCoverage], availability: Option<&[ResearchCoverage]>) -> Vec<ResearchCoverage> {
    let read = within(first, last, coverage);
    match availability.filter(|spans| !spans.is_empty()) {
        None => read,
        Some(open) => read.iter().flat_map(|piece| within(piece.first_hour, piece.last_hour, open)).collect(),
    }
}

pub(crate) struct Asked<'a> {
    pub hour: i32,
    pub branch_first: i32,
    pub tag: &'a str,
}

pub(crate) struct Answer {
    pub reply: ResearchHistoryReply,
    pub uses: BTreeMap<String, BTreeSet<u32>>,
}

pub(crate) fn answer(index: &Index, asked: &Asked<'_>) -> Answer {
    let hour = asked.hour;
    let coverage = clip(&index.coverage, hour);
    let mut slots: BTreeMap<u32, ResearchSlotAt> = BTreeMap::new();
    for (&slot, spans) in &index.availability {
        let availability = clip(spans, hour);
        if !availability.is_empty() {
            slots.insert(slot, ResearchSlotAt { index: slot, availability, items: Vec::new() });
        }
    }
    let mut uses: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for at in 0..index.assignments.len() {
        let Some(entry) = index.entry_at(at, hour) else { continue };
        let slot = index.assignments[at].slot;
        let held = slots.entry(slot).or_insert_with(|| ResearchSlotAt { index: slot, availability: Vec::new(), items: Vec::new() });
        let last = match entry.outcome {
            ResearchOutcome::Active => hour,
            ResearchOutcome::Unknown => entry.last_observed_hour,
            _ => entry.ended_by_hour.map_or(entry.last_observed_hour, |end| end - 1).max(entry.first_observed_hour),
        }
        .min(hour);
        let pieces = pieces(entry.first_observed_hour, last, &coverage, index.availability.get(&slot).map(Vec::as_slice));
        let start = start_bound(&entry, &coverage);
        let end = end_bound(&entry);
        let origin = (entry.outcome == ResearchOutcome::Completed).then(|| index.origin(at, &entry, hour, &coverage)).flatten();
        let began = origin.map_or(start, |origin| origin.start);
        let took_days = match end {
            Some(end) if entry.outcome == ResearchOutcome::Completed && end.kind == BoundKind::Exact && began.kind == BoundKind::Exact => Some(day(end.hour) - day(began.hour)),
            _ => None,
        };
        uses.entry(entry.token.clone()).or_default().insert(entry.database_index);
        held.items.push(ResearchItem {
            token: entry.token,
            database_index: entry.database_index,
            first_observed_hour: entry.first_observed_hour,
            outcome: entry.outcome,
            pieces,
            start,
            end,
            origin,
            took_days,
        });
    }
    let mut warnings = Vec::new();
    let unreadable = index.unreadable.partition_point(|&at| at <= hour);
    if unreadable > 0 {
        warnings.push(format!("{unreadable} research assignment records have a cell that was not recorded, so they are not shown."));
    }
    for &(first, last) in index.unresolved.iter().filter(|(first, _)| *first <= hour) {
        warnings.push(format!("{}'s country is not resolved from hour {first} through {}, so its research there is not shown.", asked.tag, last.min(hour)));
    }
    let unrecorded = if hour < asked.branch_first {
        Some(BEFORE_BRANCH.to_owned())
    } else {
        coverage.is_empty().then(|| NOT_RECORDED.to_owned())
    };
    let partial = if index.unresolved.iter().any(|(first, _)| *first <= hour) || unreadable > 0 || index.unread_through(hour, &coverage) {
        Some(UNREAD.to_owned())
    } else {
        None
    };
    Answer {
        reply: ResearchHistoryReply {
            hour,
            next_change: index.next_change(hour),
            unrecorded,
            partial,
            completed: 0,
            coverage,
            slots: slots.into_values().collect(),
            artwork: Vec::new(),
            folders: Vec::new(),
            technology_folders: Vec::new(),
            definitions: Vec::new(),
            unnamed: UNLOCALISED.to_owned(),
            warnings,
        },
        uses,
    }
}

pub(crate) fn count(reply: &mut ResearchHistoryReply, folders: &[String], named: bool) {
    let chosen: BTreeSet<&str> = folders.iter().map(String::as_str).collect();
    let by_token: HashMap<&str, Option<&str>> = reply.technology_folders.iter().filter(|item| item.database_index.is_none()).map(|item| (item.token.as_str(), item.folder.as_deref())).collect();
    let by_definition: HashMap<(&str, u32), Option<&str>> =
        reply.technology_folders.iter().filter_map(|item| item.database_index.map(|index| ((item.token.as_str(), index), item.folder.as_deref()))).collect();
    let counted = |item: &ResearchItem| {
        chosen.is_empty()
            || by_definition.get(&(item.token.as_str(), item.database_index)).or_else(|| by_token.get(item.token.as_str())).copied().flatten().is_some_and(|folder| chosen.contains(folder))
    };
    reply.completed = reply.slots.iter().flat_map(|slot| &slot.items).filter(|item| item.outcome == ResearchOutcome::Completed && counted(item)).count() as u32;
    if !chosen.is_empty() && !named && reply.partial.is_none() {
        reply.partial = Some(NO_FOLDERS.to_owned());
    }
}

pub(crate) fn build(source: &Source, query: &Query<'_>, country: &Country, cancelled: &AtomicBool) -> io::Result<Index> {
    let (stretches, unresolved) = spans(query, country);
    let recording = query.replay.recording();
    let coverage_columns = [dashboard_declare::field(TableId::ResearchCoverage, "since")?];
    let assignment_columns = all_columns(TableId::ResearchAssignments);
    let mut index = Index { unresolved, ..Index::default() };
    let mut next = query.timeline.first;
    for stretch in &query.timeline.stretches {
        if stretch.first > next {
            index.gaps.push((next, stretch.first - 1));
        }
        next = next.max(stretch.last.saturating_add(1));
    }
    for run in query.holders.runs(country) {
        if let Holder::Found { .. } = run.holder {
            index.runs.push((run.first, run.last));
        }
    }
    for span in &stretches {
        check_cancel(cancelled)?;
        let (low, high) = coverage_keys(span.slot, span.id);
        let covered = cost::storage(|| lanes::history(recording, span.run, TableId::ResearchCoverage, &coverage_columns, Keys::Range(low, high), span.first, span.last))?;
        coverage(&covered, &mut index.coverage, &mut index.availability);
        let (low, high) = assignment_keys(span.slot, span.id);
        let assigned = cost::storage(|| lanes::history(recording, span.run, TableId::ResearchAssignments, &assignment_columns, Keys::Range(low, high), span.first, span.last))?;
        let held = revisions(&assigned, &mut index.unreadable)?;
        let active = held.values().any(|versions| versions.last().is_some_and(|(_, entry)| entry.outcome == ResearchOutcome::Active));
        let last_read = if active { last_read(source, query, span)? } else { None };
        for ((slot, _, _), revisions) in held {
            if !revisions.is_empty() {
                index.assignments.push(Assignment { slot, revisions, stretch_last: span.last, last_read });
            }
        }
    }
    Ok(index.finish())
}

const KEPT: usize = 8;

#[derive(Default)]
struct Kept(Vec<((u64, Country), Arc<Mutex<Option<Arc<Index>>>>)>);

fn indexed(source: &Source, query: &Query<'_>, branch: u64, country: &Country, cancelled: &AtomicBool) -> io::Result<Arc<Index>> {
    let slot = {
        let state = source.state::<Kept>();
        let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (branch, country.clone());
        let entry = match kept.0.iter().position(|(held, _)| *held == key) {
            Some(at) => kept.0.remove(at),
            None => (key, Arc::new(Mutex::new(None))),
        };
        let slot = Arc::clone(&entry.1);
        kept.0.push(entry);
        if kept.0.len() > KEPT {
            kept.0.remove(0);
        }
        slot
    };
    let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(index) = held.as_ref() {
        return Ok(Arc::clone(index));
    }
    let index = Arc::new(build(source, query, country, cancelled)?);
    *held = Some(Arc::clone(&index));
    Ok(index)
}

pub struct ResearchHistoryAt;

impl Product for ResearchHistoryAt {
    const KEY: &'static str = "research-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ResearchHistoryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, folders];
    type Reply = ResearchHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ResearchHistoryParams::decl(),
            ResearchHistoryReply::decl(),
            ResearchSlotAt::decl(),
            ResearchItem::decl(),
            ResearchBound::decl(),
            BoundKind::decl(),
            ResearchOrigin::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let index = indexed(source, &query, branch, &params.country, cancelled)?;
        let hour = params.hour.min(query.timeline.last);
        let tag = query.holders.name(&params.country, hour);
        let Answer { mut reply, uses } = answer(&index, &Asked { hour, branch_first: query.timeline.first, tag: &tag });
        let named = match content(source, context) {
            Ok(art) => {
                let indices: BTreeMap<&str, BTreeSet<u32>> = uses.iter().map(|(token, at)| (token.as_str(), at.clone())).collect();
                let Presentation { artwork, folders, technology_folders, definitions, warnings } = presentation(&art, &indices, cancelled)?;
                reply.artwork = artwork;
                reply.folders = folders;
                reply.technology_folders = technology_folders;
                reply.definitions = definitions;
                let dated = std::mem::replace(&mut reply.warnings, warnings);
                reply.warnings.extend(dated);
                reply.warnings.extend(art.refusal_lines());
                true
            }
            Err(error) => {
                reply.unnamed = format!("Research content unavailable: {error}");
                reply.warnings.insert(0, reply.unnamed.clone());
                false
            }
        };
        reply.warnings.push(OBSERVATION_NOTE.to_owned());
        count(&mut reply, &params.folders, named);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
