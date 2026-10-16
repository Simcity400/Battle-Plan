//! Host-direct reads of the integrated slice's prepared pages.

pub(crate) use dashboard_base::prepared::query::progressive;

#[cfg(test)]
mod forces_tests;

use crate::{
    Source,
    metadata::check_cancel,
    products::{ContentOptions, Product},
    transport::{ProductRequest, bounded_json},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, sync::atomic::AtomicBool};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusPoint {
    branch_id: String,
    tag: String,
    hour: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusUnavailable {
    branch_id: String,
    hour: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusStaticText {
    tokens: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusPage {
    branch_id: String,
    tag: String,
    hour: i32,
    pages: crate::prepared_focus::Pages,
    page: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductionSelection {
    hour: i32,
    selection: crate::products::equipment_produced::ProducedParams,
    offset: String,
    limit: usize,
    #[serde(default)]
    ranked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reply<'a> {
    source_id: &'a str,
    revision: &'a str,
    product: &'a str,
    data: Value,
}

fn branch(source: &Source, value: &str) -> io::Result<u64> {
    let id = value
        .parse()
        .map_err(|_| crate::invalid("Invalid prepared query branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|branch| branch.id == id)
    {
        return Err(crate::invalid(
            "Prepared query branch is not in this recording",
        ));
    }
    Ok(id)
}

fn coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    hour: Option<i32>,
    require_complete: bool,
    cancelled: &AtomicBool,
) -> io::Result<bool> {
    let Some(coverage) = super::pipeline::coverage(source, content, branch, cancelled)? else {
        return Ok(false);
    };
    if !coverage.complete
        && (require_complete || hour.is_some_and(|hour| hour > coverage.last_hour))
    {
        return Ok(false);
    }
    Ok(true)
}

fn series_coverage(product: &str, params: &Value) -> io::Result<(Option<i32>, bool)> {
    let invalid = || crate::invalid("Invalid series query hour");
    let integer = |value: &Value| -> io::Result<i32> {
        i32::try_from(value.as_i64().ok_or_else(invalid)?).map_err(|_| invalid())
    };
    let terminal = |field: &str| -> io::Result<i32> {
        let hour = params
            .get(field)
            .and_then(Value::as_f64)
            .ok_or_else(invalid)?
            .ceil();
        if !hour.is_finite() || hour < f64::from(i32::MIN) || hour > f64::from(i32::MAX) {
            return Err(invalid());
        }
        Ok(hour as i32)
    };
    match product {
        "series-state" => {
            let reads = params
                .get("reads")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            let mut last = None;
            for read in reads {
                let hour = integer(read.get("hour").ok_or_else(invalid)?)?;
                last = Some(last.map_or(hour, |old: i32| old.max(hour)));
            }
            Ok((last, false))
        }
        "series-summary" | "events-clusters" => Ok((Some(terminal("to")?), false)),
        "series-next-change" => Ok((Some(terminal("lastHour")?), false)),
        "events-page" => match params.get("to").filter(|value| !value.is_null()) {
            Some(_) => Ok((Some(terminal("to")?), false)),
            None => Ok((None, true)),
        },
        _ => unreachable!("only explicit series products reach this helper"),
    }
}

fn query_data(
    source: &Source,
    request: &ProductRequest,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let (store, key) = super::files(source, content)?;
    Ok(match request.product.as_str() {
        "party-history" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing party branch"))?;
            let branch = branch(source, id)?;
            let accepted = match super::pipeline::coverage(source, content, branch, cancelled) {
                Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
                other => other?,
            };
            // The full parity history includes catalogues first seen later. Native
            // popularity and prepared event pages can answer during preparation.
            if request.params.get("mode").and_then(Value::as_str) == Some("history")
                && !accepted.as_ref().is_some_and(|coverage| coverage.complete)
            {
                return Ok(Value::Null);
            }
            crate::products::party_history::prepared::query(
                source,
                serde_json::from_value(request.params.clone())?,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                accepted.as_ref(),
                cancelled,
            )?
        }

        "scoreboard" => crate::products::scoreboard::prepared::query(
            source,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            &store,
            &key,
            cancelled,
        )?,
        "equipment-stockpile" => {
            serde_json::to_value(crate::products::equipment_stockpile::prepared::query(
                source,
                &request.params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                cancelled,
            )?)?
        }
        product @ ("land-battles"
        | "land-battles-manifest"
        | "land-battles-ranking"
        | "land-battle-course") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid battle branch"))?;
            let branch = branch(source, id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::land_battles::prepared::query(
                source,
                product,
                request.params.clone(),
                &store,
                &key,
                cancelled,
            )?
        }
        product @ ("naval-battles"
        | "naval-battles-manifest"
        | "naval-battles-ranking"
        | "naval-battle-details") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid battle branch"))?;
            let branch = branch(source, id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::naval_battles::prepared::query(
                source,
                product,
                request.params.clone(),
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?
        }
        product @ ("division-log" | "division-log-manifest" | "division-log-totals") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid division log branch"))?;
            let branch = branch(source, id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::division_log::prepared::query(
                source,
                product,
                request.params.clone(),
                &store,
                &key,
                cancelled,
            )?
        }
        product @ ("ships-sunk" | "ships-sunk-manifest" | "ships-sunk-battle-titles") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid ships sunk branch"))?;
            let branch = branch(source, id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::ships_sunk::prepared::query(
                source,
                product,
                request.params.clone(),
                &store,
                &key,
                cancelled,
            )?
        }
        "naval-losses" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid naval loss branch"))?;
            let branch = branch(source, id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::naval_losses::prepared::query(
                source,
                "naval-losses",
                &request.params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?
        }
        "equipment-losses" => crate::products::equipment_losses::prepared::query(
            source,
            &request.params,
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "army-size" | "supply-health" => {
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            let mut data = if request.product == "army-size" {
                serde_json::to_value(crate::products::army_size::prepared::query(
                    source,
                    &serde_json::from_value(request.params.clone())?,
                    &context,
                    cancelled,
                )?)?
            } else {
                serde_json::to_value(crate::products::supply_health::prepared::query(
                    source,
                    &request.params,
                    &context,
                    cancelled,
                )?)?
            };
            if let Some(reply) = data.as_object_mut() {
                let id = request.params["branchId"]
                    .as_str()
                    .ok_or_else(|| crate::invalid("Invalid force branch"))?;
                reply.insert(
                    "coverage".into(),
                    serde_json::to_value(super::pipeline::coverage(
                        source,
                        content,
                        branch(source, id)?,
                        cancelled,
                    )?)?,
                );
            }
            data
        }
        "event-feed" => {
            let params: crate::products::event_feed::prepared::query::Params =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            let accepted = super::pipeline::coverage(source, content, branch, cancelled)?;
            if !accepted
                .as_ref()
                .is_some_and(|coverage| coverage.complete || params.last_hour <= coverage.last_hour)
            {
                return Ok(Value::Null);
            }
            let mut data = serde_json::to_value(crate::products::event_feed::prepared::query(
                source,
                params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?)?;
            if let Some(reply) = data.as_object_mut() {
                reply.insert("coverage".into(), serde_json::to_value(accepted)?);
            }
            data
        }
        "land-owners" => {
            let _: crate::products::land_owners::LandOwnersParams =
                serde_json::from_value(request.params.clone())?;
            let Some(file) = store.read(&key, "land-owners", 0)? else {
                return Ok(Value::Null);
            };
            let scan = crate::products::land_owners::Scan::read(io::BufReader::new(file))?;
            if !scan.complete {
                return Ok(Value::Null);
            }
            check_cancel(cancelled)?;
            serde_json::to_value(scan.reply())?
        }
        "country-identities" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing prepared identity branch"))?;
            let branch = branch(source, id)?;
            serde_json::to_value(crate::products::country_history::prepared_branch_histories(
                source, branch, content, cancelled,
            )?)?
        }
        "idea-history" => {
            use crate::products::{
                CountryParams, idea_history, politics_content::politics_content,
            };
            let params: CountryParams = serde_json::from_value(request.params.clone())?;
            params.validate().map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            let Some(file) = store.read(&key, &format!("idea-history-{branch}"), 0)? else {
                return Ok(Value::Null);
            };
            let scan = idea_history::Scan::read(io::BufReader::new(file))?;
            check_cancel(cancelled)?;
            let scanned = scan.country(source.replay_ref(), branch, &params.tag)?;
            let content = politics_content(
                source,
                content.ok_or_else(|| {
                    crate::invalid("Choose the game and user folders in Settings first")
                })?,
            )?;
            let mut reply =
                idea_history::reply(&content, source.replay_ref(), &params, &scanned, cancelled)?;
            source.append_content_warnings(&mut reply.warnings);
            serde_json::to_value(reply)?
        }
        "air-superiority" => crate::products::air_superiority::prepared::query(
            source,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "raids" => crate::products::raids::prepared::query(
            source,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "air-aces" | "air-ace-portraits" => crate::products::air_aces::prepared::query(
            source,
            &request.product,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "military-air-ledger" => {
            let branch = branch(
                source,
                request
                    .params
                    .get("branchId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| crate::invalid("Invalid ledger branch"))?,
            )?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::military_air_ledger::prepared::query(
                source,
                &store,
                &key,
                request.params.clone(),
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                cancelled,
            )?
        }
        product @ ("decryption"
        | "operative-placements"
        | "agency-comparison"
        | "operatives-roster"
        | "espionage-log") => {
            let selection = request.params.get("selection").unwrap_or(&request.params);
            let id = selection
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing prepared intelligence branch"))?;
            let branch = branch(source, id)?;
            let last = request
                .params
                .get("lastHour")
                .filter(|value| !value.is_null())
                .map(|value| {
                    value
                        .as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        .ok_or_else(|| crate::invalid("Invalid prepared intelligence hour"))
                })
                .transpose()?;
            if !coverage(source, content, branch, last, last.is_none(), cancelled)? {
                return Ok(Value::Null);
            }
            match product {
                "decryption" => {
                    let params = serde_json::from_value(request.params.clone())?;
                    serde_json::to_value(crate::products::decryption::prepared::query(
                        &store, &key, branch, &params, cancelled,
                    )?)?
                }
                "operative-placements" => {
                    let params = serde_json::from_value(request.params.clone())?;
                    serde_json::to_value(crate::products::operative_placements::prepared::query(
                        &store, &key, branch, &params, cancelled,
                    )?)?
                }
                "agency-comparison" => {
                    let params = serde_json::from_value(request.params.clone())?;
                    serde_json::to_value(crate::products::agency_comparison::prepared::query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        &store,
                        &key,
                        branch,
                        &params,
                        cancelled,
                    )?)?
                }
                "operatives-roster" => {
                    let params = serde_json::from_value(request.params.clone())?;
                    serde_json::to_value(crate::products::operatives_roster::prepared::query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        &store,
                        &key,
                        branch,
                        &params,
                        cancelled,
                    )?)?
                }
                "espionage-log" => {
                    let params = serde_json::from_value(request.params.clone())?;
                    serde_json::to_value(crate::products::espionage_log::prepared::query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        &store,
                        &key,
                        branch,
                        &params,
                        cancelled,
                    )?)?
                }
                _ => unreachable!("explicit intelligence product arms"),
            }
        }
        product @ ("construction-log" | "construction-log-manifest" | "construction-spending") => {
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid construction branch"))?;
            branch(source, id)?;
            crate::products::construction_log::prepared::query(
                source,
                product,
                request.params.clone(),
                content,
                cancelled,
            )?
            .unwrap_or(Value::Null)
        }
        "resource-balance" => {
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid resource branch"))?;
            branch(source, id)?;
            crate::products::resource_balance::prepared::query(
                source,
                request.params.clone(),
                content,
                cancelled,
            )?
            .unwrap_or(Value::Null)
        }
        "trade-flows" => {
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid trade branch"))?;
            branch(source, id)?;
            crate::products::trade_flows::prepared::query(
                source,
                request.params.clone(),
                content,
                cancelled,
            )?
            .unwrap_or(Value::Null)
        }
        "production-spending" => crate::products::production_spending::prepared::query(
            source,
            request.params.clone(),
            content,
            cancelled,
        )?
        .unwrap_or(Value::Null),

        "casualty-breakdown" => {
            let params: crate::products::war_casualties::breakdown::CasualtyBreakdownParams =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            serde_json::to_value(crate::products::war_casualties::breakdown::prepared::query(
                source,
                &params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?)?
        }
        "casualty-pairs" => crate::products::war_casualties::pairs::prepared_query(
            source,
            &serde_json::from_value(request.params.clone())?,
            content,
            cancelled,
        )?,
        "wars" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing prepared wars branch"))?;
            let branch = branch(source, id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::wars::prepared::query(
                source,
                &request.params,
                &store,
                &key,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                cancelled,
            )?
        }
        "subjects" => {
            let params: crate::products::subjects::SubjectParams =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            let Some(file) = store.read(&key, &format!("subjects-{branch}"), 0)? else {
                return Ok(Value::Null);
            };
            let scan: crate::products::subjects::Scan =
                serde_json::from_reader(zstd::stream::Decoder::new(file)?)?;
            check_cancel(cancelled)?;
            serde_json::to_value(scan.finish(source.replay_ref(), branch).reply(
                source,
                &params.branch_id,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
            )?)?
        }
        product @ ("occupied-states" | "occupied-states-manifest") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing occupation branch"))?;
            let branch = branch(source, id)?;
            let hour = if product == "occupied-states" {
                let window = request
                    .params
                    .get("window")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| crate::invalid("Invalid occupied states window"))?;
                Some(
                    source
                        .block_windows(branch)
                        .get(window)
                        .ok_or_else(|| {
                            crate::invalid("Occupied states window not in this recording")
                        })?
                        .last_hour,
                )
            } else {
                None
            };
            if !coverage(
                source,
                content,
                branch,
                hour,
                product == "occupied-states-manifest",
                cancelled,
            )? {
                return Ok(Value::Null);
            }
            crate::products::occupied_states::prepared::window_query(
                source,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                product,
                request.params.clone(),
                cancelled,
            )?
        }
        "unit-leaders" => {
            use crate::products::unit_leaders::{UnitLeaders, UnitLeadersParams};
            let params: UnitLeadersParams = serde_json::from_value(request.params.clone())?;
            UnitLeaders::validate(&params).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            let history = if matches!(
                params.selection,
                crate::products::unit_leaders::LeaderSelection::Countries { .. }
            ) {
                crate::products::country_history::prepared_branch_histories(
                    source, branch, content, cancelled,
                )?
            } else {
                None
            };
            if matches!(
                params.selection,
                crate::products::unit_leaders::LeaderSelection::Countries { .. }
            ) && history.is_none()
            {
                return Ok(Value::Null);
            }
            serde_json::to_value(crate::products::unit_leaders::prepared::query(
                &store,
                &key,
                &params,
                history.as_deref(),
                cancelled,
            )?)?
        }
        "special-projects" => {
            let params: crate::products::special_projects::SpecialProjectsParams =
                serde_json::from_value(request.params.clone())?;
            crate::products::special_projects::SpecialProjects::validate(&params)
                .map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::special_projects::prepared::query(
                request.params.clone(),
                &store,
                &key,
                cancelled,
            )?
        }
        "country-groups" => {
            let params: crate::products::country_groups::CountryGroupParams =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, None, true, cancelled)? {
                return Ok(Value::Null);
            }
            let Some(file) = store.read(&key, &format!("country-groups-{branch}"), 0)? else {
                return Ok(Value::Null);
            };
            let scan = crate::products::country_groups::Scan::read(io::BufReader::new(file))?;
            check_cancel(cancelled)?;
            serde_json::to_value(scan.reply(
                source,
                &params.branch_id,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
            )?)?
        }
        "politics-ledger" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid politics ledger branch"))?;
            let branch = branch(source, id)?;
            let hour = request
                .params
                .get("hour")
                .filter(|value| !value.is_null())
                .map(|value| {
                    value
                        .as_i64()
                        .and_then(|value| i32::try_from(value).ok())
                        .ok_or_else(|| crate::invalid("Invalid politics ledger hour"))
                })
                .transpose()?;
            if !coverage(source, content, branch, hour, hour.is_none(), cancelled)? {
                return Ok(Value::Null);
            }
            crate::products::politics_ledger::prepared::query(
                &request.params,
                &store,
                &key,
                cancelled,
            )?
        }
        "focus-static-text" => {
            let params: FocusStaticText = serde_json::from_value(request.params.clone())?;
            if params
                .tokens
                .iter()
                .any(|token| token.is_empty() || token.contains('\0'))
            {
                return Err(crate::invalid("Invalid focus static text token"));
            }
            serde_json::to_value(crate::prepared_focus::static_text_page(
                &store,
                &key,
                &params.tokens,
                cancelled,
            )?)?
        }
        "focus-hour" => {
            let params: FocusPoint = serde_json::from_value(request.params.clone())?;
            crate::products::validate_tag(&params.tag).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, Some(params.hour), false, cancelled)? {
                return Ok(Value::Null);
            }
            serde_json::to_value(crate::prepared_focus::query_hour(
                &store,
                &key,
                branch,
                &params.tag,
                params.hour,
                cancelled,
            )?)?
        }
        "focus-unavailable" => {
            let params: FocusUnavailable = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, Some(params.hour), false, cancelled)? {
                return Ok(Value::Null);
            }
            serde_json::to_value(crate::prepared_focus::query_unavailable(
                &store,
                &key,
                branch,
                params.hour,
                cancelled,
            )?)?
        }
        "focus-order-page" | "focus-presentation-page" | "focus-details-page" => {
            let params: FocusPage = serde_json::from_value(request.params.clone())?;
            crate::products::validate_tag(&params.tag).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(source, content, branch, Some(params.hour), false, cancelled)? {
                return Ok(Value::Null);
            }
            let page = params
                .page
                .parse::<u64>()
                .map_err(|_| crate::invalid("Invalid focus page ordinal"))?;
            crate::prepared_focus::query_page(
                &store,
                &key,
                branch,
                &params.tag,
                params.hour,
                &params.pages,
                page,
                &request.product,
                cancelled,
            )?
        }
        "production-select" => {
            let params: ProductionSelection = serde_json::from_value(request.params.clone())?;
            crate::products::equipment_produced::EquipmentProduced::validate(&params.selection)
                .map_err(crate::invalid)?;
            let branch = branch(source, &params.selection.branch_id)?;
            // The reference joins Rocket knownness over a completed equipment
            // read interval and resolves stable type facts over the branch.
            if !coverage(source, content, branch, Some(params.hour), true, cancelled)? {
                return Ok(Value::Null);
            }
            let offset = params
                .offset
                .parse::<usize>()
                .map_err(|_| crate::invalid("Invalid production page offset"))?;
            let value = if params.ranked {
                crate::prepared_production::select_ranked_at(
                    &store,
                    &key,
                    params.hour,
                    &params.selection,
                    offset,
                    params.limit,
                    cancelled,
                )?
            } else {
                crate::prepared_production::select_at(
                    &store,
                    &key,
                    params.hour,
                    &params.selection,
                    offset,
                    params.limit,
                    cancelled,
                )?
            };
            serde_json::to_value(value)?
        }
        product @ ("series-state" | "series-summary" | "series-next-change")
            if request
                .params
                .get("family")
                .and_then(|family| family.get("kind"))
                .and_then(Value::as_str)
                == Some("autonomy") =>
        {
            crate::products::autonomy::prepared::progressive_query(
                source,
                product,
                &request.params,
                &store,
                &key,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                cancelled,
            )?
        }
        product @ ("series-state" | "series-summary" | "series-next-change" | "events-page"
        | "events-clusters") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid series query branch"))?;
            let branch = branch(source, id)?;
            let family = request
                .params
                .get("family")
                .and_then(|family| family.get("kind"))
                .and_then(Value::as_str);
            if family == Some("game-score") && product == "series-state" {
                return crate::products::game_score::prepared::state(
                    source,
                    request.params.clone(),
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    cancelled,
                );
            }
            match family {
                Some("country-fuel") => {
                    return crate::products::country_fuel::prepared::query(
                        source,
                        content,
                        product,
                        request.params.clone(),
                        &store,
                        &key,
                        cancelled,
                    );
                }
                Some("country-factories") => {
                    return crate::products::country_factories::prepared::query(
                        source,
                        product,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    );
                }
                Some("country-nukes") => {
                    return crate::products::country_nukes::prepared::query(
                        source,
                        product,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    );
                }
                _ => {}
            }
            let (hour, complete) = series_coverage(product, &request.params)?;
            let accepted = match super::pipeline::coverage(source, content, branch, cancelled) {
                Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
                other => other?,
            };
            // Families whose requests name one measure per tag register here; others
            // expand their own request shape and call `progressive::tail` themselves.
            let sampler: Option<(&[replay::rows::TableId], progressive::Sample<'_>)> = match family
            {
                Some("country-measure") => Some((
                    crate::prepared_measure::TABLES,
                    &crate::prepared_measure::sample,
                )),
                _ => None,
            };
            let partial_events =
                matches!(product, "events-page" | "events-clusters") && accepted.is_some();
            // Stability parts expand countries to parts and sample their own tail.
            let parts_tail = family == Some("stability-parts")
                && matches!(
                    product,
                    "series-state" | "series-summary" | "series-next-change"
                );
            if family != Some("territory")
                && sampler.is_none()
                && !parts_tail
                && !partial_events
                && !matches!(
                    family,
                    Some("army-size" | "supply-health" | "air-force-size" | "navy-size")
                )
                && !coverage(source, content, branch, hour, complete, cancelled)?
            {
                return Ok(Value::Null);
            }
            let partial = !accepted.as_ref().is_some_and(|coverage| coverage.complete);
            let mut sampled = (
                std::collections::BTreeMap::new(),
                Vec::new(),
                std::collections::BTreeMap::new(),
            );

            if let Some((tables, sample)) = sampler
                && partial
                && matches!(
                    product,
                    "series-state" | "series-summary" | "series-next-change"
                )
            {
                let measure = request
                    .params
                    .get("family")
                    .and_then(|family| family.get("measure"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| crate::invalid("Missing series measure"))?;
                let labels: Vec<String> = if product == "series-next-change" {
                    request
                        .params
                        .get("measures")
                        .and_then(Value::as_array)
                        .filter(|values| !values.is_empty())
                        .map(|values| {
                            values
                                .iter()
                                .map(|value| {
                                    value
                                        .as_str()
                                        .map(str::to_owned)
                                        .ok_or_else(|| crate::invalid("Invalid series measure"))
                                })
                                .collect()
                        })
                        .transpose()?
                        .unwrap_or_else(|| vec![measure.to_owned()])
                } else {
                    vec![measure.to_owned()]
                };
                let mut requested = Vec::new();
                let mut hours = Vec::new();
                if product == "series-state" {
                    for read in request
                        .params
                        .get("reads")
                        .and_then(Value::as_array)
                        .ok_or_else(|| crate::invalid("Missing series reads"))?
                    {
                        let tag = read
                            .get("tag")
                            .and_then(Value::as_str)
                            .ok_or_else(|| crate::invalid("Invalid series tag"))?;
                        let hour = read
                            .get("hour")
                            .and_then(Value::as_i64)
                            .and_then(|hour| i32::try_from(hour).ok())
                            .ok_or_else(|| crate::invalid("Invalid series hour"))?;
                        hours.push(hour);
                        for label in &labels {
                            requested.push((tag.to_owned(), label.clone()));
                        }
                    }
                } else {
                    for tag in request
                        .params
                        .get("tags")
                        .and_then(Value::as_array)
                        .ok_or_else(|| crate::invalid("Missing series tags"))?
                    {
                        let tag = tag
                            .as_str()
                            .ok_or_else(|| crate::invalid("Invalid series tag"))?;
                        for label in &labels {
                            requested.push((tag.to_owned(), label.clone()));
                        }
                    }
                }
                requested.sort();
                requested.dedup();
                for (tag, _) in &requested {
                    crate::products::validate_tag(tag).map_err(crate::invalid)?;
                }
                sampled = if product == "series-state" {
                    progressive::state_tail(
                        source,
                        branch,
                        accepted.as_ref(),
                        tables,
                        &requested,
                        &hours,
                        sample,
                        cancelled,
                    )?
                } else {
                    let (from, to) = if product == "series-summary" {
                        super::series::range(
                            request.params.get("from").and_then(Value::as_f64),
                            request.params.get("to").and_then(Value::as_f64),
                        )?
                    } else {
                        super::series::range(
                            request.params.get("firstHour").and_then(Value::as_f64),
                            request.params.get("lastHour").and_then(Value::as_f64),
                        )?
                    };
                    if product == "series-summary" {
                        let width = request
                            .params
                            .get("width")
                            .and_then(Value::as_u64)
                            .and_then(|width| u32::try_from(width).ok())
                            .filter(|width| *width > 0)
                            .ok_or_else(|| crate::invalid("Invalid series width"))?;
                        super::series::summary_budget(from, to, width, requested.len())?;
                    }
                    progressive::tail(
                        source,
                        branch,
                        accepted.as_ref(),
                        tables,
                        &requested,
                        (from.floor() as i32, to.floor() as i32),
                        sample,
                        cancelled,
                    )?
                };
            }
            let tail_context = (sampler.is_some() && partial).then(|| {
                let start = source
                    .replay_ref()
                    .branches
                    .iter()
                    .find(|start| start.id == branch)
                    .unwrap();
                (
                    crate::metadata::branch_bounds(source.replay_ref(), start),
                    &sampled.0,
                    sampled.1.as_slice(),
                    &sampled.2,
                    accepted.is_some(),
                )
            });
            let mut data = match family {
                Some("country-measure" | "country-stability" | "country-experience") => {
                    crate::prepared_measure::query::query(
                        product,
                        request.params.clone(),
                        &store,
                        &key,
                        tail_context,
                        cancelled,
                    )?
                }
                Some("naval-losses") => crate::products::naval_losses::prepared::query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    &store,
                    &key,
                    cancelled,
                )?,
                Some("equipment-losses") => {
                    crate::products::equipment_losses::prepared::series_query(
                        source,
                        product,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    )?
                }
                Some("war-casualties") => crate::products::war_casualties::prepared::query(
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    cancelled,
                )?,
                Some("world-tension") => {
                    let Some(history) =
                        crate::products::country_history::prepared_branch_histories(
                            source, branch, content, cancelled,
                        )?
                    else {
                        return Ok(Value::Null);
                    };
                    crate::products::world_tension::prepared::query(
                        source,
                        product,
                        &request.params,
                        &store,
                        &key,
                        &history,
                        cancelled,
                    )?
                }
                Some("occupation-trend") => {
                    crate::products::occupied_states::prepared::trend_query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        product,
                        request.params.clone(),
                        cancelled,
                    )?
                }
                Some("territory") => crate::products::territory::prepared::query(
                    source,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    product,
                    request.params.clone(),
                    cancelled,
                )?,
                Some("faction-totals") => crate::products::faction_totals::prepared::query(
                    source,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    product,
                    request.params.clone(),
                    cancelled,
                )?,
                Some("peace-deals") => {
                    // This adapter builds complete outcomes from earlier reads;
                    // a partial event prefix cannot answer an unprepared span.
                    if !coverage(source, content, branch, hour, complete, cancelled)? {
                        return Ok(Value::Null);
                    }
                    crate::products::peace_deals::prepared::query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        request.params.clone(),
                        &store,
                        &key,
                        cancelled,
                    )?
                }
                Some("research") => {
                    // Complete assignment history and carried slot state need
                    // the scan, including when requested through events-page.
                    if !coverage(source, content, branch, hour, complete, cancelled)? {
                        return Ok(Value::Null);
                    }
                    crate::products::research::prepared::query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        product,
                        request.params.clone(),
                        &store,
                        &key,
                        cancelled,
                    )?
                }
                Some("stability-parts") => crate::products::stability_parts::prepared_query(
                    source,
                    content,
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    accepted.as_ref(),
                    cancelled,
                )?,
                Some("army-size") if product == "series-summary" => {
                    crate::products::army_size::prepared::summary(
                        source,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    )?
                }
                Some("navy-size") => crate::products::navy::prepared::query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    cancelled,
                )?,
                Some("air-force-size") => crate::products::air_force::prepared::query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    cancelled,
                )?,
                Some("army-size") if product == "series-next-change" => {
                    crate::products::army_size::prepared::next_change(
                        source,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    )?
                }
                Some("supply-health") if product == "series-next-change" => {
                    crate::products::supply_health::prepared::next_change(
                        source,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    )?
                }
                Some("game-score") => crate::products::game_score::prepared::query(
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    cancelled,
                )?,
                _ => return Err(crate::invalid("Unknown prepared series family")),
            };
            if let Some(states) = data.get_mut("states").and_then(Value::as_array_mut) {
                for state in states
                    .iter_mut()
                    .filter(|state| state["approximate"] == true)
                {
                    if let Some(hour) = state["hour"]
                        .as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        && let Some(block) = source.replay_ref().index.iter().find(|block| {
                            block.branch == branch
                                && block.first_hour <= hour
                                && block.last_hour >= hour
                        })
                    {
                        state["sampleHour"] = Value::from(block.first_hour);
                        if !state["run"].is_null() {
                            for field in ["firstHour", "lastHour", "segmentFirstHour"] {
                                state["run"][field] = Value::from(block.first_hour);
                            }
                        }
                    }
                }
            }
            if let Some(reply) = data.as_object_mut() {
                reply.insert("coverage".into(), serde_json::to_value(accepted)?);
            }
            data
        }
        _ => {
            return Err(crate::invalid(format!(
                "Unknown prepared query: {}",
                request.product
            )));
        }
    })
}

pub fn query(
    source: &Source,
    request: &ProductRequest,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<String> {
    check_cancel(cancelled)?;
    if source.key().revision != request.revision {
        return Err(crate::invalid("Prepared query recording revision changed"));
    }
    // A query routes by its series family kind, or else by its product key.
    let name = match request.product.as_str() {
        "series-state" | "series-summary" | "series-next-change" | "events-page"
        | "events-clusters" => request
            .params
            .get("family")
            .and_then(|family| family.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        product => product,
    };
    if let Some(branch) = request
        .params
        .get("branchId")
        .or_else(|| request.params.get("selection")?.get("branchId"))
        .and_then(Value::as_str)
        .and_then(|id| branch(source, id).ok())
    {
        super::pipeline::refusal(source, content, branch, name, cancelled)?;
    }
    // Null is a query result whose coverage has not been accepted yet. Clients
    // inspect real preparation status and retry; it is not an empty history.
    let data = query_data(source, request, content, cancelled)?;
    check_cancel(cancelled)?;
    bounded_json(
        &Reply {
            source_id: &request.source_id,
            revision: &request.revision,
            product: &request.product,
            data,
        },
        cancelled,
    )
    .map_err(crate::invalid)
}
