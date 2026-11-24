
use dashboard_base::products::Country;
use super::{COST_SCALE, Content, ConstructionStateName, ConstructionType, Line, ProjectKind, SpendingGap, gap_of, missing};
use dashboard_base::{Source, products::LoadContext, queries::country_ids::Names};
use dashboard_query::{Query, identity::Holder, series::MissingSpan};
use replay_lanes::lanes::{Cell, Keys};
use replay_data::rows::{TableId, TagHolderRow, key_field};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendingUnknown {
    pub known_at: i32,
    pub reason: SpendingGap,
    pub project: String,
    pub until: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeafProject {
    pub project: String,
    pub first_read: i32,
    pub until: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendingLeaf {
    pub repair: bool,
    pub kind: ProjectKind,
    #[doc = " The building type's lexer id ([`ConstructionType::id`])."]
    pub building: Option<u32>,
    pub state: Option<u32>,
    pub to_state: Option<u32>,
    #[doc = " In hour order."]
    pub spent: Vec<SpentStep>,
    pub unknown: Vec<SpendingUnknown>,
    #[doc = " By first read."]
    pub projects: Vec<LeafProject>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConstructionSpendingReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " Units per factory output point of a total."]
    pub cost_scale: u32,
    #[doc = " Either construction queue was read in the branch."]
    pub recorded: bool,
    pub types: Vec<ConstructionType>,
    pub repair_icon: Option<String>,
    pub states: Vec<ConstructionStateName>,
    #[doc = " The tag's lines whose output before observation began is not counted."]
    pub before_first_read: u32,
    pub leaves: Vec<SpendingLeaf>,
    #[doc = " Output no country can be given: lines named by output alone with no recorded owner."]
    pub unattributed: Vec<SpendingUnknown>,
    #[doc = " Hours of the branch the recording does not hold: output then is not counted."]
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ConstructionSpendingReply);

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

pub(crate) fn history(source: &Source, params: &ConstructionSpendingParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ConstructionSpendingReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
    let names = Names::new(&query.holders);
    let end = query.timeline.last;
    let mut leaves: BTreeMap<LeafKey, SpendingLeaf> = BTreeMap::new();
    for (key, spent) in spent_by_leaf(&query, &params.country)? {
        leaf_mut(&mut leaves, key).spent = spent;
    }
    let mut before_first_read = 0u32;
    let mut unattributed = Vec::new();
    let mut seen = BTreeSet::new();
    let mut tokens = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lines = crate::products::construction::cached_construction(source, stretch.run)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let mut ids = BTreeSet::new();
        for line in lines.within(stretch.first, stretch.last) {
            if !seen.insert(line.id) {
                continue;
            }
            let until = (line.last < end).then_some(line.last + 1);
            if line.output_alone && gap_of(line.output_reason) == Some(SpendingGap::OwnerUnknown) {
                unattributed.push(SpendingUnknown { known_at: line.first, reason: SpendingGap::OwnerUnknown, project: line.id.to_string(), until });
                continue;
            }
            if !names.tag_at(&links, line.country, line.first_read).is_some_and(|tag| query.holders.goes_by(&params.country, tag, line.first_read)) {
                continue;
            }
            before_first_read += u32::from(line.before_observation);
            ids.extend(line.building);
            for key in leaf_keys(line) {
                let leaf = leaf_mut(&mut leaves, key);
                leaf.building = leaf.building.or(line.building);
                leaf.projects.push(LeafProject { project: line.id.to_string(), first_read: line.first_read, until });
                if let Some(gap) = line_gap(line, until) {
                    leaf.unknown.push(gap);
                }
            }
        }
        for leaf in leaves.values() {
            ids.extend(leaf.building);
        }
        super::spellings(&query, stretch.run, stretch.last, &ids, &mut tokens)?;
    }
    let states: BTreeSet<u32> = leaves.values().flat_map(|leaf| [leaf.state, leaf.to_state]).flatten().collect();
    let mut warnings = Vec::new();
    let content = Content::load(source, context).name(source, &tokens, &states, &mut warnings);
    let mut leaves: Vec<SpendingLeaf> = leaves.into_values().collect();
    for leaf in &mut leaves {
        leaf.projects.sort_by_key(|project| project.first_read);
    }
    let recorded = super::recorded(&query)?;
    if recorded && leaves.is_empty() {
        warnings.push(query.holders.say(&params.country, query.timeline.last, |name| format!("{name}: no recorded construction in this branch")));
    }
    let reply = ConstructionSpendingReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.name(&params.country, query.timeline.last),
        cost_scale: COST_SCALE,
        recorded,
        types: content.types,
        repair_icon: content.repair_icon,
        states: content.states,
        before_first_read,
        leaves,
        unattributed,
        missing: missing(&query, query.timeline.first, end),
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

fn leaf_mut(leaves: &mut BTreeMap<LeafKey, SpendingLeaf>, key: LeafKey) -> &mut SpendingLeaf {
    leaves.entry(key).or_insert_with(|| SpendingLeaf {
        repair: key.0,
        kind: if key.1 { ProjectKind::Railway } else { ProjectKind::Building },
        building: named(key.2),
        state: named(key.3),
        to_state: named(key.4),
        spent: Vec::new(),
        unknown: Vec::new(),
        projects: Vec::new(),
    })
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

pub(crate) fn line_gap(line: &Line, until: Option<i32>) -> Option<SpendingUnknown> {
    let reason = gap_of(line.output_reason)?;
    if reason == SpendingGap::NotRecorded && !line.had_output {
        return None;
    }
    let known_at = if reason == SpendingGap::FinalOutputMissing { line.last } else { line.first };
    Some(SpendingUnknown { known_at, reason, project: line.id.to_string(), until })
}
