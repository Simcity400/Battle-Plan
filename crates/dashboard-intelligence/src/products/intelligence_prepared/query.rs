//! The intelligence products' replies, read from the family's publication through its
//! accepted coverage: the hours past it are pending, never read from the recording.
use super::super::{
    agency_comparison, decryption, espionage_log, operative_placements, operatives_roster,
};
use dashboard_base::{
    Source,
    prepared::{self, query::progressive},
    products::Product,
};
use dashboard_core::config::LoadContext;
use serde_json::{Value, json};
use std::{io, sync::atomic::AtomicBool};

pub fn query(
    source: &Source,
    product: &str,
    params: Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    match product {
        "decryption" => {
            let _: decryption::prepared::Params = serde_json::from_value(params.clone())?;
        }
        "operative-placements" => {
            let _: operative_placements::prepared::Params = serde_json::from_value(params.clone())?;
        }
        "agency-comparison" => {
            let parsed: agency_comparison::prepared::Params =
                serde_json::from_value(params.clone())?;
            agency_comparison::AgencyComparison::validate(&parsed.selection)
                .map_err(dashboard_core::invalid)?;
        }
        "operatives-roster" => {
            let parsed: operatives_roster::prepared::Params =
                serde_json::from_value(params.clone())?;
            for tag in &parsed.tags {
                dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
            }
        }
        "espionage-log" => {
            let parsed = serde_json::from_value(params.clone())?;
            espionage_log::EspionageLog::validate(&parsed).map_err(dashboard_core::invalid)?;
        }
        _ => unreachable!("intelligence product"),
    }
    let selection = params.get("selection").unwrap_or(&params);
    let branch = selection
        .get("branchId")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u64>().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid intelligence branch"))?;
    let windows = source.block_windows(branch);
    let first = windows
        .first()
        .ok_or_else(|| dashboard_core::invalid("Unknown intelligence branch"))?
        .first_hour;
    let end = windows.last().expect("branch has windows").last_hour;
    let requested = params
        .get("lastHour")
        .and_then(Value::as_i64)
        .map(i32::try_from)
        .transpose()
        .map_err(|_| dashboard_core::invalid("Invalid intelligence hour"))?;
    let gate = if product == "agency-comparison"
        && selection
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|tags| !tags.is_empty())
    {
        "agency-comparison-identities"
    } else {
        product
    };
    let mut prefix = prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        gate,
        branch,
        requested,
        cancelled,
    )?;
    let accepted =
        (prefix.coverage.first_hour <= prefix.coverage.last_hour).then(|| prefix.coverage.clone());
    let (store, key) = prepared::files(source, context.content.as_ref())?;
    let identity_coverage = if product == "operatives-roster" {
        prepared::coverage::product_coverage(
            source,
            context.content.as_ref(),
            branch,
            dashboard_base::products::country_history::IDENTITIES_PRODUCT,
            cancelled,
        )?
    } else {
        None
    };
    let last = requested.unwrap_or(end);
    let from = params
        .get("firstHour")
        .and_then(Value::as_i64)
        .map(i32::try_from)
        .transpose()
        .map_err(|_| dashboard_core::invalid("Invalid intelligence hour"))?
        .unwrap_or(first);
    if from > last {
        return Err(dashboard_core::invalid("Invalid intelligence range"));
    }
    if product != "espionage-log" {
        prefix.pending.retain(|span| span.last_hour >= from);
        for span in &mut prefix.pending {
            span.first_hour = span.first_hour.max(from);
        }
    }
    let reached = accepted
        .as_ref()
        .is_some_and(|coverage| coverage.complete || last <= coverage.last_hour);
    let mut bounded = params.clone();
    if product != "espionage-log" && !reached {
        bounded["lastHour"] = json!(prefix.coverage.last_hour);
    }
    if product == "espionage-log" {
        let mut data = espionage_log::prepared::reply(
            source,
            context,
            &store,
            &key,
            branch,
            &serde_json::from_value(params)?,
            accepted.as_ref().map(|coverage| coverage.last_hour),
            prefix.coverage.complete,
            cancelled,
        )?;
        if data.get("window").is_some_and(|window| !window.is_null()) {
            let from = data["firstHour"].as_i64().expect("espionage day start") as i32;
            let to = data["lastHour"].as_i64().expect("espionage day end") as i32;
            prefix
                .pending
                .retain(|span| span.last_hour >= from && span.first_hour <= to);
            for span in &mut prefix.pending {
                span.first_hour = span.first_hour.max(from);
                span.last_hour = span.last_hour.min(to);
            }
        }
        prefix.mark_coverage(&mut data, "preparedCoverage")?;
        return Ok(data);
    }
    let mut data = None;
    if accepted.is_some() && from <= prefix.coverage.last_hour {
        data = match product {
            "decryption" => serde_json::to_value(decryption::prepared::query(
                &store,
                &key,
                branch,
                &serde_json::from_value(bounded)?,
                cancelled,
            )?)?,
            "operative-placements" => serde_json::to_value(operative_placements::prepared::query(
                &store,
                &key,
                branch,
                &serde_json::from_value(bounded)?,
                cancelled,
            )?)?,
            "agency-comparison" => serde_json::to_value(agency_comparison::prepared::query(
                source,
                context,
                &store,
                &key,
                branch,
                &serde_json::from_value(bounded)?,
                cancelled,
            )?)?,
            "operatives-roster" => serde_json::to_value(operatives_roster::prepared::query(
                source,
                context,
                &store,
                &key,
                branch,
                &serde_json::from_value(bounded)?,
                cancelled,
            )?)?,
            _ => unreachable!("intelligence product"),
        }
        .as_object()
        .cloned()
        .map(Value::Object);
    }
    let mut data = data.unwrap_or_else(|| empty(product, branch));
    if product == "operatives-roster" {
        // New aliases can add rows without moving the numeric frontier.
        data["selectionCoverage"] = serde_json::to_value(&identity_coverage)?;
    }
    if product == "operatives-roster"
        && !operatives_roster::prepared::selection_complete(
            source,
            branch,
            identity_coverage.as_ref(),
        )
    {
        // Prepared day values can be ready before the branch-wide selection.
        let preparing = data
            .as_object_mut()
            .expect("roster reply")
            .entry("preparing")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("roster preparing fields");
        if !preparing.iter().any(|field| field == "countries") {
            preparing.push(json!("countries"));
        }
    }
    // These products already use coverage for captured read evidence.
    prefix.mark_coverage(&mut data, "preparedCoverage")?;
    Ok(data)
}

fn empty(product: &str, branch: u64) -> Value {
    let mut data = match product {
        "decryption" => json!({"pairs":[],"coverage":[]}),
        "operative-placements" => json!({"pairs":[],"periods":[],"readHours":[]}),
        "agency-comparison" => {
            json!({"available":false,"observed":[],"operativeReadHours":[],"series":[],"defenseLabel":null,"upgradesUnrecorded":null})
        }
        "operatives-roster" => {
            json!({"countries":[],"operations":[],"agencyAbsent":[],"artwork":[],"reasons":[]})
        }
        _ => unreachable!("intelligence product"),
    };
    data["branchId"] = json!(branch.to_string());
    data["warnings"] = json!([]);
    data
}

pub(crate) fn histories(
    source: &Source,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<dashboard_base::products::country_history::RecordedHistories> {
    let coverage = prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        dashboard_base::products::country_history::IDENTITIES_PRODUCT,
        cancelled,
    )?;
    progressive::histories(
        store,
        key,
        branch,
        coverage.as_ref(),
        &Default::default(),
        &Default::default(),
        source.replay_ref(),
        &[],
        cancelled,
    )
}
