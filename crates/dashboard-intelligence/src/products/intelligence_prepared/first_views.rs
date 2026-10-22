//! Prepared history first; independent screen values beyond its frontier.
use super::super::{
    agency_comparison, decryption, espionage_log, operative_placements, operatives_roster,
};
use dashboard_base::products::Product;
use dashboard_base::{
    Source,
    prepared::{
        self,
        query::{observations, progressive},
    },
};
use dashboard_core::config::LoadContext;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{cell::RefCell, io, sync::atomic::AtomicBool};

fn screen(
    source: &Source,
    key: &store::SourceKey,
    product: &'static str,
    branch: u64,
    params: &Value,
    cancelled: &AtomicBool,
    read: impl FnOnce() -> io::Result<Value>,
) -> io::Result<Value> {
    let replay = source.replay_ref();
    let last = params["lastHour"].as_i64().expect("screen day end");
    let block = replay
        .index
        .iter()
        .rev()
        .find(|block| block.branch == branch && i64::from(block.first_hour) <= last)
        .or_else(|| replay.index.iter().find(|block| block.branch == branch))
        .expect("screen branch has blocks");
    let cache = observations::scope(source, product, Some(key));
    cache
        .get(block, params, cancelled, observations::json_bytes, read)
        .map(|value| (*value).clone())
}

/// Visit the supplying block and its predecessor through the shared frame cache.
/// The observations before the requested day supply held reads, never history dates.
pub(crate) fn visit(
    source: &Source,
    branch: u64,
    first: i32,
    last: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
    observe: impl FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<()> {
    visit_from(
        source, branch, first, last, tables, cancelled, true, observe,
    )
}

/// The screen's daily supplying read is in this block unless the day starts
/// within a read cadence of its keyframe. Missing evidence stays pending.
pub(crate) fn day(
    source: &Source,
    branch: u64,
    first: i32,
    last: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
    observe: impl FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<()> {
    let predecessor = source
        .block_windows(branch)
        .iter()
        .find(|block| block.first_hour <= first && block.last_hour >= first)
        .is_some_and(|block| i64::from(first) - i64::from(block.first_hour) < 24);
    visit_from(
        source,
        branch,
        first,
        last,
        tables,
        cancelled,
        predecessor,
        observe,
    )
}

#[allow(clippy::too_many_arguments)]
fn visit_from(
    source: &Source,
    branch: u64,
    first: i32,
    last: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
    predecessor: bool,
    mut observe: impl FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<()> {
    let observe = RefCell::new(&mut observe);
    let sample = |frame: &DecodedFrame| -> io::Result<progressive::Samples> {
        if frame.hour <= last {
            (observe.borrow_mut())(frame)?;
        }
        Ok(Default::default())
    };
    let mut hours = (first..=last).collect::<Vec<_>>();
    if predecessor {
        let windows = source.block_windows(branch);
        if let Some(before) = windows
            .iter()
            .position(|block| block.first_hour <= last && first <= block.last_hour)
            .and_then(|index| index.checked_sub(1))
        {
            hours.push(windows[before].first_hour);
        }
    }
    progressive::state_tail(
        source,
        branch,
        None,
        tables,
        &[],
        &hours,
        &|frame, _| sample(frame),
        cancelled,
    )?;
    Ok(())
}

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
        let mut data = espionage_log::prepared::first_view(
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
        if let Some(exact) = data
            .get("exact")
            .and_then(Value::as_i64)
            .and_then(|hour| i32::try_from(hour).ok())
        {
            prefix.sampled(&[dashboard_base::products::country_groups::HourSpan {
                first_hour: exact,
                last_hour: end,
            }]);
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
    // Day requests have bounded work. Whole histories keep their exact prefix.
    if !reached && gate != "agency-comparison-identities" && i64::from(last) - i64::from(from) < 24
    {
        let mut day_params = params.clone();
        day_params["firstHour"] = json!(from);
        day_params["lastHour"] = json!(last);
        let cache_product = match product {
            "decryption" => "decryption:day",
            "operative-placements" => "operative-placements:day",
            "agency-comparison" => "agency-comparison:day",
            "operatives-roster" => "operatives-roster:day",
            _ => unreachable!("screen product"),
        };
        let mut screen_params = day_params.clone();
        if product == "operatives-roster" {
            // Historical holder selection follows the accepted identity page.
            // A pending day must be reconsidered when that prefix advances.
            screen_params["identityCoverage"] = serde_json::to_value(&identity_coverage)?;
        }
        data = Some(screen(
            source,
            &key,
            cache_product,
            branch,
            &screen_params,
            cancelled,
            || {
                Ok(match product {
                    "decryption" => decryption::prepared::first_view(
                        source,
                        &serde_json::from_value(day_params.clone())?,
                        context,
                        cancelled,
                    )?,
                    "operative-placements" => operative_placements::prepared::first_view(
                        source, branch, from, last, context, cancelled,
                    )?,
                    "agency-comparison" => agency_comparison::prepared::first_view(
                        source,
                        &serde_json::from_value(day_params.clone())?,
                        context,
                        cancelled,
                    )?,
                    "operatives-roster" => operatives_roster::prepared::first_view(
                        source,
                        &serde_json::from_value(day_params.clone())?,
                        context,
                        cancelled,
                    )?,
                    _ => unreachable!("screen product"),
                })
            },
        )?);
        let supplying = data.as_ref().and_then(|data| data.get("sampledFrom"));
        let exact_from = match supplying {
            None => Some(from),
            Some(value) => value
                .as_i64()
                .and_then(|hour| i32::try_from(hour).ok())
                .map(|hour| hour.max(from)),
        };
        if let Some(first_hour) = exact_from {
            prefix.sampled(&[dashboard_base::products::country_groups::HourSpan {
                first_hour,
                last_hour: last,
            }]);
        }
        if let Some(data) = &mut data {
            data.as_object_mut()
                .expect("screen reply")
                .remove("sampledFrom");
            // The screen cache keeps the roster's artwork as descriptors; this request draws
            // them as its own selection shows them.
            if product == "operatives-roster" {
                operatives_roster::present_artwork_json(source, context, data)?;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{
        continuation_records, file_source, replay, source as frame,
    };
    use std::collections::BTreeSet;

    #[test]
    fn a_state_read_stops_decoding_at_its_last_requested_hour() {
        let (source, root) =
            file_source(replay((0..100).map(|hour| frame(1, hour, None)).collect()));
        let observed = RefCell::new(Vec::new());
        progressive::state_tail(
            &source,
            1,
            None,
            &[TableId::Campaign],
            &[],
            &[1],
            &|frame, _| {
                observed.borrow_mut().push(frame.hour);
                Ok(Default::default())
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(observed.into_inner(), [0, 1]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn repeated_visits_keep_the_predecessor_and_stop_at_the_requested_hour() {
        let recorded = replay(vec![frame(1, 0, None), frame(1, 1, None)]);
        let recorded = recorded
            .append(&[continuation_records(
                &recorded,
                1,
                vec![frame(1, 2, None), frame(1, 3, None), frame(1, 4, None)],
            )])
            .unwrap();
        let (source, root) = file_source(recorded);
        for _ in 0..2 {
            let mut observed = Vec::new();
            visit(
                &source,
                1,
                2,
                3,
                &[TableId::Campaign],
                &AtomicBool::new(false),
                |frame| {
                    observed.push(frame.hour);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(observed, [0, 1, 2, 3]);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_day_of_short_blocks_visits_every_hour_exactly() {
        let mut recorded = replay(vec![frame(1, 0, None), frame(1, 1, None)]);
        let mut hour = 2;
        while hour < 24 {
            let count = if hour % 3 == 0 { 2 } else { 1 };
            let end = (hour + count).min(24);
            recorded = recorded
                .append(&[continuation_records(
                    &recorded,
                    1,
                    (hour..end).map(|hour| frame(1, hour, None)).collect(),
                )])
                .unwrap();
            hour = end;
        }
        let (source, root) = file_source(recorded);
        let mut observed = BTreeSet::new();
        visit(
            &source,
            1,
            0,
            23,
            &[TableId::Campaign],
            &AtomicBool::new(false),
            |frame| {
                observed.insert(frame.hour);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(observed, (0..24).collect());
        let _ = std::fs::remove_dir_all(root);
    }
}
