
use dashboard_base::products::Country;
use super::{Content, Line, SpendingGap, col, gap_of, missing};
use dashboard_base::{Source, products::LoadContext, queries::country_ids::Names};
use dashboard_query::{Query, identity::Holder};
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineFacts {
    pub leaf: (bool, u32, u32, u32),
    pub building: Option<u32>,
    pub new_from: Option<i32>,
    pub repair_from: Option<i32>,
    pub purpose: Vec<(i32, bool)>,
    pub gaps: Vec<(i32, i32, SpendingGap)>,
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

pub(crate) fn gap_assignment(new_from: Option<i32>, repair_from: Option<i32>, hour: i32, until: i32) -> Option<(bool, i32)> {
    let pick = match (new_from, repair_from) {
        (Some(new), Some(repair)) if hour >= new && hour >= repair && repair != new => (repair > new, hour),
        (Some(new), Some(repair)) if hour >= new && hour >= repair => return None,
        (_, Some(repair)) if hour >= repair => (true, hour),
        (Some(new), _) if hour >= new => (false, hour),
        (Some(new), Some(repair)) if repair != new => if repair < new { (true, repair) } else { (false, new) },
        (Some(new), None) => (false, new),
        (None, Some(repair)) => (true, repair),
        _ => return None,
    };
    (pick.1 <= until).then_some(pick)
}

pub(crate) fn gap_purpose(line: &LineFacts, hour: i32, until: i32) -> Option<(bool, i32)> {
    if let Some(&(_, repair)) = line.purpose[..line.purpose.partition_point(|&(at, _)| at <= hour)].last() {
        return Some((repair, hour));
    }
    gap_assignment(line.new_from, line.repair_from, hour, until)
}

const BUILDINGS_UNREAD: &str = "The building construction queue was not read in full, so building output in those hours is not counted";
const RAILWAYS_UNREAD: &str = "The railway construction queue was not read in full, so railway output in those hours is not counted";
const HOURS_UNRECORDED: &str = "Output in hours that were not recorded is not counted";

pub(crate) fn dated_gaps(steps: &[(i32, Option<u8>, bool)], final_state: Option<(i32, u8, bool)>, purpose: &[(i32, bool)]) -> Vec<(i32, i32, SpendingGap)> {
    let mut runs: Vec<(i32, i32, SpendingGap)> = Vec::new();
    let mut open: Option<(i32, SpendingGap)> = None;
    let tail = final_state.map(|(last, reason, output)| (last, Some(reason), output));
    for &(hour, reason, output) in steps.iter().chain(&tail) {
        let active = reason.and_then(gap_of).filter(|&gap| !(gap == SpendingGap::NotRecorded && !output));
        if let Some((start, prev)) = open && active != Some(prev) {
            runs.push((start, hour - 1, prev));
            open = None;
        }
        if open.is_none() && let Some(gap) = active { open = Some((hour, gap)); }
    }
    if let Some((start, gap)) = open { runs.push((start, i32::MAX, gap)); }
    let mut out = Vec::new();
    for (start, end, gap) in runs {
        let mut cursor = start;
        for &(at, _) in purpose.iter().filter(|&&(at, _)| start < at && at <= end) {
            out.push((cursor, at - 1, gap));
            cursor = at;
        }
        out.push((cursor, end, gap));
    }
    out
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
struct LineHistory {
    read: Vec<(i32, Option<u8>, bool)>,
    purpose: Vec<(i32, bool)>,
    buildings: Vec<(i32, Option<u32>)>,
    repair_tick: Option<i32>,
    first_new: Option<i32>,
    first_repair: Option<i32>,
    first_known_owner: Option<i32>,
    last_known_owner: Option<bool>,
    before_observation: Option<i32>,
}

type ReasonSteps = HashMap<u64, LineHistory>;

const NEW_COLS: [usize; 6] = [col::REMAINING, col::ADDED_NEW, col::UNACCOUNTED_NEW, col::POSSIBLE, col::BUILT, col::CONSTRUCTION_RAW];
const REPAIR_COLS: [usize; 5] = [col::REPAIR_LEVELS, col::ADDED_REPAIR, col::UNACCOUNTED_REPAIR, col::REPAIRED, col::REPAIR_RAW];

fn reason_steps(query: &Query<'_>, run: u32, ids: &[u128], first: i32, last: i32) -> io::Result<ReasonSteps> {
    let columns = [
        col::OUTPUT_ALONE, col::COUNTRY, col::BUILDING_TYPE, col::REMAINING, col::REPAIR_LEVELS, col::BUILT, col::REPAIRED, col::ADDED_NEW, col::ADDED_REPAIR, col::POSSIBLE,
        col::UNACCOUNTED_NEW, col::UNACCOUNTED_REPAIR, col::CONSTRUCTION_RAW, col::REPAIR_RAW, col::OUTPUT_SINCE, col::OBSERVED_SINCE_CREATION, col::OUTPUT_REASON, col::FIRST_REPAIR_TICK,
    ];
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(query.replay.recording(), run, TableId::ConstructionProjects, &columns, Keys::These(ids), first, last))?;
    let at = |column: usize| history.columns.binary_search(&column).map_err(|_| dashboard_core::invalid("A construction project column was not read"));
    let (alone, owner, btype, reason, since, obs, repair, rem, rep_lvl) = (
        at(col::OUTPUT_ALONE)?, at(col::COUNTRY)?, at(col::BUILDING_TYPE)?, at(col::OUTPUT_REASON)?, at(col::OUTPUT_SINCE)?,
        at(col::OBSERVED_SINCE_CREATION)?, at(col::FIRST_REPAIR_TICK)?, at(col::REMAINING)?, at(col::REPAIR_LEVELS)?,
    );
    let new_at = NEW_COLS.into_iter().map(at).collect::<io::Result<Vec<_>>>()?;
    let repair_at = REPAIR_COLS.into_iter().map(at).collect::<io::Result<Vec<_>>>()?;
    let number = |cells: &[Cell], at: usize| match cells.get(at) { Some(Cell::Number(value)) => Some(*value), _ => None };
    let unknown_owner = replay_data::rows::ConstructionProjectRow::OUTPUT_OWNER_UNKNOWN;
    let unread_output = replay_data::rows::ConstructionProjectRow::OUTPUT_NOT_RECORDED;
    let mut out = ReasonSteps::new();
    for (key, steps) in &history.keys {
        let held = out.entry(*key as u64).or_default();
        let (mut prev_new, mut prev_repair, mut repairs_cleared) = ([0_i64; NEW_COLS.len()], [0_i64; REPAIR_COLS.len()], false);
        for step in steps {
            let Some(cells) = step.cells.as_deref() else { continue };
            let building = number(cells, btype).and_then(|v| u32::try_from(v).ok());
            if held.buildings.last().is_none_or(|&(_, prev)| prev != building) { held.buildings.push((step.hour, building)); }
            let had_output = number(cells, since).is_some();
            let reason_val = number(cells, reason).and_then(|value| u8::try_from(value).ok());
            let known_owner = number(cells, alone) == Some(0)
                || match reason_val {
                    Some(r) if r == unknown_owner => false,
                    Some(r) if r == unread_output => held.last_known_owner.unwrap_or_else(|| number(cells, owner).is_some_and(|v| v != 0)),
                    Some(_) => true,
                    None => false,
                };
            held.last_known_owner = Some(known_owner);
            if known_owner && held.first_known_owner.is_none() { held.first_known_owner = Some(step.hour); }
            let effective_reason = if had_output && !known_owner { Some(unknown_owner) } else { reason_val };
            held.read.push((step.hour, effective_reason, had_output));
            if held.before_observation.is_none() && had_output && known_owner && number(cells, obs) == Some(0) {
                held.before_observation = Some(step.hour);
            }
            let tick = number(cells, repair).and_then(|value| i32::try_from(value).ok());
            held.repair_tick = held.repair_tick.or(tick);
            let (rep_now, rem_now, prev_rep) = (number(cells, rep_lvl).unwrap_or(0), number(cells, rem).unwrap_or(0), prev_repair[0]);
            let mut new_grew = false;
            for (slot, &column) in new_at.iter().enumerate() {
                if let Some(value) = number(cells, column) {
                    if value > 0 && held.first_new.is_none() { held.first_new = Some(step.hour); }
                    if slot >= 4 && value > prev_new[slot] { new_grew = true; }
                    prev_new[slot] = value;
                }
            }
            let mut repair_grew = tick == Some(step.hour);
            for (slot, &column) in repair_at.iter().enumerate() {
                if let Some(value) = number(cells, column) {
                    if value > 0 && held.first_repair.is_none() { held.first_repair = Some(step.hour); }
                    if slot >= 3 && value > prev_repair[slot] { repair_grew = true; }
                    prev_repair[slot] = value;
                }
            }
            if rep_now > 0 || (prev_rep == 0 && repair_grew) { repairs_cleared = false; } else if prev_rep > 0 { repairs_cleared = true; }
            let active = if rep_now > 0 || (repair_grew && !new_grew) {
                Some(true)
            } else if new_grew && !repair_grew {
                repairs_cleared = false;
                Some(false)
            } else if rem_now > 0 && !repair_grew && (repairs_cleared || held.purpose.last().is_none_or(|&(_, r)| !r)) {
                repairs_cleared = false;
                Some(false)
            } else {
                None
            };
            if let Some(is_repair) = active && held.purpose.last().is_none_or(|&(_, prev)| prev != is_repair) {
                held.purpose.push((step.hour, is_repair));
            }
        }
    }
    Ok(out)
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
        for (repair, start) in [(false, line.new_from), (true, line.repair_from)] {
            if start.is_some() {
                let held = buildings.entry((repair, railway, building, state, to_state)).or_insert(None);
                *held = held.or(line.building);
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
            for &(hour, until, _) in &line.gaps {
                if let Some((_, at)) = gap_purpose(line, hour, until) { note(id, at); }
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

struct SeenLine { line: Line, matched: bool, history: LineHistory }

fn min_opt(a: Option<i32>, b: Option<i32>) -> Option<i32> { [a, b].into_iter().flatten().min() }

pub(crate) fn facts(source: &Source, query: &Query<'_>, country: &Country, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Facts> {
    let names = Names::new(&query.holders);
    let mut out = Facts { from: query.timeline.first, to: query.timeline.last, spent: spent_by_leaf(query, country)?, ..Facts::default() };
    let mut seen: BTreeMap<u64, SeenLine> = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lines = crate::products::construction::cached_construction(source, stretch.run)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let unowned = owner_unknown_lines(query, stretch.run, stretch.first, stretch.last)?;
        let is_owner = |owner: u32, at: i32| match country {
            Country::Slot(slot) => links.slot_at(owner, at) == Some(*slot),
            Country::Tag(_) => names.tag_at(&links, owner, at).is_some_and(|tag| query.holders.goes_by(country, tag, at)),
        };
        let mut kept: Vec<(&Line, bool)> = Vec::new();
        for line in lines.within(stretch.first, stretch.last) {
            let matched = line.country.is_some_and(|owner| is_owner(owner, line.first_read));
            if matched || line.country.is_none() || line.output_alone || unowned.contains(&line.id) { kept.push((line, matched)); }
        }
        let ids: Vec<u128> = kept.iter().map(|(line, _)| u128::from(line.id)).collect();
        let mut steps = if ids.is_empty() { ReasonSteps::new() } else { reason_steps(query, stretch.run, &ids, stretch.first, stretch.last)? };
        for (line, matched) in kept {
            let entry = seen.entry(line.id).or_insert_with(|| SeenLine { line: line.clone(), matched, history: LineHistory::default() });
            entry.matched |= matched;
            entry.line.country = entry.line.country.or(line.country);
            let mut part_last_known = None;
            if let Some(part) = steps.remove(&line.id) {
                part_last_known = part.last_known_owner;
                if let (Some(owner), Some(at)) = (line.country, part.first_known_owner) { entry.matched |= is_owner(owner, at); }
                entry.history.read.extend(part.read);
                entry.history.purpose.extend(part.purpose);
                entry.history.buildings.extend(part.buildings);
                entry.history.repair_tick = min_opt(entry.history.repair_tick, part.repair_tick);
                entry.history.first_new = min_opt(entry.history.first_new, part.first_new);
                entry.history.first_repair = min_opt(entry.history.first_repair, part.first_repair);
                entry.history.before_observation = min_opt(entry.history.before_observation, part.before_observation);
            }
            if stretch.first <= line.last && line.last <= stretch.last {
                let unknown = replay_data::rows::ConstructionProjectRow::OUTPUT_OWNER_UNKNOWN;
                let unread = replay_data::rows::ConstructionProjectRow::OUTPUT_NOT_RECORDED;
                let end_known = !line.output_alone
                    || (line.output_reason != unknown && line.output_reason != unread)
                    || (line.output_reason == unread && part_last_known.unwrap_or(line.country.is_some()));
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
        acc.history.purpose.sort_by_key(|step| step.0);
        acc.history.purpose.dedup_by(|next, prev| next.1 == prev.1);
        acc.history.buildings.sort_by_key(|step| step.0);
        acc.history.buildings.dedup_by(|next, prev| next.1 == prev.1);
        let gaps = dated_gaps(&merged_read, None, &acc.history.purpose);
        let owner_gap = gaps.iter().find(|gap| gap.2 == SpendingGap::OwnerUnknown).map(|gap| gap.0);
        if acc.line.country.is_none() {
            let first_output = merged_read.iter().find(|&&(_, _, had)| had).map(|&(hour, ..)| hour);
            if let Some(hour) = owner_gap.into_iter().chain(first_output).min() { out.unattributed.push(hour); }
            continue;
        }
        if let Some(hour) = owner_gap { out.unattributed.push(hour); }
        if !acc.matched { continue; }
        let gaps: Vec<(i32, i32, SpendingGap)> = gaps.into_iter().filter(|&(_, _, gap)| gap != SpendingGap::OwnerUnknown).collect();
        let leaf = leaf_keys(&acc.line)[0];
        let repair_from = min_opt(acc.history.repair_tick, acc.history.first_repair).map(|hour| hour.max(acc.line.first_read));
        let new_from = acc.history.first_new.map(|hour| hour.max(acc.line.first_read));
        let mut before_first_read = acc.history.before_observation.map(|hour| hour.max(acc.line.first_read));
        let building_at = |hour: i32| acc.history.buildings[..acc.history.buildings.partition_point(|&(at, _)| at <= hour)].last().and_then(|&(_, b)| b);
        let mut by_building: BTreeMap<Option<u32>, Vec<(i32, i32, SpendingGap)>> = BTreeMap::new();
        for (start, end, gap) in gaps {
            let mut cursor = start;
            for &(at, _) in acc.history.buildings.iter().filter(|&&(at, _)| start < at && at <= end) {
                by_building.entry(building_at(cursor)).or_default().push((cursor, at - 1, gap));
                cursor = at;
            }
            by_building.entry(building_at(cursor)).or_default().push((cursor, end, gap));
        }
        if by_building.is_empty() {
            by_building.insert(acc.history.buildings.last().and_then(|&(_, b)| b).or(acc.line.building), Vec::new());
        }
        for (building, gaps) in by_building {
            let bcol = if leaf.1 { TagHolderRow::NONE } else { building.unwrap_or(TagHolderRow::NONE) };
            out.lines.push(LineFacts {
                leaf: (leaf.1, bcol, leaf.3, leaf.4), building, new_from, repair_from,
                purpose: acc.history.purpose.clone(), gaps, before_first_read: before_first_read.take(),
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
