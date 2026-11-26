use super::peace::{PeaceDeal, PeaceTransferCountry, PeaceUnobserved, PeaceWarning, assemble, deal_known_at};
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

fn identity(country: &PeaceTransferCountry) -> String {
    format!("{}:{}", country.slot, country.country_id)
}

struct Step {
    hour: i32,
    gained: u32,
    lost: u32,
    named: usize,
}

struct Read {
    hour: i32,
    points: i128,
}

struct History {
    key: String,
    names: Vec<(String, i32)>,
    steps: Vec<Step>,
    reads: Vec<Read>,
}

impl History {
    fn row(&self, through: i32, partial: bool) -> Option<TerritoryCountry> {
        let counted = self.steps.partition_point(|step| step.hour <= through);
        let step = self.steps.get(counted.checked_sub(1)?)?;
        let read = self.reads.partition_point(|read| read.hour <= through);
        let victory_points = (read == counted).then(|| self.reads[read - 1].points);
        let (tag, hour) = &self.names[step.named];
        Some(TerritoryCountry {
            key: self.key.clone(),
            tag: tag.clone(),
            hour: *hour,
            gained: step.gained,
            lost: step.lost,
            net: (!partial).then(|| {
                let net = i64::from(step.gained) - i64::from(step.lost);
                i32::try_from(net).unwrap_or(if net < 0 { i32::MIN } else { i32::MAX })
            }),
            net_victory_points: if partial { None } else { victory_points.map(|sum| sum.to_string()) },
            victory_points_missing: if partial { Some(PARTIAL.to_owned()) } else { victory_points.is_none().then(|| VP_NOT_RECORDED.to_owned()) },
        })
    }
}

pub struct TransfersIndex {
    branch_id: String,
    shown: Vec<i32>,
    countries: Vec<History>,
    gap_starts: Vec<i32>,
    warnings: Vec<PeaceWarning>,
    changes: Vec<i32>,
}

impl TransfersIndex {
    pub fn new(branch_id: String, deals: &[PeaceDeal], unobserved: &PeaceUnobserved, mut warnings: Vec<PeaceWarning>) -> Self {
        let mut moves: Vec<(i32, &PeaceTransferCountry, &PeaceTransferCountry, Option<(i32, i128)>)> = Vec::new();
        for deal in deals {
            let known = deal_known_at(deal);
            for state in &deal.transfers {
                let shown = state.dates.after_hour.max(known);
                let worth = match (state.victory_points.as_deref().map(str::parse::<i128>), state.victory_points_read_hour) {
                    (Some(Ok(points)), Some(read)) => Some((read.max(shown), points)),
                    _ => None,
                };
                moves.push((shown, &state.to_country, &state.from_country, worth));
            }
        }
        moves.sort_by_key(|item| item.0);
        let mut histories: BTreeMap<String, History> = BTreeMap::new();
        for &(shown, to, from, worth) in &moves {
            for (country, gained) in [(to, true), (from, false)] {
                let history = histories.entry(identity(country)).or_insert_with_key(|key| History { key: key.clone(), names: Vec::new(), steps: Vec::new(), reads: Vec::new() });
                let (mut gains, mut losses, mut named) = history.steps.last().map_or((0, 0, None), |step| (step.gained, step.lost, Some(step.named)));
                if gained { gains += 1 } else { losses += 1 }
                if named.is_none_or(|at| country.hour >= history.names[at].1) {
                    history.names.push((country.tag.clone(), country.hour));
                    named = Some(history.names.len() - 1);
                }
                history.steps.push(Step { hour: shown, gained: gains, lost: losses, named: named.unwrap_or_default() });
                if let Some((hour, points)) = worth {
                    history.reads.push(Read { hour, points: if gained { points } else { -points } });
                }
            }
        }
        let mut countries: Vec<History> = histories.into_values().collect();
        for history in &mut countries {
            history.reads.sort_by_key(|read| read.hour);
            let mut sum = 0_i128;
            for read in &mut history.reads {
                sum += read.points;
                read.points = sum;
            }
        }
        let mut gap_starts: Vec<i32> = unobserved.settlements.iter().chain(&unobserved.ownership).map(|span| span.first_hour).collect();
        gap_starts.sort_unstable();
        gap_starts.dedup();
        warnings.sort_by_key(|warning| warning.hour);
        let shown: Vec<i32> = moves.iter().map(|item| item.0).collect();
        let mut changes: Vec<i32> = shown
            .iter()
            .copied()
            .chain(moves.iter().filter_map(|item| item.3.map(|(hour, _)| hour)))
            .chain(gap_starts.iter().copied())
            .chain(warnings.iter().filter_map(|warning| warning.hour))
            .collect();
        changes.sort_unstable();
        changes.dedup();
        Self { branch_id, shown, countries, gap_starts, warnings, changes }
    }

    pub fn answer(&self, through: i32) -> TerritoryTransfersReply {
        let partial = self.gap_starts.first().is_some_and(|&first| first <= through);
        let mut countries: Vec<TerritoryCountry> = self.countries.iter().filter_map(|history| history.row(through, partial)).collect();
        if !partial {
            countries.sort_by(|a, b| b.net.cmp(&a.net).then(b.gained.cmp(&a.gained)).then_with(|| a.tag.cmp(&b.tag)).then_with(|| a.key.cmp(&b.key)));
        }
        let warnings = self.warnings.iter().take_while(|warning| warning.hour.is_none_or(|hour| hour <= through)).map(|warning| warning.text.clone()).collect();
        TerritoryTransfersReply {
            branch_id: self.branch_id.clone(),
            through,
            transfers: u32::try_from(self.shown.partition_point(|&hour| hour <= through)).unwrap_or(u32::MAX),
            countries,
            next_change: self.changes.get(self.changes.partition_point(|&hour| hour <= through)).copied(),
            partial: partial.then(|| PARTIAL.to_owned()),
            warnings,
        }
    }
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
