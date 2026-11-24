use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::LoadContext;
use dashboard_base::{Source, products::at_hour::{At, AtHour, Keys, Owner, Read}};
use dashboard_text::catalogue::{View, localised};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const LABELS: [&str; 3] = ["TOPBAR_ARMY", "TOPBAR_AIR", "TOPBAR_NAVY"];

pub const SCALE: u32 = 100_000;

pub struct FuelBranches;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelBranchesParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelConsumerRow {
    #[doc = " Its native consumer index: the series part."]
    pub index: u32,
    pub requested: String,
    pub received: String,
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelBranchesReply {
    pub tag: String,
    pub scale: u32,
    #[doc = " Each consumer index's label from the recorded content; null where it names none."]
    pub consumer_names: Vec<Option<String>>,
    #[doc = " The hour the consumers were last read: they are read once a day."]
    pub read_hour: Option<i32>,
    #[doc = " The country's consumers in index order; none when it requested no fuel."]
    pub consumers: Vec<FuelConsumerRow>,
    #[doc = " Why the consumers are not known at the hour."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FuelBranchesReply);

pub struct Labels {
    names: [Option<String>; 3],
    warnings: Vec<String>,
}

fn labels(source: &Source, load: &LoadContext) -> Labels {
    let localisation = match load.content.as_ref() {
        Some(content) => dashboard_base::content::localisation(source, content).map_err(|error| error.to_string()),
        None => Err("the recorded content stack is not configured".to_owned()),
    };
    let mut warnings = Vec::new();
    let names = LABELS.map(|key| {
        let name = match &localisation {
            Ok(loc) if loc.get(key).is_some() => localised(loc, key, View::Plain).ok(),
            Ok(_) => None,
            Err(_) => None,
        };
        if name.is_none() {
            warnings.push(match &localisation {
                Err(why) => format!("Fuel consumer label {key} cannot be read: {why}"),
                Ok(_) => format!("Fuel consumer label {key} is absent or refused by the recorded content"),
            });
        }
        name
    });
    Labels { names, warnings }
}

fn range(id: u32) -> Keys {
    let owner = u128::from(id) << 32;
    Keys::Range(owner, owner | u128::from(u32::MAX))
}

fn reply(frame: &DecodedFrame, tag: &str, id: u32, labels: &Labels) -> FuelBranchesReply {
    let view = &frame.tables[TableId::CountryFuelConsumers.index()];
    let mut out = FuelBranchesReply {
        tag: tag.to_owned(),
        scale: SCALE,
        consumer_names: Vec::new(),
        read_hour: view.sampled_at(),
        consumers: Vec::new(),
        missing: None,
        warnings: labels.warnings.clone(),
    };
    match frame.complete::<tables::CountryFuelConsumers>() {
        Ok(rows) => {
            out.consumers = rows
                .iter()
                .filter(|row| row.country == id)
                .map(|row| FuelConsumerRow { index: row.index, requested: row.requested_raw.to_string(), received: row.received_raw.to_string(), priority: row.priority })
                .collect();
            out.consumers.sort_by_key(|row| row.index);
        }
        Err(error) => {
            out.read_hour = None;
            out.missing = Some(format!("Fuel consumers are not recorded at this hour: {error}"));
        }
    }
    let beyond = out.consumers.iter().filter(|row| row.index as usize >= LABELS.len()).count();
    let count = out.consumers.last().map_or(0, |row| row.index as usize + 1).max(LABELS.len());
    out.consumer_names = (0..count).map(|index| labels.names.get(index).cloned().flatten()).collect();
    if beyond > 0 {
        out.warnings.push(format!("{beyond} fuel consumers beyond the fuel panel's three have no content label"));
    }
    out
}

impl AtHour for FuelBranches {
    const KEY: &'static str = "fuel-branches";
    const TITLE: &'static str = "Fuel by branch";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = FuelBranchesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = FuelBranchesReply;
    type Context = Labels;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FuelBranchesParams::decl(), FuelBranchesReply::decl(), FuelConsumerRow::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryFuelConsumers]
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, Some(owner)) => vec![Read { table: TableId::CountryFuelConsumers, keys: range(owner.id) }],
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(labels(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Fuel by branch country is unavailable at this hour"))?;
        Ok(reply(frame, &at.holders.name(&params.country, at.hour), owner.id, at.context))
    }
}
