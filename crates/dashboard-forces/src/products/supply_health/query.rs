//! Supply health's replies, read from the family's series publication ([`super::emit`]) and
//! joined with the countries' identity histories: a country's runs only while a selected
//! holder holds its tag, and, over a viewport wider than its pixels, the summaries of each
//! count's share with the runs their buckets' edges name.

use super::{
    SupplyHealth, SupplyHealthDisplay, SupplyHealthParams, SupplyHealthReply, SupplyHealthRun,
    SupplyHealthSeries,
    emit::{AVAILABLE, COUNTS, FAILURE, SHARE_DECIMALS, SHARES, WARNINGS, unpack},
};
use dashboard_base::{
    prepared::{engine, series::Navigation},
    products::{
        MAX_PRODUCT_REPLY_BYTES, Product,
        country_groups::HourSpan,
        country_history::{
            RecordedHistories, selected_aliases, selected_identities, selected_spans,
        },
    },
};
use dashboard_core::transport::json_len;
use dashboard_series::{PageRead, Snapshot, family::Directory};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

/// The key no definition has: an absent series reads as no value at every hour.
const ABSENT: u64 = u64::MAX;

#[derive(Deserialize)]
struct Viewport {
    from: i32,
    to: i32,
    width: Option<u32>,
}

fn reply_too_large() -> io::Error {
    io::Error::other(
        "The complete supply health reply exceeds MAX_PRODUCT_REPLY_BYTES; nothing was omitted",
    )
}

fn ensure(reply: &SupplyHealthReply) -> io::Result<()> {
    if json_len(reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(reply_too_large());
    }
    Ok(())
}

fn empty(tags: &[String]) -> Vec<SupplyHealthSeries> {
    let mut seen = BTreeSet::new();
    tags.iter()
        .filter(|tag| seen.insert(tag.as_str()))
        .map(|tag| SupplyHealthSeries {
            tag: tag.clone(),
            identities: None,
            runs: Vec::new(),
        })
        .collect()
}

fn reply(
    branch_id: String,
    available: bool,
    series: Vec<SupplyHealthSeries>,
    warnings: Vec<String>,
) -> io::Result<SupplyHealthReply> {
    let reply = SupplyHealthReply {
        branch_id,
        available,
        reason: (!available).then(|| "DivisionLandInputs is not in this recording.".into()),
        missing_inputs: if available {
            Vec::new()
        } else {
            vec!["DivisionLandInputs".into()]
        },
        series,
        warnings,
        summaries: None,
    };
    ensure(&reply)?;
    Ok(reply)
}

/// Whether any hour held DivisionLandInputs, and the warnings of an available family, or why
/// its content was refused.
fn scalars(directory: &Directory) -> (bool, Option<String>, Vec<String>) {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let available = matches!(
        directory.scalar(AVAILABLE),
        Some(dashboard_series::family::Scalar::Boolean(true))
    );
    let failure = directory
        .scalar(FAILURE)
        .and_then(|failure| failure.text())
        .map(text);
    let warnings = directory
        .scalar(WARNINGS)
        .map(|warnings| {
            warnings
                .list()
                .iter()
                .filter_map(|warning| warning.text())
                .map(text)
                .collect()
        })
        .unwrap_or_default();
    (available, failure, warnings)
}

/// The supply health reply to `params` over the published hours.
pub fn reply_to<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &Value,
    history: &RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<SupplyHealthReply> {
    let Viewport { from, to, width } = serde_json::from_value(params.clone())?;
    if from > to {
        return Err(dashboard_core::invalid("Invalid supply viewport"));
    }
    if width == Some(0) {
        return Err(dashboard_core::invalid("Invalid supply summary width"));
    }
    let params: SupplyHealthParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"],
    }))?;
    SupplyHealth::validate(&params).map_err(dashboard_core::invalid)?;
    let (available, failure, warnings) = scalars(directory);
    if !available {
        return reply(params.branch_id, false, empty(&params.tags), Vec::new());
    }
    if let Some(failure) = failure {
        return Err(dashboard_core::invalid(failure));
    }
    let selected = selected_identities(history, &params.tags);
    let wanted = selected_aliases(history, &params.tags);
    let mut series = empty(&wanted);
    let mut display = Vec::new();
    let summarised = width.filter(|width| i64::from(to) - i64::from(from) + 1 > i64::from(*width));
    for row in &mut series {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Some(actor) = directory.actor(row.tag.as_bytes()) else {
            continue;
        };
        let key = |measure| directory.key(actor, measure, None).unwrap_or(ABSENT);
        let identities: Vec<_> = history
            .countries
            .get(&row.tag)
            .into_iter()
            .flat_map(|country| country.identities.iter())
            .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
            .cloned()
            .collect();
        let spans: Vec<_> = identities
            .iter()
            .map(|identity| HourSpan {
                first_hour: identity.first_hour.max(from),
                last_hour: identity.last_hour.min(to),
            })
            .filter(|span| span.first_hour <= span.last_hour)
            .collect();
        let runs = if let Some(width) = summarised {
            let view = (f64::from(from), f64::from(to), width);
            // Only the hours a selected native country holds the alias are its.
            let mut dates = BTreeSet::new();
            let counts = engine::summary(snapshot, key(COUNTS), (from, to), view, 0, Some(&spans))?;
            for bucket in counts.buckets.iter().flatten() {
                dates.extend([bucket.first_hour, bucket.last_hour]);
            }
            let mut summaries = Vec::with_capacity(SHARES.len());
            for measure in SHARES {
                let summary = engine::summary(
                    snapshot,
                    key(measure),
                    (from, to),
                    view,
                    SHARE_DECIMALS,
                    Some(&spans),
                )?;
                for bucket in summary.buckets.iter().flatten() {
                    dates.extend([
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.max_hour,
                    ]);
                }
                summaries.push(summary);
            }
            display.push(SupplyHealthDisplay {
                tag: row.tag.clone(),
                values: summaries,
            });
            let mut runs = BTreeMap::new();
            for hour in dates {
                if let Some((mut run, _)) = engine::run_at(snapshot, key(COUNTS), hour)? {
                    run.span.first = run.span.first.max(from);
                    run.span.last = run.span.last.min(to);
                    runs.insert(run.span.first, run);
                }
            }
            runs.into_values().collect()
        } else {
            engine::runs(snapshot, key(COUNTS), from, to)?
        };
        dashboard_base::ranking::for_each_piece(
            &runs,
            &spans,
            |run| (run.span.first, run.span.last),
            cancelled,
            |run, first, last| {
                let [no_supply, strained, in_supply] = unpack(run.value)?;
                row.runs.push(SupplyHealthRun {
                    first_hour: first,
                    last_hour: last,
                    starts_segment: run.break_before,
                    no_supply,
                    strained,
                    in_supply,
                });
                Ok(())
            },
        )?;
        row.identities = Some(identities);
    }
    let mut reply = reply(params.branch_id, true, series, warnings)?;
    reply.summaries = (!display.is_empty()).then_some(display);
    ensure(&reply)?;
    Ok(reply)
}

/// The hour a keyboard step over the selected countries' runs reaches, each only while a
/// selected native country holds its tag.
pub fn next_change<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &Value,
    history: &RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let selection: SupplyHealthParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"],
    }))?;
    SupplyHealth::validate(&selection).map_err(dashboard_core::invalid)?;
    let navigation: Navigation = serde_json::from_value(params.clone())?;
    let selected = selected_spans(history, &selection.tags, &BTreeMap::new(), cancelled)?;
    let keys: Vec<_> = selected
        .iter()
        .filter_map(|(tag, spans)| {
            let actor = directory.actor(tag.as_bytes())?;
            Some((
                directory.key(actor, COUNTS, None).unwrap_or(ABSENT),
                Some(spans.as_slice()),
            ))
        })
        .collect();
    let hour = engine::navigate(snapshot, &keys, &navigation)?;
    Ok(json!({ "hour": hour }))
}
