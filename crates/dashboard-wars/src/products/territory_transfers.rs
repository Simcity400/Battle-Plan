use super::peace::{PeaceDeal, PeaceTransferCountry, PeaceUnobserved, PeaceWarning, assemble};
use dashboard_base::Source;
use dashboard_base::products::{LoadContext, Product};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
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
    #[doc = " Transfers observed; only those observed when the reply is `partial`."]
    pub gained: u32,
    pub lost: u32,
    #[doc = " Null when the reply is `partial`: an unobserved transfer could change it."]
    pub net: Option<i32>,
    #[doc = " Exact signed sum; null when a counted state's victory points are not known by `through`, or the reply is `partial`."]
    pub net_victory_points: Option<String>,
    pub victory_points_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryTransfersReply {
    pub branch_id: String,
    pub through: i32,
    #[doc = " State transfers known by `through`, a state transferred twice counted twice; only those observed when `partial`."]
    pub transfers: u32,
    #[doc = " Largest net gain first; in key order, unranked, when `partial`."]
    pub countries: Vec<TerritoryCountry>,
    #[doc = " The first hour after `through` at which this reply would differ, a new unrecorded stretch included; null when none."]
    pub next_change: Option<i32>,
    #[doc = " Why the counts are only those observed: settlements or ownership were not recorded for part of the campaign by `through`."]
    pub partial: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TerritoryTransfersReply);

const VP_NOT_RECORDED: &str = "Victory points of a transferred state are not recorded by this date";
const PARTIAL: &str = "Settlements or ownership were not recorded for part of the campaign before this date: only the transfers observed are counted";

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

fn identity(country: &PeaceTransferCountry) -> String {
    format!("{}:{}", country.slot, country.country_id)
}

enum Worth {
    NotRecorded,
    Read { hour: i32, points: i128 },
}

struct Move {
    shown: i32,
    from: PeaceTransferCountry,
    to: PeaceTransferCountry,
    worth: Worth,
}

pub struct TransfersIndex {
    branch_id: String,
    moves: Vec<Move>,
    gap_starts: Vec<i32>,
    warnings: Vec<PeaceWarning>,
    changes: Vec<i32>,
}

impl TransfersIndex {
    pub fn new(branch_id: String, deals: &[PeaceDeal], unobserved: &PeaceUnobserved, mut warnings: Vec<PeaceWarning>) -> Self {
        let mut moves = Vec::new();
        for deal in deals {
            let known = deal_known_at(deal);
            for state in deal.transfers.links.iter().flat_map(|link| &link.states) {
                let shown = state.dates.after_hour.max(known);
                let worth = match (state.victory_points.as_deref().map(str::parse::<i128>), state.victory_points_read_hour) {
                    (Some(Ok(points)), Some(read)) => Worth::Read { hour: read.max(shown), points },
                    _ => Worth::NotRecorded,
                };
                moves.push(Move { shown, from: state.from_country.clone(), to: state.to_country.clone(), worth });
            }
        }
        moves.sort_by_key(|item| item.shown);
        let mut gap_starts: Vec<i32> = unobserved.settlements.iter().chain(&unobserved.ownership).map(|span| span.first_hour).collect();
        gap_starts.sort_unstable();
        gap_starts.dedup();
        warnings.sort_by_key(|warning| warning.hour);
        let read_hours = moves.iter().filter_map(|item| match item.worth {
            Worth::Read { hour, .. } => Some(hour),
            Worth::NotRecorded => None,
        });
        let mut changes: Vec<i32> = moves
            .iter()
            .map(|item| item.shown)
            .chain(read_hours)
            .chain(gap_starts.iter().copied())
            .chain(warnings.iter().filter_map(|warning| warning.hour))
            .collect();
        changes.sort_unstable();
        changes.dedup();
        Self { branch_id, moves, gap_starts, warnings, changes }
    }

    pub fn answer(&self, through: i32) -> TerritoryTransfersReply {
        let counted = &self.moves[..self.moves.partition_point(|item| item.shown <= through)];
        let partial = self.gap_starts.first().is_some_and(|&first| first <= through);
        let mut countries: BTreeMap<String, Tally> = BTreeMap::new();
        for item in counted {
            let worth = match item.worth {
                Worth::Read { hour, points } if hour <= through => Ok(points),
                _ => Err(VP_NOT_RECORDED),
            };
            for (country, sign) in [(&item.to, 1i128), (&item.from, -1)] {
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
        let mut countries: Vec<TerritoryCountry> = countries
            .into_iter()
            .map(|(key, row)| TerritoryCountry {
                key,
                tag: row.tag,
                hour: row.hour,
                gained: row.gained,
                lost: row.lost,
                net: (!partial).then(|| {
                    let net = i64::from(row.gained) - i64::from(row.lost);
                    i32::try_from(net).unwrap_or(if net < 0 { i32::MIN } else { i32::MAX })
                }),
                net_victory_points: if partial { None } else { row.victory_points.ok().map(|sum| sum.to_string()) },
                victory_points_missing: if partial { Some(PARTIAL.to_owned()) } else { row.victory_points.err().map(str::to_owned) },
            })
            .collect();
        if !partial {
            countries.sort_by(|a, b| b.net.cmp(&a.net).then(b.gained.cmp(&a.gained)).then_with(|| a.tag.cmp(&b.tag)).then_with(|| a.key.cmp(&b.key)));
        }
        let warnings = self.warnings.iter().take_while(|warning| warning.hour.is_none_or(|hour| hour <= through)).map(|warning| warning.text.clone()).collect();
        TerritoryTransfersReply {
            branch_id: self.branch_id.clone(),
            through,
            transfers: u32::try_from(counted.len()).unwrap_or(u32::MAX),
            countries,
            next_change: self.changes.get(self.changes.partition_point(|&hour| hour <= through)).copied(),
            partial: partial.then(|| PARTIAL.to_owned()),
            warnings,
        }
    }
}

struct Tally {
    tag: String,
    hour: i32,
    gained: u32,
    lost: u32,
    victory_points: Result<i128, &'static str>,
}

#[derive(Default)]
struct Indexes(HashMap<String, Arc<TransfersIndex>>);

fn index(source: &Source, branch_id: &str, cancelled: &AtomicBool) -> io::Result<Arc<TransfersIndex>> {
    let state = source.state::<Indexes>();
    if let Some(held) = state.lock().unwrap_or_else(PoisonError::into_inner).0.get(branch_id) {
        return Ok(Arc::clone(held));
    }
    let branch = branch_id.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid territory-transfers branch"))?;
    let assembled = assemble(source, branch, cancelled)?;
    let built = Arc::new(TransfersIndex::new(branch_id.to_owned(), &assembled.deals, &assembled.unobserved, assembled.warnings));
    state.lock().unwrap_or_else(PoisonError::into_inner).0.insert(branch_id.to_owned(), Arc::clone(&built));
    Ok(built)
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

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let reply = index(source, &params.branch_id, cancelled)?.answer(params.through);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
