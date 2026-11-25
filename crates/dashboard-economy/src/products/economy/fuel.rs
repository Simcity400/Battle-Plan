
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::LoadContext;
use dashboard_base::{Source, products::at_hour::{At, AtHour, Keys, Owner, Read}};
use dashboard_text::catalogue::{Catalogue, View};
use engine_fuel::ccountry_fuel_status::{HISTORY_RING_ENTRIES, MODIFIER_SCALE, QUANTITY_SCALE};
use replay_data::rows::{CountryFuelRow, FuelDayRow, FuelDemandTotalRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub mod awards;
pub mod demand;

#[cfg(test)]
mod tests;

pub const DAY_HOURS: i32 = HISTORY_RING_ENTRIES as i32;
pub const DETAIL_HOURS: i32 = 720;
const QUANTITY_UNITS: u32 = QUANTITY_SCALE as u32;
const GAIN_UNITS: u32 = MODIFIER_SCALE as u32;
const _: () = assert!(QUANTITY_UNITS as i64 == QUANTITY_SCALE && GAIN_UNITS as i64 == MODIFIER_SCALE);

const CONSUMERS: [&str; 3] = ["TOPBAR_ARMY", "TOPBAR_AIR", "TOPBAR_NAVY"];
const GAINS: [(&str, bool); 4] =
    [("FUEL_DAILY_GAIN_BASE", false), ("FUEL_DAILY_GAIN_FROM_STATES", false), ("FUEL_DAILY_GAIN_FROM_OIL", true), ("FUEL_DAILY_GAIN_FROM_OTHER", false)];
const TRANSFERS: [&str; 3] = ["FUEL_DAILY_GAIN_FROM_LEND_LEASE", "FUEL_DAILY_REQUIRED_FROM_LEND_LEASE", "FUEL_DAILY_REQUIRED_FROM_OTHER"];

pub(crate) fn recorded(value: &Recorded<i64>) -> Option<i64> {
    match value {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    }
}

pub(crate) fn reason(bytes: &[u8]) -> Option<String> {
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(bytes).into_owned())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelLabel {
    pub key: String,
    pub template: Option<String>,
    pub arguments: Vec<String>,
    pub reason: Option<String>,
}

pub struct Content {
    pub catalogue: Result<Arc<Catalogue>, String>,
    pub consumer_names: [Option<String>; 3],
    pub gain_labels: [FuelLabel; 4],
    pub transfer_labels: [FuelLabel; 3],
    pub warnings: Vec<String>,
}

fn label(catalogue: &Result<Arc<Catalogue>, String>, key: &str, oil: bool) -> FuelLabel {
    use content_reader_localisation::localisation::Argument;
    let arguments: Vec<String> = if oil { vec!["NUM".into(), "OIL".into()] } else { vec!["NUM".into()] };
    let (template, mut notes) = match catalogue {
        Err(why) => (None, vec![format!("Fuel labels cannot be read: {why}")]),
        Ok(catalogue) if catalogue.localisation.get(key).is_none() => (None, vec![format!("Fuel label {key} is absent from the recorded content")]),
        Ok(catalogue) => {
            let operands = [("NUM", Argument::Text("{NUM}")), ("OIL", Argument::Text("{OIL}"))];
            let operands = if oil { &operands[..] } else { &operands[..1] };
            let (text, findings) =
                dashboard_text::catalogue::localised_details(&catalogue.localisation, key, operands, View::Plain, dashboard_core::findings::Kind::Unsupported);
            (text, findings.into_iter().map(|finding| finding.message).collect())
        }
    };
    if oil {
        notes.push("The oil count behind FUEL_DAILY_GAIN_FROM_OIL is not recorded".into());
    }
    FuelLabel { key: key.into(), template, arguments, reason: (!notes.is_empty()).then(|| notes.join("; ")) }
}

impl Content {
    pub fn read(source: &Source, load: &LoadContext) -> Self {
        let catalogue = match load.content.as_ref() {
            Some(options) => dashboard_base::catalogue::catalogue(source, options).map_err(|error| error.to_string()),
            None => Err("the recorded content stack is not configured".to_owned()),
        };
        let mut warnings = Vec::new();
        let consumer_names = CONSUMERS.map(|key| {
            let name = catalogue
                .as_ref()
                .ok()
                .filter(|catalogue| catalogue.localisation.get(key).is_some())
                .and_then(|catalogue| dashboard_text::catalogue::localised(&catalogue.localisation, key, View::Plain).ok());
            if name.is_none() {
                warnings.push(match &catalogue {
                    Err(why) => format!("Fuel consumer label {key} cannot be read: {why}"),
                    Ok(_) => format!("Fuel consumer label {key} is absent or refused by the recorded content"),
                });
            }
            name
        });
        let gain_labels = GAINS.map(|(key, oil)| label(&catalogue, key, oil));
        let transfer_labels = TRANSFERS.map(|key| label(&catalogue, key, false));
        for label in gain_labels.iter().chain(&transfer_labels) {
            if let Some(why) = &label.reason {
                warnings.push(format!("Fuel label {}: {why}", label.key));
            }
        }
        if let Ok(catalogue) = &catalogue {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(catalogue.refusal_lines());
        }
        Self { catalogue, consumer_names, gain_labels, transfer_labels, warnings }
    }

    pub fn stored(&self, name: Option<&[u8]>, refused: &[u8], what: &str) -> (Option<String>, Option<String>) {
        if let Some(why) = reason(refused) {
            return (None, Some(why));
        }
        let Some(bytes) = name else { return (None, Some(format!("The {what} was not recorded"))) };
        if bytes.is_empty() {
            return (Some(String::new()), None);
        }
        let named = self.catalogue.as_ref().map_err(Clone::clone).and_then(|catalogue| {
            dashboard_text::scripted_text::stored_text(&catalogue.localisation, &catalogue.recorded_language, bytes, what)
                .and_then(|text| dashboard_text::catalogue::display_text(text.as_bytes()))
        });
        match named {
            Ok(name) => (Some(name), None),
            Err(why) => (None, Some(why)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DayLeftOut {
    Overlaps,
    NotUpdated,
    OwnershipNotRecorded,
    NotRecorded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardTotals {
    pub gains_raw: [String; 4],
    pub own_gain_raw: String,
    pub own_gain_quantity_raw: String,
    pub transfers_raw: [String; 4],
    pub consumed_raw: String,
    pub received_raw: [String; 3],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDay {
    #[doc = " Own fuel gain, fixed5."]
    pub gained_raw: String,
    #[doc = " Everything taken out, /32768, `other_raw` included."]
    pub burned_raw: String,
    #[doc = " The consumers' receipts, fixed5."]
    pub received_raw: [String; 3],
    #[doc = " Flat cost and lend-lease sent, /32768; already in `burned_raw`."]
    pub other_raw: String,
    #[doc = " The applied updates, when known: their sources and transfers."]
    pub awards: Option<FuelAwardTotals>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelReadAt {
    #[doc = " The hour the status was read; its values hold until the next read."]
    pub read_hour: Option<i32>,
    #[doc = " The raw country id read, and its instance where recorded."]
    pub country: u32,
    pub country_instance: Option<u32>,
    pub fuel_raw: String,
    pub capacity_raw: String,
    #[doc = " Lend-lease fuel received in the hour before the read: the tank gains it, the day does not."]
    pub lend_lease_raw: String,
    #[doc = " The read the day is of: the last day read at or before the hour."]
    pub day_hour: Option<i32>,
    pub day: Option<FuelDay>,
    pub left_out: Option<DayLeftOut>,
    #[doc = " Why the applied updates (sources, recipients) are not known for the day."]
    pub awards_reason: Option<String>,
    pub continuity_start_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelConsumptionAt {
    pub native_daily_raw: [Option<String>; 3],
    pub known_daily_raw: [Option<String>; 3],
    pub positive_daily_raw: [Option<String>; 3],
    pub reasons: [Option<String>; 3],
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryFuelParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFuelReply {
    pub tag: String,
    #[doc = " Units per fuel of the level, capacity, burned and transfer values."]
    pub quantity_scale: u32,
    #[doc = " Units per fuel of the gains and the consumers' amounts."]
    pub gain_scale: u32,
    #[doc = " Hours of fuel updates a day sums."]
    pub day_hours: i32,
    #[doc = " Hours of one window of a detail history."]
    pub detail_hours: i32,
    pub consumer_names: [Option<String>; 3],
    pub gain_labels: [FuelLabel; 4],
    pub transfer_labels: [FuelLabel; 3],
    #[doc = " None with `missing` saying why."]
    pub read: Option<FuelReadAt>,
    pub missing: Option<String>,
    #[doc = " None with `consumption_missing` saying why."]
    pub consumption: Option<FuelConsumptionAt>,
    pub consumption_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryFuelReply);

pub(crate) fn day_of(row: &FuelDayRow) -> (Option<FuelDay>, Option<DayLeftOut>) {
    let left = match row.left_out {
        FuelDayRow::KEPT => None,
        FuelDayRow::OVERLAPS => Some(DayLeftOut::Overlaps),
        FuelDayRow::NOT_UPDATED => Some(DayLeftOut::NotUpdated),
        _ => Some(DayLeftOut::OwnershipNotRecorded),
    };
    if left.is_some() {
        return (None, left);
    }
    let core = (|| {
        Some((recorded(&row.gained)?, recorded(&row.burned)?, [recorded(&row.received_army)?, recorded(&row.received_air)?, recorded(&row.received_navy)?], recorded(&row.other)?))
    })();
    let Some((gained, burned, received, other)) = core else { return (None, Some(DayLeftOut::NotRecorded)) };
    let awards = recorded(&row.own_gain_quantity).and_then(|quantity| {
        Some(FuelAwardTotals {
            gains_raw: [recorded(&row.gain_base)?, recorded(&row.gain_states)?, recorded(&row.gain_oil)?, recorded(&row.gain_negative_cost)?].map(|value| value.to_string()),
            own_gain_raw: gained.to_string(),
            own_gain_quantity_raw: quantity.to_string(),
            transfers_raw: [recorded(&row.incoming)?, recorded(&row.outgoing)?, recorded(&row.flat)?, other].map(|value| value.to_string()),
            consumed_raw: burned.to_string(),
            received_raw: received.map(|value| value.to_string()),
        })
    });
    let day = FuelDay { gained_raw: gained.to_string(), burned_raw: burned.to_string(), received_raw: received.map(|value| value.to_string()), other_raw: other.to_string(), awards };
    (Some(day), None)
}

pub(crate) fn consumption_of(row: &FuelDemandTotalRow) -> FuelConsumptionAt {
    let text = |value: &Recorded<i64>| recorded(value).map(|value| value.to_string());
    FuelConsumptionAt {
        native_daily_raw: [text(&row.native_army), text(&row.native_air), text(&row.native_navy)],
        known_daily_raw: [text(&row.known_army), text(&row.known_air), text(&row.known_navy)],
        positive_daily_raw: [text(&row.positive_army), text(&row.positive_air), text(&row.positive_navy)],
        reasons: [reason(&row.army_reason), reason(&row.air_reason), reason(&row.navy_reason)],
    }
}

pub(crate) fn read_hour(fuel: &CountryFuelRow, sampled_at: Option<i32>) -> Option<i32> {
    use replay_data::rows::{FuelAwardDayHour, FuelDemandHour};
    sampled_at
        .or(match &fuel.award_day {
            Recorded::Value(day) if day.reason.is_empty() => match day.last_hour {
                FuelAwardDayHour::Stamped(hour) | FuelAwardDayHour::Historical(hour) => Some(hour),
                FuelAwardDayHour::Host => None,
            },
            _ => None,
        })
        .or(match &fuel.demand {
            Recorded::Value(pack) => match pack.hour {
                FuelDemandHour::Stamped(hour) | FuelDemandHour::Historical(hour) => Some(hour),
                FuelDemandHour::Host => None,
            },
            Recorded::NotRecorded => None,
        })
}

pub(crate) fn read_of(fuel: &CountryFuelRow, read_hour: Option<i32>, day: Option<&FuelDayRow>) -> FuelReadAt {
    let (day_value, left_out) = match day {
        Some(row) => day_of(row),
        None => (None, Some(DayLeftOut::NotRecorded)),
    };
    FuelReadAt {
        read_hour,
        country: fuel.country,
        country_instance: match fuel.country_instance {
            Recorded::Value(value) => value,
            Recorded::NotRecorded => None,
        },
        fuel_raw: fuel.current_raw.to_string(),
        capacity_raw: fuel.maximum_raw.to_string(),
        lend_lease_raw: fuel.lend_lease_gain_raw.to_string(),
        day_hour: day.map(|row| row.read_hour),
        day: day_value,
        left_out,
        awards_reason: day.and_then(|row| reason(&row.awards_reason)),
        continuity_start_hour: day.and_then(|row| row.continuity_start_hour),
    }
}

fn reply(frame: &DecodedFrame, tag: &str, id: u32, content: &Content) -> CountryFuelReply {
    let mut out = CountryFuelReply {
        tag: tag.to_owned(),
        quantity_scale: QUANTITY_UNITS,
        gain_scale: GAIN_UNITS,
        day_hours: DAY_HOURS,
        detail_hours: DETAIL_HOURS,
        consumer_names: content.consumer_names.clone(),
        gain_labels: content.gain_labels.clone(),
        transfer_labels: content.transfer_labels.clone(),
        read: None,
        missing: None,
        consumption: None,
        consumption_missing: None,
        warnings: content.warnings.clone(),
    };
    let day = match frame.complete::<tables::FuelDays>() {
        Ok(rows) => Ok(rows.iter().find(|row| row.country == id)),
        Err(error) => Err(error.to_string()),
    };
    match frame.complete::<tables::CountryFuel>() {
        Ok(rows) => match rows.iter().find(|row| row.country == id) {
            Some(fuel) => {
                let held = day.as_ref().ok().copied().flatten();
                let read_hour = read_hour(fuel, frame.tables[TableId::CountryFuel.index()].sampled_at());
                let mut read = read_of(fuel, read_hour, held);
                if let Err(why) = &day {
                    read.awards_reason = Some(format!("Fuel days are not recorded at this hour: {why}"));
                }
                out.read = Some(read);
            }
            None => out.missing = Some("No fuel status is recorded for this country at this hour".into()),
        },
        Err(error) => out.missing = Some(format!("Fuel is not recorded at this hour: {error}")),
    }
    match frame.complete::<tables::FuelDemandTotals>() {
        Ok(rows) => match rows.iter().find(|row| row.country == id) {
            Some(row) => out.consumption = Some(consumption_of(row)),
            None => out.consumption_missing = Some("No current consumption is recorded for this country at this hour".into()),
        },
        Err(error) => out.consumption_missing = Some(format!("Current consumption is not recorded at this hour: {error}")),
    }
    out
}

pub struct CountryFuel;

impl AtHour for CountryFuel {
    const KEY: &'static str = "country-fuel";
    const TITLE: &'static str = "Fuel";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = CountryFuelParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = CountryFuelReply;
    type Context = Content;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CountryFuelParams::decl(),
            CountryFuelReply::decl(),
            FuelLabel::decl(),
            DayLeftOut::decl(),
            FuelAwardTotals::decl(),
            FuelDay::decl(),
            FuelReadAt::decl(),
            FuelConsumptionAt::decl(),
        ]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryFuel, TableId::FuelDays, TableId::FuelDemandTotals]
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, Some(owner)) => {
                let key = u128::from(owner.id);
                [TableId::CountryFuel, TableId::FuelDays, TableId::FuelDemandTotals].map(|table| Read { table, keys: Keys::These(vec![key]) }).into()
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::read(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Fuel country is unavailable at this hour"))?;
        Ok(reply(frame, &at.holders.name(&params.country, at.hour), owner.id, at.context))
    }
}
