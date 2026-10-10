//! Exact recorded political and command power over one branch, per country.
//! Both stored fixed5 balances share the country identity/balance walk with the
//! Scoreboard. Runs never bridge missing values, gaps or country identity changes.
//! Spend ticks are net drops between consecutive recorded hours, not gross spend.
//! Both power measures use their recorded signed fixed5 balances.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, validate_countries};
use crate::{Source, transport::json_len};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct CountryMeasure;

/// Decimal places of both recorded native fixed-point balances.
pub const MEASURE_DECIMALS: u8 = 5;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum Measure {
    PoliticalPower,
    CommandPower,
}

impl Measure {
    fn noun(self) -> &'static str {
        match self {
            Self::PoliticalPower => "political power",
            Self::CommandPower => "command power",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureParams {
    pub branch_id: String,
    pub measure: Measure,
    pub tags: Vec<String>,
}

/// Exact stored value held over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FixedRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value_raw: String,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceDrop {
    pub hour: i32,
    pub before_raw: String,
    pub after_raw: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSeries {
    pub tag: String,
    pub runs: Vec<FixedRun>,
    pub drops: Vec<BalanceDrop>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureReply {
    pub branch_id: String,
    pub measure: Measure,
    pub decimals: u8,
    pub series: Vec<MeasureSeries>,
    pub warnings: Vec<String>,
}

impl Product for CountryMeasure {
    const KEY: &'static str = "country-measure";
    const CONTENT_DEPENDENT: bool = false;
    type Params = MeasureParams;
    type Reply = MeasureReply;
    fn types() -> Vec<String> {
        vec![
            Measure::decl(),
            MeasureParams::decl(),
            FixedRun::decl(),
            BalanceDrop::decl(),
            MeasureSeries::decl(),
            MeasureReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_countries(&params.branch_id, &params.tags)
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let histories = super::country_history::branch_histories(
            source,
            branch,
            false,
            &[params.measure],
            cancelled,
        )?;
        let mut warnings = Vec::new();
        let series = params
            .tags
            .iter()
            .map(|tag| {
                let found = histories
                    .countries
                    .get(tag)
                    .map(|country| match params.measure {
                        Measure::PoliticalPower => &country.political_power,
                        Measure::CommandPower => &country.command_power,
                    });
                let runs = found.map_or_else(Vec::new, |found| {
                    found
                        .runs
                        .iter()
                        .map(|run| FixedRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            value_raw: run.value.to_string(),
                            starts_segment: run.starts_segment,
                        })
                        .collect()
                });
                let drops = found.map_or_else(Vec::new, |found| {
                    found
                        .drops
                        .iter()
                        .map(|drop| BalanceDrop {
                            hour: drop.hour,
                            before_raw: drop.before.to_string(),
                            after_raw: drop.after.to_string(),
                        })
                        .collect()
                });
                if runs.is_empty() {
                    warnings.push(format!(
                        "{tag}: no recorded {} in this branch",
                        params.measure.noun()
                    ));
                }
                MeasureSeries {
                    tag: tag.clone(),
                    runs,
                    drops,
                }
            })
            .collect();
        checked_reply(MeasureReply {
            branch_id: params.branch_id.clone(),
            measure: params.measure,
            decimals: MEASURE_DECIMALS,
            series,
            warnings,
        })
    }
}

/// Refuses, by name, a complete reply larger than the transport budget; no
/// country or run is ever dropped to fit.
fn checked_reply(reply: MeasureReply) -> io::Result<MeasureReply> {
    if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "MeasureReplyTooLarge: the complete country balance reply exceeds the transport budget; no countries or runs were omitted",
        ));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
