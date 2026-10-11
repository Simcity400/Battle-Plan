//! Every country's nuclear stockpile as the game's top bar shows it.
//!
//! The top bar displays a signed integer: each nuclear amount truncates,
//! then the amounts add with signed-dword wrapping. Ready count instead
//! counts queued drops whose delay elapsed; it is not the available arsenal.
//! Every value is dated to a complete hourly Nukes read. Missing rows,
//! incomplete tables, gaps and identity changes never become zero.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::HourSpan,
    country_history::{RecordedHistories, branch_histories, observe_identity, selected_identities},
    validate_tag,
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{
        IdentityTrack, TaggedCountry, link_slot, slot_read, tagged_countries, validate_campaign,
    },
};
use engine::raid_tooltip_operands::{NUCLEAR_BOMB_TOKEN, THERMONUCLEAR_BOMB_TOKEN};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeTypeInfo {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeCount {
    pub ordinal: u32,
    pub value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
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
    if super::finite_fixed64(raw) {
        (raw.wrapping_mul(100_000) / 10_000_000_000_000) as i32
    } else {
        0xa7c5
    }
}

/// Maps recorded nuclear ordinals to their name tokens. The token spelling
/// and displayed name come from recorded content.
fn name_token(ordinal: u32) -> Option<u32> {
    match ordinal {
        0 => Some(NUCLEAR_BOMB_TOKEN),
        1 => Some(THERMONUCLEAR_BOMB_TOKEN),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScannedNukes {
    pub read_hours: Vec<i32>,
    pub empty_read_hours: Vec<i32>,
    pub series: BTreeMap<String, Vec<NukeRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub types: BTreeSet<u32>,
    pub tokens: BTreeMap<u32, String>,
    pub refused: BTreeMap<u32, usize>,
    pub unreadable: usize,
}

#[derive(Default)]
struct Track {
    runs: Vec<NukeRun>,
    segments: Vec<HourSpan>,
    last: Option<((u32, u32), i32)>,
    last_identity: IdentityTrack,
}

pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedNukes> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Nukes,
        TableId::LexicalTokens,
    ];
    let mut scanned = ScannedNukes::default();
    let mut tracks = BTreeMap::<String, Track>::new();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error());
                }
                let Ok(frames) = frames else {
                    continue;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancelled_error());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    if scanned.tokens.len() < 2
                        && let Ok(lexical) = decoded.complete::<tables::LexicalTokens>()
                    {
                        for ordinal in 0..=1 {
                            if scanned.tokens.contains_key(&ordinal) {
                                continue;
                            }
                            let token = name_token(ordinal).expect("native mapping");
                            if let Ok(index) = lexical.binary_search_by_key(&token, |row| row.token)
                                && let Ok(text) = std::str::from_utf8(&lexical[index].literal)
                            {
                                scanned.tokens.insert(ordinal, text.to_owned());
                            }
                        }
                    }
                    let rows = decoded.complete::<tables::Nukes>().ok();
                    let read = slot_read(&decoded.metrics, TableId::Nukes, decoded.hour);
                    if read && rows.is_some() {
                        scanned.read_hours.push(frame.hour);
                    }
                    let (Ok(countries), Ok(links)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                    ) else {
                        continue;
                    };
                    let mut by_slot = HashMap::<u32, BTreeMap<u32, Option<i32>>>::new();
                    let mut resolved_read = true;
                    if read {
                        for row in rows.into_iter().flatten() {
                            scanned.types.insert(row.nuke_type);
                            if name_token(row.nuke_type).is_none() {
                                *scanned.refused.entry(row.nuke_type).or_default() += 1;
                            }
                            let Some(slot) = link_slot(links, row.country) else {
                                resolved_read = false;
                                scanned.unreadable += 1;
                                continue;
                            };
                            let values = by_slot.entry(slot).or_default();
                            match values.entry(row.nuke_type) {
                                std::collections::btree_map::Entry::Vacant(entry) => {
                                    entry.insert(Some(displayed_amount(row.amount_raw)));
                                }
                                std::collections::btree_map::Entry::Occupied(mut entry) => {
                                    entry.insert(None);
                                    scanned.unreadable += 1;
                                }
                            }
                        }
                    }
                    let mut empty_read = read && rows.is_some() && resolved_read;
                    for (tag, lookup) in tagged_countries(countries, links) {
                        let TaggedCountry::Found(country, slot) = lookup else {
                            empty_read = false;
                            continue;
                        };
                        let Ok(tag) = std::str::from_utf8(tag) else {
                            empty_read = false;
                            scanned.unreadable += 1;
                            continue;
                        };
                        if !tracks.contains_key(tag) {
                            tracks.insert(tag.to_owned(), Track::default());
                        }
                        let track = tracks.get_mut(tag).expect("inserted above");
                        let identity = (slot, country.id);
                        observe_identity(
                            &mut track.segments,
                            &mut track.last_identity,
                            identity,
                            frame.hour,
                        );
                        if rows.is_none() {
                            continue;
                        }
                        let continues = track.last.is_some_and(|(seen, hour)| {
                            seen == identity && hour.checked_add(1) == Some(frame.hour)
                        });
                        if !read {
                            if continues && let Some(run) = track.runs.last_mut() {
                                run.last_hour = frame.hour;
                                track.last = Some((identity, frame.hour));
                            }
                            continue;
                        }
                        let Some(values) = by_slot.get(&slot) else {
                            continue;
                        };
                        if values.values().any(Option::is_none) {
                            empty_read = false;
                            continue;
                        }
                        let counts: Vec<_> = values
                            .iter()
                            .map(|(&ordinal, value)| NukeCount {
                                ordinal,
                                value: value.expect("unique row"),
                            })
                            .collect();
                        // The stockpile sums every array entry without testing its type.
                        // The recorded type equals its ordinal: only a missing entry inside
                        // this recorded prefix makes the total unknown.
                        let total = values
                            .keys()
                            .enumerate()
                            .all(|(index, ordinal)| usize::try_from(*ordinal) == Ok(index))
                            .then(|| {
                                counts
                                    .iter()
                                    .fold(0i32, |total, count| total.wrapping_add(count.value))
                            });
                        empty_read &= total == Some(0);
                        track.last = Some((identity, frame.hour));
                        match track.runs.last_mut() {
                            Some(run)
                                if continues && run.counts == counts && run.total == total =>
                            {
                                run.last_hour = frame.hour;
                            }
                            _ => {
                                track.runs.push(NukeRun {
                                    first_hour: frame.hour,
                                    last_hour: frame.hour,
                                    starts_segment: !continues,
                                    counts,
                                    total,
                                });
                            }
                        }
                    }
                    if empty_read {
                        scanned.empty_read_hours.push(frame.hour);
                    }
                }
            }
            Ok(())
        },
    )?;
    // Stockpile totals use this country's own array count; independent countries
    // can have different counts. A complete contiguous prefix proves that array,
    // even when another country records more types.
    for (tag, track) in tracks {
        if track.runs.is_empty() {
            continue;
        }
        scanned.segments.insert(tag.clone(), track.segments);
        scanned.series.insert(tag, track.runs);
    }
    scanned.read_hours.sort_unstable();
    scanned.read_hours.dedup();
    scanned.empty_read_hours.sort_unstable();
    scanned.empty_read_hours.dedup();
    Ok(scanned)
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
        .map(|&ordinal| {
            let token = scanned.tokens.get(&ordinal).cloned();
            NukeTypeInfo {
                ordinal,
                name: token.as_deref().and_then(&localise),
                token,
                supported: name_token(ordinal).is_some(),
            }
        })
        .collect()
}

pub(crate) fn requested_series(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<NukeSeries>> {
    if params.origin_hour.is_some() && history.is_none() {
        return Err(io::Error::other(
            "Nuclear drill identity history is unavailable",
        ));
    }
    let membership = if let Some(history) = history.filter(|_| params.top.is_none()) {
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
        Some(aliases)
    } else {
        params
            .top
            .map(|count| {
                crate::ranking::top_membership(
                    &scanned.series,
                    count as usize,
                    |run| (run.first_hour, run.last_hour),
                    |run| run.total.filter(|total| *total != 0).map(i64::from),
                    cancelled,
                )
            })
            .transpose()?
    };
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
            crate::transport::json_len(&measure, MAX_PRODUCT_REPLY_BYTES)
                .ok_or_else(reply_too_large)?,
        );
        if let Some(spans) = spans {
            let mut first_piece = true;
            crate::ranking::for_each_piece(
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
                        crate::transport::json_len(&piece, MAX_PRODUCT_REPLY_BYTES)
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
                crate::ranking::for_each_piece(
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
    io::Error::other(
        "NuclearReplyTooLarge: the complete nuclear stockpile reply exceeds the transport budget; no countries or runs were omitted",
    )
}

#[derive(Default)]
struct Scans {
    branch: Option<(u64, Arc<ScannedNukes>)>,
}

pub(crate) fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedNukes>> {
    let state = source.state::<Scans>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((id, scanned)) = &cache.branch
        && *id == branch
    {
        return Ok(Arc::clone(scanned));
    }
    let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
    cache.branch = Some((branch, Arc::clone(&scanned)));
    Ok(scanned)
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
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scanned = scanned_branch(
            source,
            params.branch_id.parse().expect("validated branch"),
            cancelled,
        )?;
        let mut warnings = Vec::new();
        let types = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(catalogue) => {
                let types = names(&catalogue, &scanned);
                warnings.extend(catalogue.verification.iter().cloned());
                warnings.extend(catalogue.warnings.iter().cloned());
                warnings.extend(catalogue.refusal_lines());
                types
            }
            Err(error) => {
                warnings.push(format!("Nuclear type names are unavailable: {error}"));
                type_info(&scanned, |_| None)
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
        let unknown_hours: u64 = scanned
            .series
            .values()
            .flatten()
            .filter(|run| run.total.is_none())
            .map(|run| (i64::from(run.last_hour) - i64::from(run.first_hour) + 1) as u64)
            .sum();
        if unknown_hours > 0 {
            warnings.push(format!("{unknown_hours} country-hours have a hole in their native nuclear array; totals stay unknown."));
        }
        let history = (params.origin_hour.is_some() || !params.tags.is_empty())
            .then(|| {
                branch_histories(
                    source,
                    params.branch_id.parse().expect("validated branch"),
                    false,
                    &[],
                    cancelled,
                )
            })
            .transpose()?;
        let series = requested_series(&scanned, params, history.as_deref(), cancelled)?;
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
        if crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(reply_too_large());
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
