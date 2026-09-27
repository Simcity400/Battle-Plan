//! One country's politics over a branch, for the Ruling party widget: each
//! party's popularity, which party ruled, and when elections were held.
//!
//! Parties are named by their ideology group token, joined to content for the
//! group's name and colour, so a mod's ideologies chart with their own colours.
//! Popularity is the engine's fixed-point percentage (`PoliticalParties.
//! popularity`, [`POPULARITY_DECIMALS`] places). Runs never bridge a gap, a
//! missing row or a change of country identity. An election is dated to the
//! hour `Politics.last_election` first shows a new date.

use super::{
    CountryParams, LoadContext, Product, country_measure::FixedRun,
    politics_content::politics_content,
};
use crate::{
    Source,
    metadata::{mapped_slot, validate_campaign},
    products::country_groups::HourSpan,
};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct PartyHistory;

/// Decimal places of recorded popularity (a percentage).
pub const POPULARITY_DECIMALS: u8 = 5;
const MAX_RUNS: usize = 400_000;
const CACHED_COUNTRIES: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartySeries {
    /// The party's ideology group token.
    pub token: String,
    pub runs: Vec<FixedRun>,
}

/// A party ruling over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulingRun {
    pub token: String,
    pub first_hour: i32,
    pub last_hour: i32,
    /// Follows a gap or a change of country identity rather than the previous run,
    /// so the change between them is not a change of government.
    pub starts_segment: bool,
}

/// An ideology group as content names and colours it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeologyInfo {
    pub token: String,
    pub name: Option<String>,
    /// sRGB bytes; null when content gives no usable colour.
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartyHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub parties: Vec<PartySeries>,
    pub ruling: Vec<RulingRun>,
    /// Hours an election was first seen held.
    pub elections: Vec<i32>,
    /// Every ideology group in content order, for colours, names and legend order.
    pub ideologies: Vec<IdeologyInfo>,
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedParties {
    pub parties: BTreeMap<String, PartySeries>,
    pub ruling: Vec<RulingRun>,
    pub elections: Vec<i32>,
    pub unobserved: Vec<HourSpan>,
}

fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) >= Some(first_hour) => {
            span.last_hour = span.last_hour.max(last_hour)
        }
        _ => spans.push(HourSpan {
            first_hour,
            last_hour,
        }),
    }
}

/// Scans one branch for one country's parties.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedParties> {
    use replay::rows::TableId;
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Politics,
        TableId::PoliticalParties,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Party request cancelled");
    let mut result = ScannedParties::default();
    let mut last: Option<((u32, u32), i32)> = None;
    let mut last_election: Option<i32> = None;
    let mut runs = 0_usize;
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
            let hour = frame.hour;
            // Elections come from Politics, which may be missing on its own; parties do not need it.
            let (Ok(countries), Ok(links), Ok(parties)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
                decoded.complete::<tables::PoliticalParties>(),
            ) else {
                mark(&mut result.unobserved, hour, hour);
                continue;
            };
            let Some((country, slot)) = countries
                .iter()
                .filter(|country| country.tag == tag.as_bytes())
                .find_map(|country| mapped_slot(country, links).ok().map(|slot| (country, slot)))
            else {
                continue;
            };
            let identity = (slot, country.id);
            // Only a change of country identity starts politics over; a gap in the same
            // country still dates an election, to the hour it is first seen.
            let same_identity = last.is_some_and(|(seen, _)| seen == identity);
            let continues =
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            if let Some((_, at)) = last
                && at.checked_add(1).is_some_and(|next| next < hour)
            {
                mark(&mut result.unobserved, at + 1, hour - 1);
            }
            last = Some((identity, hour));
            let owner = |raw: u32| {
                links
                    .binary_search_by_key(&raw, |link| link.id)
                    .ok()
                    .map(|index| links[index].canonical_slot)
                    == Some(slot)
            };
            for party in parties.iter().filter(|party| owner(party.country)) {
                let Ok(token) = std::str::from_utf8(&party.token) else {
                    continue;
                };
                if token.is_empty() {
                    continue;
                }
                let series =
                    result
                        .parties
                        .entry(token.to_owned())
                        .or_insert_with(|| PartySeries {
                            token: token.to_owned(),
                            runs: Vec::new(),
                        });
                match series.runs.last_mut() {
                    Some(run)
                        if continues
                            && run.last_hour.checked_add(1) == Some(hour)
                            && run.value_raw.parse::<i64>() == Ok(party.popularity) =>
                    {
                        run.last_hour = hour
                    }
                    previous => {
                        let starts_segment = !(continues
                            && previous
                                .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                        runs += 1;
                        if runs > MAX_RUNS {
                            return Err(io::Error::other(
                                "Party history exceeds its memory budget",
                            ));
                        }
                        series.runs.push(FixedRun {
                            first_hour: hour,
                            last_hour: hour,
                            value_raw: party.popularity.to_string(),
                            starts_segment,
                        });
                    }
                }
                if party.is_ruling != 0 {
                    match result.ruling.last_mut() {
                        Some(run)
                            if continues
                                && run.token == token
                                && run.last_hour.checked_add(1) == Some(hour) =>
                        {
                            run.last_hour = hour
                        }
                        previous => {
                            let starts_segment = !(continues
                                && previous
                                    .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                            result.ruling.push(RulingRun {
                                token: token.to_owned(),
                                first_hour: hour,
                                last_hour: hour,
                                starts_segment,
                            })
                        }
                    }
                }
            }
            if !same_identity {
                last_election = None;
            }
            // When Politics is unrecorded this hour, the next recorded date still shows
            // any election since.
            if let Ok(politics) = decoded.complete::<tables::Politics>()
                && let Some(row) = politics.iter().find(|row| owner(row.country))
            {
                // The first date seen is when the recording started, not an election.
                if last_election.is_some_and(|previous| previous != row.last_election) {
                    result.elections.push(hour);
                }
                last_election = Some(row.last_election);
            }
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedParties>)>,
}

impl Product for PartyHistory {
    const KEY: &'static str = "party-history";
    type Params = CountryParams;
    type Reply = PartyHistoryReply;

    fn types() -> Vec<String> {
        vec![
            PartySeries::decl(),
            RulingRun::decl(),
            IdeologyInfo::decl(),
            PartyHistoryReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = Vec::new();
        // Without the game folders the chart still draws, in palette colours and with tokens for names.
        let ideologies = match context.content() {
            Ok(content) => match politics_content(source, content) {
                Ok(politics) => {
                    warnings.extend(politics.warnings.iter().cloned());
                    politics
                        .ideologies
                        .groups
                        .iter()
                        .map(|group| IdeologyInfo {
                            token: group.token.clone(),
                            name: politics.localise(&group.token),
                            color: group.color,
                        })
                        .collect()
                }
                Err(error) => {
                    warnings.push(format!(
                        "Ideology names and colours are unavailable: {error}"
                    ));
                    Vec::new()
                }
            },
            Err(error) => {
                warnings.push(format!(
                    "Ideology names and colours are unavailable: {error}"
                ));
                Vec::new()
            }
        };
        if scanned.parties.is_empty() {
            warnings.push(format!(
                "{}: no recorded parties in this branch",
                params.tag
            ));
        }
        Ok(PartyHistoryReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: POPULARITY_DECIMALS,
            parties: scanned.parties.values().cloned().collect(),
            ruling: scanned.ruling.clone(),
            elections: scanned.elections.clone(),
            ideologies,
            unobserved: scanned.unobserved.clone(),
            warnings,
        })
    }
}
