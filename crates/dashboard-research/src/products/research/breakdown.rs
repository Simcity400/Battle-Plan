
use dashboard_base::products::{Country, validate_country};
use super::{
    BONUSES, INPUTS, Joined, MODIFIERS, ResearchProgress, ResearchSlotState, all_columns, cells_at, continues, number, slice_key, slot_read,
};
use dashboard_base::{Source, products::{LoadContext, Product}};
use dashboard_text::metadata::check_cancel;
use dashboard_query::{Query, cost, identity::Holder};
use replay_lanes::lanes::{self, Cell, History, Keys};
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{Recorded, ResearchAppliedBonusRow, ResearchInputRow, ResearchModifierRow, TableId},
};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ResearchBreakdown;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchBreakdownPart {
    pub slot: u32,
    pub first_observed_hour: i32,
    pub last_hour: i32,
    pub started: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub parts: Vec<ResearchBreakdownPart>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchBreakdownSlot {
    pub slot: u32,
    pub first_observed_hour: i32,
    pub states: Vec<ResearchSlotState>,
    pub progress: Vec<ResearchProgress>,
    #[doc = " Why no read of it is known, when none is."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchBreakdownReply {
    pub parts: Vec<ResearchBreakdownSlot>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ResearchBreakdownReply);

impl Product for ResearchBreakdown {
    const KEY: &'static str = "research-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ResearchBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, "parts[].firstObservedHour": Read, "parts[].lastHour": End, "parts[].slot"];
    type Reply = ResearchBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ResearchBreakdownParams::decl(), ResearchBreakdownReply::decl(), ResearchBreakdownPart::decl(), ResearchBreakdownSlot::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params.parts.is_empty() || params.parts.iter().any(|part| part.last_hour < part.first_observed_hour) {
            return Err("Invalid research breakdown parts");
        }
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| io::Error::other("Invalid branch"))?)?;
        let mut warnings = Vec::new();
        let art = super::content(source, context);
        if let Err(error) = &art {
            warnings.push(format!("Research names are unavailable: {error}"));
        }
        let localise = |key: &str| art.as_ref().ok().and_then(|art| art.localise(key));
        let mut parts = Vec::with_capacity(params.parts.len());
        for part in &params.parts {
            check_cancel(cancelled)?;
            parts.push(match reads(source, &query, &params.country, part, &localise) {
                Ok((states, progress)) => ResearchBreakdownSlot { slot: part.slot, first_observed_hour: part.first_observed_hour, states, progress, missing: None },
                Err(missing) => ResearchBreakdownSlot { slot: part.slot, first_observed_hour: part.first_observed_hour, states: Vec::new(), progress: Vec::new(), missing: Some(missing) },
            });
        }
        Ok(ResearchBreakdownReply { parts, warnings })
    }
}

fn rows_at<'h>(history: &'h History, hour: i32) -> impl Iterator<Item = (u128, &'h [Cell])> + 'h {
    history.keys.iter().filter_map(move |(key, steps)| cells_at(steps, hour).map(|cells| (*key, cells)))
}

fn text(history: &History, cells: &[Cell], at: usize) -> Option<Vec<u8>> {
    history.text(*cells.get(at)?).map(<[u8]>::to_vec)
}

pub(crate) fn input_row(history: &History, key: u128, cells: &[Cell]) -> Option<ResearchInputRow> {
    let n = |at: usize| number(cells, at);
    Some(ResearchInputRow {
        country: (key >> 32) as u32,
        index: key as u32,
        technology_handle: n(0)? as u32,
        technology_present: n(1)? as u8,
        token: text(history, cells, 2).unwrap_or_default(),
        classifier: n(3)? as u32,
        level: n(4)? as i32,
        progress_raw: n(5)?,
        cost_raw: n(6)?,
        cost_scale_raw: n(7)?,
        cost_debug: n(8)? as u8,
        research_speed_raw: n(9)?,
        banked_days_raw: n(10)?,
        applied_days_raw: n(11)?,
        ahead_reduction_raw: n(12)?,
        speed_bonus_raw: n(13)?,
        bonus_count: n(14)? as u32,
        database_index: n(15).map_or(Recorded::NotRecorded, |index| Recorded::Value(index as i32)),
    })
}

pub(crate) fn modifier_row(history: &History, key: u128, cells: &[Cell]) -> Option<ResearchModifierRow> {
    let n = |at: usize| number(cells, at);
    Some(ResearchModifierRow {
        country: (key >> 64) as u32,
        index: (key >> 32) as u32,
        ordinal: key as u32,
        technology_handle: n(0)? as u32,
        kind: n(1)? as u8,
        raw: n(2)?,
        auxiliary: n(3)?,
        count: n(4)? as u32,
        source_id: n(5)? as u32,
        name: text(history, cells, 6).unwrap_or_default(),
    })
}

pub(crate) fn bonus_row(history: &History, key: u128, cells: &[Cell]) -> Option<ResearchAppliedBonusRow> {
    let n = |at: usize| number(cells, at);
    Some(ResearchAppliedBonusRow {
        country: (key >> 64) as u32,
        index: (key >> 32) as u32,
        ordinal: key as u32,
        technology_handle: n(0)? as u32,
        id: n(1)? as i32,
        resolved: n(2)? as u8,
        name: text(history, cells, 3).unwrap_or_default(),
        ahead_reduction_raw: n(4)?,
        speed_bonus_raw: n(5)?,
        cost_reduction_raw: n(6),
    })
}

pub(crate) struct Histories {
    pub samples: History,
    pub inputs: History,
    pub slot_modifiers: History,
    pub shared_modifiers: History,
    pub bonuses: History,
}

fn read_hours(samples: &History, owner: u32, first: i32, last: i32) -> Vec<i32> {
    let key = slice_key(INPUTS, owner);
    let mut reads: Vec<i32> = samples
        .keys
        .iter()
        .filter(|(held, _)| *held == key)
        .flat_map(|(_, steps)| steps.iter().filter_map(|step| step.cells.as_deref().and_then(|cells| number(cells, 0))))
        .map(|read| read as i32)
        .filter(|read| (first..=last).contains(read))
        .collect();
    reads.sort_unstable();
    reads.dedup();
    reads
}

pub(crate) fn assignment(histories: &Histories, owner: u32, index: u32, first: i32, last: i32, started: bool, localise: &dyn Fn(&str) -> Option<String>) -> Result<(Vec<ResearchSlotState>, Vec<ResearchProgress>), String> {
    let reads = read_hours(&histories.samples, owner, first, last);
    let input_key = u128::from(owner) << 32 | u128::from(index);
    let input_at = |hour: i32| {
        histories.inputs.keys.iter().find(|(key, _)| *key == input_key).and_then(|(key, steps)| cells_at(steps, hour).and_then(|cells| input_row(&histories.inputs, *key, cells)))
    };
    let sample_at = |table: u32, hour: i32| {
        histories.samples.keys.iter().find(|(key, _)| *key == slice_key(table, owner)).and_then(|(_, steps)| cells_at(steps, hour)).and_then(|cells| number(cells, 0)).map(|read| read as i32)
    };
    let reference = input_at(first).filter(|input| input.technology_present != 0).ok_or("The slot's read at the assignment's first observation is not recorded")?;
    let mut states: Vec<ResearchSlotState> = Vec::new();
    let mut progress: Vec<ResearchProgress> = Vec::new();
    let mut previous: Option<(i32, ResearchInputRow)> = None;
    for (at, &read) in reads.iter().enumerate() {
        let Some(input) = input_at(read) else { break };
        let same = input.technology_present != 0 && input.technology_handle == reference.technology_handle && input.database_index == reference.database_index && input.level == reference.level && input.token == reference.token;
        if !same {
            break;
        }
        let through = reads.get(at + 1).map_or(last, |next| next - 1).min(read + STAGGER_PERIOD - 1).min(last);
        let mut modifiers: Vec<ResearchModifierRow> = rows_at(&histories.slot_modifiers, read).chain(rows_at(&histories.shared_modifiers, read)).filter_map(|(key, cells)| {
            let history = if (key >> 32) as u32 == u32::MAX { &histories.shared_modifiers } else { &histories.slot_modifiers };
            modifier_row(history, key, cells)
        }).collect();
        modifiers.sort_unstable_by_key(|row| (row.country, row.index, row.ordinal));
        let bonuses: Vec<ResearchAppliedBonusRow> = rows_at(&histories.bonuses, read).filter_map(|(key, cells)| bonus_row(&histories.bonuses, key, cells)).collect();
        let daily = previous
            .as_ref()
            .filter(|(hour, before)| hour.checked_add(24) == Some(read) && before.applied_days_raw == input.applied_days_raw)
            .map(|(_, before)| i128::from(input.progress_raw) - i128::from(before.progress_raw));
        let joined = Joined { first_observed: first, fresh: started && read - first < 24 };
        let slot = slot_read(
            &input,
            read,
            through,
            Some(joined),
            (sample_at(BONUSES, read) == Some(read)).then_some(bonuses.as_slice()),
            (sample_at(MODIFIERS, read) == Some(read)).then_some(modifiers.as_slice()),
            daily,
            localise,
        );
        match states.last_mut() {
            Some(last) if continues(last, &slot.state) => last.last_hour = slot.state.last_hour,
            _ => states.push(slot.state),
        }
        if let Some(sample) = slot.progress {
            match progress.last_mut() {
                Some(held) if held.progress_raw == sample.progress_raw && held.base_days_raw == sample.base_days_raw && previous.is_some() => held.last_hour = sample.last_hour,
                _ => progress.push(sample),
            }
        }
        previous = Some((read, input));
    }
    if states.is_empty() {
        return Err("No research read of the assignment is recorded".into());
    }
    Ok((states, progress))
}

fn reads(source: &Source, query: &Query<'_>, tag: &Country, part: &ResearchBreakdownPart, localise: &dyn Fn(&str) -> Option<String>) -> Result<(Vec<ResearchSlotState>, Vec<ResearchProgress>), String> {
    let first = part.first_observed_hour;
    let run = query.holders.at(tag, first).ok_or("The country is not held at the assignment's first observation")?;
    let Holder::Found { slot, .. } = run.holder else { return Err("The country is not resolved at the assignment's first observation".into()) };
    let stretch = query.timeline.at(first).ok_or("The assignment's first observation is not recorded")?;
    let last = part.last_hour.min(stretch.last).min(run.last);
    let links = dashboard_base::queries::links(source, stretch.run).map_err(|error| error.to_string())?;
    let mut owners: Vec<u32> = links.slots.keys().copied().filter(|&raw| links.slot_at(raw, first) == Some(slot)).collect();
    owners.sort_unstable();
    let read = |table: TableId, keys: Keys| history(query, stretch.run, table, keys, first, last);
    let mut samples: Vec<u128> = owners.iter().flat_map(|&raw| [INPUTS, BONUSES, MODIFIERS].map(|table| slice_key(table, raw))).collect();
    samples.sort_unstable();
    let inputs: Vec<u128> = owners.iter().map(|&raw| u128::from(raw) << 32 | u128::from(part.slot)).collect();
    let samples = read(TableId::SliceReads, Keys::These(&samples))?;
    let inputs = read(TableId::ResearchInputs, Keys::These(&inputs))?;
    let owner = inputs.keys.iter().find(|(_, steps)| cells_at(steps, first).is_some()).map(|(key, _)| (*key >> 32) as u32).ok_or("The slot's read at the assignment's first observation is not recorded")?;
    let scope = |index: u32| (u128::from(owner) << 64 | u128::from(index) << 32, u128::from(owner) << 64 | u128::from(index) << 32 | u128::from(u32::MAX));
    let (low, high) = scope(part.slot);
    let slot_modifiers = read(TableId::ResearchModifiers, Keys::Range(low, high))?;
    let bonuses = read(TableId::ResearchAppliedBonuses, Keys::Range(low, high))?;
    let (low, high) = scope(u32::MAX);
    let shared_modifiers = read(TableId::ResearchModifiers, Keys::Range(low, high))?;
    let histories = Histories { samples, inputs, slot_modifiers, shared_modifiers, bonuses };
    assignment(&histories, owner, part.slot, first, last, part.started, localise)
}

fn history(query: &Query<'_>, run: u32, table: TableId, keys: Keys<'_>, first: i32, last: i32) -> Result<History, String> {
    cost::storage(|| lanes::history(query.replay.recording(), run, table, &all_columns(table), keys, first, last)).map_err(|error| error.to_string())
}
