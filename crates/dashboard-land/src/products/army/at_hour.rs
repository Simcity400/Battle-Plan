use super::{ArmyAttribution, group_info, group_key};
use crate::products::forces::{
    ForceAtReply, ForcePart, Named,
    at_hour::{Chosen, ForceContext, attach_parts, chosen, chosen_reads, chosen_set, frame_literals, frame_rows, load_context, load_context_for, parts_keys, recorded_value, select_explicit, select_population, validate_group},
    validate_selection,
};
use content_reader_script::sub_units::SubUnits;
use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour, Clock, Owner, Read, ReadsAt},
    },
};
use dashboard_query::{coverage::population, series::ranked::GroupSelection};
use replay_data::rows::{ArmyMixRow, ArmyTotalRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, Keys};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ArmyAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmyAtParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The group whose members at the hour are the rows; never with countries or a top."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    pub count_for: ArmyAttribution,
}

fn name_army_groups(frame: &DecodedFrame, tokens: &BTreeSet<u128>, catalogue: Option<&Catalogue>, warnings: &mut Vec<String>) -> Named {
    let literals = frame_literals(frame);
    let units: Option<Arc<SubUnits>> = catalogue.and_then(|catalogue| {
        match catalogue
            .content
            .layers
            .derived::<catalogue::CachedResult<SubUnits>>()
            .get_or_init(|| content_reader_key_names::unit_leader::sub_units(&catalogue.content.layers))
        {
            Ok(units) => {
                warnings.extend(units.warnings().iter().cloned());
                Some(units)
            }
            Err(error) => {
                warnings.push(format!("Unit-group reader refused: {error}"));
                None
            }
        }
    });
    let mut resolved = BTreeMap::new();
    let mut seen_names = BTreeSet::new();
    let mut unresolved = 0usize;
    for &part in tokens {
        let token = part as u32;
        let name = literals.get(&token).cloned().unwrap_or_else(|| format!("Unresolved sub-unit token {token}"));
        let (key, info, warning) = group_info(units.as_deref(), catalogue, &name);
        let is_unresolved = warning.is_some() || key.starts_with("unknown:");
        if seen_names.insert(name)
            && let Some(warning) = warning
        {
            unresolved += 1;
            warnings.push(warning);
        }
        resolved.insert(part, ForcePart { key, name: Some(info.name), unresolved: is_unresolved });
    }
    if unresolved > 0 {
        warnings.push(format!("{unresolved} sub-unit group/name lookups are unresolved or refused; battalions remain counted."));
    }
    let groups = units.as_deref().into_iter().flat_map(|units| units.in_content_order().filter_map(|unit| Some(group_key(units.group(unit).ok()??)))).collect::<Vec<_>>();
    Named::group(resolved).in_content_order(groups)
}

impl AtHour for ArmyAt {
    const KEY: &'static str = "army-at";
    const TITLE: &'static str = "Army size";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: Clock = Clock::Boundaries;
    type Params = ArmyAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, group, countFor, "countries[]"];
    type Reply = ForceAtReply;
    type Context = ForceContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ArmyAtParams::decl(), ForceAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_selection(&params.branch_id, &params.countries, params.top.map(|n| n as usize))?;
        validate_group(params.group.as_ref(), &params.countries, params.top)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::ArmyTotals, TableId::ArmyMix, TableId::LexicalTokens]
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::ArmyTotals), Read::all(TableId::ArmyMix)],
            1 => {
                let attr_code = u8::from(params.count_for == ArmyAttribution::Host);
                let tokens: Vec<u128> = frame_rows::<tables::ArmyMix>(held)
                    .into_iter()
                    .flatten()
                    .filter(|row| row.attribution == attr_code)
                    .map(|row| u128::from(row.token))
                    .collect();
                if tokens.is_empty() { Vec::new() } else { vec![Read::these(TableId::LexicalTokens, tokens)] }
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(load_context(source, load, "Unit-group content"))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        load_context_for(source, load, "Unit-group content", &params.branch_id, params.group.as_ref())
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        match (step, chosen_set(&params.countries, params.group.as_ref(), &at.context.groups, at.holders, at.hour)) {
            (0, Some(slots)) => chosen_reads(&slots, TableId::ArmyTotals, &[(TableId::ArmyMix, 40)]),
            _ => Self::reads(params, owner, step, held),
        }
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.groups.valid_through(params.group.as_ref(), at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = at.context.warnings.clone();
        let total_of = |row: &ArmyTotalRow| match params.count_for {
            ArmyAttribution::Owner => recorded_value(row.owner_battalions),
            ArmyAttribution::Host => recorded_value(row.host_battalions),
        };
        let selected = match chosen(&params.countries, params.group.as_ref(), &at.context.groups, frame.hour)? {
            Chosen::Refused(reason) => return Ok(ForceAtReply { parts: Vec::new(), ordinals: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
            Chosen::Population => {
                let totals = match population::<tables::ArmyTotals>(frame, &Keys::All) {
                    Ok(rows) => rows,
                    Err(reason) => return Ok(ForceAtReply { parts: Vec::new(), ordinals: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
                };
                match select_population(totals, |row| row.slot, total_of, |_| None, at.holders, frame.hour, params.top, "battalions") {
                    Ok(rows) => rows,
                    Err(reason) => return Ok(ForceAtReply { parts: Vec::new(), ordinals: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
                }
            }
            Chosen::These(countries) => select_explicit::<tables::ArmyTotals>(frame, &countries, total_of, |_| None, None, at.holders, frame.hour, "battalions"),
        };
        let selected_slots: BTreeSet<u32> = selected.iter().filter_map(|row| row.slot).collect();
        let attr_code = u8::from(params.count_for == ArmyAttribution::Host);
        let mut met = BTreeSet::new();
        let mut by_slot: HashMap<u32, Vec<(u128, Option<i64>)>> = HashMap::new();
        let parts_error = match population::<tables::ArmyMix>(frame, &parts_keys(&params.countries, params.group.as_ref(), at, 40)) {
            Ok(rows) => {
                for row in rows {
                    let row: &ArmyMixRow = row;
                    if row.attribution != attr_code || !selected_slots.contains(&row.slot) {
                        continue;
                    }
                    let part = u128::from(row.token);
                    met.insert(part);
                    by_slot.entry(row.slot).or_default().push((part, Some(row.battalions)));
                }
                None
            }
            Err(reason) => Some(reason),
        };
        let named = if parts_error.is_some() { Named::default() } else { name_army_groups(frame, &met, at.context.catalogue.as_deref(), &mut warnings) };
        let ordinals = named.ordinals();
        let (parts, rows, extra_warnings) = attach_parts(selected, named, &by_slot, parts_error);
        warnings.extend(extra_warnings);
        let mut seen = BTreeSet::new();
        warnings.retain(|warning| seen.insert(warning.clone()));
        Ok(ForceAtReply { parts, ordinals, rows, missing: None, warnings })
    }
}
