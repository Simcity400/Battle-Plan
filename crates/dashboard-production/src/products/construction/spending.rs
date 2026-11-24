
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

pub(crate) fn deltas(steps: &[(i32, Option<Cell>)], from: i32) -> Vec<Delta> {
    let mut out = Vec::new();
    let mut held: Option<i64> = None;
    let mut lost = false;
    for &(hour, cell) in steps {
        match cell {
            Some(Cell::Number(value)) => {
                if lost {
                    continue;
                }
                if hour >= from {
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
            for (key, steps) in &history.keys {
                let cells: Vec<(i32, Option<Cell>)> = steps.iter().map(|step| (step.hour, step.cells.as_ref().map(|cells| cells[0]))).collect();
                by_leaf.entry(leaf_of(*key)).or_default().extend(deltas(&cells, stretch.first));
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
    pub names: HashMap<u32, String>,
    pub warnings: Vec<String>,
}

const BUILDINGS_UNREAD: &str = "The building construction queue was not read in full, so building output in those hours is not counted";
const RAILWAYS_UNREAD: &str = "The railway construction queue was not read in full, so railway output in those hours is not counted";
const HOURS_UNRECORDED: &str = "Output in hours that were not recorded is not counted";

pub(crate) fn dated_gaps(steps: &[(i32, Option<u8>, bool)], last: i32, final_reason: u8, had_output: bool) -> Vec<(i32, SpendingGap)> {
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
    push(last, gap_of(final_reason), had_output);
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

type ReasonSteps = HashMap<u64, (Vec<(i32, Option<u8>, bool)>, Option<i32>)>;

fn reason_steps(query: &Query<'_>, run: u32, ids: &[u128], first: i32, last: i32) -> io::Result<ReasonSteps> {
    let columns = [col::OUTPUT_REASON, col::OUTPUT_SINCE, col::FIRST_REPAIR_TICK];
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(query.replay.recording(), run, TableId::ConstructionProjects, &columns, Keys::These(ids), first, last))?;
    let at = |column: usize| history.columns.binary_search(&column).map_err(|_| dashboard_core::invalid("A construction project column was not read"));
    let (reason, since, repair) = (at(columns[0])?, at(columns[1])?, at(columns[2])?);
    let number = |cells: &[Cell], at: usize| match cells.get(at) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    };
    let mut out = ReasonSteps::new();
    for (key, steps) in &history.keys {
        let held = out.entry(*key as u64).or_default();
        for step in steps {
            let Some(cells) = step.cells.as_deref() else { continue };
            held.0.push((step.hour, number(cells, reason).and_then(|value| u8::try_from(value).ok()), number(cells, since).is_some()));
            if held.1.is_none() {
                held.1 = number(cells, repair).and_then(|value| i32::try_from(value).ok());
            }
        }
    }
    Ok(out)
}

pub(crate) fn facts(source: &Source, query: &Query<'_>, country: &Country, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Facts> {
    let names = Names::new(&query.holders);
    let mut out = Facts { from: query.timeline.first, to: query.timeline.last, spent: spent_by_leaf(query, country)?, ..Facts::default() };
    let mut seen = BTreeSet::new();
    let mut tokens = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lines = crate::products::construction::cached_construction(source, stretch.run)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let mut kept: Vec<(&Line, bool)> = Vec::new();
        for line in lines.within(stretch.first, stretch.last) {
            if !seen.insert(line.id) {
                continue;
            }
            if line.output_alone && gap_of(line.output_reason) == Some(SpendingGap::OwnerUnknown) {
                kept.push((line, false));
            } else if names.tag_at(&links, line.country, line.first_read).is_some_and(|tag| query.holders.goes_by(country, tag, line.first_read)) {
                kept.push((line, true));
            }
        }
        let ids: Vec<u128> = kept.iter().map(|(line, _)| u128::from(line.id)).collect();
        let steps = if ids.is_empty() { ReasonSteps::new() } else { reason_steps(query, stretch.run, &ids, stretch.first, stretch.last)? };
        let mut ids = BTreeSet::new();
        for (line, attributed) in kept {
            let (read, repair_tick) = steps.get(&line.id).map(|(read, tick)| (read.as_slice(), *tick)).unwrap_or_default();
            let gaps = dated_gaps(read, line.last, line.output_reason, line.had_output);
            if !attributed {
                out.unattributed.push(gaps.iter().find(|gap| gap.1 == SpendingGap::OwnerUnknown).map_or(line.first, |gap| gap.0));
                continue;
            }
            ids.extend(line.building);
            let leaf = leaf_keys(line)[0];
            out.lines.push(LineFacts {
                leaf: (leaf.1, leaf.2, leaf.3, leaf.4),
                building: line.building,
                new_from: (line.new_work || !line.repair_work).then_some(line.first_read),
                repair_from: line.repair_work.then(|| repair_tick.unwrap_or(line.last).max(line.first_read)),
                gaps,
                before_first_read: line.before_observation.then_some(line.first_read),
            });
        }
        for key in out.spent.keys() {
            ids.extend(named(key.2));
        }
        super::spellings(query, stretch.run, stretch.last, &ids, &mut tokens)?;
    }
    queue_gaps(query, &mut out.unrecorded)?;
    out.unrecorded.extend(missing(query, out.from, out.to).into_iter().map(|span| (span.first_hour, span.last_hour, HOURS_UNRECORDED.to_owned())));
    let named = Content::load(source, context).name(source, &tokens, &BTreeSet::new(), &mut out.warnings);
    out.names = named.types.into_iter().filter_map(|kind| kind.name.or(kind.token).map(|name| (kind.id, name))).collect();
    Ok(out)
}
