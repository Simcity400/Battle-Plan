
pub mod breakdown;
mod grants;
mod modifiers;
pub mod slots;
#[cfg(test)]
mod tests;

use dashboard_base::products::{Country, CountryParams, LoadContext, Product, artwork};
use dashboard_core::dto::Artwork;
use dashboard_base::Source;
use dashboard_text::metadata::check_cancel;
use dashboard_base::{ResearchFolder, ResearchTechnologyFolder};
use dashboard_query::{Query, cost, identity::Holder};
use replay_lanes::lanes::{self, Cell, History, Keys, Step};
use replay_data::rows::{ResearchAppliedBonusRow, ResearchAssignmentRow, ResearchCoverageRow, ResearchInputRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchCoverage {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ResearchOutcome {
    Active,
    Completed,
    Switched,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchEntry {
    pub token: String,
    pub technology_handle: u32,
    #[doc = " The technology's template occurrence: repeated ids have their own."]
    pub database_index: u32,
    #[doc = " Attribution never joins across missing hours or a changed holder."]
    pub continuity_start_hour: i32,
    #[doc = " Completed level when it was first observed."]
    pub level: i32,
    pub first_observed_hour: i32,
    #[doc = " The read before, which saw something else; none at a first read or after a gap."]
    pub started_after_hour: Option<i32>,
    pub last_observed_hour: i32,
    #[doc = " The read at which it had left the slot; never shown before this hour."]
    pub ended_by_hour: Option<i32>,
    #[doc = " The read that proved a completion arriving after the end."]
    pub outcome_observed_hour: Option<i32>,
    pub outcome: ResearchOutcome,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotHistory {
    pub index: u32,
    pub availability: Vec<ResearchCoverage>,
    pub entries: Vec<ResearchEntry>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchHistory {
    pub coverage: Vec<ResearchCoverage>,
    pub slots: Vec<ResearchSlotHistory>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchDefinitionText {
    pub token: String,
    pub database_index: u32,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchReply {
    pub history: ResearchHistory,
    pub artwork: Vec<Artwork>,
    pub folders: Vec<ResearchFolder>,
    pub technology_folders: Vec<ResearchTechnologyFolder>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub definitions: Option<Vec<ResearchDefinitionText>>,
}

dashboard_base::reply_warnings!(ResearchReply);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotState {
    pub first_hour: i32,
    pub last_hour: i32,
    pub token: Option<String>,
    #[doc = " The technology's template occurrence as read; none while idle."]
    pub database_index: Option<i32>,
    pub entry_first_observed_hour: Option<i32>,
    #[doc = " Native fixed5 operands as decimal text."]
    pub speed_multiplier_raw: String,
    pub banked_days_raw: String,
    pub applied_days_raw: String,
    pub ahead_reduction_years_raw: Option<String>,
    pub details: Option<ResearchDetails>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchDetails {
    #[doc = " The game's days-left estimate at the read; none where its rate is not known."]
    pub remaining_days: Option<u32>,
    #[doc = " The same estimate before the game rounds it up to whole days, as fixed5 days."]
    pub estimate_raw: Option<String>,
    #[doc = " None where the bonuses were not read with the slot or do not attribute."]
    pub bonuses: Option<Vec<ResearchBonus>>,
    pub modifiers: Option<ResearchModifiers>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifiers {
    pub components: Vec<ResearchModifier>,
    pub total_bonus_raw: String,
    pub year: i32,
    pub ahead_factor_raw: String,
    pub minimum_speed_raw: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifier {
    pub kind: String,
    pub raw: String,
    pub sources: Vec<ResearchModifierSource>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifierSource {
    pub token: Option<String>,
    pub name: Option<String>,
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchBonus {
    pub id: i32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub speed_bonus_raw: Option<String>,
    pub cost_reduction_raw: Option<String>,
    pub ahead_reduction_years_raw: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchProgress {
    pub first_hour: i32,
    pub last_hour: i32,
    pub entry_first_observed_hour: Option<i32>,
    #[doc = " Base duration in fixed5 days, from the recorded cost operands."]
    pub base_days_raw: Option<String>,
    pub progress_raw: String,
}

pub(crate) const INPUTS: u32 = TableId::ResearchInputs.index() as u32;
pub(crate) const BONUSES: u32 = TableId::ResearchAppliedBonuses.index() as u32;
pub(crate) const MODIFIERS: u32 = TableId::ResearchModifiers.index() as u32;

pub(crate) fn slice_key(table: u32, owner: u32) -> u128 {
    u128::from(table) << 32 | u128::from(owner)
}

pub(crate) fn assignment_keys(slot: u32, id: u32) -> (u128, u128) {
    let base = u128::from(slot) << 80 | u128::from(id) << 48;
    (base, base | ((1 << 48) - 1))
}

pub(crate) fn coverage_keys(slot: u32, id: u32) -> (u128, u128) {
    let base = u128::from(slot) << 64 | u128::from(id) << 32;
    (base, base | u128::from(u32::MAX))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    pub slot: u32,
    pub id: u32,
}

pub(crate) fn spans(query: &Query<'_>, tag: &Country) -> (Vec<Span>, Vec<(i32, i32)>) {
    let mut out = Vec::new();
    let mut unresolved = Vec::new();
    for run in query.holders.runs(tag).iter() {
        match run.holder {
            Holder::Found { slot, id } => out.extend(query.timeline.within(run.first, run.last).map(|stretch| Span { run: stretch.run, first: stretch.first, last: stretch.last, slot, id })),
            Holder::Unresolved => unresolved.push((run.first, run.last)),
        }
    }
    (out, unresolved)
}

pub(crate) fn number(cells: &[Cell], at: usize) -> Option<i64> {
    match cells.get(at) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

pub(crate) fn cells_at(steps: &[Step], hour: i32) -> Option<&[Cell]> {
    let at = steps.partition_point(|step| step.hour <= hour);
    steps[..at].last()?.cells.as_deref()
}

pub(crate) fn all_columns(table: TableId) -> Vec<usize> {
    (0..table.field_count()).collect()
}

fn push_span(spans: &mut Vec<ResearchCoverage>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(held) if held.last_hour.checked_add(1).is_some_and(|next| next >= first) => held.last_hour = held.last_hour.max(last),
        _ => spans.push(ResearchCoverage { first_hour: first, last_hour: last }),
    }
}

pub(crate) fn live_spans(steps: &[Step], to: i32, spans: &mut Vec<ResearchCoverage>) {
    for (at, step) in steps.iter().enumerate() {
        if step.cells.is_none() {
            continue;
        }
        let end = steps.get(at + 1).map_or(to, |next| next.hour - 1);
        if end >= step.hour {
            push_span(spans, step.hour, end);
        }
    }
}

pub(crate) fn coverage(history: &History, holder: &mut Vec<ResearchCoverage>, slots: &mut BTreeMap<u32, Vec<ResearchCoverage>>) {
    for (key, steps) in &history.keys {
        let member = *key as u32;
        let spans = if member == ResearchCoverageRow::HOLDER { &mut *holder } else { slots.entry(member).or_default() };
        live_spans(steps, history.to, spans);
    }
}

pub(crate) struct AssignmentCells {
    handle: usize,
    occurrence: usize,
    token: usize,
    level: usize,
    continuity: usize,
    first: usize,
    after: usize,
    last: usize,
    ended: usize,
    outcome: usize,
    observed: usize,
}

impl AssignmentCells {
    pub(crate) fn of(history: &History) -> io::Result<Self> {
        let at = |name: &str| -> io::Result<usize> {
            let field = dashboard_declare::field(TableId::ResearchAssignments, name)?;
            history.columns.binary_search(&field).map_err(|_| io::Error::other(format!("Research assignments were read without {name}")))
        };
        Ok(Self {
            handle: at("technology_handle")?,
            occurrence: at("database_index")?,
            token: at("token")?,
            level: at("level")?,
            continuity: at("continuity_start_hour")?,
            first: at("first_observed_hour")?,
            after: at("started_after_hour")?,
            last: at("last_observed_hour")?,
            ended: at("ended_by_hour")?,
            outcome: at("outcome")?,
            observed: at("outcome_observed_hour")?,
        })
    }

    fn entry(&self, history: &History, cells: &[Cell]) -> Option<ResearchEntry> {
        let optional = |at: usize| number(cells, at).map(|value| value as i32);
        let first = optional(self.first)?;
        let outcome = match number(cells, self.outcome)? as u8 {
            ResearchAssignmentRow::ACTIVE => ResearchOutcome::Active,
            ResearchAssignmentRow::COMPLETED => ResearchOutcome::Completed,
            ResearchAssignmentRow::SWITCHED => ResearchOutcome::Switched,
            _ => ResearchOutcome::Unknown,
        };
        let ended = optional(self.ended);
        Some(ResearchEntry {
            token: String::from_utf8(history.text(*cells.get(self.token)?)?.to_vec()).ok()?,
            technology_handle: u32::try_from(number(cells, self.handle)?).ok()?,
            database_index: u32::try_from(number(cells, self.occurrence)?).ok().filter(|&index| index > 0)?,
            continuity_start_hour: optional(self.continuity)?,
            level: optional(self.level)?,
            first_observed_hour: first,
            started_after_hour: optional(self.after),
            last_observed_hour: optional(self.last).unwrap_or(first).max(first).min(ended.unwrap_or(i32::MAX)),
            ended_by_hour: ended,
            outcome_observed_hour: optional(self.observed),
            outcome: if ended.is_none() { ResearchOutcome::Active } else { outcome },
        })
    }
}

pub(crate) fn entries(history: &History, last_read: Option<i32>, ends: Option<i32>, unreadable: &mut usize) -> io::Result<BTreeMap<u32, Vec<ResearchEntry>>> {
    let cells = AssignmentCells::of(history)?;
    let mut held: BTreeMap<(u32, i32, i32), (i32, ResearchEntry)> = BTreeMap::new();
    for (key, steps) in &history.keys {
        let index = (*key >> 16) as u32;
        for step in steps {
            let Some(row) = step.cells.as_deref() else { continue };
            let Some(entry) = cells.entry(history, row) else {
                *unreadable += 1;
                continue;
            };
            let id = (index, entry.first_observed_hour, entry.continuity_start_hour);
            if held.get(&id).is_none_or(|(hour, _)| *hour <= step.hour) {
                held.insert(id, (step.hour, entry));
            }
        }
    }
    let mut slots: BTreeMap<u32, Vec<ResearchEntry>> = BTreeMap::new();
    for ((index, _, _), (_, mut entry)) in held {
        if entry.outcome == ResearchOutcome::Active {
            let read = last_read.unwrap_or(entry.first_observed_hour).max(entry.first_observed_hour);
            entry.last_observed_hour = read;
            if let Some(end) = ends {
                entry.outcome = ResearchOutcome::Unknown;
                entry.ended_by_hour = Some(end.max(read));
            }
        }
        slots.entry(index).or_default().push(entry);
    }
    for entries in slots.values_mut() {
        entries.sort_by_key(|entry| (entry.first_observed_hour, entry.ended_by_hour.unwrap_or(i32::MAX)));
    }
    Ok(slots)
}

fn last_read(source: &Source, query: &Query<'_>, span: &Span) -> io::Result<Option<i32>> {
    let links = dashboard_base::queries::links(source, span.run)?;
    let mut keys: Vec<u128> = links.slots.keys().filter(|&&raw| links.slot_at(raw, span.last) == Some(span.slot)).map(|&raw| slice_key(INPUTS, raw)).collect();
    if keys.is_empty() {
        return Ok(None);
    }
    keys.sort_unstable();
    let reads = [replay_lanes::at::Read { table: TableId::SliceReads, columns: None, keys: replay_lanes::at::Keys::These(&keys) }];
    let frame = cost::storage(|| replay_lanes::at::frame(query.replay.recording(), span.run, span.last, &reads))?;
    Ok(frame.complete::<tables::SliceReads>().ok().and_then(|rows| rows.iter().map(|row| row.sample_hour).filter(|&read| read <= span.last).max()))
}

fn history(source: &Source, query: &Query<'_>, tag: &Country, cancelled: &AtomicBool) -> io::Result<(ResearchHistory, Vec<String>)> {
    let (spans, unresolved) = spans(query, tag);
    if spans.is_empty() {
        return Err(dashboard_core::refused(format!("{tag} holds no country on this branch")));
    }
    let recording = query.replay.recording();
    let coverage_columns = [dashboard_declare::field(TableId::ResearchCoverage, "since")?];
    let assignment_columns = all_columns(TableId::ResearchAssignments);
    let mut held = ResearchHistory::default();
    let mut availability: BTreeMap<u32, Vec<ResearchCoverage>> = BTreeMap::new();
    let mut assignments: BTreeMap<u32, Vec<ResearchEntry>> = BTreeMap::new();
    let mut unreadable = 0;
    for (at, span) in spans.iter().enumerate() {
        check_cancel(cancelled)?;
        let (low, high) = coverage_keys(span.slot, span.id);
        let covered = cost::storage(|| lanes::history(recording, span.run, TableId::ResearchCoverage, &coverage_columns, Keys::Range(low, high), span.first, span.last))?;
        coverage(&covered, &mut held.coverage, &mut availability);
        let (low, high) = assignment_keys(span.slot, span.id);
        let assigned = cost::storage(|| lanes::history(recording, span.run, TableId::ResearchAssignments, &assignment_columns, Keys::Range(low, high), span.first, span.last))?;
        let positions = AssignmentCells::of(&assigned)?;
        let active = assigned.keys.iter().any(|(_, steps)| steps.last().and_then(|step| step.cells.as_deref()).is_some_and(|cells| number(cells, positions.ended).is_none()));
        let read = if active { last_read(source, query, span)? } else { None };
        let ends = spans[at + 1..].iter().any(|later| later.slot == span.slot && later.id == span.id).then_some(span.last + 1);
        for (index, entries) in entries(&assigned, read, ends, &mut unreadable)? {
            assignments.entry(index).or_default().extend(entries);
        }
    }
    let mut warnings = Vec::new();
    for (index, spans) in availability {
        held.slots.push(ResearchSlotHistory { index, availability: spans, entries: assignments.remove(&index).unwrap_or_default() });
    }
    let orphaned: usize = assignments.values().map(Vec::len).sum();
    if orphaned > 0 {
        warnings.push(format!("{orphaned} research assignments name a slot whose availability was not recorded, so they are not shown."));
    }
    if unreadable > 0 {
        warnings.push(format!("{unreadable} research assignment records have a cell that was not recorded, so they are not shown."));
    }
    for (first, last) in unresolved {
        warnings.push(format!("{tag}'s country is not resolved from hour {first} through {last}, so its research there is not shown."));
    }
    Ok((held, warnings))
}

pub struct Research;

#[derive(Default)]
struct State {
    artwork: Option<Arc<artwork::Context>>,
}

pub(crate) fn content(source: &Source, context: &LoadContext) -> io::Result<Arc<artwork::Context>> {
    let state = source.state::<State>();
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let art = context.content().and_then(|content| artwork::shared(&mut state.artwork, source, content, artwork::Kind::Technology));
    if art.is_err() {
        state.artwork = None;
    }
    art
}

impl Product for Research {
    const KEY: &'static str = "research";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = ResearchReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CountryParams::decl(),
            ResearchReply::decl(),
            ResearchHistory::decl(),
            ResearchSlotHistory::decl(),
            ResearchEntry::decl(),
            ResearchOutcome::decl(),
            ResearchCoverage::decl(),
            ResearchDefinitionText::decl(),
            ResearchFolder::decl(),
            ResearchTechnologyFolder::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch())?;
        let (history, warnings) = history(source, &query, &params.country, cancelled)?;
        present(source, context, history, warnings, cancelled)
    }
}

fn present(source: &Source, context: &LoadContext, history: ResearchHistory, mut warnings: Vec<String>, cancelled: &AtomicBool) -> io::Result<ResearchReply> {
    let art = match content(source, context) {
        Ok(art) => art,
        Err(error) => {
            warnings.insert(0, format!("Research content unavailable: {error}"));
            return Ok(ResearchReply { history, artwork: Vec::new(), folders: Vec::new(), technology_folders: Vec::new(), warnings, definitions: None });
        }
    };
    let mut indices: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for entry in history.slots.iter().flat_map(|slot| &slot.entries) {
        indices.entry(entry.token.as_str()).or_default().insert(entry.database_index);
    }
    let single = |recorded: &BTreeSet<u32>| (recorded.len() == 1).then(|| recorded.first().copied()).flatten();
    let mut definitions = Vec::new();
    let mut placed = BTreeSet::new();
    for (token, recorded) in &indices {
        if single(recorded).is_none() {
            for &index in recorded {
                check_cancel(cancelled)?;
                let (name, description) = art.definition_text(token, index);
                definitions.push(ResearchDefinitionText { token: (*token).to_owned(), database_index: index, name, description });
                placed.insert((*token, index));
            }
        }
    }
    let tokens: BTreeMap<&str, Option<u32>> = indices.iter().map(|(token, recorded)| (*token, single(recorded))).collect();
    let shown: BTreeMap<&str, Option<u32>> = tokens.iter().filter(|(_, at)| at.is_some()).map(|(&token, &at)| (token, at)).collect();
    let (folders, technology_folders, folder_warnings) = art.research_folders(&shown, &placed);
    let (artwork, mut named) = art.recorded_text_catalogue(tokens, cancelled)?;
    named.extend(folder_warnings);
    named.append(&mut warnings);
    named.push("Research dates are observation bounds: a country's research is read about once a day. Elapsed durations follow a technology through the slots it was assigned to; progress made before its first observed assignment and gaps in the recording leave a duration unknown.".into());
    named.extend(art.refusal_lines());
    Ok(ResearchReply { history, artwork, folders, technology_folders, warnings: named, definitions: (!definitions.is_empty()).then_some(definitions) })
}

pub(crate) fn base_cost(row: &ResearchInputRow) -> Option<i128> {
    let cost = if row.cost_debug != 0 { 100_000 } else { i128::from(row.cost_raw).checked_mul(i128::from(row.cost_scale_raw))? / 100_000 };
    (cost > 0 && cost <= i128::from(i64::MAX)).then_some(cost)
}

pub(crate) fn remaining(row: &ResearchInputRow, rate: i128) -> Option<(u32, i128)> {
    if rate <= 0 || row.progress_raw < 0 {
        return None;
    }
    let left = (base_cost(row)? - i128::from(row.progress_raw)).max(0);
    Some((u32::try_from((left + rate - 1) / rate).ok()?, (left.checked_mul(100_000)? + rate - 1) / rate))
}

pub(crate) fn valid_inputs(inputs: &[&ResearchInputRow]) -> bool {
    inputs.iter().enumerate().all(|(at, row)| {
        row.index as usize == at
            && row.technology_present <= 1
            && (row.technology_present == 0
                || (matches!(row.database_index, replay_data::rows::Recorded::Value(index) if index > 0) && !row.token.is_empty() && row.token.len() <= 512 && std::str::from_utf8(&row.token).is_ok()))
    })
}

pub(crate) fn attributed_bonuses(input: &ResearchInputRow, rows: &[ResearchAppliedBonusRow]) -> Option<Vec<ResearchBonus>> {
    let matching: Vec<_> = rows.iter().filter(|row| row.country == input.country && row.index == input.index && row.technology_handle == input.technology_handle).collect();
    if matching.len() != input.bonus_count as usize || matching.iter().enumerate().any(|(at, row)| row.ordinal as usize != at || row.resolved > 1 || row.name.len() > 512) {
        return None;
    }
    Some(
        matching
            .into_iter()
            .map(|row| {
                let resolved = row.resolved != 0;
                ResearchBonus {
                    id: row.id,
                    token: resolved.then(|| std::str::from_utf8(&row.name).ok().map(str::to_owned)).flatten(),
                    name: None,
                    cost_reduction_raw: resolved.then_some(row.cost_reduction_raw).flatten().map(|value| value.to_string()),
                    speed_bonus_raw: resolved.then(|| row.speed_bonus_raw.to_string()),
                    ahead_reduction_years_raw: resolved.then(|| row.ahead_reduction_raw.to_string()),
                }
            })
            .collect(),
    )
}

pub(crate) struct SlotRead {
    pub state: ResearchSlotState,
    pub progress: Option<ResearchProgress>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Joined {
    pub first_observed: i32,
    pub fresh: bool,
}

pub(crate) fn slot_read(
    input: &ResearchInputRow,
    read: i32,
    through: i32,
    joined: Option<Joined>,
    bonuses: Option<&[ResearchAppliedBonusRow]>,
    modifier_rows: Option<&[replay_data::rows::ResearchModifierRow]>,
    daily: Option<i128>,
    localise: &dyn Fn(&str) -> Option<String>,
) -> SlotRead {
    let present = input.technology_present != 0;
    let mut bonuses = bonuses.and_then(|rows| attributed_bonuses(input, rows));
    let mut modifiers = modifier_rows.and_then(|rows| modifiers::breakdown(input, Some(rows)));
    let estimate = present
        .then(|| {
            let fresh = joined.is_some_and(|joined| joined.fresh).then_some(i128::from(input.research_speed_raw));
            modifiers.as_ref().and_then(|modifiers| modifiers::rate(modifiers, input.ahead_reduction_raw, read)).or(daily).or(fresh).and_then(|rate| remaining(input, rate))
        })
        .flatten();
    let (remaining_days, estimate_raw) = (estimate.map(|(days, _)| days), estimate.map(|(_, raw)| raw.to_string()));
    for bonus in bonuses.iter_mut().flatten() {
        bonus.name = bonus.token.as_deref().and_then(localise);
    }
    for source in modifiers.iter_mut().flat_map(|modifiers| modifiers.components.iter_mut()).flat_map(|component| &mut component.sources) {
        source.name = source.token.as_deref().and_then(localise);
    }
    let token = present.then(|| String::from_utf8_lossy(&input.token).into_owned());
    SlotRead {
        progress: joined.filter(|_| present).map(|joined| ResearchProgress {
            first_hour: read,
            last_hour: read,
            entry_first_observed_hour: Some(joined.first_observed),
            base_days_raw: base_cost(input).map(|cost| cost.to_string()),
            progress_raw: input.progress_raw.to_string(),
        }),
        state: ResearchSlotState {
            first_hour: read,
            last_hour: through,
            token,
            database_index: match input.database_index {
                replay_data::rows::Recorded::Value(index) if present => Some(index),
                _ => None,
            },
            entry_first_observed_hour: joined.filter(|_| present).map(|joined| joined.first_observed),
            speed_multiplier_raw: input.research_speed_raw.to_string(),
            banked_days_raw: input.banked_days_raw.to_string(),
            applied_days_raw: input.applied_days_raw.to_string(),
            ahead_reduction_years_raw: present.then(|| input.ahead_reduction_raw.to_string()),
            details: Some(ResearchDetails { remaining_days, estimate_raw, bonuses, modifiers }),
        },
    }
}

pub(crate) fn continues(last: &ResearchSlotState, next: &ResearchSlotState) -> bool {
    let (before, now) = (last.details.as_ref(), next.details.as_ref());
    let details = match (before, now) {
        (None, None) => true,
        (Some(before), Some(now)) => before.bonuses == now.bonuses && before.modifiers == now.modifiers,
        _ => false,
    };
    let countdown = match (before, now) {
        (Some(before), Some(now)) => {
            let counted = before.remaining_days.map(|days| days.saturating_sub(((i64::from(next.first_hour) - i64::from(last.first_hour)) / 24) as u32));
            counted == now.remaining_days || before.remaining_days == now.remaining_days
        }
        _ => true,
    };
    last.last_hour.checked_add(1) == Some(next.first_hour)
        && details
        && countdown
        && last.token == next.token
        && last.database_index == next.database_index
        && last.entry_first_observed_hour == next.entry_first_observed_hour
        && last.speed_multiplier_raw == next.speed_multiplier_raw
        && last.banked_days_raw == next.banked_days_raw
        && last.applied_days_raw == next.applied_days_raw
        && last.ahead_reduction_years_raw == next.ahead_reduction_years_raw
}
