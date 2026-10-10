//! Army, navy and air experience dated by each CountryExperience read.
//!
//! Banked values hold until the next read within one recorded country stretch.
//! Change per day is their net difference from a read exactly 24 hours earlier
//! in that stretch, including experience spent and the game's cap. The daily
//! accumulators are not daily gains: updates add passive gain and zero them
//! at the day boundary before the daily-slot read. Read dating and hourly
//! identity checks follow country_fuel.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_history::{self, CountryIdentityRun, unique_by_slot},
    validate_countries,
};
use crate::{
    Source,
    metadata::{IdentityTrack, slot_read, tagged_countries, validate_campaign},
};
use engine::ccountry_experience_status::SCALE;
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct CountryExperience;
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum GainUnknown {
    /// One or more preceding hourly intervals was not fully observed for this
    /// native country instance; a partial day must never become zero income.
    NoCompleteDay,
}

/// Banked, accepted gross income and net change, held from this read's hour.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceRead {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The supplying daily read, retained when a rename splits its held value.
    pub read_hour: i32,
    pub starts_segment: bool,
    pub army_raw: String,
    pub navy_raw: String,
    pub air_raw: String,
    pub change: Option<ExperienceChange>,
    pub change_unknown: Option<ChangeUnknown>,
    pub gain: Option<ExperienceChange>,
    pub gain_unknown: Option<GainUnknown>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceSeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
    pub reads: Vec<ExperienceRead>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceReply {
    pub branch_id: String,
    /// Units per experience point for every measure.
    pub scale: u32,
    pub day_hours: i32,
    pub series: Vec<ExperienceSeries>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Seen {
    track: IdentityTrack,
    native_instance: Option<u32>,
    open: bool,
    recent: VecDeque<(i32, [i64; 3])>,
    gains: VecDeque<(i32, u32, [i64; 3])>,
    held: Option<ExperienceRead>,
}

/// One numeric walk; native country state survives a change of its recorded tag.
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
        TableId::CountryExperienceGains,
    ];
    let mut result: BTreeMap<String, ExperienceSeries> = tags
        .iter()
        .map(|tag| {
            (
                (*tag).to_owned(),
                ExperienceSeries {
                    tag: (*tag).to_owned(),
                    identities: None,
                    reads: Vec::new(),
                },
            )
        })
        .collect();
    let mut seen: BTreeMap<(u32, u32), Seen> = BTreeMap::new();
    let mut published = BTreeMap::new();
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
            let inputs = decoded
                .complete::<tables::Countries>()
                .ok()
                .zip(decoded.complete::<tables::CountryLinks>().ok());
            let mut identities = BTreeMap::new();
            if let Some((countries, links)) = inputs {
                for (tag, country) in tagged_countries(countries, links) {
                    let Some((country, slot)) = country.found() else {
                        continue;
                    };
                    let Ok(tag) = std::str::from_utf8(tag) else {
                        continue;
                    };
                    // Two tags claiming one native identity cannot name its history.
                    identities
                        .entry((slot, country.id))
                        .and_modify(|tag| *tag = None)
                        .or_insert(Some(tag));
                }
            }
            let rows = inputs.and_then(|(_, links)| {
                decoded
                    .complete::<tables::CountryExperience>()
                    .ok()
                    .map(|rows| unique_by_slot(rows, links, |row| row.country))
            });
            let gains = inputs.and_then(|(_, links)| {
                decoded
                    .complete::<tables::CountryExperienceGains>()
                    .ok()
                    .map(|rows| unique_by_slot(rows, links, |row| row.country))
            });
            for (identity, tag) in identities {
                let Some(tag) = tag else {
                    continue;
                };
                let state = seen.entry(identity).or_default();
                let mut continues = state.track.step(Some(identity), hour);
                if !continues {
                    state.native_instance = None;
                }
                let observed = gains
                    .as_ref()
                    .and_then(|rows| rows.rows.get(&identity.0))
                    .and_then(|row| *row);
                if let Some(row) = observed {
                    continues &= state
                        .native_instance
                        .is_none_or(|instance| instance == row.instance);
                    state.native_instance = Some(row.instance);
                }
                if !continues {
                    state.gains.clear();
                }
                let gained = observed.filter(|row| row.complete == 1);
                if let Some(gained) = gained {
                    if state.gains.back().is_some_and(|(previous, instance, _)| {
                        previous.checked_add(1) != Some(hour) || *instance != gained.instance
                    }) {
                        state.gains.clear();
                    }
                    while state.gains.front().is_some_and(|(at, _, _)| {
                        i64::from(hour) - i64::from(*at) >= i64::from(DAY_HOURS)
                    }) {
                        state.gains.pop_front();
                    }
                    state.gains.push_back((
                        hour,
                        gained.instance,
                        [gained.army_raw, gained.navy_raw, gained.air_raw],
                    ));
                } else {
                    state.gains.clear();
                }
                state.open &= continues;
                if !state.open {
                    state.recent.clear();
                }
                let Some(rows) = rows.as_ref() else {
                    state.open = false;
                    continue;
                };
                let Some(row) = rows.rows.get(&identity.0).and_then(|row| *row) else {
                    state.open = false;
                    continue;
                };
                if !read_hour {
                    if state.open
                        && let Some(held) = &state.held
                    {
                        publish(&mut result, &mut published, tag, identity, held, hour);
                    }
                    continue;
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
                let gain = (state.gains.len() == DAY_HOURS as usize).then(|| {
                    let totals = state
                        .gains
                        .iter()
                        .fold([0_i128; 3], |mut sum, (_, _, values)| {
                            for index in 0..3 {
                                sum[index] += i128::from(values[index]);
                            }
                            sum
                        });
                    ExperienceChange {
                        army_raw: totals[0].to_string(),
                        navy_raw: totals[1].to_string(),
                        air_raw: totals[2].to_string(),
                    }
                });
                let held = ExperienceRead {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour: hour,
                    starts_segment: !state.open,
                    army_raw: banked[0].to_string(),
                    navy_raw: banked[1].to_string(),
                    air_raw: banked[2].to_string(),
                    change_unknown: change.is_none().then_some(ChangeUnknown::NoMatchingRead),
                    change,
                    gain_unknown: gain.is_none().then_some(GainUnknown::NoCompleteDay),
                    gain,
                };
                publish(&mut result, &mut published, tag, identity, &held, hour);
                state.held = Some(held);
                state.recent.push_back((hour, banked));
                state.open = true;
            }
        }
    }
    Ok(result)
}

fn publish(
    result: &mut BTreeMap<String, ExperienceSeries>,
    published: &mut BTreeMap<String, (u32, u32)>,
    tag: &str,
    identity: (u32, u32),
    held: &ExperienceRead,
    hour: i32,
) {
    let Some(series) = result.get_mut(tag) else {
        return;
    };
    let same_identity = if let Some(previous) = published.get_mut(tag) {
        let same = *previous == identity;
        *previous = identity;
        same
    } else {
        published.insert(tag.to_owned(), identity);
        false
    };
    if let Some(read) = series.reads.last_mut()
        && same_identity
        && read.read_hour == held.read_hour
        && read.last_hour.checked_add(1) == Some(hour)
    {
        read.last_hour = hour;
    } else {
        let mut read = held.clone();
        read.first_hour = hour;
        read.last_hour = hour;
        read.starts_segment = !same_identity || held.starts_segment && held.read_hour == hour;
        series.reads.push(read);
    }
}

#[derive(Default)]
struct Cache {
    entries: BTreeMap<(u64, String), ExperienceSeries>,
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
            GainUnknown::decl(),
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
        let history = country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let identities = country_history::selected_identities(&history, &params.tags);
        let mut tags = params.tags.clone();
        let mut included: BTreeSet<_> = tags.iter().cloned().collect();
        for alias in country_history::selected_aliases(&history, &params.tags) {
            if included.insert(alias.clone()) {
                tags.push(alias);
            }
        }
        let key = |tag: &str| (branch, tag.to_owned());
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        let missing: Vec<&str> = tags
            .iter()
            .map(String::as_str)
            .filter(|tag| !cache.entries.contains_key(&key(tag)))
            .collect();
        let scanned = if missing.is_empty() {
            BTreeMap::new()
        } else {
            scan(&source.replay(), branch, &missing, cancelled)?
        };
        let mut warnings = Vec::new();
        let series = tags
            .iter()
            .map(|tag| {
                let found = scanned.get(tag).or_else(|| cache.entries.get(&key(tag)));
                let mut found = found.cloned().unwrap_or_else(|| ExperienceSeries {
                    tag: tag.clone(),
                    identities: None,
                    reads: Vec::new(),
                });
                let selected: Vec<_> = history
                    .countries
                    .get(tag)
                    .map_or(&[][..], |country| country.identities.as_slice())
                    .iter()
                    .filter(|identity| identities.contains(&(identity.slot, identity.country_id)))
                    .cloned()
                    .collect();
                found.reads = found
                    .reads
                    .iter()
                    .flat_map(|read| {
                        let first = selected
                            .partition_point(|identity| identity.last_hour < read.first_hour);
                        selected[first..]
                            .iter()
                            .take_while(move |identity| identity.first_hour <= read.last_hour)
                            .filter_map(move |identity| {
                                let mut piece = read.clone();
                                piece.first_hour = piece.first_hour.max(identity.first_hour);
                                piece.last_hour = piece.last_hour.min(identity.last_hour);
                                (piece.first_hour <= piece.last_hour).then_some(piece)
                            })
                    })
                    .collect();
                found.identities = Some(selected);
                if found.reads.is_empty() {
                    warnings.push(format!("{tag}: no recorded experience in this branch"));
                }
                found
            })
            .collect();
        for (tag, series) in scanned {
            cache.entries.insert(key(&tag), series);
        }
        let reply = ExperienceReply {
            branch_id: params.branch_id.clone(),
            scale: UNITS,
            day_hours: DAY_HOURS,
            series,
            warnings,
        };
        if crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(io::Error::other(
                "ExperienceReplyTooLarge: the complete experience history exceeds the transport budget; no countries or reads were omitted",
            ));
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
