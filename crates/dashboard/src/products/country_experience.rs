//! Army, navy and air experience dated by each CountryExperience read.
//!
//! Banked values hold until the next read within one recorded country stretch.
//! Change per day is their net difference from a read exactly 24 hours earlier
//! in that stretch, including experience spent and the game's cap. The engine's
//! daily accumulators are not daily gains: DailyUpdate@141292fc0, called by
//! CCountry::daily_update@1406daaf0 inside CGameState::DailyUpdate@1401d4390,
//! adds passive gain and zeroes them at the day boundary before the daily-slot
//! read. Read dating and hourly identity checks follow country_fuel.

use super::{LoadContext, Product, validate_countries};
use crate::{
    Source,
    metadata::{IdentityTrack, link_slot, slot_read, tagged_country, validate_campaign},
};
use engine::ccountry_experience_status::SCALE;
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct CountryExperience;
const MAX_READS: usize = 400_000;
const CACHED_COUNTRIES: usize = 32;
/// A change compares reads one day apart, rather than partial-day accumulators.
const DAY_HOURS: i32 = 24;
const UNITS: u32 = SCALE as u32;
const _: () = assert!(UNITS as i64 == SCALE);

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExperienceParams {
    pub branch_id: String,
    pub tags: Vec<String>,
}

/// All three branches share the same dating and availability.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceChange {
    pub army_raw: String,
    pub navy_raw: String,
    pub air_raw: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ChangeUnknown {
    /// No read exactly a day earlier in this uninterrupted country stretch.
    NoMatchingRead,
}

/// Exact banked values and their net change, held from this read's hour.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceRead {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub army_raw: String,
    pub navy_raw: String,
    pub air_raw: String,
    pub change: Option<ExperienceChange>,
    pub change_unknown: Option<ChangeUnknown>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceSeries {
    pub tag: String,
    pub reads: Vec<ExperienceRead>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceReply {
    pub branch_id: String,
    /// Units per experience point, for banked values and changes alike.
    pub scale: u32,
    pub day_hours: i32,
    pub series: Vec<ExperienceSeries>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Seen {
    track: IdentityTrack,
    open: bool,
    recent: VecDeque<(i32, [i64; 3])>,
}

/// Scans the branch once for all requested tags, resolving each tag every hour.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tags: &[&str],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, ExperienceSeries>> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperience,
    ];
    let mut result: BTreeMap<String, ExperienceSeries> = tags
        .iter()
        .map(|tag| {
            (
                (*tag).to_owned(),
                ExperienceSeries {
                    tag: (*tag).to_owned(),
                    reads: Vec::new(),
                },
            )
        })
        .collect();
    let mut seen: BTreeMap<String, Seen> = result
        .keys()
        .map(|tag| (tag.clone(), Seen::default()))
        .collect();
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Experience request cancelled");
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
            let read_hour = slot_read(&decoded.metrics, TableId::CountryExperience, hour);
            for (tag, series) in &mut result {
                let state = seen.get_mut(tag).expect("selected tag");
                let identity = decoded
                    .complete::<tables::Countries>()
                    .ok()
                    .zip(decoded.complete::<tables::CountryLinks>().ok())
                    .and_then(|(countries, links)| {
                        tagged_country(countries, links, tag)
                            .found()
                            .map(|(country, slot)| ((slot, country.id), links))
                    });
                let continues = state
                    .track
                    .step(identity.map(|(identity, _)| identity), hour);
                state.open &= continues;
                if !state.open {
                    state.recent.clear();
                }
                let (Some(((slot, _), links)), Ok(rows)) =
                    (identity, decoded.complete::<tables::CountryExperience>())
                else {
                    state.open = false;
                    continue;
                };
                if !read_hour {
                    if state.open
                        && let Some(read) = series.reads.last_mut()
                    {
                        read.last_hour = hour;
                    }
                    continue;
                }
                let Some(row) = rows
                    .iter()
                    .find(|row| link_slot(links, row.country) == Some(slot))
                else {
                    state.open = false;
                    continue;
                };
                if series.reads.len() >= MAX_READS {
                    return Err(io::Error::other(
                        "Experience history exceeds its memory budget",
                    ));
                }
                while state
                    .recent
                    .front()
                    .is_some_and(|(at, _)| i64::from(hour) - i64::from(*at) > i64::from(DAY_HOURS))
                {
                    state.recent.pop_front();
                }
                let banked = [row.army_raw, row.navy_raw, row.air_raw];
                let change = hour
                    .checked_sub(DAY_HOURS)
                    .and_then(|before| state.recent.iter().find(|(at, _)| *at == before))
                    .map(|(_, previous)| {
                        // Widen before subtraction: two valid signed raws can differ by more than i64.
                        let delta = |index: usize| {
                            (i128::from(banked[index]) - i128::from(previous[index])).to_string()
                        };
                        ExperienceChange {
                            army_raw: delta(0),
                            navy_raw: delta(1),
                            air_raw: delta(2),
                        }
                    });
                series.reads.push(ExperienceRead {
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment: !state.open,
                    army_raw: banked[0].to_string(),
                    navy_raw: banked[1].to_string(),
                    air_raw: banked[2].to_string(),
                    change_unknown: change.is_none().then_some(ChangeUnknown::NoMatchingRead),
                    change,
                });
                state.recent.push_back((hour, banked));
                state.open = true;
            }
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), ExperienceSeries)>,
}

impl Product for CountryExperience {
    const KEY: &'static str = "country-experience";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ExperienceParams;
    type Reply = ExperienceReply;

    fn types() -> Vec<String> {
        vec![
            ExperienceParams::decl(),
            ExperienceChange::decl(),
            ChangeUnknown::decl(),
            ExperienceRead::decl(),
            ExperienceSeries::decl(),
            ExperienceReply::decl(),
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
        let key = |tag: &str| (branch, tag.to_owned());
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        let missing: Vec<&str> = params
            .tags
            .iter()
            .map(String::as_str)
            .filter(|tag| !cache.entries.iter().any(|(cached, _)| *cached == key(tag)))
            .collect();
        let scanned = if missing.is_empty() {
            BTreeMap::new()
        } else {
            scan(&source.replay(), branch, &missing, cancelled)?
        };
        let mut warnings = Vec::new();
        // Build before eviction, as country-measure does, so every selected country stays in the reply.
        let series = params
            .tags
            .iter()
            .map(|tag| {
                let found = scanned.get(tag).or_else(|| {
                    cache
                        .entries
                        .iter()
                        .find(|(cached, _)| *cached == key(tag))
                        .map(|(_, series)| series)
                });
                let found = found.cloned().unwrap_or_else(|| ExperienceSeries {
                    tag: tag.clone(),
                    reads: Vec::new(),
                });
                if found.reads.is_empty() {
                    warnings.push(format!("{tag}: no recorded experience in this branch"));
                }
                found
            })
            .collect();
        for (tag, series) in scanned {
            cache.entries.push_back((key(&tag), series));
        }
        while cache.entries.len() > CACHED_COUNTRIES {
            cache.entries.pop_front();
        }
        Ok(ExperienceReply {
            branch_id: params.branch_id.clone(),
            scale: UNITS,
            day_hours: DAY_HOURS,
            series,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
