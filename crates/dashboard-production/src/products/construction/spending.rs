
use dashboard_base::products::Country;
use super::{Content, Line, SpendingGap, col, gap_of, missing};
use dashboard_base::{Source, products::LoadContext, queries::country_ids::{Names, RunLinks}};
use dashboard_query::{Query, identity::Holder, timeline::Stretch};
use replay_lanes::lanes::{Cell, Keys, Reading};
use replay_data::rows::{TableId, TagHolderRow, key_field};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstructionSpendingParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpentStep {
    pub known_at: i32,
    #[doc = " Fixed5; null where the total is not recorded (past what the recording holds)."]
    pub total_raw: Option<String>,
}


pub(crate) type LeafKey = (bool, bool, u32, u32, u32);

fn named(value: u32) -> Option<u32> {
    (value != TagHolderRow::NONE).then_some(value)
}

pub(crate) fn leaf_of(key: u128) -> LeafKey {
    let railway = (key >> 64) & 1 == 1;
    let shared = key as u32;
    let (building_type, to_state) = if railway { (TagHolderRow::NONE, shared) } else { (shared, TagHolderRow::NONE) };
    ((key >> 65) & 1 == 1, railway, building_type, (key >> 32) as u32, to_state)
}

pub(crate) fn identity_keys(slot: u32, id: u32) -> (u128, u128) {
    let low = key_field(slot) << 90 | key_field(id) << 66;
    (low, low | ((1u128 << 66) - 1))
}

pub(crate) type Delta = (i32, Option<i128>);

fn read_within(complete: &[(i32, i32)], first: i32, last: i32) -> bool {
    let at = complete.partition_point(|&(_, end)| end < first);
    complete.get(at).is_some_and(|&(start, _)| start <= last)
}

pub(crate) fn deltas(steps: &[(i32, Option<Cell>)], read_from: i32, from: i32, opens_run: bool, complete: &[(i32, i32)]) -> Vec<Delta> {
    let mut out = Vec::new();
    let mut held: Option<i64> = None;
    let first_step = steps.first().map_or(i32::MAX, |step| step.0);
    let mut unread = !opens_run && !read_within(complete, read_from, first_step.saturating_sub(1));
    let mut lost = false;
    for (at, &(hour, cell)) in steps.iter().enumerate() {
        match cell {
            Some(Cell::Number(value)) => {
                if lost {
                    continue;
                }
                if hour < from {
                    unread = false;
                } else if unread {
                    lost = true;
                    out.push((hour.max(from), None));
                    continue;
                } else {
                    let by = i128::from(value) - i128::from(held.unwrap_or(0));
                    if by != 0 {
                        out.push((hour.max(from), Some(by)));
                    }
                }
                held = Some(value);
            }
            Some(Cell::Unrecorded) if hour >= from && !lost => {
                lost = true;
                out.push((hour, None));
            }
            Some(Cell::Unrecorded) if hour < from => {
                held = None;
                unread = true;
            }
            None if held.is_none() && read_within(complete, hour, steps.get(at + 1).map_or(i32::MAX, |next| next.0 - 1)) => unread = false,
            _ => {}
        }
    }
    out
}

pub(crate) fn totals(mut deltas: Vec<Delta>) -> Vec<SpentStep> {
    deltas.sort_by_key(|delta| delta.0);
    let mut out: Vec<SpentStep> = Vec::new();
    let mut total: Option<i128> = Some(0);
    for (hour, by) in deltas {
        total = match (total, by) {
            (Some(total), Some(by)) => Some(total + by),
            _ => None,
        };
        let raw = total.map(|total| total.to_string());
        match out.last_mut() {
            Some(step) if step.known_at == hour => step.total_raw = raw,
            _ => out.push(SpentStep { known_at: hour, total_raw: raw }),
        }
    }
    out
}

pub fn spent_by_leaf(query: &Query<'_>, tag: &Country) -> io::Result<BTreeMap<LeafKey, Vec<SpentStep>>> {
    let recording = query.replay.recording();
    let total = dashboard_declare::field(TableId::ConstructionSpend, "total")?;
    let mut by_leaf: BTreeMap<LeafKey, Vec<Delta>> = BTreeMap::new();
    for run in query.holders.within(tag, query.timeline.first, query.timeline.last) {
        let Holder::Found { slot, id } = run.holder else { continue };
        let (low, high) = identity_keys(slot, id);
        for stretch in query.timeline.within(run.first, run.last) {
            let run_first = recording.run(stretch.run)?.first_hour;
            let read_from = if stretch.first > run_first { stretch.first - 1 } else { stretch.first };
            let history = dashboard_query::cost::storage(|| {
                replay_lanes::lanes::history(recording, stretch.run, TableId::ConstructionSpend, &[total], Keys::Range(low, high), read_from, stretch.last)
            })?;
            let complete: Vec<(i32, i32)> =
                history.statuses.iter().filter(|held| replay_recording::columnar_frames::complete(held.status)).map(|held| (held.first, held.last)).collect();
            for (key, steps) in &history.keys {
                let cells: Vec<(i32, Option<Cell>)> = steps.iter().map(|step| (step.hour, step.cells.as_ref().map(|cells| cells[0]))).collect();
                by_leaf.entry(leaf_of(*key)).or_default().extend(deltas(&cells, read_from, stretch.first, read_from == stretch.first, &complete));
            }
        }
    }
    Ok(by_leaf.into_iter().map(|(leaf, deltas)| (leaf, totals(deltas))).collect())
}

pub(crate) fn leaf_keys(line: &Line) -> Vec<LeafKey> {
    let none = TagHolderRow::NONE;
    let place = (if line.railway { none } else { line.building.unwrap_or(none) }, line.state.unwrap_or(none), if line.railway { line.to_state.unwrap_or(none) } else { none });
    let mut out = Vec::new();
    if line.new_work || !line.repair_work {
        out.push((false, line.railway, place.0, place.1, place.2));
    }
    if line.repair_work {
        out.push((true, line.railway, place.0, place.1, place.2));
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GapKind {
    pub new: bool,
    pub repair: bool,
}

impl GapKind {
    pub const NEITHER: Self = Self { new: false, repair: false };
    pub const NEW: Self = Self { new: true, repair: false };
    pub const REPAIR: Self = Self { new: false, repair: true };
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineFacts {
    pub leaf: (bool, u32, u32, u32),
    pub building: Option<u32>,
    pub gaps: Vec<(i32, i32, SpendingGap, GapKind)>,
    pub before_first_read: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    pub from: i32,
    pub to: i32,
    pub spent: BTreeMap<LeafKey, Vec<SpentStep>>,
    pub lines: Vec<LineFacts>,
    pub unattributed: Vec<i32>,
    pub unrecorded: Vec<(i32, i32, String)>,
    pub names: HashMap<u32, Vec<(i32, String)>>,
    pub warnings: Vec<(i32, Vec<String>)>,
}

const BUILDINGS_UNREAD: &str = "The building construction queue was not read in full, so building output in those hours is not counted";
const RAILWAYS_UNREAD: &str = "The railway construction queue was not read in full, so railway output in those hours is not counted";
const HOURS_UNRECORDED: &str = "Output in hours that were not recorded is not counted";

pub(crate) fn dated_gaps(steps: &[(i32, Option<u8>, bool)]) -> Vec<(i32, i32, SpendingGap)> {
    let mut out: Vec<(i32, i32, SpendingGap)> = Vec::new();
    let mut open: Option<(i32, SpendingGap)> = None;
    for &(hour, reason, output) in steps {
        let active = reason.and_then(gap_of).filter(|&gap| !(gap == SpendingGap::NotRecorded && !output));
        if let Some((start, prev)) = open && active != Some(prev) {
            out.push((start, hour - 1, prev));
            open = None;
        }
        if open.is_none() && let Some(gap) = active { open = Some((hour, gap)); }
    }
    if let Some((start, gap)) = open { out.push((start, i32::MAX, gap)); }
    out
}

fn split_gaps(
    gaps: &[(i32, i32, SpendingGap)],
    kinds: &[(i32, GapKind)],
    buildings: &[(i32, Option<u32>)],
) -> BTreeMap<Option<u32>, Vec<(i32, i32, SpendingGap, GapKind)>> {
    let kind_at = |hour: i32| kinds[..kinds.partition_point(|&(at, _)| at <= hour)].last().map_or(GapKind::NEITHER, |&(_, k)| k);
    let building_at = |hour: i32| buildings[..buildings.partition_point(|&(at, _)| at <= hour)].last().and_then(|&(_, b)| b);
    let mut by_building: BTreeMap<Option<u32>, Vec<(i32, i32, SpendingGap, GapKind)>> = BTreeMap::new();
    for &(start, end, gap) in gaps {
        let cuts: BTreeSet<i32> = kinds.iter().map(|&(at, _)| at).chain(buildings.iter().map(|&(at, _)| at)).filter(|&at| start < at && at <= end).collect();
        let mut cursor = start;
        for at in cuts {
            by_building.entry(building_at(cursor)).or_default().push((cursor, at - 1, gap, kind_at(cursor)));
            cursor = at;
        }
        by_building.entry(building_at(cursor)).or_default().push((cursor, end, gap, kind_at(cursor)));
    }
    by_building
}

pub(crate) fn unread_spans(first: i32, last: i32, mut whole: Vec<(i32, i32)>, reason: &str, out: &mut Vec<(i32, i32, String)>) {
    whole.sort_unstable();
    let mut cursor = first;
    for (from, to) in whole {
        if from > cursor {
            out.push((cursor, from - 1, reason.to_owned()));
        }
        cursor = cursor.max(to.saturating_add(1));
    }
    if cursor <= last {
        out.push((cursor, last, reason.to_owned()));
    }
}

fn queue_gaps(query: &Query<'_>, out: &mut Vec<(i32, i32, String)>) -> io::Result<()> {
    let recording = query.replay.recording();
    for stretch in &query.timeline.stretches {
        for (table, reason) in [(TableId::BuildingConstruction, BUILDINGS_UNREAD), (TableId::RailwayConstruction, RAILWAYS_UNREAD)] {
            let statuses = dashboard_query::cost::storage(|| replay_lanes::lanes::statuses(recording, stretch.run, table, stretch.first, stretch.last))?;
            let whole = statuses.iter().filter(|held| replay_recording::columnar_frames::complete(held.status)).map(|held| (held.first, held.last)).collect();
            unread_spans(stretch.first, stretch.last, whole, reason, out);
        }
    }
    Ok(())
}

#[derive(Default)]
struct WitnessState {
    built: i64,
    repaired: i64,
    output_since: Option<i64>,
    new_raw: i64,
    repair_raw: i64,
    observation: GapKind,
}

fn witnessed_work(
    state: &mut WitnessState,
    hour: i32,
    since: Option<i64>,
    built: Option<i64>,
    repaired: Option<i64>,
    new_raw: Option<i64>,
    repair_raw: Option<i64>,
    repair_tick: Option<i32>,
) -> (GapKind, bool) {
    if let Some(s) = since && state.output_since != Some(s) {
        state.output_since = Some(s);
        state.new_raw = 0;
        state.repair_raw = 0;
        state.observation = GapKind::NEITHER;
    }
    let mut kind = GapKind::NEITHER;
    if let Some(v) = built {
        if v > state.built { kind.new = true; }
        state.built = v;
    }
    if let Some(v) = new_raw {
        if v != state.new_raw { kind.new = true; }
        if v != 0 { state.observation.new = true; }
        state.new_raw = v;
    }
    let mut repaired_rose = false;
    if let Some(v) = repaired {
        if v > state.repaired {
            kind.repair = true;
            repaired_rose = true;
        }
        state.repaired = v;
    }
    if let Some(v) = repair_raw {
        if v != state.repair_raw { kind.repair = true; }
        if v != 0 { state.observation.repair = true; }
        state.repair_raw = v;
    }
    if repair_tick == Some(hour) {
        kind.repair = true;
        if since.is_some() { state.observation.repair = true; }
    }
    (kind, repaired_rose)
}

fn inferred_queue_kind(open: bool, repair_levels: Option<i64>, remaining: Option<i64>, repair_tick_unresolved: bool) -> GapKind {
    if !open {
        GapKind::NEITHER
    } else if repair_levels.is_some_and(|r| r > 0) {
        GapKind::REPAIR
    } else if repair_levels == Some(0) && remaining.is_some_and(|n| n > 0) && !repair_tick_unresolved {
        GapKind::NEW
    } else {
        GapKind::NEITHER
    }
}

#[derive(Default)]
struct LineHistory {
    read: Vec<(i32, Option<u8>, bool)>,
    kinds: Vec<(i32, GapKind)>,
    buildings: Vec<(i32, Option<u32>)>,
    owned: Vec<(i32, i32)>,
    unattributed: Option<i32>,
    last_run: Option<u32>,
    witness: WitnessState,
    repair_tick_unresolved: bool,
    census_valid: bool,
    last_known_owner: Option<bool>,
    before_observation: Option<i32>,
}

impl LineHistory {
    fn attribute(&mut self, owner: Option<bool>, first: i32, last: i32, had_output: bool, before_observation: bool) {
        match owner {
            Some(true) => {
                if let Some(held) = self.owned.last_mut() && first <= held.1.saturating_add(1) {
                    held.1 = held.1.max(last);
                } else {
                    self.owned.push((first, last));
                }
                if had_output && before_observation {
                    self.before_observation.get_or_insert(first);
                }
            }
            None if had_output => { self.unattributed.get_or_insert(first); }
            _ => {}
        }
    }

    fn push_kind(&mut self, hour: i32, kind: GapKind) {
        if let Some(last) = self.kinds.last_mut() && last.0 == hour {
            last.1 = kind;
        } else if self.kinds.last().is_none_or(|&(_, prev)| prev != kind) {
            self.kinds.push((hour, kind));
        }
    }
}

struct QueueCoverage {
    building_reads: Vec<(i32, i32)>,
    building_complete: Vec<(i32, i32)>,
    railway_reads: Vec<(i32, i32)>,
    railway_complete: Vec<(i32, i32)>,
}

type OwnerRun = (Option<bool>, i32, i32);

struct Owners<'a> {
    by_raw: HashMap<u32, Vec<OwnerRun>>,
    links: &'a RunLinks,
    names: &'a Names,
    country: &'a Country,
    stretch: &'a Stretch,
}

impl Owners<'_> {
    fn within(&mut self, raw: u32, first: i32, last: i32) -> impl Iterator<Item = OwnerRun> + '_ {
        let runs = self.by_raw.entry(raw).or_insert_with(|| {
            let (first, last) = (self.stretch.first, self.stretch.last);
            match self.country {
                Country::Slot(chosen) => self.links.within(raw, first, last).map(|(slot, a, b)| (slot.map(|slot| slot == *chosen), a, b)).collect(),
                Country::Tag(chosen) => self.names.tags_over(self.links, raw, first, last).into_iter().map(|(tag, a, b)| (tag.map(|tag| tag == chosen), a, b)).collect(),
            }
        });
        let at = runs.partition_point(|&(_, _, end)| end < first);
        runs[at..].iter().copied().take_while(move |&(_, start, _)| start <= last).map(move |(owner, start, end)| (owner, start.max(first), end.min(last)))
    }
}

fn read_stretch_steps(
    query: &Query<'_>,
    stretch: &Stretch,
    ids: &[u128],
    queues: &QueueCoverage,
    owners: &mut Owners<'_>,
    into: &mut BTreeMap<u64, SeenLine>,
) -> io::Result<()> {
    let (run, first, last) = (stretch.run, stretch.first, stretch.last);
    let columns = [
        col::OUTPUT_ALONE, col::COUNTRY, col::BUILDING_TYPE, col::REMAINING, col::REPAIR_LEVELS, col::BUILT, col::REPAIRED,
        col::END, col::CONSTRUCTION_RAW, col::REPAIR_RAW, col::OUTPUT_SINCE, col::OBSERVED_SINCE_CREATION, col::OUTPUT_REASON, col::FIRST_REPAIR_TICK,
    ];
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(query.replay.recording(), run, TableId::ConstructionProjects, &columns, Keys::These(ids), first, last))?;
    let at = |column: usize| history.columns.binary_search(&column).map_err(|_| dashboard_core::invalid("A construction project column was not read"));
    let (alone, owner, btype, ended, rem, rep_lvl, built, repaired, new_raw, rep_raw, since, obs, reason, repair) = (
        at(col::OUTPUT_ALONE)?, at(col::COUNTRY)?, at(col::BUILDING_TYPE)?, at(col::END)?,
        at(col::REMAINING)?, at(col::REPAIR_LEVELS)?, at(col::BUILT)?, at(col::REPAIRED)?,
        at(col::CONSTRUCTION_RAW)?, at(col::REPAIR_RAW)?, at(col::OUTPUT_SINCE)?,
        at(col::OBSERVED_SINCE_CREATION)?, at(col::OUTPUT_REASON)?, at(col::FIRST_REPAIR_TICK)?,
    );
    let number = |cells: &[Cell], at: usize| match cells.get(at) { Some(Cell::Number(value)) => Some(*value), _ => None };
    let unknown_owner = replay_data::rows::ConstructionProjectRow::OUTPUT_OWNER_UNKNOWN;
    let unread_output = replay_data::rows::ConstructionProjectRow::OUTPUT_NOT_RECORDED;
    let open = i64::from(replay_data::rows::ConstructionProjectRow::OPEN);
    for (key, steps) in &history.keys {
        let Some(entry) = into.get_mut(&(*key as u64)) else { continue };
        let (q_reads, q_complete) = if entry.line.railway { (&queues.railway_reads, &queues.railway_complete) } else { (&queues.building_reads, &queues.building_complete) };
        let held = &mut entry.history;
        if held.last_run != Some(run) {
            held.last_run = Some(run);
            held.witness.built = 0;
            held.witness.repaired = 0;
            held.repair_tick_unresolved = false;
            held.census_valid = false;
        }
        for (idx, step) in steps.iter().enumerate() {
            let Some(cells) = step.cells.as_deref() else {
                held.census_valid = false;
                continue;
            };
            let building = number(cells, btype).and_then(|v| u32::try_from(v).ok());
            if held.buildings.last().is_none_or(|&(_, prev)| prev != building) { held.buildings.push((step.hour, building)); }
            let is_census = number(cells, alone) == Some(0);
            let since_val = number(cells, since);
            let had_output = since_val.is_some();
            let reason_val = number(cells, reason).and_then(|value| u8::try_from(value).ok());
            let raw_owner = number(cells, owner).and_then(|value| u32::try_from(value).ok()).filter(|&value| value != 0);
            let known_owner = is_census
                || match reason_val {
                    Some(r) if r == unknown_owner => false,
                    Some(r) if r == unread_output => held.last_known_owner.unwrap_or(raw_owner.is_some()),
                    Some(_) => true,
                    None => false,
                };
            held.last_known_owner = Some(known_owner);
            let effective_reason = if had_output && !known_owner { Some(unknown_owner) } else { reason_val };
            held.read.push((step.hour, effective_reason, had_output));
            let hi = steps.get(idx + 1).map_or(last.saturating_add(1), |next| next.hour).min(last.saturating_add(1));
            let before_observation = number(cells, obs) == Some(0);
            if let Some(raw_owner) = raw_owner.filter(|_| known_owner) {
                for (owner, first, last) in owners.within(raw_owner, step.hour, hi.saturating_sub(1)) {
                    held.attribute(owner, first, last, had_output, before_observation);
                }
            } else {
                held.attribute(None, step.hour, step.hour, had_output, before_observation);
            }
            let tick = number(cells, repair).and_then(|value| i32::try_from(value).ok());
            let (witnessed, repaired_rose) = witnessed_work(
                &mut held.witness, step.hour, since_val, number(cells, built), number(cells, repaired), number(cells, new_raw), number(cells, rep_raw), tick,
            );
            let has_witness = witnessed.new || witnessed.repair;
            if !is_census {
                held.census_valid = false;
                held.push_kind(step.hour, if has_witness { witnessed } else { held.witness.observation });
                continue;
            }
            let (is_open, rep_now, rem_now) = (number(cells, ended) == Some(open), number(cells, rep_lvl), number(cells, rem));
            let apply_queue = |held: &mut LineHistory, hour: i32, has_witness: bool, witnessed: GapKind, repaired_rose: bool| {
                let fresh = read_within(q_reads, hour, hour);
                let complete = read_within(q_complete, hour, hour);
                if repaired_rose || (fresh && rep_now.is_some_and(|r| r > 0)) || witnessed.new {
                    held.repair_tick_unresolved = false;
                } else if witnessed.repair && !(fresh && rep_now.is_some_and(|r| r > 0)) {
                    held.repair_tick_unresolved = true;
                }
                if fresh { held.census_valid = true; } else if !complete || has_witness { held.census_valid = false; }
                let kind = if has_witness {
                    witnessed
                } else if held.census_valid && complete {
                    inferred_queue_kind(is_open, rep_now, rem_now, held.repair_tick_unresolved)
                } else {
                    GapKind::NEITHER
                };
                held.push_kind(hour, kind);
            };
            apply_queue(held, step.hour, has_witness, witnessed, repaired_rose);
            let mut checks = BTreeSet::new();
            let after_step = step.hour.saturating_add(1);
            if has_witness && after_step < hi { checks.insert(after_step); }
            for &(s, e) in q_reads {
                let at = s.max(after_step);
                if at <= e && at < hi { checks.insert(at); }
            }
            for &(_, e) in q_complete {
                let after = e.saturating_add(1);
                if after > step.hour && after < hi { checks.insert(after); }
            }
            for h in checks {
                apply_queue(held, h, false, GapKind::NEITHER, false);
            }
        }
    }
    Ok(())
}

fn dated_spellings(query: &Query<'_>, run: u32, first: i32, last: i32, ids: &BTreeSet<u32>, into: &mut BTreeMap<u32, Vec<(i32, Option<String>)>>) -> io::Result<()> {
    let recording = query.replay.recording();
    if ids.is_empty() || recording.predates(TableId::LexicalTokens.index()).is_some() || recording.withheld_reason(TableId::LexicalTokens.index()).is_some() {
        return Ok(());
    }
    let wanted: Vec<u128> = ids.iter().map(|&id| u128::from(id)).collect();
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, run, TableId::LexicalTokens, &[0], Keys::These(&wanted), first, last))?;
    let mut asks = Vec::new();
    for &id in ids {
        for (hour, _, reading) in history.readings(u128::from(id)) {
            match reading {
                Reading::Cells([Cell::Written(_)]) => asks.push((hour, u128::from(id))),
                Reading::Cells([cell]) => {
                    let text = history.text(*cell).and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned);
                    into.entry(id).or_default().push((hour, text));
                }
                Reading::Cells(_) | Reading::Absent => into.entry(id).or_default().push((hour, None)),
                Reading::Unread { .. } => {}
            }
        }
    }
    if !asks.is_empty() {
        dashboard_query::cost::storage(|| {
            replay_lanes::lanes::at_each(recording, run, TableId::LexicalTokens, Some(&[0]), &asks, &mut |hour, key, row| {
                let text = row
                    .and_then(|(view, slot)| view.list(0, slot).or_else(|| dashboard_query::read::text(&view, 0, slot)))
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .map(str::to_owned);
                into.entry(key as u32).or_default().push((hour, text));
                Ok(())
            })
        })?;
    }
    Ok(())
}

pub(crate) fn place_buildings(spent: &BTreeMap<LeafKey, Vec<SpentStep>>, lines: &[LineFacts]) -> HashMap<LeafKey, Option<u32>> {
    let mut buildings: HashMap<LeafKey, Option<u32>> = spent.keys().map(|&place| (place, named(place.2))).collect();
    for line in lines {
        let (railway, building, state, to_state) = line.leaf;
        for repair in [false, true] {
            if let Some(held) = buildings.get_mut(&(repair, railway, building, state, to_state)) {
                *held = held.or(line.building);
            }
        }
        for &(_, _, _, kind) in &line.gaps {
            for (repair, active) in [(false, kind.new), (true, kind.repair)] {
                if active {
                    let held = buildings.entry((repair, railway, building, state, to_state)).or_insert(None);
                    *held = held.or(line.building);
                }
            }
        }
    }
    buildings
}

fn first_building_hours(from: i32, spent: &BTreeMap<LeafKey, Vec<SpentStep>>, lines: &[LineFacts], buildings: &HashMap<LeafKey, Option<u32>>) -> BTreeMap<u32, i32> {
    let mut first_at: BTreeMap<u32, i32> = BTreeMap::new();
    let mut note = |id: u32, hour: i32| {
        let hour = hour.max(from);
        first_at.entry(id).and_modify(|held| *held = (*held).min(hour)).or_insert(hour);
    };
    for (place, steps) in spent {
        if let (Some(id), Some(step)) = (buildings.get(place).copied().flatten(), steps.first()) { note(id, step.known_at); }
    }
    for line in lines {
        if let Some(id) = line.building {
            for &(hour, _, _, kind) in &line.gaps {
                if kind.new || kind.repair { note(id, hour); }
            }
        }
    }
    first_at
}

fn owner_unknown_lines(query: &Query<'_>, run: u32, first: i32, last: i32) -> io::Result<BTreeSet<u64>> {
    let history = dashboard_query::cost::storage(|| {
        replay_lanes::lanes::history(query.replay.recording(), run, TableId::ConstructionProjects, &[col::OUTPUT_REASON], Keys::All, first, last)
    })?;
    let unknown = i64::from(replay_data::rows::ConstructionProjectRow::OUTPUT_OWNER_UNKNOWN);
    Ok(history
        .keys
        .iter()
        .filter(|(_, steps)| steps.iter().any(|step| matches!(step.cells.as_deref(), Some([Cell::Number(value)]) if *value == unknown)))
        .map(|(key, _)| *key as u64)
        .collect())
}

struct SeenLine { line: Line, history: LineHistory }

pub(crate) fn facts(source: &Source, query: &Query<'_>, country: &Country, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Facts> {
    let names = Names::new(&query.holders);
    let mut out = Facts { from: query.timeline.first, to: query.timeline.last, spent: spent_by_leaf(query, country)?, ..Facts::default() };
    let mut seen: BTreeMap<u64, SeenLine> = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lines = crate::products::construction::cached_construction(source, stretch.run)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let unowned = owner_unknown_lines(query, stretch.run, stretch.first, stretch.last)?;
        let mut owners = Owners { by_raw: HashMap::new(), links: &links, names: &names, country, stretch };
        let mut kept = Vec::new();
        for line in lines.within(stretch.first, stretch.last) {
            let possible = line.country.is_none_or(|raw| {
                owners.within(raw, line.first.max(stretch.first), line.last.min(stretch.last)).any(|(owner, ..)| owner != Some(false))
            });
            if possible || line.output_alone || unowned.contains(&line.id) {
                seen.entry(line.id).or_insert_with(|| SeenLine { line: line.clone(), history: LineHistory::default() });
                kept.push(line);
            }
        }
        let ids: Vec<u128> = kept.iter().map(|line| u128::from(line.id)).collect();
        let recording = query.replay.recording();
        let complete_spans = |table: TableId| -> io::Result<Vec<(i32, i32)>> {
            let statuses = dashboard_query::cost::storage(|| replay_lanes::lanes::statuses(recording, stretch.run, table, stretch.first, stretch.last))?;
            Ok(statuses.iter().filter(|held| replay_recording::columnar_frames::complete(held.status)).map(|held| (held.first, held.last)).collect())
        };
        let queues = if ids.is_empty() {
            QueueCoverage { building_reads: Vec::new(), building_complete: Vec::new(), railway_reads: Vec::new(), railway_complete: Vec::new() }
        } else {
            QueueCoverage {
                building_reads: dashboard_query::cost::storage(|| replay_lanes::lanes::complete_reads(recording, stretch.run, TableId::BuildingConstruction, stretch.first, stretch.last))?,
                building_complete: complete_spans(TableId::BuildingConstruction)?,
                railway_reads: dashboard_query::cost::storage(|| replay_lanes::lanes::complete_reads(recording, stretch.run, TableId::RailwayConstruction, stretch.first, stretch.last))?,
                railway_complete: complete_spans(TableId::RailwayConstruction)?,
            }
        };
        if !ids.is_empty() {
            read_stretch_steps(query, stretch, &ids, &queues, &mut owners, &mut seen)?;
        }
        for line in kept {
            let Some(entry) = seen.get_mut(&line.id) else { continue };
            if stretch.first <= line.last && line.last <= stretch.last {
                let unknown = replay_data::rows::ConstructionProjectRow::OUTPUT_OWNER_UNKNOWN;
                let unread = replay_data::rows::ConstructionProjectRow::OUTPUT_NOT_RECORDED;
                let end_known = !line.output_alone
                    || (line.output_reason != unknown && line.output_reason != unread)
                    || (line.output_reason == unread && entry.history.last_known_owner.unwrap_or(line.country.is_some()));
                let end_reason = if line.had_output && !end_known { unknown } else { line.output_reason };
                entry.history.read.push((line.last, Some(end_reason), line.had_output));
            }
        }
    }
    for mut acc in seen.into_values() {
        acc.history.read.sort_by_key(|step| step.0);
        let mut merged_read: Vec<(i32, Option<u8>, bool)> = Vec::with_capacity(acc.history.read.len());
        for step in acc.history.read {
            if merged_read.last().is_some_and(|last| last.0 == step.0) { *merged_read.last_mut().unwrap() = step; } else { merged_read.push(step); }
        }
        let gaps = dated_gaps(&merged_read);
        let owner_gap = gaps.iter().find(|gap| gap.2 == SpendingGap::OwnerUnknown).map(|gap| gap.0);
        if let Some(hour) = owner_gap.into_iter().chain(acc.history.unattributed).min() { out.unattributed.push(hour); }
        if acc.history.owned.is_empty() { continue; }
        let gaps: Vec<_> = gaps.into_iter().filter(|&(_, _, gap)| gap != SpendingGap::OwnerUnknown).flat_map(|(first, last, gap)| {
            let owned = &acc.history.owned;
            let at = owned.partition_point(|&(_, end)| end < first);
            owned[at..].iter().copied().take_while(move |&(start, _)| start <= last).map(move |(start, end)| (first.max(start), last.min(end), gap))
        }).collect();
        let leaf = leaf_keys(&acc.line)[0];
        let mut before_first_read = acc.history.before_observation.map(|hour| hour.max(acc.line.first_read));
        let mut by_building = split_gaps(&gaps, &acc.history.kinds, &acc.history.buildings);
        if by_building.is_empty() {
            by_building.insert(acc.history.buildings.last().and_then(|&(_, b)| b).or(acc.line.building), Vec::new());
        }
        for (building, gaps) in by_building {
            let bcol = if leaf.1 { TagHolderRow::NONE } else { building.unwrap_or(TagHolderRow::NONE) };
            out.lines.push(LineFacts {
                leaf: (leaf.1, bcol, leaf.3, leaf.4), building, gaps, before_first_read: before_first_read.take(),
            });
        }
    }
    let buildings = place_buildings(&out.spent, &out.lines);
    let first_at = first_building_hours(out.from, &out.spent, &out.lines, &buildings);
    let ids: BTreeSet<u32> = first_at.keys().copied().collect();
    let mut raw_spellings: BTreeMap<u32, Vec<(i32, Option<String>)>> = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        dated_spellings(query, stretch.run, stretch.first, stretch.last, &ids, &mut raw_spellings)?;
    }
    for steps in raw_spellings.values_mut() {
        steps.sort_by_key(|(hour, _)| *hour);
        let mut compacted: Vec<(i32, Option<String>)> = Vec::with_capacity(steps.len());
        for (hour, text) in std::mem::take(steps) {
            if let Some(last) = compacted.last_mut() && last.0 == hour { last.1 = text; }
            else if compacted.last().is_none_or(|last| last.1 != text) { compacted.push((hour, text)); }
        }
        *steps = compacted;
    }
    let mut change_hours = BTreeSet::new();
    for (&id, &start) in &first_at {
        change_hours.insert(start);
        change_hours.extend(raw_spellings.get(&id).into_iter().flatten().filter(|(at, _)| *at > start).map(|(at, _)| *at));
    }
    let content = Content::load(source, context);
    let mut active: BTreeMap<u32, Option<String>> = BTreeMap::new();
    for hour in change_hours {
        for (&id, &start) in &first_at {
            if start <= hour {
                active.insert(id, raw_spellings.get(&id).and_then(|s| s[..s.partition_point(|(at, _)| *at <= hour)].last()).and_then(|(_, t)| t.clone()));
            }
        }
        let mut warnings = Vec::new();
        let named = content.name(source, &active, &BTreeSet::new(), &mut warnings);
        if out.warnings.last().is_none_or(|(_, held)| *held != warnings) { out.warnings.push((hour, warnings)); }
        for kind in named.types {
            let label = kind.name.or(kind.token).unwrap_or_else(|| "Building name unavailable".into());
            let entry = out.names.entry(kind.id).or_default();
            if entry.last().is_none_or(|(_, held)| *held != label) { entry.push((hour, label)); }
        }
    }
    queue_gaps(query, &mut out.unrecorded)?;
    out.unrecorded.extend(missing(query, out.from, out.to).into_iter().map(|span| (span.first_hour, span.last_hour, HOURS_UNRECORDED.to_owned())));
    Ok(out)
}
