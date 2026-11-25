
use dashboard_base::products::{Country, validate_country};
use super::{Content, DAY_HOURS, reason};
use dashboard_base::{Source, products::LoadContext};
use dashboard_base::products::Product;
use dashboard_query::Query;
use replay_data::rows::{CountryFuelRow, FuelAwardDay, FuelAwardDayRecipient, Recorded, TableId};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(crate) fn award_day(row: &CountryFuelRow, hour: i32) -> Result<&FuelAwardDay, String> {
    let Recorded::Value(day) = &row.award_day else {
        return Err("The applied fuel sources and recipients were not recorded".into());
    };
    if let Some(why) = reason(&day.reason) {
        return Err(why);
    }
    if !day.continuity_start_hour.is_some_and(|start| hour.checked_sub(DAY_HOURS - 1).is_some_and(|first| start <= first)) {
        return Err("The applied fuel window lacks 24 consecutive updates of this country identity".into());
    }
    if day.last_hour.at(hour) != hour {
        return Err("The applied fuel window does not end at this read".into());
    }
    Ok(day)
}

pub(crate) fn recipient_key(part: &FuelAwardDayRecipient) -> String {
    serde_json::to_string(&(part.kind, part.reference, part.owner, part.host, part.name.as_deref(), part.name_reason.as_slice()))
        .unwrap_or_default()
}

pub(crate) fn read_at(query: &Query<'_>, tag: &Country, hour: i32) -> io::Result<(u32, Option<i32>, CountryFuelRow)> {
    use dashboard_base::products::at_hour::{Keys, Read};
    let id = super::demand::holder_id(query, tag, hour)?;
    let key = u128::from(id);
    let split = query.replay.recording().predates(TableId::FuelAwardRecipients.index()).is_none();
    let mut reads = vec![Read { table: TableId::CountryFuel, keys: Keys::These(vec![key]) }];
    if split {
        reads.push(Read { table: TableId::FuelAwardRecipients, keys: Keys::Range(key << 83, (key << 83) | ((1u128 << 83) - 1)) });
    }
    let frame = dashboard_base::products::at_hour::frame(query, hour, &reads, "Fuel")?;
    let rows = frame.complete::<replay_data::rows::tables::CountryFuel>().map_err(|error| dashboard_core::refused(format!("Fuel is not recorded at this hour: {error}")))?;
    let mut row = rows.iter().find(|row| row.country == id).cloned().ok_or_else(|| dashboard_core::refused("No fuel status is recorded for this country at this hour"))?;
    let hour = super::read_hour(&row, frame.tables[TableId::CountryFuel.index()].sampled_at());
    if split && let Some(read_hour) = hour {
        let recipients = frame
            .complete::<replay_data::rows::tables::FuelAwardRecipients>()
            .map_err(|_| "fuel award recipients are not recorded at this read");
        replay_data::rows::assemble_award_recipients(std::slice::from_mut(&mut row), recipients, read_hour);
    }
    Ok((id, hour, row))
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelAwardsParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " Any hour the read holds: the reply names the read."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardPart {
    pub key: String,
    pub kind: u32,
    pub reference: String,
    pub owner: u32,
    pub host: String,
    #[doc = " The first and last update hours that paid it."]
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Fixed5 country fuel it received over the day."]
    pub amount_raw: String,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardsReply {
    pub tag: String,
    #[doc = " The read the day is of."]
    pub read_hour: i32,
    pub country: u32,
    pub country_instance: u32,
    #[doc = " The consumers' receipts, fixed5: what each consumer's recipients sum to."]
    pub received_raw: [String; 3],
    pub recipients: Vec<FuelAwardPart>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FuelAwardsReply);

fn recipients(day: &FuelAwardDay, content: &Content) -> io::Result<(Vec<FuelAwardPart>, Vec<String>)> {
    let mut refused = std::collections::BTreeMap::<String, usize>::new();
    let mut out = Vec::with_capacity(day.recipients.len());
    for part in &day.recipients {
        if part.kind > 2 {
            return Err(dashboard_core::refused(format!("Fuel recipient kind {} is not supported", part.kind)));
        }
        let (recorded_name, mut name_reason) = content.stored(part.name.as_deref(), &part.name_reason, "fuel recipient's stored name");
        if part.kind == 1 && recorded_name.is_none() {
            name_reason = Some("The recipient is an air base or country container; no wing's receipt is recorded".into());
        } else if let Some(why) = &name_reason {
            *refused.entry(why.clone()).or_default() += 1;
        }
        out.push(FuelAwardPart {
            key: recipient_key(part),
            kind: part.kind,
            reference: part.reference.to_string(),
            owner: part.owner,
            host: part.host.to_string(),
            first_hour: part.first_hour,
            last_hour: part.last_hour,
            amount_raw: part.amount_raw.to_string(),
            recorded_name,
            name_reason,
        });
    }
    Ok((out, refused.into_iter().map(|(why, count)| format!("{count} fuel recipient names refused: {why}")).collect()))
}

fn branch(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

pub struct CountryFuelAwards;

impl Product for CountryFuelAwards {
    const KEY: &'static str = "country-fuel-awards";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = FuelAwardsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = FuelAwardsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FuelAwardsParams::decl(), FuelAwardsReply::decl(), FuelAwardPart::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let (_, read_hour, row) = read_at(&query, &params.country, params.hour)?;
        let read_hour = read_hour.ok_or_else(|| dashboard_core::refused("The fuel read holding this hour is not known"))?;
        let day = award_day(&row, read_hour).map_err(dashboard_core::refused)?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        let content = Content::read(source, load);
        let (recipients, mut warnings) = recipients(day, &content)?;
        warnings.extend(content.warnings.iter().cloned());
        let reply = FuelAwardsReply {
            tag: query.holders.name(&params.country, params.hour),
            read_hour,
            country: row.country,
            country_instance: day.country_instance,
            received_raw: day.received_raw.map(|value| value.to_string()),
            recipients,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelAwardHistoryParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The hour the recipient was picked at: the country is its holder then."]
    pub hour: i32,
    #[doc = " The window: hours `window * detailHours` through the next window's first less one."]
    pub window: i32,
    pub recipient_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub amount_raw: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardHistoryReply {
    pub tag: String,
    pub country: u32,
    pub window: i32,
    pub recipient_key: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub runs: Vec<FuelAwardRun>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FuelAwardHistoryReply);

pub fn award_runs(reads: &[super::demand::ReadRow], key: &str, first: i32, last: i32) -> Vec<FuelAwardRun> {
    use super::demand::{Span, spans};
    let mut known = false;
    spans(reads, first, last, false)
        .into_iter()
        .map(|span| {
            let (read, first, last) = match span {
                Span::Read { read, first, last } => (read, first, last),
                Span::Gap { first, last } => {
                    known = false;
                    return FuelAwardRun { first_hour: first, last_hour: last, starts_segment: true, amount_raw: None, reason: Some("Fuel supplying read missing in this interval".into()) };
                }
            };
            let (amount_raw, reason) = match award_day(&read.row, read.hour) {
                Err(why) => (None, Some(why)),
                Ok(day) => match day.recipients.iter().find(|part| recipient_key(part) == key) {
                    Some(part) => (Some(part.amount_raw.to_string()), None),
                    None => (Some("0".to_owned()), None),
                },
            };
            let starts_segment = read.starts_segment || !known;
            known = amount_raw.is_some();
            FuelAwardRun { first_hour: first, last_hour: last, starts_segment, amount_raw, reason }
        })
        .collect()
}

pub struct CountryFuelAwardHistory;

impl Product for CountryFuelAwardHistory {
    const KEY: &'static str = "country-fuel-award-history";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FuelAwardHistoryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, window, recipientKey];
    type Reply = FuelAwardHistoryReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)?;
        if params.recipient_key.is_empty() { Err("Invalid fuel recipient") } else { Ok(()) }
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FuelAwardHistoryParams::decl(), FuelAwardHistoryReply::decl(), FuelAwardRun::decl()]
    }

    fn load(source: &Source, params: &Self::Params, _load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let id = super::demand::holder_id(&query, &params.country, params.hour)?;
        let (first, last) = super::demand::window_span(params.window)?;
        let reads = super::demand::country_reads(&query, id, first, last, super::demand::Detail::AwardRecipients, cancelled)?;
        let runs = award_runs(&reads.reads, &params.recipient_key, first, last);
        Ok(FuelAwardHistoryReply {
            tag: query.holders.name(&params.country, params.hour),
            country: id,
            window: params.window,
            recipient_key: params.recipient_key.clone(),
            first_hour: first,
            last_hour: last,
            runs,
            warnings: reads.warnings,
        })
    }
}

