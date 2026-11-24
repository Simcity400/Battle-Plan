use super::{PowerSpendingParams, PowerSpendingReply, SpendCurrency, naming, spending};
use dashboard_base::products::breakdown::{self, Amount, Breakdown, Leaf, Node, Unrecorded};
use dashboard_base::products::{Country, LoadContext, Product, check_reply_budget, validate_country};
use dashboard_base::Source;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

const AHEAD: i32 = 24 * 31;
const OTHER_CATEGORY: &str = "notIdentified";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PowerSpendingBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub currency: SpendCurrency,
    #[doc = " The period's first hour."]
    pub from: i32,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerSpendingBreakdownReply {
    pub from: i32,
    pub hour: i32,
    #[doc = " The breakdown stands unchanged from `hour` through this hour."]
    pub through: i32,
    pub breakdown: Breakdown,
    #[doc = " Gained over the same period before the cap, scaled by the breakdown's decimals; none when not"]
    #[doc = " recorded whole."]
    pub gained_raw: Option<String>,
    pub lost_at_cap_raw: Option<String>,
    pub gained_reason: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerSpendingBreakdownReply);

pub struct PowerSpendingBreakdown;

impl Product for PowerSpendingBreakdown {
    const KEY: &'static str = "power-spending-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PowerSpendingBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, currency, from: Start, hour: End];
    type Reply = PowerSpendingBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use dashboard_base::products::breakdown::{BreakdownPart, BreakdownState, BreakdownTotal};
        vec![
            PowerSpendingBreakdownParams::decl(),
            PowerSpendingBreakdownReply::decl(),
            SpendCurrency::decl(),
            Breakdown::decl(),
            BreakdownTotal::decl(),
            BreakdownPart::decl(),
            BreakdownState::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid power spending branch")?;
        validate_country(&params.country)?;
        if params.from > params.hour {
            return Err("A power spending breakdown ends before it starts");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid power spending branch"))?)?;
        let naming = naming::context(source, context)?;
        let span = PowerSpendingParams {
            branch_id: params.branch_id.clone(),
            country: params.country.clone(),
            currency: params.currency,
            from: params.from,
            to: params.hour.saturating_add(AHEAD).min(query.timeline.last.max(params.hour)),
        };
        let reply = at_hour(&spending(source, &query, &span, &naming, cancelled)?, params.hour);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn parse(raw: Option<&str>) -> Option<i128> {
    raw.and_then(|raw| raw.parse().ok())
}

fn plural(count: u32, one: &str) -> String {
    format!("{count} {one}{}", if count == 1 { "" } else { "s" })
}

struct Entry {
    item: u32,
    read: i32,
    charge: Option<i128>,
    payments: u32,
    unknown: u32,
    uncovered: bool,
}

pub(super) fn at_hour(reply: &PowerSpendingReply, hour: i32) -> PowerSpendingBreakdownReply {
    let mut entries: Vec<Entry> = reply
        .payments
        .iter()
        .map(|payment| Entry {
            item: payment.item,
            read: payment.read,
            charge: parse(payment.known_raw.as_deref()),
            payments: payment.payments,
            unknown: payment.unknown,
            uncovered: payment.uncovered,
        })
        .chain(reply.days.iter().map(|day| Entry {
            item: day.item,
            read: day.read,
            charge: parse(day.charge_raw.as_deref()),
            payments: day.payments,
            unknown: day.unknown,
            uncovered: day.uncovered > 0,
        }))
        .collect();
    entries.sort_by_key(|entry| (entry.read, entry.item));
    let mut through = reply.to;
    let names: HashMap<&str, (&str, bool)> = reply.categories.iter().map(|category| (category.id.as_str(), (category.name.as_str(), category.identified))).collect();
    let mut categories: HashMap<String, u32> = HashMap::new();
    let mut groups: HashMap<String, u32> = HashMap::new();
    let mut named: HashMap<String, String> = HashMap::new();
    let mut leaves = Vec::new();
    for entry in &entries {
        if entry.read > hour {
            through = through.min(entry.read - 1);
            continue;
        }
        let Some(item) = reply.items.get(entry.item as usize) else { continue };
        let ordinal = categories.len() as u32;
        let category_ordinal = *categories.entry(item.category.clone()).or_insert(ordinal);
        let (name, identified) = names.get(item.category.as_str()).copied().unwrap_or((OTHER_CATEGORY, false));
        let mut category = Node::new(item.category.clone(), if identified { name } else { "Other spending" }, category_ordinal);
        if !identified {
            category = category.other();
        }
        let path = match &item.name {
            None => vec![category],
            Some(name) => {
                named.insert(item.group_id.clone(), name.clone());
                let ordinal = groups.len() as u32;
                let group_ordinal = *groups.entry(item.group_id.clone()).or_insert(ordinal);
                vec![category, Node::new(item.group_id.clone(), name.clone(), group_ordinal)]
            }
        };
        let count = Some(u64::from(entry.payments));
        let unknown = if entry.charge.is_none() { entry.payments.max(1) } else { entry.unknown };
        if let Some(charge) = entry.charge {
            let amount = if entry.uncovered { Amount::Partial(charge, "Some payments may be missing.".into()) } else { Amount::Known(charge) };
            leaves.push(Leaf { path: path.clone(), amount, count, last: Some(entry.read) });
        }
        if unknown > 0 {
            let reason = format!("The cost of {} was not recorded.", plural(unknown, "payment"));
            leaves.push(Leaf { path, amount: Amount::NotRecorded(reason), count: entry.charge.is_none().then_some(count).flatten(), last: Some(entry.read) });
        }
    }
    for leaf in &mut leaves {
        if let Some(node) = leaf.path.get_mut(1)
            && let Some(name) = named.get(&node.key)
        {
            node.name.clone_from(name);
        }
    }
    let mut unrecorded = Unrecorded::default();
    for run in reply.coverage.iter().filter(|run| !run.covered) {
        if run.first_hour > hour {
            through = through.min(run.first_hour - 1);
            continue;
        }
        let (first, last) = (run.first_hour.max(reply.from), run.last_hour.min(hour));
        if first > last {
            continue;
        }
        if run.last_hour > hour {
            through = hour;
        }
        unrecorded.hours = unrecorded.hours.saturating_add(u32::try_from(last - first + 1).unwrap_or(u32::MAX));
        unrecorded.reason.get_or_insert_with(|| run.reason.clone().unwrap_or_else(|| "Spending was not recorded".into()));
    }
    let (mut gained, mut lost, mut gained_reason) = (Some(0_i128), Some(0_i128), None);
    for gain in &reply.gains {
        if gain.read > hour {
            through = through.min(gain.read - 1);
            continue;
        }
        match parse(gain.gained_raw.as_deref()) {
            Some(value) if gain.unknown_days == 0 => gained = gained.map(|held| held.saturating_add(value)),
            _ => {
                gained = None;
                gained_reason.get_or_insert_with(|| "Some gains were not recorded.".to_owned());
            }
        }
        lost = match (lost, parse(gain.capped_raw.as_deref())) {
            (Some(held), Some(value)) => Some(held.saturating_add(value)),
            _ => None,
        };
    }
    PowerSpendingBreakdownReply {
        from: reply.from,
        hour,
        through: through.max(hour),
        breakdown: breakdown::breakdown(reply.decimals, &leaves, &unrecorded),
        gained_raw: gained.map(|value| value.to_string()),
        lost_at_cap_raw: lost.map(|value| value.to_string()),
        gained_reason,
        warnings: reply.warnings.clone(),
    }
}
