//! Every country's nuclear stockpile as the game's top bar shows it, prepared as the `nukes`
//! series family ([`facts`], [`emit`]) and answered from its publication ([`query`]), which the
//! scoreboard and the event feed read too.
//!
//! The top bar displays a signed integer: each nuclear amount truncates,
//! then the amounts add with signed-dword wrapping. Ready count instead
//! counts queued drops whose delay elapsed; it is not the available arsenal.
//! Every value is dated to a complete hourly Nukes read. Missing rows,
//! incomplete tables, gaps and identity changes never become zero.

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
};
use dashboard_base::{
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::country_groups::HourSpan,
    products::country_history::{RecordedHistories, selected_identities},
    products::validate_tag,
};
use engine_native::{
    raid_tooltip_operands::NUCLEAR_BOMB_TOKEN, raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;

#[cfg(any(test, feature = "test-support"))]
pub use row::{scan, scanned_branch};

pub struct CountryNukes;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NukesParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    /// A drill selects the native country holding its tag at this recorded hour.
    #[serde(default)]
    pub origin_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeTypeInfo {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeCount {
    pub ordinal: u32,
    pub value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub counts: Vec<NukeCount>,
    /// Unknown for a hole in this country's native array; unnamed types still count.
    pub total: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeSeries {
    pub tag: String,
    pub runs: Vec<NukeRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryNukesReply {
    pub branch_id: String,
    /// Every fresh complete Nukes read, including reads supplying unchanged runs.
    pub read_hours: Vec<i32>,
    /// Complete reads with no nonzero or unknown array among countries present.
    /// Absent rows are outside the race, never supplied as synthetic zero values.
    pub empty_read_hours: Vec<i32>,
    pub types: Vec<NukeTypeInfo>,
    pub series: Vec<NukeSeries>,
    pub warnings: Vec<String>,
}

/// Recorded nuclear amount to displayed whole stockpile, including its sentinel
/// and machine-width multiply/divide/narrowing.
pub fn displayed_amount(raw: i64) -> i32 {
    if dashboard_base::products::finite_fixed64(raw) {
        (raw.wrapping_mul(100_000) / 10_000_000_000_000) as i32
    } else {
        0xa7c5
    }
}

/// Maps recorded nuclear ordinals to their name tokens. The token spelling
/// and displayed name come from recorded content.
pub(crate) fn name_token(ordinal: u32) -> Option<u32> {
    match ordinal {
        0 => Some(NUCLEAR_BOMB_TOKEN),
        1 => Some(THERMONUCLEAR_BOMB_TOKEN),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedNukes {
    pub read_hours: Vec<i32>,
    pub empty_read_hours: Vec<i32>,
    pub series: BTreeMap<String, Vec<NukeRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub types: BTreeSet<u32>,
    pub tokens: BTreeMap<u32, String>,
    pub refused: BTreeMap<u32, usize>,
    pub unreadable: usize,
    pub unknown_hours: u64,
}

fn cancelled_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Nuclear stockpile request cancelled",
    )
}

pub fn type_info(
    scanned: &ScannedNukes,
    localise: impl Fn(&str) -> Option<String>,
) -> Vec<NukeTypeInfo> {
    scanned
        .types
        .iter()
        .map(|&ordinal| info(scanned, ordinal, &localise))
        .collect()
}

/// Every type the game names, spelled from the recorded lexical tokens
/// whether or not a stockpile read saw it.
pub fn native_type_info(
    scanned: &ScannedNukes,
    localise: impl Fn(&str) -> Option<String>,
) -> Vec<NukeTypeInfo> {
    (0..)
        .take_while(|&ordinal| name_token(ordinal).is_some())
        .map(|ordinal| info(scanned, ordinal, &localise))
        .collect()
}

fn info(
    scanned: &ScannedNukes,
    ordinal: u32,
    localise: impl Fn(&str) -> Option<String>,
) -> NukeTypeInfo {
    let token = scanned.tokens.get(&ordinal).cloned();
    NukeTypeInfo {
        ordinal,
        name: token.as_deref().and_then(&localise),
        token,
        supported: name_token(ordinal).is_some(),
    }
}

pub(crate) fn requested_series(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<NukeSeries>> {
    let membership = membership(scanned, params, history, cancelled)?;
    requested_selected(scanned, params, history, cancelled, membership)
}
fn membership(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<Option<BTreeMap<String, Vec<HourSpan>>>> {
    if params.origin_hour.is_some() && history.is_none() {
        return Err(dashboard_core::refused(
            "Nuclear drill identity history is unavailable",
        ));
    }
    let membership = if let Some(history) = history.filter(|_| params.top.is_none()) {
        Some(history_membership(params, history)?)
    } else {
        params
            .top
            .map(|count| {
                dashboard_base::ranking::top_membership(
                    &scanned.series,
                    count as usize,
                    |run| (run.first_hour, run.last_hour),
                    |run| run.total.filter(|total| *total != 0).map(i64::from),
                    cancelled,
                )
            })
            .transpose()?
    };
    Ok(membership)
}

/// The hours each tag held a native country the selection names: one its tags held, or the
/// one its tag held at its origin hour.
pub(crate) fn history_membership(
    params: &NukesParams,
    history: &RecordedHistories,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let selected = if let Some(hour) = params.origin_hour {
        params
            .tags
            .first()
            .and_then(|tag| history.countries.get(tag))
            .and_then(|country| {
                country
                    .identities
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            })
            .map(|run| (run.slot, run.country_id))
            .into_iter()
            .collect()
    } else {
        selected_identities(history, &params.tags)
    };
    let aliases: BTreeMap<String, Vec<HourSpan>> = history
        .countries
        .iter()
        .filter_map(|(tag, country)| {
            let spans: Vec<_> = country
                .identities
                .iter()
                .filter(|run| selected.contains(&(run.slot, run.country_id)))
                .map(|run| HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                })
                .collect();
            (!spans.is_empty()).then(|| (tag.clone(), spans))
        })
        .collect();
    if params.origin_hour.is_some() {
        let mut spans: Vec<_> = aliases.values().flatten().collect();
        spans.sort_by_key(|span| span.first_hour);
        if spans
            .windows(2)
            .any(|pair| pair[0].last_hour >= pair[1].first_hour)
        {
            return Err(io::Error::other(
                "NuclearDrillIdentityAmbiguous: multiple recorded tags hold the selected native country at one hour",
            ));
        }
    }
    Ok(aliases)
}
fn requested_selected(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<Vec<NukeSeries>> {
    let tags: BTreeSet<&str> = match &membership {
        Some(top) => top.keys().map(String::as_str).collect(),
        None => params.tags.iter().map(String::as_str).collect(),
    };
    let mut series = Vec::new();
    // Measure the serialized selection before copying histories. Only the
    // complete reply's transport budget bounds a recording's country/run count.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SeriesMeasure<'a> {
        tag: &'a str,
        runs: &'a [NukeRun],
        segments: &'a [HourSpan],
        membership: Option<&'a [HourSpan]>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RunMeasure<'a> {
        first_hour: i32,
        last_hour: i32,
        starts_segment: bool,
        counts: &'a [NukeCount],
        total: Option<i32>,
    }
    let mut bytes = 2usize;
    for tag in &tags {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        let Some(runs) = scanned.series.get(*tag) else {
            continue;
        };
        let spans = membership.as_ref().and_then(|top| top.get(*tag));
        let measure = SeriesMeasure {
            tag,
            runs: if spans.is_some() { &[] } else { runs },
            segments: if params.top.is_none() && history.is_some() {
                spans.map_or(&[][..], Vec::as_slice)
            } else {
                scanned.segments.get(*tag).map_or(&[][..], Vec::as_slice)
            },
            membership: spans.map(Vec::as_slice),
        };
        if bytes > 2 {
            bytes = bytes.saturating_add(1);
        }
        bytes = bytes.saturating_add(
            dashboard_core::transport::json_len(&measure, MAX_PRODUCT_REPLY_BYTES)
                .ok_or_else(reply_too_large)?,
        );
        if let Some(spans) = spans {
            let mut first_piece = true;
            dashboard_base::ranking::for_each_piece(
                runs,
                spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                |run, first_hour, last_hour| {
                    let piece = RunMeasure {
                        first_hour,
                        last_hour,
                        starts_segment: run.starts_segment && first_hour == run.first_hour,
                        counts: &run.counts,
                        total: run.total,
                    };
                    if !first_piece {
                        bytes = bytes.saturating_add(1);
                    }
                    first_piece = false;
                    bytes = bytes.saturating_add(
                        dashboard_core::transport::json_len(&piece, MAX_PRODUCT_REPLY_BYTES)
                            .ok_or_else(reply_too_large)?,
                    );
                    if bytes > MAX_PRODUCT_REPLY_BYTES {
                        return Err(reply_too_large());
                    }
                    Ok(())
                },
            )?;
        }
        if bytes > MAX_PRODUCT_REPLY_BYTES {
            return Err(reply_too_large());
        }
    }
    for tag in tags {
        let Some(runs) = scanned.series.get(tag) else {
            continue;
        };
        let spans = membership.as_ref().and_then(|top| top.get(tag));
        let kept = match spans {
            None => runs.clone(),
            Some(spans) => {
                let mut kept = Vec::new();
                dashboard_base::ranking::for_each_piece(
                    runs,
                    spans,
                    |run| (run.first_hour, run.last_hour),
                    cancelled,
                    |run, first_hour, last_hour| {
                        let mut piece = run.clone();
                        piece.first_hour = first_hour;
                        piece.last_hour = last_hour;
                        piece.starts_segment = run.starts_segment && first_hour == run.first_hour;
                        kept.push(piece);
                        Ok(())
                    },
                )?;
                kept
            }
        };
        series.push(NukeSeries {
            tag: tag.to_owned(),
            runs: kept,
            segments: if params.top.is_none() && history.is_some() {
                spans.cloned().unwrap_or_default()
            } else {
                scanned.segments.get(tag).cloned().unwrap_or_default()
            },
            membership: spans.cloned(),
        });
    }
    Ok(series)
}

fn reply_too_large() -> io::Error {
    dashboard_base::products::over_budget(
        "NuclearReplyTooLarge: the complete nuclear stockpile reply exceeds the transport budget; no countries or runs were omitted",
    )
}

fn names(catalogue: &Catalogue, scanned: &ScannedNukes) -> Vec<NukeTypeInfo> {
    type_info(scanned, |token| {
        catalogue.localisation.get(token)?;
        catalogue.localised(token, catalogue::View::Plain)
    })
}

impl Product for CountryNukes {
    const KEY: &'static str = "country-nukes";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = NukesParams;
    type Reply = CountryNukesReply;

    fn types() -> Vec<String> {
        vec![
            NukesParams::decl(),
            NukeTypeInfo::decl(),
            NukeCount::decl(),
            NukeRun::decl(),
            NukeSeries::decl(),
            CountryNukesReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params
            .top
            .is_some_and(|count| count == 0 || !params.tags.is_empty())
        {
            return Err("Invalid top countries");
        }
        if params.origin_hour.is_some() && (params.top.is_some() || params.tags.len() != 1) {
            return Err("A nuclear drill needs one tag at its origin hour");
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        row::load(source, params, context, cancelled)
    }
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;

pub(crate) fn present(
    source: &Source,
    params: &NukesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scanned: &ScannedNukes,
    history: Option<&RecordedHistories>,
    prepared_membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<CountryNukesReply> {
    let mut warnings = Vec::new();
    let types = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => {
            let types = names(&catalogue, scanned);
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(catalogue.refusal_lines());
            types
        }
        Err(error) => {
            warnings.push(format!("Nuclear type names are unavailable: {error}"));
            type_info(scanned, |_| None)
        }
    };
    for (ordinal, count) in &scanned.refused {
        warnings.push(format!("Unknown type {ordinal}: {count} recorded nuclear rows have no proven native name mapping; their recorded amounts still count in complete country totals."));
    }
    for kind in &types {
        if kind.supported && kind.name.is_none() {
            warnings.push(format!(
                "Nuclear type {}: no name in the recorded content; shown as unknown.",
                kind.token.as_deref().unwrap_or("unknown token")
            ));
        }
    }
    if scanned.unreadable > 0 {
        warnings.push(format!(
            "{} nuclear rows or country tags could not be resolved uniquely and are left out.",
            scanned.unreadable
        ));
    }
    let unknown_hours = scanned.unknown_hours;
    if unknown_hours > 0 {
        warnings.push(format!("{unknown_hours} country-hours have a hole in their native nuclear array; totals stay unknown."));
    }
    let series = if let Some(membership) = prepared_membership {
        requested_selected(scanned, params, history, cancelled, Some(membership))?
    } else {
        requested_series(scanned, params, history, cancelled)?
    };
    if params.origin_hour.is_some() && series.is_empty() {
        warnings.push("No nuclear stockpile or native country identity was recorded for the selected country at its origin hour.".into());
    }
    let reply = CountryNukesReply {
        branch_id: params.branch_id.clone(),
        read_hours: scanned.read_hours.clone(),
        empty_read_hours: scanned.empty_read_hours.clone(),
        types,
        series,
        warnings,
    };
    if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(reply_too_large());
    }
    Ok(reply)
}
