
use dashboard_base::products::Country;
use super::{occupied, slot};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Edits, Owner, Read, ReadsAt, before, field_columns}}};
use dashboard_intelligence::products::intelligence::{Tags, branch, missing};
use dashboard_follow::live::Source as Cell;
use dashboard_query::identity::Holder;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const TOTALS: [(&str, &str); 5] = [("resistanceSum", "resistance_sum"), ("complianceSum", "compliance_sum"), ("states", "states"), ("heldStates", "held_states"), ("heldComplete", "held_complete")];

const COLLABORATION: [TableId; 4] = [TableId::StateResistance, TableId::States, TableId::StateGarrisons, TableId::CountryCollaboration];

pub struct OccupationDetail;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupationDetailParams {
    pub branch_id: String,
    #[doc = " None asks every occupier with a row at the hour."]
    pub countries: Vec<Country>,
    pub collaboration: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationCollaboration {
    pub slot: u32,
    pub tag: Option<String>,
    pub progress_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationDetailRow {
    pub country: Country,
    #[doc = " Why the tag names no occupier at the hour."]
    pub missing: Option<String>,
    #[doc = " By tag; empty unless asked."]
    pub collaboration: Vec<OccupationCollaboration>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationDetailReply {
    pub occupiers: Vec<OccupationDetailRow>,
    #[doc = " Why collaboration is not shown, when asked and not known."]
    pub collaboration_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OccupationDetailReply);

pub(super) fn occupied_by(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, BTreeMap<u32, u32>>> {
    let links = frame.complete::<tables::CountryLinks>()?;
    let controllers: HashMap<u32, u32> = frame.complete::<tables::States>()?.iter().map(|state| (state.id, state.controller)).collect();
    let mut out: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    let mut add = |controller: u32, raw: u32| {
        if let (Some(occupier), Some(occupied)) = (slot(links, controller), slot(links, raw)) {
            out.entry(occupier).or_default().entry(occupied).or_insert(raw);
        }
    };
    for row in frame.complete::<tables::StateResistance>()?.iter().filter(|row| occupied(row)) {
        if let Some(&controller) = controllers.get(&row.state) {
            add(controller, row.country);
        }
    }
    for row in frame.complete::<tables::StateGarrisons>()? {
        add(row.controller, row.occupied);
    }
    Ok(out)
}

impl AtHour for OccupationDetail {
    const KEY: &'static str = "occupation-at";
    const TITLE: &'static str = "Occupation";
    const CONTENT_DEPENDENT: bool = false;
    type Params = OccupationDetailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, collaboration, "countries[]"];
    type Reply = OccupationDetailReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OccupationDetailParams::decl(), OccupationDetailReply::decl(), OccupationDetailRow::decl(), OccupationCollaboration::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.countries.iter().try_for_each(dashboard_base::products::validate_country)
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::OccupierTotals];
        if params.collaboration {
            tables.extend(COLLABORATION);
            tables.extend([TableId::Countries, TableId::CountryLinks]);
        }
        tables
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => {
                let mut reads = Vec::new();
                if params.countries.is_empty() {
                    reads.push(Read::all(TableId::OccupierTotals));
                }
                if params.collaboration {
                    reads.extend([
                        TableId::StateResistance,
                        TableId::StateGarrisons,
                        TableId::Countries,
                        TableId::CountryLinks,
                        TableId::CountryCollaboration,
                    ].into_iter().map(Read::all));
                }
                reads
            }
            1 if params.collaboration => {
                let states: Vec<u128> = held
                    .complete::<tables::StateResistance>()
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter(|row| occupied(row))
                    .map(|row| u128::from(row.state))
                    .collect();
                vec![Read::these(TableId::States, states)]
            }
            _ => Vec::new(),
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        match table {
            TableId::OccupierTotals => Some(field_columns(table, &["resistance_sum", "compliance_sum", "states", "held_states", "held_complete"])),
            TableId::StateResistance => Some(field_columns(table, &["country", "active"])),
            TableId::States => Some(field_columns(table, &["controller"])),
            TableId::StateGarrisons => Some(field_columns(table, &["occupied"])),
            TableId::Countries => Some(field_columns(table, &["canonical_slot", "tag"])),
            TableId::CountryLinks => Some(field_columns(table, &["canonical_slot"])),
            TableId::CountryCollaboration => Some(field_columns(table, &["progress"])),
            _ => None,
        }
    }

    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        match table {
            TableId::OccupierTotals => Some(Vec::new()),
            _ => Self::columns(table),
        }
    }

    fn plan_changes(stage: usize, edits: &Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        stage > 0 && (edits.replaced.contains(&TableId::StateResistance) || edits.tables.contains_key(&TableId::StateResistance))
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        before(at.holders.next_change(frame.hour))
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let fields: Vec<usize> = TOTALS.iter().map(|(_, field)| dashboard_declare::field(TableId::OccupierTotals, field)).collect::<io::Result<_>>()?;
        let mut collaboration_missing = None;
        let mut by: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
        let mut progress: HashMap<(u32, u32), i64> = HashMap::new();
        let mut tags = None;
        let mut warnings = Vec::new();
        if params.collaboration {
            match (|| -> io::Result<_> {
                if let Some(why) = COLLABORATION.into_iter().chain([TableId::Countries, TableId::CountryLinks]).find_map(|table| missing(frame, table)) {
                    return Ok(Err(why));
                }
                let links = frame.complete::<tables::CountryLinks>()?;
                let mut progress = HashMap::new();
                let mut unresolved = BTreeSet::new();
                for row in frame.complete::<tables::CountryCollaboration>()? {
                    match (slot(links, row.source), slot(links, row.target)) {
                        (Some(source), Some(target)) => {
                            progress.insert((source, target), row.progress);
                        }
                        _ => {
                            unresolved.insert((row.source, row.target));
                        }
                    }
                }
                if !unresolved.is_empty() {
                    warnings.push(format!("country collaboration link is unresolved ({} rows)", unresolved.len()));
                }
                Ok(Ok((occupied_by(frame)?, progress, Tags::read(frame)?)))
            })()? {
                Ok((occupiers, read, named)) => {
                    by = occupiers;
                    progress = read;
                    tags = Some(named);
                }
                Err(why) => collaboration_missing = Some(why),
            }
        }
        let every: Vec<Country>;
        let asked = if params.countries.is_empty() {
            let names = dashboard_base::queries::country_ids::Names::new(at.holders);
            let rows = frame.complete::<tables::OccupierTotals>()?;
            every = rows
                .iter()
                .filter_map(|row| names.slot_tag(row.slot, at.hour).filter(|tag| matches!(at.holders.at(tag, at.hour).map(|run| run.holder), Some(Holder::Found { slot, id }) if slot == row.slot && id == row.id)).map(Country::from))
                .collect();
            &every
        } else {
            &params.countries
        };
        let mut occupiers = Vec::with_capacity(asked.len());
        for tag in asked {
            let mut row = OccupationDetailRow { country: tag.clone(), missing: None, collaboration: Vec::new() };
            match at.holders.at(tag, at.hour).map(|run| run.holder) {
                Some(Holder::Found { slot, id }) => {
                    let key = u128::from(slot) << 32 | u128::from(id);
                    for ((column, _), &field) in TOTALS.iter().zip(&fields) {
                        at.live.value(&tag.key(), column, Cell { table: TableId::OccupierTotals, column: field, key }, 1.0);
                    }
                    if let Some(tags) = &tags {
                        row.collaboration = by
                            .get(&slot)
                            .into_iter()
                            .flatten()
                            .map(|(&occupied, &raw)| OccupationCollaboration { slot: occupied, tag: tags.tag(raw), progress_raw: progress.get(&(occupied, slot)).map(i64::to_string) })
                            .collect();
                        row.collaboration.sort_by(|a, b| a.tag.cmp(&b.tag).then(a.slot.cmp(&b.slot)));
                    }
                }
                Some(Holder::Unresolved) | None => row.missing = Some(at.holders.not_known(tag, at.hour).unwrap_or_else(|| "No country holds this tag at this hour".into())),
            }
            occupiers.push(row);
        }
        Ok(OccupationDetailReply { occupiers, collaboration_missing, warnings })
    }
}
