
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::LoadContext;
use dashboard_base::{Source, products::at_hour::{At, AtHour, Owner, Read}};
use replay_data::rows::{CountryExperienceDailyRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SCALE: u32 = 32_768;
pub const DAY_HOURS: i32 = 24;

const TABLES: [TableId; 2] = [TableId::CountryExperience, TableId::CountryExperienceDaily];

pub struct CountryExperience;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExperienceParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExperienceValues {
    pub army_raw: String,
    pub navy_raw: String,
    pub air_raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExperienceReply {
    pub tag: String,
    #[doc = " Units per experience point for every value."]
    pub scale: u32,
    pub day_hours: i32,
    #[doc = " The daily read the values are from; none where the recording does not tell it (the journal"]
    #[doc = " door gives CountryExperience no sample hour)."]
    pub read_hour: Option<i32>,
    pub banked: Option<ExperienceValues>,
    pub banked_missing: Option<String>,
    pub change: Option<ExperienceValues>,
    pub change_missing: Option<String>,
    pub gain: Option<ExperienceValues>,
    pub gain_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ExperienceReply);

pub fn read_hour(frame: &DecodedFrame) -> Option<i32> {
    frame.tables[TableId::CountryExperience.index()].sampled_at()
}

fn values(cells: [&Recorded<i64>; 3]) -> Option<ExperienceValues> {
    let [army, navy, air] = cells.map(|cell| match cell {
        Recorded::Value(value) => Some(value.to_string()),
        Recorded::NotRecorded => None,
    });
    Some(ExperienceValues { army_raw: army?, navy_raw: navy?, air_raw: air? })
}

impl AtHour for CountryExperience {
    const KEY: &'static str = "country-experience";
    const TITLE: &'static str = "Experience";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ExperienceParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = ExperienceReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ExperienceParams::decl(), ExperienceReply::decl(), ExperienceValues::decl()]
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

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, Some(owner)) => TABLES.map(|table| Read::these(table, [u128::from(owner.id)])).into(),
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Experience country is unavailable at this hour"))?;
        let mut reply = ExperienceReply {
            tag: at.holders.name(&params.country, at.hour),
            scale: SCALE,
            day_hours: DAY_HOURS,
            read_hour: read_hour(frame),
            banked: None,
            banked_missing: None,
            change: None,
            change_missing: None,
            gain: None,
            gain_missing: None,
            warnings: Vec::new(),
        };
        match frame.complete::<tables::CountryExperience>() {
            Err(error) => reply.banked_missing = Some(format!("Banked experience is not recorded at this hour: {error}")),
            Ok(rows) => match rows.iter().find(|row| row.country == owner.id) {
                Some(row) => reply.banked = Some(ExperienceValues { army_raw: row.army_raw.to_string(), navy_raw: row.navy_raw.to_string(), air_raw: row.air_raw.to_string() }),
                None => reply.banked_missing = Some(at.holders.say(&params.country, at.hour, |name| format!("{name} has no experience row at this hour"))),
            },
        }
        let daily: Option<&CountryExperienceDailyRow> = match frame.complete::<tables::CountryExperienceDaily>() {
            Err(error) => {
                let why = format!("The daily experience is not recorded at this hour: {error}");
                reply.change_missing = Some(why.clone());
                reply.gain_missing = Some(why);
                None
            }
            Ok(rows) => rows.iter().find(|row| row.country == owner.id),
        };
        if let Some(row) = daily {
            reply.change = values([&row.army_change, &row.navy_change, &row.air_change]);
            if reply.change.is_none() {
                reply.change_missing = Some("No read exactly a day earlier in this unbroken stretch".into());
            }
            reply.gain = values([&row.army_gain, &row.navy_gain, &row.air_gain]);
            if reply.gain.is_none() {
                reply.gain_missing = Some("The day before this read lacks a complete hourly interval".into());
            }
        } else if reply.change_missing.is_none() {
            let why = at.holders.say(&params.country, at.hour, |name| format!("{name} has no daily experience row at this hour"));
            reply.change_missing = Some(why.clone());
            reply.gain_missing = Some(why);
        }
        Ok(reply)
    }
}
