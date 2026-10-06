//! Occupier means from Occupied states' single cached branch scan.
//! P: RefreshGarrisons 140FE9420 selects controlled states with a positive
//! original occupied link and CheckActivation(...,1) (140F86330), including
//! annexed non-cores. Ownership is not the admission test.
//! P: occupation-group averages 140FE98F0 sum stored fixed5 resistance/compliance
//! and divide by the state count, truncating signed fixed5 division. The widget
//! applies that arithmetic across an occupier's readable held states, rather
//! than averaging rounded country-group UI values (14155C270).
//! I: this whole-occupier scope is the widget definition. 140FE1EC0 refreshes
//! native cached group means BEFORE updating state scalars; their +0x38/+0x40
//! fields are not recorded. Our mean is explicitly of the supplying state read.
use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_groups::HourSpan, occupied_states,
    validate_tag,
};
use crate::{Source, ranking, transport::json_len};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct OccupationTrend;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OccupationMeasure {
    Resistance,
    Compliance,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupationTop {
    pub count: u32,
    pub measure: OccupationMeasure,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupationTrendParams {
    pub branch_id: String,
    /// Empty with no top request means every occupier, independently of status.
    pub tags: Vec<String>,
    pub top: Option<OccupationTop>,
    /// Tooltip context only; omitted requests keep the smaller numeric history.
    #[serde(default)]
    pub collaboration: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationCollaboration {
    pub tag: String,
    /// P: CCountryCollaborationData+0x10, 1413DFC50; fixed5 fraction, unlike
    /// the state percentage-point scalars. Politics meter: 141C441D0.
    pub raw: Option<String>,
    pub read_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationTrendRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    pub starts_segment: bool,
    pub resistance_raw: String,
    pub compliance_raw: String,
    pub states: u32,
    /// Known held inventory at this read. Incomplete inventories are a lower bound.
    pub held_states: u32,
    pub held_complete: bool,
    pub collaboration: Vec<OccupationCollaboration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationTrendSeries {
    pub tag: String,
    pub runs: Vec<OccupationTrendRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationTrendReply {
    pub branch_id: String,
    pub decimals: u32,
    pub series: Vec<OccupationTrendSeries>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub(super) struct History {
    pub series: BTreeMap<String, Vec<OccupationTrendRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub warnings: Vec<String>,
    pub collaboration_warnings: Vec<String>,
}

pub(super) fn same_value(left: &OccupationTrendRun, right: &OccupationTrendRun) -> bool {
    left.read_hour == right.read_hour
        && left.resistance_raw == right.resistance_raw
        && left.compliance_raw == right.compliance_raw
        && left.states == right.states
        && left.held_states == right.held_states
        && left.held_complete == right.held_complete
        && left.collaboration == right.collaboration
}

pub(super) fn reply(
    branch: &str,
    history: &History,
    params: &OccupationTrendParams,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<OccupationTrendReply> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Occupation trend request cancelled",
        ));
    }
    let membership = params
        .top
        .as_ref()
        .map(|top| {
            ranking::top_membership(
                &history.series,
                top.count as usize,
                |run| (run.first_hour, run.last_hour),
                |run| {
                    match top.measure {
                        OccupationMeasure::Resistance => &run.resistance_raw,
                        OccupationMeasure::Compliance => &run.compliance_raw,
                    }
                    .parse::<i64>()
                    .ok()
                },
                cancelled,
            )
        })
        .transpose()?;
    let mut result = OccupationTrendReply {
        branch_id: branch.into(),
        decimals: 5,
        series: Vec::new(),
        warnings,
    };
    let tags: HashSet<_> = params.tags.iter().collect();
    let mut bytes = json_len(&result, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        io::Error::other("Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES")
    })?;
    for (tag, runs) in &history.series {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Occupation trend request cancelled",
            ));
        }
        let selected = membership
            .as_ref()
            .and_then(|membership| membership.get(tag));
        if membership.is_some() && selected.is_none()
            || membership.is_none() && !tags.is_empty() && !tags.contains(tag)
        {
            continue;
        }
        let mut series = OccupationTrendSeries {
            tag: tag.clone(),
            runs: Vec::new(),
            segments: history.segments.get(tag).cloned().unwrap_or_default(),
            membership: selected.cloned(),
        };
        bytes += json_len(&series, MAX_PRODUCT_REPLY_BYTES).unwrap_or(MAX_PRODUCT_REPLY_BYTES) + 1;
        let mut add = |run: &OccupationTrendRun, first, last| {
            let mut run = run.clone();
            run.first_hour = first;
            run.last_hour = last;
            if !params.collaboration {
                run.collaboration.clear();
            }
            // The shared scan retains collaboration reads for both settings.
            // Without that context, its independent read boundaries do not
            // split the numeric history or inflate the reply budget.
            if let Some(last) = series.runs.last_mut()
                && !run.starts_segment
                && last.last_hour.checked_add(1) == Some(run.first_hour)
                && same_value(last, &run)
            {
                last.last_hour = run.last_hour;
                return Ok(());
            }
            bytes += json_len(&run, MAX_PRODUCT_REPLY_BYTES).unwrap_or(MAX_PRODUCT_REPLY_BYTES) + 1;
            if bytes > MAX_PRODUCT_REPLY_BYTES {
                return Err(io::Error::other(
                    "Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES",
                ));
            }
            series.runs.push(run);
            Ok(())
        };
        if let Some(spans) = selected {
            ranking::for_each_piece(
                runs,
                spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                &mut add,
            )?;
        } else {
            for run in runs {
                add(run, run.first_hour, run.last_hour)?;
            }
        }
        result.series.push(series);
    }
    if json_len(&result, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES",
        ));
    }
    Ok(result)
}

impl Product for OccupationTrend {
    const KEY: &'static str = "occupation-trend";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OccupationTrendParams;
    type Reply = OccupationTrendReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params
            .top
            .as_ref()
            .is_some_and(|top| !params.tags.is_empty() || top.count == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            OccupationMeasure::decl(),
            OccupationTop::decl(),
            OccupationTrendParams::decl(),
            OccupationCollaboration::decl(),
            OccupationTrendRun::decl(),
            OccupationTrendSeries::decl(),
            OccupationTrendReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scanned = occupied_states::history(
            source,
            params.branch_id.parse().expect("validated branch"),
            context,
            cancelled,
        )?;
        let mut warnings = scanned.trend.warnings.clone();
        if params.collaboration {
            warnings.extend(scanned.trend.collaboration_warnings.iter().cloned());
        }
        reply(
            &params.branch_id,
            &scanned.trend,
            params,
            warnings,
            cancelled,
        )
    }
}

#[cfg(test)]
mod tests;
