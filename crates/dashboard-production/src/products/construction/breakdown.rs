use super::spending::{ConstructionSpendingParams, ConstructionSpendingReply, SpendingLeaf, history};
use super::SpendingGap;
use dashboard_base::products::breakdown::{self, Amount, Breakdown, Leaf, Node, Unrecorded};
use dashboard_base::products::{Country, LoadContext, Product, validate_country};
use dashboard_base::Source;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstructionBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConstructionBreakdownReply {
    #[doc = " The branch's first hour: spending is counted from it."]
    pub from: i32,
    pub hour: i32,
    #[doc = " The breakdown stands unchanged from `hour` through this hour."]
    pub through: i32,
    pub breakdown: Breakdown,
    #[doc = " Projects whose output before observation began is not counted."]
    pub before_first_read: u32,
    #[doc = " Projects whose output no country can be given, not counted."]
    pub unattributed: u32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ConstructionBreakdownReply);

pub struct ConstructionBreakdown;

impl Product for ConstructionBreakdown {
    const KEY: &'static str = "construction-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ConstructionBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: End];
    type Reply = ConstructionBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ConstructionBreakdownParams::decl(), ConstructionBreakdownReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let whole = ConstructionSpendingParams { branch_id: params.branch_id.clone(), country: params.country.clone() };
        let key = (whole.clone(), query.timeline.first, query.timeline.last);
        let reply = breakdown::held(source, key, || history(source, &whole, context, cancelled))?;
        Ok(at_hour(&reply, query.timeline.first, params.hour))
    }
}

fn gap_reason(gap: SpendingGap) -> &'static str {
    match gap {
        SpendingGap::NotRecorded => "Some output was not recorded",
        SpendingGap::LinkedOutput => "Some output was shared with another project and not recorded exactly",
        SpendingGap::InexactOutput => "Some output was not recorded exactly",
        SpendingGap::ObservationRestart => "Observation restarted, so some output was not counted",
        SpendingGap::FinalOutputMissing => "A project's final output was not recorded",
        SpendingGap::OwnerUnknown => "Some output was not traced to a country",
    }
}

fn total_at(leaf: &SpendingLeaf, hour: i32) -> Result<i128, ()> {
    let at = leaf.spent.partition_point(|step| step.known_at <= hour);
    match at.checked_sub(1).map(|at| &leaf.spent[at]) {
        None => Ok(0),
        Some(step) => step.total_raw.as_deref().and_then(|raw| raw.parse().ok()).ok_or(()),
    }
}

fn type_name(reply: &ConstructionSpendingReply, building: Option<u32>) -> String {
    let found = building.and_then(|id| reply.types.iter().find(|kind| kind.id == id));
    found.and_then(|kind| kind.name.clone().or_else(|| kind.token.clone())).unwrap_or_else(|| "Building name unavailable".into())
}

pub(super) fn at_hour(reply: &ConstructionSpendingReply, from: i32, hour: i32) -> ConstructionBreakdownReply {
    let mut through = i32::MAX;
    let mut later = |at: i32| {
        if at > hour {
            through = through.min(at - 1);
        }
    };
    let mut types: HashMap<Option<u32>, u32> = HashMap::new();
    let mut leaves = Vec::new();
    for leaf in &reply.leaves {
        for step in &leaf.spent {
            later(step.known_at);
        }
        let gaps: Vec<SpendingGap> = leaf
            .unknown
            .iter()
            .inspect(|gap| {
                later(gap.known_at);
                if let Some(until) = gap.until {
                    later(until);
                }
            })
            .filter(|gap| gap.known_at <= hour && gap.until.is_none_or(|until| until > hour))
            .map(|gap| gap.reason)
            .collect();
        let started = leaf.projects.iter().any(|project| project.first_read <= hour);
        let amount = match total_at(leaf, hour) {
            Err(()) => Amount::NotRecorded("Spending was not recorded at this hour".into()),
            Ok(0) if gaps.is_empty() => {
                if !started {
                    continue;
                }
                Amount::Known(0)
            }
            Ok(total) => match gaps.first() {
                None => Amount::Known(total),
                Some(&gap) => Amount::Partial(total, gap_reason(gap).into()),
            },
        };
        let purpose = if leaf.repair { Node::new("repair", "Repairs", 1) } else { Node::new("new", "New buildings", 0) };
        let next = types.len() as u32;
        let ordinal = *types.entry(leaf.building).or_insert(next);
        let kind = Node::new(format!("type:{}", leaf.building.map_or_else(|| "none".to_owned(), |id| id.to_string())), type_name(reply, leaf.building), ordinal);
        leaves.push(Leaf { path: vec![purpose, kind], amount, count: None, last: None });
    }
    let mut unrecorded = Unrecorded::default();
    for span in &reply.missing {
        later(span.first_hour);
        later(span.last_hour.saturating_add(1));
        let (first, last) = (span.first_hour.max(from), span.last_hour.min(hour));
        if first > last {
            continue;
        }
        unrecorded.hours = unrecorded.hours.saturating_add(u32::try_from(last - first + 1).unwrap_or(u32::MAX));
        unrecorded.reason.get_or_insert_with(|| "Output in hours that were not recorded is not counted".into());
    }
    let decimals = reply.cost_scale.checked_ilog10().unwrap_or(0) as u8;
    let unattributed = reply.unattributed.iter().filter(|gap| gap.known_at <= hour).count() as u32;
    ConstructionBreakdownReply {
        from,
        hour,
        through: through.max(hour),
        breakdown: breakdown::breakdown(decimals, &leaves, &unrecorded),
        before_first_read: reply.before_first_read,
        unattributed,
        warnings: reply.warnings.clone(),
    }
}
