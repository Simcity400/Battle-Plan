
use dashboard_base::products::{Country, validate_country};
use super::{
    BONUSES, INPUTS, Joined, MODIFIERS, ResearchDefinitionText, ResearchProgress, ResearchSlotState, assignment_keys, coverage_keys,
    grants::{self, GrantInventory},
    slice_key, slot_read, valid_inputs,
};
#[cfg(any(test, feature = "export-types"))]
use super::grants::{GrantRestriction, ResearchGrant};
use dashboard_base::{Source, products::{LoadContext, artwork, at_hour::{At, AtHour, Keys, Owner, Read}}};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_data::rows::{ResearchAssignmentRow, ResearchCoverageRow, ResearchInputRow, ResearchModifierRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 8] = [
    TableId::CountryLinks,
    TableId::ResearchCoverage,
    TableId::ResearchAssignments,
    TableId::SliceReads,
    TableId::ResearchInputs,
    TableId::ResearchAppliedBonuses,
    TableId::ResearchModifiers,
    TableId::LexicalTokens,
];

pub struct ResearchSlots;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchSlotsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotNow {
    pub index: u32,
    #[doc = " As read: `firstHour` and `lastHour` are the read's hour."]
    pub state: ResearchSlotState,
    #[doc = " The joined assignment's progress at the read; none while idle."]
    pub progress: Option<ResearchProgress>,
    #[doc = " The text of the definition an unjoined read names, looked up by its template; none when idle or joined."]
    pub definition: Option<ResearchDefinitionText>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotsReply {
    pub tag: String,
    #[doc = " The holder's research read the slots are from."]
    pub read_hour: Option<i32>,
    #[doc = " Why no slot is known at the hour, when none is."]
    pub missing: Option<String>,
    pub slots: Vec<ResearchSlotNow>,
    pub grants: GrantInventory,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ResearchSlotsReply);

pub type Content = Result<Arc<artwork::Context>, String>;

fn owners(frame: &DecodedFrame, owner: Owner) -> Option<Vec<u32>> {
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    Some(links.iter().filter(|link| link.canonical_slot == owner.slot).map(|link| link.id).collect())
}

fn owned(owners: &[u32], shift: u32) -> Keys {
    Keys::Ranges(owners.iter().map(|&owner| { let low = u128::from(owner) << shift; (low, low | ((1u128 << shift) - 1)) }).collect())
}

fn spelled(rows: &[ResearchModifierRow], owners: &[u32]) -> Vec<u128> {
    rows.iter().filter(|row| row.index == grants::INVENTORY_INDEX && owners.contains(&row.country) && grants::spells(row)).filter_map(|row| u32::try_from(row.raw).ok()).map(u128::from).collect()
}

impl AtHour for ResearchSlots {
    const KEY: &'static str = "research-slots";
    const TITLE: &'static str = "Research";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ResearchSlotsParams;
    type Reply = ResearchSlotsReply;
    type Context = Content;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ResearchSlotsParams::decl(),
            ResearchSlotsReply::decl(),
            ResearchSlotNow::decl(),
            ResearchSlotState::decl(),
            ResearchProgress::decl(),
            super::ResearchDetails::decl(),
            super::ResearchModifiers::decl(),
            super::ResearchModifier::decl(),
            super::ResearchModifierSource::decl(),
            super::ResearchBonus::decl(),
            GrantInventory::decl(),
            ResearchGrant::decl(),
            GrantRestriction::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn compares_previous(table: TableId) -> bool { table == TableId::SliceReads }


    const CLOCK_DEPENDENT: bool = false;

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> bool {
        match stage {
            1 => edits.fields(TableId::CountryLinks, u64::MAX),
            2 => edits.fields(TableId::CountryLinks, u64::MAX) || edits.fields(TableId::ResearchModifiers, u64::MAX),
            _ => false,
        }
    }

    fn advance(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        if !edits.replaced.is_empty() || !data["missing"].is_null() { return Ok(false); }
        let table = TableId::ResearchInputs;
        let moving = dashboard_base::products::at_hour::field_mask(table, &["progress_raw", "banked_days_raw", "applied_days_raw"]);
        for (&t, changes) in &edits.tables {
            if t != TableId::SliceReads && (t != table || changes.values().any(|mask| mask & !moving != 0)) { return Ok(false); }
        }
        let Ok(inputs) = frame.complete::<tables::ResearchInputs>() else { return Ok(false) };
        let Ok(samples) = frame.complete::<tables::SliceReads>() else { return Ok(false) };
        let Some(input) = inputs.first() else { return Ok(false) };
        let Some(read) = replay_data::sliced::read_hour(samples, table, input.country) else { return Ok(false) };
        if read > at.hour { return Ok(false); }
        let Some(previous) = edits.previous.get(&TableId::SliceReads) else { return Ok(false) };
        let replay_frame::frame::TableView::Complete(previous) = previous.held() else { return Ok(false) };
        let previous = previous.rows::<tables::SliceReads>().unwrap();
        let Some(old_read) = data["readHour"].as_i64().and_then(|h| i32::try_from(h).ok()) else { return Ok(false) };
        let sample = replay_data::sliced::read_hour;
        if inputs.iter().any(|r| sample(samples, table, r.country) != Some(read))
            || samples.iter().chain(previous).filter(|r| r.table == BONUSES || r.table == MODIFIERS).any(|r| {
                let t = if r.table == BONUSES { TableId::ResearchAppliedBonuses } else { TableId::ResearchModifiers };
                (sample(previous, t, r.owner) == Some(old_read)) != (sample(samples, t, r.owner) == Some(read))
            }) { return Ok(false); }
        data["readHour"] = read.into();
        let Some(slots) = data["slots"].as_array_mut() else { return Ok(false) };
        let assignments = frame.complete::<tables::ResearchAssignments>()?;
        let mods = frame.complete::<tables::ResearchModifiers>().ok();
        for r in inputs {
            let index = r.index;
            let key = (u128::from(r.country) << 32) | u128::from(index);
            if old_read == read && !edits.tables.get(&table).is_some_and(|keys| keys.contains_key(&key)) { continue; }
            let Some(slot) = slots.get_mut(index as usize) else { return Ok(false) };
            slot["state"]["firstHour"] = read.into();
            slot["state"]["lastHour"] = read.into();
            if !slot["progress"].is_null() {
                slot["progress"]["firstHour"] = read.into();
                slot["progress"]["lastHour"] = read.into();
            }
            let joined = assignments.iter().find(|a| a.index == index && a.lane == 0 && a.outcome == ResearchAssignmentRow::ACTIVE && r.technology_present != 0 && r.database_index == replay_data::rows::Recorded::Value(a.database_index) && r.token == a.token);
            let rate = mods.filter(|_| sample(samples, TableId::ResearchModifiers, r.country) == Some(read)).and_then(|m| super::modifiers::breakdown(r, Some(m))).and_then(|m| super::modifiers::rate(&m, r.ahead_reduction_raw, read))
                .or_else(|| joined.filter(|a| a.started_after_hour.is_some() && read - a.first_observed_hour < 24).map(|_| i128::from(r.research_speed_raw)));
            let estimate = if r.technology_present != 0 { rate.and_then(|v| super::remaining(r, v)) } else { None };
            slot["state"]["details"]["remainingDays"] = estimate.map(|(days, _)| days).into();
            slot["state"]["details"]["estimateRaw"] = estimate.map(|(_, raw)| raw.to_string()).into();
            slot["state"]["bankedDaysRaw"] = r.banked_days_raw.to_string().into();
            slot["state"]["appliedDaysRaw"] = r.applied_days_raw.to_string().into();
            if !slot["progress"].is_null() { slot["progress"]["progressRaw"] = r.progress_raw.to_string().into(); }
        }
        Ok(true)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => {
                let (low, high) = coverage_keys(owner.slot, owner.id);
                let (first, last) = assignment_keys(owner.slot, owner.id);
                vec![
                    Read::all(TableId::CountryLinks),
                    Read { table: TableId::ResearchCoverage, keys: Keys::Range(low, high) },
                    Read { table: TableId::ResearchAssignments, keys: Keys::Range(first, last) },
                ]
            }
            1 => {
                let Some(owners) = owners(held, owner).filter(|owners| !owners.is_empty()) else { return Vec::new() };
                vec![
                    Read::these(TableId::SliceReads, owners.iter().flat_map(|&raw| [INPUTS, BONUSES, MODIFIERS].map(|table| slice_key(table, raw)))),
                    Read { table: TableId::ResearchInputs, keys: owned(&owners, 32) },
                    Read { table: TableId::ResearchAppliedBonuses, keys: owned(&owners, 64) },
                    Read { table: TableId::ResearchModifiers, keys: owned(&owners, 64) },
                ]
            }
            2 => {
                let (Some(owners), Ok(rows)) = (owners(held, owner), held.complete::<tables::ResearchModifiers>()) else { return Vec::new() };
                let tokens = spelled(rows, &owners);
                if tokens.is_empty() { Vec::new() } else { vec![Read::these(TableId::LexicalTokens, tokens)] }
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(super::content(source, load).map_err(|error| error.to_string()))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Research country is unavailable at this hour"))?;
        let mut reply = ResearchSlotsReply { tag: at.holders.name(&params.country, at.hour), read_hour: None, missing: None, slots: Vec::new(), grants: GrantInventory::unavailable("No research read covers this hour"), warnings: Vec::new() };
        let localise = |key: &str| at.context.as_ref().ok().and_then(|art| art.localise(key));
        if let Err(error) = at.context {
            reply.warnings.push(format!("Research names are unavailable: {error}"));
        }
        match slots(frame, owner, at.hour, &localise, at.live) {
            Ok((read, mut slots, inventory)) => {
                defined(&mut slots, &|token, index| at.context.as_ref().map_or((None, None), |art| art.definition_text(token, index)));
                reply.read_hour = Some(read);
                reply.slots = slots;
                reply.grants = inventory;
            }
            Err(missing) => reply.missing = Some(missing),
        }
        Ok(reply)
    }
}

pub(crate) fn defined(slots: &mut [ResearchSlotNow], text: &dyn Fn(&str, u32) -> (Option<String>, Option<String>)) {
    for slot in slots {
        let state = &slot.state;
        slot.definition = match (&state.token, state.database_index, state.entry_first_observed_hour) {
            (Some(token), Some(index), None) => u32::try_from(index).ok().map(|index| {
                let (name, description) = text(token, index);
                ResearchDefinitionText { token: token.clone(), database_index: index, name, description }
            }),
            _ => None,
        };
    }
}

fn progress_cell(input: &ResearchInputRow, live: &Cells) {
    let (Ok(progress), Ok(cost)) = (dashboard_declare::field(TableId::ResearchInputs, "progress_raw"), dashboard_declare::field(TableId::ResearchInputs, "cost_raw")) else { return };
    let key = u128::from(input.country) << 32 | u128::from(input.index);
    let source = |column: usize| Cell { table: TableId::ResearchInputs, column, key };
    let row = input.index.to_string();
    if input.cost_debug != 0 {
        live.value(&row, "progress", source(progress), 1e-5);
    } else if input.cost_scale_raw > 0 {
        live.ratio(&row, "progress", source(progress), source(cost), 100_000.0 / input.cost_scale_raw as f64);
    }
}

pub(crate) fn slots(frame: &DecodedFrame, owner: Owner, hour: i32, localise: &dyn Fn(&str) -> Option<String>, live: &Cells) -> Result<(i32, Vec<ResearchSlotNow>, GrantInventory), String> {
    let unread = |table: &str, error: io::Error| format!("{table} is not recorded at this hour: {error}");
    let coverage = frame.complete::<tables::ResearchCoverage>().map_err(|error| unread("Research coverage", error))?;
    if !coverage.iter().any(|row| row.holder_slot == owner.slot && row.holder_id == owner.id && row.member == ResearchCoverageRow::HOLDER) {
        return Err("No research read covers this hour".into());
    }
    let owners = owners(frame, owner).ok_or("Country links are not recorded at this hour")?;
    let samples = frame.complete::<tables::SliceReads>().map_err(|error| unread("Research read hours", error))?;
    let sample = |table: u32, raw: u32| samples.iter().find(|row| row.table == table && row.owner == raw).map(|row| row.sample_hour);
    let inputs = frame.complete::<tables::ResearchInputs>().map_err(|error| unread("Research slots", error))?;
    let mine: Vec<&ResearchInputRow> = inputs.iter().filter(|row| owners.contains(&row.country)).collect();
    let mut reads = mine.iter().map(|row| sample(INPUTS, row.country));
    let read = match reads.next() {
        Some(first) => {
            let first = first.ok_or("The research read of these slots is not recorded")?;
            if !reads.all(|read| read == Some(first)) {
                return Err("This country's research slots were not read together".into());
            }
            first
        }
        None => owners.iter().filter_map(|&raw| sample(INPUTS, raw)).max().ok_or("No research read of this country is recorded")?,
    };
    if read > hour {
        return Err("No research read of this country is recorded by this hour".into());
    }
    if !valid_inputs(&mine) {
        return Err("The research read of these slots cannot be observed".into());
    }
    let assignments = frame.complete::<tables::ResearchAssignments>().map_err(|error| unread("Research assignments", error))?;
    let active: BTreeMap<u32, &ResearchAssignmentRow> =
        assignments.iter().filter(|row| row.holder_slot == owner.slot && row.holder_id == owner.id && row.lane == 0 && row.outcome == ResearchAssignmentRow::ACTIVE).map(|row| (row.index, row)).collect();
    let bonuses = frame.complete::<tables::ResearchAppliedBonuses>().ok();
    let modifiers = frame.complete::<tables::ResearchModifiers>().ok();
    let mut out = Vec::with_capacity(mine.len());
    for input in &mine {
        let joined = active
            .get(&input.index)
            .filter(|row| input.technology_present != 0 && input.database_index == replay_data::rows::Recorded::Value(row.database_index) && row.token == input.token)
            .map(|row| Joined { first_observed: row.first_observed_hour, fresh: row.started_after_hour.is_some() && read - row.first_observed_hour < 24 });
        let with = |table: u32| sample(table, input.country) == Some(read);
        let slot = slot_read(input, read, read, joined, bonuses.filter(|_| with(BONUSES)), modifiers.filter(|_| with(MODIFIERS)), None, localise);
        if slot.progress.as_ref().is_some_and(|progress| progress.base_days_raw.is_some()) {
            progress_cell(input, live);
        }
        out.push(ResearchSlotNow { index: input.index, state: slot.state, progress: slot.progress, definition: None });
    }
    let inventory = match modifiers {
        None => GrantInventory::unavailable("Research grants are not recorded at this hour"),
        Some(rows) => {
            let held: Vec<&ResearchModifierRow> = rows.iter().filter(|row| row.index == grants::INVENTORY_INDEX && owners.contains(&row.country)).collect();
            if held.iter().any(|row| sample(MODIFIERS, row.country) != Some(read)) {
                GrantInventory::unavailable("Research grants were not read with this country's slots")
            } else {
                let tokens = frame.complete::<tables::LexicalTokens>().ok();
                let spelled = grants::assemble(&held, |id| {
                    let rows = tokens.ok_or_else(|| format!("Research grant restriction token {id} is not recorded"))?;
                    let at = rows.binary_search_by_key(&id, |row| row.token).map_err(|_| format!("Research grant restriction token {id} is not recorded"))?;
                    String::from_utf8(rows[at].literal.clone()).map_err(|_| format!("Research grant restriction token {id} has invalid text"))
                });
                match spelled {
                    Ok(inventory) => grants::localised(inventory, localise),
                    Err(reason) => GrantInventory::unavailable(reason),
                }
            }
        }
    };
    Ok((read, out, inventory))
}
