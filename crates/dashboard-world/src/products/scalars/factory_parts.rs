
use dashboard_base::products::{Country, validate_country};
use super::rows;
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Owner, Read}}};
use replay_data::rows::{CountryFactoriesRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactoryParts;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactoryPartsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryPartValue {
    #[doc = " `military`, `naval` or `civilian`."]
    pub group: String,
    pub part: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub value: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryPartsReply {
    pub hour: i32,
    pub tag: String,
    pub parts: Vec<FactoryPartValue>,
    #[doc = " Groups whose recorded `unavailable` is not the sum of what they withhold."]
    pub unexplained: Vec<String>,
    #[doc = " Why the country's factories are not known at the hour."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactoryPartsReply);

pub(super) const GROUPS: [&str; 3] = ["military", "naval", "civilian"];

fn shown(raw: i64) -> Option<i64> {
    i32::try_from(raw / 100_000).ok().map(i64::from)
}

pub(super) fn parts(row: &CountryFactoriesRow) -> Option<(Vec<FactoryPartValue>, Vec<String>)> {
    let groups = [
        (row.military_total_raw, row.military_controlled_raw, row.military_owned_raw, row.military_damaged_raw, row.military_unavailable, row.military_from_subjects, row.military_to_overlord, row.military_from_others, row.military_to_others),
        (row.naval_total_raw, row.naval_controlled_raw, row.naval_owned_raw, row.naval_damaged_raw, row.naval_unavailable, row.naval_from_subjects, row.naval_to_overlord, row.naval_from_others, row.naval_to_others),
        (row.civilian_total_raw, row.civilian_controlled_raw, row.civilian_owned_raw, row.civilian_damaged_raw, row.civilian_unavailable, row.civilian_from_subjects, row.civilian_to_overlord, row.civilian_from_others, row.civilian_to_others),
    ];
    let consumers = [
        ("consumerGoods", row.consumer_goods),
        ("contractPayments", row.contract_payments),
        ("specialProjects", row.special_projects),
        ("licenseImports", row.license_imports),
        ("tradeImports", row.trade_imports),
    ];
    let mut out = Vec::new();
    let mut unexplained = Vec::new();
    for (group, (total, controlled, owned, damaged, unavailable, from_subjects, to_overlord, from_others, to_others)) in GROUPS.into_iter().zip(groups) {
        let mut push = |part: &str, value: i64| out.push(FactoryPartValue { group: group.to_owned(), part: part.to_owned(), value });
        push("total", shown(total)?);
        push("controlled", shown(controlled)?);
        push("owned", shown(owned)?);
        push("damaged", shown(damaged)?);
        for (part, value) in [("fromSubjects", from_subjects), ("toOverlord", to_overlord), ("fromOthers", from_others), ("toOthers", to_others)] {
            push(part, i64::from(value));
        }
        let withheld = i64::from(to_overlord) + i64::from(to_others) + if group == "civilian" { consumers.iter().map(|(_, value)| i64::from(*value)).sum() } else { 0 };
        if withheld != i64::from(unavailable) {
            unexplained.push(group.to_owned());
        }
    }
    for (part, value) in [("tradeExports", row.civilian_trade_exports), ("licenseExports", row.civilian_license_exports)].into_iter().chain(consumers) {
        out.push(FactoryPartValue { group: "civilian".into(), part: part.to_owned(), value: i64::from(value) });
    }
    Some((out, unexplained))
}

impl AtHour for FactoryParts {
    const KEY: &'static str = "factory-parts";
    const TITLE: &'static str = "Factory parts";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FactoryPartsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = FactoryPartsReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactoryPartsParams::decl(), FactoryPartValue::decl(), FactoryPartsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)?;
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryFactories]
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, Some(owner)) => vec![Read::these(TableId::CountryFactories, [u128::from(owner.id)])],
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Factory parts country is unavailable at this hour"))?;
        let mut reply = FactoryPartsReply { hour: frame.hour, tag: at.holders.name(&params.country, at.hour), parts: Vec::new(), unexplained: Vec::new(), missing: None, warnings: Vec::new() };
        match rows::<tables::CountryFactories>(frame).map(|held| held.iter().find(|row| row.country == owner.id)) {
            Err(reason) => reply.missing = Some(format!("Factories are not known at this hour: {reason}")),
            Ok(None) => reply.missing = Some(at.holders.say(&params.country, at.hour, |name| format!("country factories held no row for {name} at this hour"))),
            Ok(Some(row)) => match parts(row) {
                Some((parts, unexplained)) => {
                    reply.parts = parts;
                    reply.unexplained = unexplained;
                }
                None => reply.missing = Some("A factory total does not fit a whole number the game shows".into()),
            },
        }
        Ok(reply)
    }
}
