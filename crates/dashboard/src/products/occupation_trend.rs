//! Occupier means from Occupied states' shared preparation observer.
//! Garrisons select controlled states with a positive original occupied link
//! and active occupation, including annexed non-cores. Ownership is not
//! the admission test.
//! Occupation-group averages sum stored fixed5 resistance/compliance and
//! divide by the state count, truncating signed fixed5 division. The widget
//! applies that arithmetic across an occupier's readable held states, rather
//! than averaging rounded country-group UI values.
//! This whole-occupier scope is the widget definition. Cached group means
//! refresh before state scalars update; those means are not recorded.
//! Our mean is explicitly of the supplying state read.
#[cfg(test)]
use super::{LoadContext, occupied_states};
use super::{
    MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun, RecordedHistories},
    validate_tag,
};
#[cfg(test)]
use crate::Source;
use crate::{ranking, transport::json_len};
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
pub struct OccupationCollaborationActor {
    pub slot: u32,
    pub country_id: u32,
    /// The current native identity supplied this tag. Null refuses hovered
    /// names/flags while retaining the held collaboration amount.
    pub name_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationCollaboration {
    pub tag: Option<String>,
    /// Collaboration is a fixed5 fraction, unlike the state percentage-point
    /// scalars, matching the politics meter.
    pub raw: Option<String>,
    pub read_hour: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actor: Option<OccupationCollaborationActor>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationTrendReply {
    pub branch_id: String,
    pub decimals: u32,
    pub series: Vec<OccupationTrendSeries>,
    pub warnings: Vec<String>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
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

#[cfg(test)]
pub(super) fn reply(
    branch: &str,
    history: &History,
    params: &OccupationTrendParams,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<OccupationTrendReply> {
    reply_with_histories(branch, history, params, warnings, cancelled, None)
}

pub(in crate::products) fn reply_with_histories(
    branch: &str,
    history: &History,
    params: &OccupationTrendParams,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
    histories: Option<&RecordedHistories>,
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
    let selected_ids = histories
        .filter(|_| params.top.is_none() && !params.tags.is_empty())
        .map(|histories| country_history::selected_identities(histories, &params.tags));
    let tags: Vec<_> = if let Some(membership) = &membership {
        membership.keys().cloned().collect()
    } else if let Some(histories) = histories.filter(|_| !params.tags.is_empty()) {
        country_history::selected_aliases(histories, &params.tags)
    } else {
        let requested: HashSet<_> = params.tags.iter().collect();
        history
            .series
            .keys()
            .filter(|tag| requested.is_empty() || requested.contains(tag))
            .cloned()
            .collect()
    };
    let mut bytes = json_len(&result, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        io::Error::other("Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES")
    })?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SeriesMeasure<'a> {
        tag: &'a str,
        runs: &'a [OccupationTrendRun],
        segments: &'a [HourSpan],
        membership: Option<&'a [HourSpan]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identities: Option<&'a [&'a CountryIdentityRun]>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RunMeasure<'a> {
        first_hour: i32,
        last_hour: i32,
        read_hour: i32,
        starts_segment: bool,
        resistance_raw: &'a str,
        compliance_raw: &'a str,
        states: u32,
        held_states: u32,
        held_complete: bool,
        collaboration: &'a [OccupationCollaboration],
    }
    let mut pieces = Vec::new();
    for tag in &tags {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Occupation trend request cancelled",
            ));
        }
        let runs = history.series.get(tag).map_or(&[][..], Vec::as_slice);
        let country = histories.and_then(|histories| histories.countries.get(tag));
        let identities = histories.map(|_| {
            country
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| {
                    selected_ids.as_ref().is_none_or(|selected| {
                        selected.contains(&(identity.slot, identity.country_id))
                    })
                })
                .collect::<Vec<_>>()
        });
        let holder_spans = selected_ids.as_ref().map(|_| {
            identities
                .as_ref()
                .into_iter()
                .flatten()
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect::<Vec<_>>()
        });
        let selected = membership
            .as_ref()
            .and_then(|membership| membership.get(tag));
        let spans = selected.map(Vec::as_slice).or(holder_spans.as_deref());
        let segments = holder_spans
            .as_deref()
            .unwrap_or_else(|| history.segments.get(tag).map_or(&[][..], Vec::as_slice));
        bytes += usize::from(!pieces.is_empty());
        bytes += json_len(
            &SeriesMeasure {
                tag,
                runs: &[],
                segments,
                membership: selected.map(Vec::as_slice),
                identities: identities.as_deref(),
            },
            MAX_PRODUCT_REPLY_BYTES.saturating_sub(bytes),
        )
        .ok_or_else(|| {
            io::Error::other("Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES")
        })?;
        let mut clipped: Vec<(&OccupationTrendRun, i32, i32, bool)> = Vec::new();
        let mut add = |index: usize, first, last| {
            let run = &runs[index];
            let starts_segment = run.starts_segment && first == run.first_hour
                || holder_spans.as_ref().is_some_and(|spans| {
                    spans
                        .binary_search_by_key(&first, |span| span.first_hour)
                        .is_ok()
                });
            // The shared scan retains collaboration reads for both settings.
            // Without that context, its independent read boundaries do not
            // split the numeric history or inflate the reply budget.
            if let Some((previous, _, end, _)) = clipped.last_mut()
                && !starts_segment
                && end.checked_add(1) == Some(first)
                && previous.read_hour == run.read_hour
                && previous.resistance_raw == run.resistance_raw
                && previous.compliance_raw == run.compliance_raw
                && previous.states == run.states
                && previous.held_states == run.held_states
                && previous.held_complete == run.held_complete
                && (!params.collaboration || previous.collaboration == run.collaboration)
            {
                *end = last;
                return Ok(());
            }
            clipped.push((run, first, last, starts_segment));
            Ok(())
        };
        if let Some(spans) = spans {
            let indices: Vec<_> = (0..runs.len()).collect();
            ranking::for_each_piece(
                &indices,
                spans,
                |index| (runs[*index].first_hour, runs[*index].last_hour),
                cancelled,
                |index, first, last| add(*index, first, last),
            )?;
        } else {
            for (index, run) in runs.iter().enumerate() {
                add(index, run.first_hour, run.last_hour)?;
            }
        }
        for (index, (run, first, last, starts_segment)) in clipped.iter().enumerate() {
            let value = RunMeasure {
                first_hour: *first,
                last_hour: *last,
                starts_segment: *starts_segment,
                read_hour: run.read_hour,
                resistance_raw: &run.resistance_raw,
                compliance_raw: &run.compliance_raw,
                states: run.states,
                held_states: run.held_states,
                held_complete: run.held_complete,
                collaboration: if params.collaboration {
                    &run.collaboration
                } else {
                    &[]
                },
            };
            bytes += usize::from(index > 0);
            bytes += json_len(&value, MAX_PRODUCT_REPLY_BYTES.saturating_sub(bytes)).ok_or_else(
                || io::Error::other("Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES"),
            )?;
        }
        pieces.push((tag, clipped, holder_spans, selected, identities));
    }
    for (tag, clipped, holder_spans, selected, identities) in pieces {
        let segments =
            holder_spans.unwrap_or_else(|| history.segments.get(tag).cloned().unwrap_or_default());
        result.series.push(OccupationTrendSeries {
            tag: tag.clone(),
            segments,
            membership: selected.cloned(),
            identities: identities.map(|identities| identities.into_iter().cloned().collect()),
            runs: clipped
                .into_iter()
                .map(
                    |(run, first_hour, last_hour, starts_segment)| OccupationTrendRun {
                        first_hour,
                        last_hour,
                        starts_segment,
                        collaboration: if params.collaboration {
                            run.collaboration.clone()
                        } else {
                            Vec::new()
                        },
                        read_hour: run.read_hour,
                        resistance_raw: run.resistance_raw.clone(),
                        compliance_raw: run.compliance_raw.clone(),
                        states: run.states,
                        held_states: run.held_states,
                        held_complete: run.held_complete,
                    },
                )
                .collect(),
        });
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
            OccupationCollaborationActor::decl(),
            OccupationCollaboration::decl(),
            OccupationTrendRun::decl(),
            OccupationTrendSeries::decl(),
            OccupationTrendReply::decl(),
        ]
    }
    #[cfg(test)]
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
        let histories = country_history::branch_histories(
            source,
            params.branch_id.parse().expect("validated branch"),
            false,
            &[],
            cancelled,
        )?;
        reply_with_histories(
            &params.branch_id,
            &scanned.trend,
            params,
            warnings,
            cancelled,
            Some(&histories),
        )
    }
}

#[cfg(test)]
mod tests;
