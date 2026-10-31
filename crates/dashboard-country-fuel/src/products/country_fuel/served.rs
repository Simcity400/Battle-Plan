//! Fuel's served replies: every country's summaries of every measure over
//! each span, and its values at each hour; then, of the supplying reads those
//! values name, each detail product: the read's awards and demand, and the
//! history of its first recipient and parts over the read's window.

use dashboard_base::{
    prepared::served::{self, Ask, Plan},
    products::production_lines::DETAIL_HOURS,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io};

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[&["series"]],
};

const MEASURES: [&str; 4] = ["fuel", "capacity", "stripGain", "stripBurn"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;
/// Supplying reads a branch's detail products are asked of at most, spread over its reads, so
/// the plan stays within the check's requests a branch.
const BASES: usize = 384;
/// Parts a demand stack is asked of.
const STACKED: usize = 3;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let family = json!({ "kind": "country-fuel" });
        let countries = ask.countries(branch.id)?;
        let mut bases = BTreeSet::new();
        for batch in served::batches(&countries) {
            for (from, to) in branch.spans() {
                ask.ask(
                    "series-summary",
                    json!({
                        "branchId": id, "family": family, "tags": batch, "measures": MEASURES,
                        "from": from, "to": to, "width": WIDTH,
                    }),
                )?;
            }
            for hour in branch.hours() {
                let reads: Vec<Value> = batch
                    .iter()
                    .map(|tag| json!({ "tag": tag, "hour": hour }))
                    .collect();
                let reply = ask.ask(
                    "series-state",
                    json!({ "branchId": id, "family": family, "reads": reads }),
                )?;
                for point in reply
                    .as_ref()
                    .map(|reply| served::values_at(reply, &["states", "*"]))
                    .unwrap_or_default()
                {
                    let read = &point["fuel"];
                    if let (Some(tag), Some(hour), Some(country), Some(instance)) = (
                        point["tag"].as_str(),
                        read["firstHour"].as_i64(),
                        read["country"].as_str(),
                        read["countryInstance"].as_u64(),
                    ) {
                        bases.insert((tag.to_owned(), hour, country.to_owned(), instance));
                    }
                }
            }
        }
        let step = bases.len().div_ceil(BASES).max(1);
        for (tag, hour, country, instance) in bases.into_iter().step_by(step) {
            let basis = json!({
                "branchId": id, "tag": tag, "hour": hour, "country": country,
                "countryInstance": instance,
            });
            let with = |extra: Value| {
                let mut params = basis.clone();
                if let (Some(params), Value::Object(extra)) = (params.as_object_mut(), extra) {
                    params.extend(extra);
                }
                params
            };
            let window = hour.div_euclid(i64::from(DETAIL_HOURS));
            if let Some(awards) = ask.ask("country-fuel-awards", basis.clone())?
                && let Some(key) = served::values_at(&awards, &["awards", "recipients", "*", "key"])
                    .first()
                    .and_then(|key| key.as_str())
            {
                ask.ask(
                    "country-fuel-award-history",
                    with(json!({ "window": window, "recipientKey": key })),
                )?;
            }
            if let Some(demand) = ask.ask("country-fuel-demand", basis.clone())? {
                let keys: Vec<&str> = served::values_at(&demand, &["parts", "*", "key"])
                    .into_iter()
                    .filter_map(Value::as_str)
                    .take(STACKED)
                    .collect();
                if let Some(first) = keys.first() {
                    ask.ask(
                        "country-fuel-demand-history",
                        with(json!({ "window": window, "key": first })),
                    )?;
                    ask.ask(
                        "country-fuel-demand-stack",
                        with(json!({ "window": window, "keys": keys })),
                    )?;
                }
            }
        }
    }
    Ok(())
}
