use super::peace::{PeaceDeal, PeaceDeals, PeaceDealsParams, PeaceDealsReply, PeaceTransferCountry};
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::{Source, span::HourSpan};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub struct TerritoryTransfers;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryTransfersParams {
    pub branch_id: String,
    #[doc = " Transfers known by this hour are counted; nothing after it is read into the reply."]
    pub through: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryCountry {
    #[doc = " The country's identity, stable across tag changes."]
    pub key: String,
    #[doc = " Its tag at `hour`, its latest transfer's read."]
    pub tag: String,
    pub hour: i32,
    pub gained: u32,
    pub lost: u32,
    pub net: i32,
    #[doc = " Exact signed sum; null when a counted state's victory points are not known by `through`."]
    pub net_victory_points: Option<String>,
    pub victory_points_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryTransfersReply {
    pub branch_id: String,
    pub through: i32,
    #[doc = " State transfers known by `through`, a state transferred twice counted twice."]
    pub transfers: u32,
    #[doc = " Largest net gain first."]
    pub countries: Vec<TerritoryCountry>,
    #[doc = " The first hour after `through` at which this reply would differ; null when none."]
    pub next_change: Option<i32>,
    #[doc = " Hours up to `through` whose settlements or ownership were not recorded."]
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TerritoryTransfersReply);

const VP_NOT_RECORDED: &str = "Victory points of a transferred state are not recorded";
const VP_NOT_YET_READ: &str = "Victory points of a transferred state are read after this date";

fn deal_hour(deal: &PeaceDeal) -> i32 {
    if let Some(captured) = &deal.captured {
        return captured.hour;
    }
    let base = deal.end_hour.or(deal.first_finalized_hour).unwrap_or(deal.first_observed_hour);
    deal.items.iter().fold(base, |hour, item| hour.max(item.dates.after_hour))
}

fn deal_known_at(deal: &PeaceDeal) -> i32 {
    let hour = deal_hour(deal).max(deal.capture_observed_hour.unwrap_or(i32::MIN));
    match (&deal.captured, deal.end_hour) {
        (Some(captured), _) => hour.max(captured.publication_hour),
        (None, Some(_)) => hour.max(deal.first_absent_hour.unwrap_or(hour)),
        (None, None) => hour,
    }
}

struct Tally {
    tag: String,
    hour: i32,
    gained: u32,
    lost: u32,
    victory_points: Result<i128, &'static str>,
}

fn identity(country: &PeaceTransferCountry) -> String {
    format!("{}:{}", country.slot, country.country_id)
}

pub fn tally(reply: &PeaceDealsReply, through: i32) -> TerritoryTransfersReply {
    let mut countries: BTreeMap<String, Tally> = BTreeMap::new();
    let mut transfers = 0u32;
    let mut next_change: Option<i32> = None;
    let mut later = |hour: i32| next_change = Some(next_change.map_or(hour, |held| held.min(hour)));
    for deal in &reply.deals {
        let known = deal_known_at(deal);
        for state in deal.transfers.links.iter().flat_map(|link| &link.states) {
            let shown = state.dates.after_hour.max(known);
            if shown > through {
                later(shown);
                continue;
            }
            transfers += 1;
            let worth = match (&state.victory_points, state.victory_points_read_hour) {
                (None, _) => Err(VP_NOT_RECORDED),
                (Some(_), read) if read.is_none_or(|read| read > through) => {
                    if let Some(read) = read {
                        later(read);
                    }
                    Err(VP_NOT_YET_READ)
                }
                (Some(points), _) => points.parse::<i128>().map_err(|_| VP_NOT_RECORDED),
            };
            for (country, sign) in [(&state.to_country, 1i128), (&state.from_country, -1)] {
                let row = countries.entry(identity(country)).or_insert_with(|| Tally { tag: String::new(), hour: i32::MIN, gained: 0, lost: 0, victory_points: Ok(0) });
                if country.hour >= row.hour {
                    row.tag.clone_from(&country.tag);
                    row.hour = country.hour;
                }
                if sign > 0 { row.gained += 1 } else { row.lost += 1 }
                row.victory_points = match (row.victory_points, worth) {
                    (Ok(sum), Ok(points)) => Ok(sum + sign * points),
                    (Err(held), _) => Err(held),
                    (_, Err(why)) => Err(why),
                };
            }
        }
    }
    let mut countries: Vec<TerritoryCountry> = countries
        .into_iter()
        .map(|(key, row)| TerritoryCountry {
            key,
            tag: row.tag,
            hour: row.hour,
            gained: row.gained,
            lost: row.lost,
            net: {
                let net = i64::from(row.gained) - i64::from(row.lost);
                i32::try_from(net).unwrap_or(if net < 0 { i32::MIN } else { i32::MAX })
            },
            net_victory_points: row.victory_points.ok().map(|sum| sum.to_string()),
            victory_points_missing: row.victory_points.err().map(str::to_owned),
        })
        .collect();
    countries.sort_by(|a, b| b.net.cmp(&a.net).then(b.gained.cmp(&a.gained)).then_with(|| a.tag.cmp(&b.tag)).then_with(|| a.key.cmp(&b.key)));
    let mut gaps: Vec<HourSpan> = [&reply.unobserved.settlements, &reply.unobserved.ownership]
        .into_iter()
        .flatten()
        .filter(|span| span.first_hour <= through)
        .map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour.min(through) })
        .collect();
    gaps.sort_by_key(|span| (span.first_hour, span.last_hour));
    let mut unobserved: Vec<HourSpan> = Vec::new();
    for span in gaps {
        match unobserved.last_mut() {
            Some(held) if span.first_hour <= held.last_hour.saturating_add(1) => held.last_hour = held.last_hour.max(span.last_hour),
            _ => unobserved.push(span),
        }
    }
    TerritoryTransfersReply { branch_id: reply.branch_id.clone(), through, transfers, countries, next_change, unobserved, warnings: reply.warnings.clone() }
}

impl Product for TerritoryTransfers {
    const KEY: &'static str = "territory-transfers";
    const CONTENT_DEPENDENT: bool = true;
    type Params = TerritoryTransfersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, through];
    type Reply = TerritoryTransfersReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TerritoryTransfersParams::decl(), TerritoryTransfersReply::decl(), TerritoryCountry::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid territory-transfers branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let deals = PeaceDeals::load(source, &PeaceDealsParams { branch_id: params.branch_id.clone() }, context, cancelled)?;
        let reply = tally(&deals, params.through);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
