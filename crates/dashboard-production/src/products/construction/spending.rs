
use dashboard_base::products::Country;
use super::{Content, Line, SpendingGap, col, gap_of, missing};
use dashboard_base::{Source, products::LoadContext, queries::country_ids::Names};
use dashboard_query::{Query, identity::Holder};
use replay_lanes::lanes::{Cell, Keys};
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
    pub gaps: Vec<(i32, SpendingGap)>,
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

pub(crate) fn gap_assignment(new_from: Option<i32>, repair_from: Option<i32>, hour: i32) -> Option<(bool, i32)> {
    Some(match (new_from, repair_from) {
        (Some(new), Some(repair)) if hour >= new && hour >= repair => (repair >= new, hour),
        (_, Some(repair)) if hour >= repair => (true, hour),
        (Some(new), _) if hour >= new => (false, hour),
        (Some(new), Some(repair)) => if repair <= new { (true, repair) } else { (false, new) },
        (Some(new), None) => (false, new),
        (None, Some(repair)) => (true, repair),
        (None, None) => return None,
    })
}

const BUILDINGS_UNREAD: &str = "The building construction queue was not read in full, so building output in those hours is not counted";
const RAILWAYS_UNREAD: &str = "The railway construction queue was not read in full, so railway output in those hours is not counted";
const HOURS_UNRECORDED: &str = "Output in hours that were not recorded is not counted";

pub(crate) fn dated_gaps(steps: &[(i32, Option<u8>, bool)], final_state: Option<(i32, u8, bool)>) -> Vec<(i32, SpendingGap)> {
    let mut out: Vec<(i32, SpendingGap)> = Vec::new();
    let mut push = |hour: i32, reason: Option<SpendingGap>, output: bool| {
        if let Some(reason) = reason
            && !(reason == SpendingGap::NotRecorded && !output)
            && out.last().is_none_or(|held| held.1 != reason)
        {
            out.push((hour, reason));
        }
    };
    for &(hour, reason, output) in steps {
        push(hour, reason.and_then(gap_of), output);
    }
    if let Some((last, final_reason, had_output)) = final_state {
        push(last, gap_of(final_reason), had_output);
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
    repair_tick: Option<i32>,
    first_new: Option<i32>,
    first_repair: Option<i32>,
}

type ReasonSteps = HashMap<u64, LineHistory>;

const NEW_COLS: [usize; 6] = [col::REMAINING, col::BUILT, col::ADDED_NEW, col::POSSIBLE, col::UNACCOUNTED_NEW, col::CONSTRUCTION_RAW];
const REPAIR_COLS: [usize; 5] = [col::REPAIR_LEVELS, col::REPAIRED, col::ADDED_REPAIR, col::UNACCOUNTED_REPAIR, col::REPAIR_RAW];

fn reason_steps(query: &Query<'_>, run: u32, ids: &[u128], first: i32, last: i32) -> io::Result<ReasonSteps> {
    let columns = [
        col::REMAINING, col::REPAIR_LEVELS, col::BUILT, col::REPAIRED, col::ADDED_NEW, col::ADDED_REPAIR, col::POSSIBLE,
        col::UNACCOUNTED_NEW, col::UNACCOUNTED_REPAIR, col::CONSTRUCTION_RAW, col::REPAIR_RAW, col::OUTPUT_SINCE, col::OUTPUT_REASON, col::FIRST_REPAIR_TICK,
    ];
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(query.replay.recording(), run, TableId::ConstructionProjects, &columns, Keys::These(ids), first, last))?;
    let at = |column: usize| history.columns.binary_search(&column).map_err(|_| dashboard_core::invalid("A construction project column was not read"));
    let (reason, since, repair) = (at(col::OUTPUT_REASON)?, at(col::OUTPUT_SINCE)?, at(col::FIRST_REPAIR_TICK)?);
    let new_at = NEW_COLS.into_iter().map(at).collect::<io::Result<Vec<_>>>()?;
    let repair_at = REPAIR_COLS.into_iter().map(at).collect::<io::Result<Vec<_>>>()?;
    let number = |cells: &[Cell], at: usize| match cells.get(at) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    };
    let mut out = ReasonSteps::new();
    for (key, steps) in &history.keys {
        let held = out.entry(*key as u64).or_default();
        for step in steps {
            let Some(cells) = step.cells.as_deref() else { continue };
            held.read.push((step.hour, number(cells, reason).and_then(|value| u8::try_from(value).ok()), number(cells, since).is_some()));
            held.repair_tick = held.repair_tick.or_else(|| number(cells, repair).and_then(|value| i32::try_from(value).ok()));
            if held.first_new.is_none() && new_at.iter().any(|&column| number(cells, column).is_some_and(|value| value > 0)) {
                held.first_new = Some(step.hour);
            }
            if held.first_repair.is_none() && repair_at.iter().any(|&column| number(cells, column).is_some_and(|value| value > 0)) {
                held.first_repair = Some(step.hour);
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
    let complete: Vec<(i32, i32)> =
        history.statuses.iter().filter(|held| replay_recording::columnar_frames::complete(held.status)).map(|held| (held.first, held.last)).collect();
    let mut asks = Vec::new();
    for (key, steps) in &history.keys {
        for step in steps {
            match step.cells.as_deref().and_then(|cells| cells.first().copied()) {
                Some(Cell::Written(_)) => asks.push((step.hour, *key)),
                Some(cell) => {
                    let text = history.text(cell).and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned);
                    if text.is_some() || read_within(&complete, step.hour, step.hour) {
                        into.entry(*key as u32).or_default().push((step.hour, text));
                    }
                }
                None if read_within(&complete, step.hour, step.hour) => into.entry(*key as u32).or_default().push((step.hour, None)),
                None => {}
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
                if text.is_some() || read_within(&complete, hour, hour) {
                    into.entry(key as u32).or_default().push((hour, text));
                }
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
        if let (Some(id), Some(step)) = (buildings.get(place).copied().flatten(), steps.first()) {
            note(id, step.known_at);
        }
    }
    for line in lines {
        if let Some(id) = line.building {
            for &(hour, _) in &line.gaps {
                if let Some((_, at)) = gap_assignment(line.new_from, line.repair_from, hour) {
                    note(id, at);
                }
            }
        }
    }
    first_at
}

struct SeenLine {
    line: Line,
    attributed: bool,
    history: LineHistory,
    ends_in_branch: bool,
}

fn min_opt(a: Option<i32>, b: Option<i32>) -> Option<i32> {
    [a, b].into_iter().flatten().min()
}

pub(crate) fn facts(source: &Source, query: &Query<'_>, country: &Country, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Facts> {
    let names = Names::new(&query.holders);
    let mut out = Facts { from: query.timeline.first, to: query.timeline.last, spent: spent_by_leaf(query, country)?, ..Facts::default() };
    let mut seen: BTreeMap<u64, SeenLine> = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lines = crate::products::construction::cached_construction(source, stretch.run)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let mut kept: Vec<(&Line, bool)> = Vec::new();
        for line in lines.within(stretch.first, stretch.last) {
            let Some(owner) = line.country.filter(|_| !(line.output_alone && gap_of(line.output_reason) == Some(SpendingGap::OwnerUnknown))) else {
                kept.push((line, false));
                continue;
            };
            if match country {
                Country::Slot(slot) => links.slot_at(owner, line.first_read) == Some(*slot),
                Country::Tag(_) => names.tag_at(&links, owner, line.first_read).is_some_and(|tag| query.holders.goes_by(country, tag, line.first_read)),
            } {
                kept.push((line, true));
            }
        }
        let ids: Vec<u128> = kept.iter().map(|(line, _)| u128::from(line.id)).collect();
        let mut steps = if ids.is_empty() { ReasonSteps::new() } else { reason_steps(query, stretch.run, &ids, stretch.first, stretch.last)? };
        for (line, attributed) in kept {
            let entry = seen.entry(line.id).or_insert_with(|| SeenLine {
                line: line.clone(),
                attributed,
                history: LineHistory::default(),
                ends_in_branch: false,
            });
            entry.attributed |= attributed;
            entry.ends_in_branch |= stretch.first <= line.last && line.last <= stretch.last;
            if let Some(part) = steps.remove(&line.id) {
                entry.history.read.extend(part.read);
                entry.history.repair_tick = min_opt(entry.history.repair_tick, part.repair_tick);
                entry.history.first_new = min_opt(entry.history.first_new, part.first_new);
                entry.history.first_repair = min_opt(entry.history.first_repair, part.first_repair);
            }
        }
    }
    for mut acc in seen.into_values() {
        acc.history.read.sort_by_key(|step| step.0);
        let gaps = dated_gaps(&acc.history.read, acc.ends_in_branch.then_some((acc.line.last, acc.line.output_reason, acc.line.had_output)));
        if !acc.attributed {
            let owner_gaps = gaps.iter().filter(|gap| gap.1 == SpendingGap::OwnerUnknown).map(|gap| gap.0);
            if acc.line.country.is_some() {
                out.unattributed.push(owner_gaps.min().unwrap_or(acc.line.first));
                continue;
            }
            let first_output = acc.history.read.iter().find(|&&(_, _, had)| had).map(|&(hour, ..)| hour);
            if let Some(hour) = owner_gaps.chain(first_output).min() {
                out.unattributed.push(hour);
            }
            continue;
        }
        let leaf = leaf_keys(&acc.line)[0];
        let repair_from = min_opt(acc.history.repair_tick, acc.history.first_repair).map(|hour| hour.max(acc.line.first_read));
        let new_from = acc.history.first_new.map(|hour| hour.max(acc.line.first_read)).or_else(|| repair_from.is_none().then_some(acc.line.first_read));
        out.lines.push(LineFacts {
            leaf: (leaf.1, leaf.2, leaf.3, leaf.4),
            building: acc.line.building,
            new_from,
            repair_from,
            gaps,
            before_first_read: acc.line.before_observation.then_some(acc.line.first_read),
        });
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
            match compacted.last_mut() {
                Some(last) if last.0 == hour => last.1 = text,
                Some(last) if last.1 == text => {}
                _ => compacted.push((hour, text)),
            }
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
                let text = raw_spellings.get(&id).and_then(|s| s[..s.partition_point(|(at, _)| *at <= hour)].last()).and_then(|(_, t)| t.clone());
                active.insert(id, text);
            }
        }
        let mut warnings = Vec::new();
        let named = content.name(source, &active, &BTreeSet::new(), &mut warnings);
        if out.warnings.last().is_none_or(|(_, held)| *held != warnings) {
            out.warnings.push((hour, warnings));
        }
        for kind in named.types {
            let label = kind.name.or(kind.token).unwrap_or_else(|| "Building name unavailable".into());
            let entry = out.names.entry(kind.id).or_default();
            if entry.last().is_none_or(|(_, held)| *held != label) {
                entry.push((hour, label));
            }
        }
    }
    queue_gaps(query, &mut out.unrecorded)?;
    out.unrecorded.extend(missing(query, out.from, out.to).into_iter().map(|span| (span.first_hour, span.last_hour, HOURS_UNRECORDED.to_owned())));
    Ok(out)
}
