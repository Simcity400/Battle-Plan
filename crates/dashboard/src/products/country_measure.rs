//! One recorded country balance over one branch, per country: political power
//! or command power, for the widgets that chart them.
//!
//! Both are the engine's fixed-point balances with [`MEASURE_DECIMALS`]
//! decimal places (`Countries.political_power_raw`, optional per country, and
//! `Countries.command_power_raw`). Runs compress equal consecutive hours and
//! never bridge a gap, a missing value or a change of country identity. Spend
//! ticks are derived: a fall in the balance between two consecutive recorded
//! hours of the same country. They are net changes, because daily income is not
//! recorded.

pub use super::MAX_COUNTRIES;
use super::{LoadContext, Product, validate_countries};
use crate::{
    Source,
    metadata::{mapped_slot, validate_campaign},
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{CountryRow, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct CountryMeasure;

/// Decimal places of the recorded fixed-point balances (10.00001 is stored as
/// 1_000_001; see the recorder's fresh-save oracle).
pub const MEASURE_DECIMALS: u8 = 5;
/// Runs kept per country before the scan refuses, far above a long campaign's
/// daily changes.
const MAX_RUNS: usize = 400_000;
const MAX_CACHED_COUNTRIES: usize = 32;

/// Which recorded balance to chart.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum Measure {
    PoliticalPower,
    CommandPower,
}

impl Measure {
    /// The recorded value for a country this hour; political power is optional
    /// per country, and a missing value stays missing.
    fn value(self, country: &CountryRow) -> Option<i64> {
        match self {
            Self::PoliticalPower => country.political_power_raw,
            Self::CommandPower => Some(country.command_power_raw),
        }
    }

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

/// A recorded fixed-point value held over consecutive hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FixedRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The exact stored integer as a decimal string.
    pub value_raw: String,
    /// The line breaks before this run: it follows a gap, a missing value or a
    /// different country identity rather than continuing the previous run.
    pub starts_segment: bool,
}

/// A fall in the balance from one recorded hour to the next: a net spend.
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
    /// Decimal places of every raw value in this reply.
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
        let measure = params.measure;
        let key = |tag: &str| (branch, measure, tag.to_owned());
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        let missing: Vec<&str> = params
            .tags
            .iter()
            .map(String::as_str)
            .filter(|tag| !cache.entries.iter().any(|(cached, _)| *cached == key(tag)))
            .collect();
        let mut scanned = if missing.is_empty() {
            BTreeMap::new()
        } else {
            scan(&source.replay(), branch, measure, &missing, cancelled)?
        };
        // Build the reply before caching, so eviction can never drop a country it needs.
        let mut warnings = Vec::new();
        let series: Vec<MeasureSeries> = params
            .tags
            .iter()
            .map(|tag| {
                let found = scanned.get(tag).cloned().or_else(|| {
                    cache
                        .entries
                        .iter()
                        .find(|(cached, _)| *cached == key(tag))
                        .map(|(_, series)| series.clone())
                });
                let found = found.unwrap_or_default();
                if found.runs.is_empty() {
                    warnings.push(format!(
                        "{tag}: no recorded {} in this branch",
                        measure.noun()
                    ));
                }
                MeasureSeries {
                    tag: tag.clone(),
                    ..found
                }
            })
            .collect();
        for (tag, series) in std::mem::take(&mut scanned) {
            cache.entries.push_back((key(&tag), series));
        }
        while cache.entries.len() > MAX_CACHED_COUNTRIES.max(params.tags.len()) {
            cache.entries.pop_front();
        }
        Ok(MeasureReply {
            branch_id: params.branch_id.clone(),
            measure,
            decimals: MEASURE_DECIMALS,
            series,
            warnings,
        })
    }
}

/// Recently requested (branch, measure, country) series, oldest first.
#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, Measure, String), MeasureSeries)>,
}

/// Scans one branch once for every tag given.
pub fn scan(
    replay: &Replay,
    branch: u64,
    measure: Measure,
    tags: &[&str],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, MeasureSeries>> {
    let selected = [
        replay::rows::TableId::Campaign,
        replay::rows::TableId::Countries,
        replay::rows::TableId::CountryLinks,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Country request cancelled");
    let mut result: BTreeMap<String, MeasureSeries> = tags
        .iter()
        .map(|tag| ((*tag).to_owned(), MeasureSeries::default()))
        .collect();
    // The identity and value each country was last seen with, and at which hour.
    let mut last: BTreeMap<String, ((u32, u32), i64, i32)> = BTreeMap::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let (Ok(countries), Ok(links)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            ) else {
                continue;
            };
            for country in countries {
                let Some(series) = std::str::from_utf8(&country.tag)
                    .ok()
                    .and_then(|tag| result.get_mut(tag))
                else {
                    continue;
                };
                let tag = std::str::from_utf8(&country.tag).expect("checked above");
                let (Ok(slot), Some(value)) = (mapped_slot(country, links), measure.value(country))
                else {
                    continue;
                };
                let identity = (slot, country.id);
                let hour = frame.hour;
                let continues = last.get(tag).is_some_and(|(seen, _, at)| {
                    *seen == identity && at.checked_add(1) == Some(hour)
                });
                match (continues, series.runs.last_mut()) {
                    (true, Some(run)) if run.value_raw.parse::<i64>() == Ok(value) => {
                        run.last_hour = hour
                    }
                    _ => {
                        if continues
                            && let Some((_, before, _)) = last.get(tag)
                            && value < *before
                        {
                            series.drops.push(BalanceDrop {
                                hour,
                                before_raw: before.to_string(),
                                after_raw: value.to_string(),
                            });
                        }
                        if series.runs.len() >= MAX_RUNS {
                            return Err(io::Error::other(format!(
                                "The {} history exceeds its memory budget",
                                measure.noun()
                            )));
                        }
                        series.runs.push(FixedRun {
                            first_hour: hour,
                            last_hour: hour,
                            value_raw: value.to_string(),
                            starts_segment: !continues,
                        });
                    }
                }
                last.insert(tag.to_owned(), (identity, value, hour));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
