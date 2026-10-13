//! World tension over a branch, for the World tension & threat widget: the world
//! tension the game showed, each country's contribution to it and the events
//! (threat sources) that raised or lowered it.
//!
//! Every value is a recorded field, kept exact (`engine::cworld_threat`,
//! `engine::cthreat_source`):
//!
//! - A threat source's contributions (`current_raw`, `final_raw`) are fixed
//!   point with [`cthreat_source::FIXED_POINT_SCALE`] units per percentage point
//!   of world tension: sum every source's current contribution,
//!   clamp the sum to [`cworld_threat::CLAMP_CEILING`] (100%) and store it
//!   divided by [`cworld_threat::SCALAR_DIVISOR`] as the world tension
//!   (`WorldTension.total_raw`). So world tension has 1,000 units per percentage
//!   point and contributions 100,000, and the sum of the contributions can pass
//!   100% or include negative contributions, such as those from a peace treaty.
//! - `ThreatCountryAggregates` is the engine's own sum of the current
//!   contributions per canonical country slot, with its array sized to the
//!   country census, so every country is listed and a zero is a recorded
//!   zero. These are the widget's layers. Each read checks it against the sources
//!   and the world tension against the clamped sum, and the reply says how many
//!   reads disagreed.
//! - A source's `reason` is the text the game built for it in the recording's
//!   language with colour codes, so its event name is the game's own
//!   ([`display_text`]).
//!   `final_raw` is the contribution it adds once its ramp ends, `date` the hour
//!   it was created. The vector has no ids and is compacted, so an event is
//!   identified by its own recorded country reference, second country, date and
//!   reason (and its place among sources equal in all four), never by the slot
//!   the reference resolves to: a reference first read unresolved and named at a
//!   later read stays one event, which takes the country once it is named.
//!
//! A country that no longer exists keeps its sources until they decay, so a
//! slot keeps the tag, and a raw link the slot, they were last recorded with, at
//! any hour recording `Countries` and `CountryLinks`, a read or not.
//!
//! A country's contributions are kept by its identity, its canonical slot, and
//! named by the tag the slot held at the read: one slot still contributing under a
//! tag that another slot now holds is a second country, never joined to it, and a
//! slot recorded under another tag is another entry.
//!
//! The three tables are read together every hour (daily, one slot, in older
//! recordings). A read is an hour whose frame timed a walk of them
//! ([`slot_read`]). Values start at a read, are carried over
//! the following recorded hours and end at a gap or at an hour whose tables are
//! incomplete, so an hour the recording lacks is never given a value.

use super::{
    BranchParams, LoadContext, Product,
    country_history::{CountryIdentityRun, branch_histories},
    country_measure::FixedRun,
};
use crate::{
    Source,
    catalogue::display_text,
    metadata::{link_slot, mapped_slot, slot_read, validate_campaign},
};
use engine::{cthreat_source, cworld_threat};
use replay::{
    container::Replay,
    frame::{FrameDecoder, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct WorldTension;

/// Branches kept per source.
const CACHED_BRANCHES: usize = 4;
/// World tension units per percentage point: contributions' units over the
/// divisor the engine stores the clamped sum with.
const TENSION_SCALE: i64 = cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR;
const _: () = assert!(
    TENSION_SCALE * cworld_threat::SCALAR_DIVISOR == cthreat_source::FIXED_POINT_SCALE
        && cthreat_source::FIXED_POINT_SCALE <= u32::MAX as i64
);

/// One read of the world's tension, held over the following hours until the next.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionRead {
    /// The hour it was read, where its values start.
    pub first_hour: i32,
    /// The last hour the read's values hold.
    pub last_hour: i32,
    /// The read follows a gap or an hour without the tables.
    pub starts_segment: bool,
    /// World tension as the game showed it, in [`TensionReply::tension_scale`]
    /// units (`WorldTension.total_raw`).
    pub tension_raw: String,
    /// Every country's contribution summed before the game's cap, in
    /// [`TensionReply::contribution_scale`] units.
    pub sum_raw: String,
    /// The part of `sum_raw` from country slots no recorded country row names,
    /// in the same units; "0" when every contributing country is named.
    pub unnamed_raw: String,
}

/// One country's contribution: runs of reads where it was not zero. An hour of a
/// read without a run of the country is a recorded zero; an hour of no read is
/// not recorded.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionCountry {
    /// The tag the country held at these reads.
    pub tag: String,
    /// Its identity: the canonical country slot its contributions were recorded
    /// under. Two entries may share a tag, never a slot and a tag.
    pub slot: u32,
    /// In [`TensionReply::contribution_scale`] units. A run starts a segment where
    /// the hours before it were not recorded (a gap, or an hour without the
    /// tables); after a recorded zero it continues the line.
    pub runs: Vec<FixedRun>,
    /// Native holders of this cached tag/slot. Colours and names may only be
    /// sampled within these recorded bounds, including after the holder vanished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}

/// The contribution an event adds, as recorded from a read on.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionAdded {
    /// The read where the source first held this value.
    pub hour: i32,
    /// The source's `final_raw`, in [`TensionReply::contribution_scale`] units.
    pub added_raw: String,
}

/// A threat source: something a country did that changed world tension.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionEvent {
    /// The hour the game created it (`date`).
    pub hour: i32,
    /// The country that caused it; null when no recorded country row names it.
    pub tag: Option<String>,
    /// The game's own text for it, without formatting codes.
    pub reason: String,
    /// What it adds, from its first read on; a later entry when the game changed it.
    pub added: Vec<TensionAdded>,
    /// The first and last read listing it.
    pub first_read: i32,
    pub last_read: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionReply {
    pub branch_id: String,
    /// World tension units per percentage point.
    pub tension_scale: u32,
    /// Contribution units per percentage point.
    pub contribution_scale: u32,
    /// The sum of contributions world tension is capped at, in contribution units.
    pub ceiling_raw: String,
    /// In hour order.
    pub reads: Vec<TensionRead>,
    /// Every country that contributed in the branch, in tag then slot order.
    pub countries: Vec<TensionCountry>,
    /// In hour order, then first read.
    pub events: Vec<TensionEvent>,
    pub warnings: Vec<String>,
}

/// An event's identity: its recorded country and second country references, date,
/// reason, and its place among sources equal in all four within a read.
type EventKey = (u32, u32, i32, Vec<u8>, u32);
/// A source's country and second country references, date and reason.
type SourceIdentity<'a> = (u32, u32, i32, &'a [u8]);

#[derive(Clone, Debug, Default)]
pub struct ScannedTension {
    pub reads: Vec<TensionRead>,
    pub countries: Vec<TensionCountry>,
    pub events: Vec<TensionEvent>,
    /// Reads whose aggregates disagree with their sources' current contributions.
    pub aggregate_mismatches: usize,
    /// Reads whose world tension is not the game's cap of the aggregates' sum.
    pub tension_mismatches: usize,
    /// Reads whose source rows are incomplete, so their events may be missing.
    pub partial_sources: usize,
}

/// A country's runs while scanning: runs by read index, so a carried read extends
/// them without touching every country.
#[derive(Default)]
struct Building {
    tag: String,
    slot: u32,
    /// (first read, last read, value, starts segment)
    runs: Vec<(usize, usize, i64, bool)>,
}

/// The tag of each canonical slot this hour, as `mapped_slot` confirms it.
fn slot_tags(countries: &[CountryRow], links: &[CountryLinkRow]) -> HashMap<u32, String> {
    countries
        .iter()
        .filter_map(|row| {
            mapped_slot(row, links)
                .ok()
                .map(|slot| (slot, String::from_utf8_lossy(&row.tag).into_owned()))
        })
        .collect()
}

/// Scans one branch for its tension reads, contributions and events.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedTension> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::WorldTension,
        TableId::ThreatSources,
        TableId::ThreatCountryAggregates,
    ];
    let cancel = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "World tension request cancelled",
        )
    };
    let mut result = ScannedTension::default();
    let mut building: Vec<Building> = Vec::new();
    // Each country by its identity (slot) and the tag it held.
    let mut by_country: HashMap<(u32, String), usize> = HashMap::new();
    // The tag a slot was last recorded under: a country that no longer exists keeps
    // its sources, and so its contribution, until they decay.
    let mut known: HashMap<u32, String> = HashMap::new();
    // The slot a raw link was last recorded for, for the same reason.
    let mut known_links: HashMap<u32, u32> = HashMap::new();
    let mut events: HashMap<EventKey, usize> = HashMap::new();
    let mut last_hour: Option<i32> = None;
    // Whether the last read is carried to the previous hour.
    let mut open = false;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    open = false;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        open = false;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        open = false;
                        break;
                    }
                    let hour = frame.hour;
                    let continues = last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
                    last_hour = Some(hour);
                    open &= continues;
                    // Identities are remembered from every hour that records them, with or
                    // without the tension tables: a country seen only then still names its
                    // slot and links at a later read.
                    let identity = decoded
                        .complete::<tables::Countries>()
                        .ok()
                        .zip(decoded.complete::<tables::CountryLinks>().ok());
                    if let Some((countries, links)) = identity {
                        known.extend(slot_tags(countries, links));
                        known_links.extend(links.iter().map(|link| (link.id, link.canonical_slot)));
                    }
                    let tables = identity
                        .map(|(_, links)| links)
                        .zip(decoded.complete::<tables::WorldTension>().ok())
                        .zip(decoded.complete::<tables::ThreatCountryAggregates>().ok());
                    let Some(((links, tension), aggregates)) = tables else {
                        open = false;
                        continue;
                    };
                    if !slot_read(replay.format, &decoded.metrics, TableId::WorldTension, hour) {
                        if open && let Some(read) = result.reads.last_mut() {
                            read.last_hour = hour;
                        }
                        continue;
                    }
                    let Some(tension) = tension.first() else {
                        open = false;
                        continue;
                    };
                    let index = result.reads.len();
                    let starts_segment = !open;
                    open = true;
                    let mut sum = 0_i64;
                    let mut unnamed = 0_i64;
                    for row in aggregates.iter().filter(|row| row.total_raw != 0) {
                        sum = sum.checked_add(row.total_raw).ok_or_else(|| {
                            io::Error::other(
                                "World tension contribution sum exceeds signed 64-bit range",
                            )
                        })?;
                        let Some(tag) = known.get(&row.country_slot) else {
                            unnamed = unnamed.checked_add(row.total_raw).ok_or_else(|| {
                            io::Error::other(
                                "World tension unnamed contribution sum exceeds signed 64-bit range",
                            )
                        })?;
                            continue;
                        };
                        let at = *by_country
                            .entry((row.country_slot, tag.clone()))
                            .or_insert_with(|| {
                                building.push(Building {
                                    tag: tag.clone(),
                                    slot: row.country_slot,
                                    runs: Vec::new(),
                                });
                                building.len() - 1
                            });
                        let runs = &mut building[at].runs;
                        match runs.last_mut() {
                            Some(run)
                                if !starts_segment
                                    && run.1 + 1 == index
                                    && run.2 == row.total_raw =>
                            {
                                run.1 = index;
                            }
                            // One identity, so only a gap breaks its line.
                            _ => runs.push((index, index, row.total_raw, starts_segment)),
                        }
                    }
                    result.reads.push(TensionRead {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment,
                        tension_raw: tension.total_raw.to_string(),
                        sum_raw: sum.to_string(),
                        unnamed_raw: unnamed.to_string(),
                    });
                    // World tension stores the clamped and scaled sum.
                    let capped =
                        sum.clamp(0, cworld_threat::CLAMP_CEILING) / cworld_threat::SCALAR_DIVISOR;
                    if capped != tension.total_raw {
                        result.tension_mismatches += 1;
                    }
                    let sources = match &*decoded.tables[TableId::ThreatSources.index()] {
                        TableView::Complete(rows) => rows
                            .rows::<tables::ThreatSources>()
                            .map(|rows| (rows, true)),
                        TableView::Truncated { rows, .. } => rows
                            .rows::<tables::ThreatSources>()
                            .map(|rows| (rows, false)),
                        TableView::Unavailable { .. } => None,
                    };
                    let whole = sources.is_some_and(|(rows, complete)| {
                        complete && usize::try_from(tension.source_count).ok() == Some(rows.len())
                    });
                    if !whole {
                        result.partial_sources += 1;
                    } else if let Some((rows, _)) = sources {
                        let current: i64 = rows.iter().try_fold(0_i64, |sum, row| {
                            sum.checked_add(row.current_raw).ok_or_else(|| {
                                io::Error::other(
                                    "World tension source sum exceeds signed 64-bit range",
                                )
                            })
                        })?;
                        if current != sum {
                            result.aggregate_mismatches += 1;
                        }
                    }
                    // Sources equal in all four, counted in stored order.
                    let mut seen: HashMap<SourceIdentity, u32> = HashMap::new();
                    for row in sources.map(|(rows, _)| rows).unwrap_or_default() {
                        let slot = link_slot(links, row.country)
                            .or_else(|| known_links.get(&row.country).copied());
                        let place = seen
                            .entry((
                                row.country,
                                row.second_country,
                                row.date,
                                row.reason.as_slice(),
                            ))
                            .or_default();
                        let key = (
                            row.country,
                            row.second_country,
                            row.date,
                            row.reason.clone(),
                            *place,
                        );
                        *place += 1;
                        let added = row.final_raw.to_string();
                        let tag = slot.and_then(|slot| known.get(&slot).cloned());
                        match events.get(&key) {
                            Some(&at) => {
                                let event = &mut result.events[at];
                                event.last_read = hour;
                                // A country first read unnamed takes its name once it is recorded.
                                if event.tag.is_none() {
                                    event.tag = tag;
                                }
                                if event
                                    .added
                                    .last()
                                    .is_none_or(|last| last.added_raw != added)
                                {
                                    event.added.push(TensionAdded {
                                        hour,
                                        added_raw: added,
                                    });
                                }
                            }
                            None => {
                                events.insert(key, result.events.len());
                                result.events.push(TensionEvent {
                                    hour: row.date,
                                    tag,
                                    reason: display_text(&row.reason).map_err(|reason| {
                                        io::Error::other(format!(
                                            "The recorded threat reason at hour {} {reason}",
                                            row.date
                                        ))
                                    })?,
                                    added: vec![TensionAdded {
                                        hour,
                                        added_raw: added,
                                    }],
                                    first_read: hour,
                                    last_read: hour,
                                });
                            }
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    let reads = &result.reads;
    let mut countries: Vec<_> = building
        .into_iter()
        .map(|country| TensionCountry {
            tag: country.tag,
            slot: country.slot,
            identities: None,
            runs: country
                .runs
                .into_iter()
                .map(|(first, last, value, starts_segment)| FixedRun {
                    first_hour: reads[first].first_hour,
                    last_hour: reads[last].last_hour,
                    value_raw: value.to_string(),
                    starts_segment,
                })
                .collect(),
        })
        .collect();
    countries.sort_by(|a, b| (&a.tag, a.slot).cmp(&(&b.tag, b.slot)));
    result.countries = countries;
    result
        .events
        .sort_by_key(|event| (event.hour, event.first_read));
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(u64, Arc<ScannedTension>)>,
}

impl Product for WorldTension {
    const KEY: &'static str = "world-tension";
    // Display text comes from the recording only.
    const CONTENT_DEPENDENT: bool = false;
    type Params = BranchParams;
    type Reply = TensionReply;

    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            TensionRead::decl(),
            TensionCountry::decl(),
            TensionAdded::decl(),
            TensionEvent::decl(),
            TensionReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == branch)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != branch);
                cache.entries.push_back((branch, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_BRANCHES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = Vec::new();
        if scanned.reads.is_empty() {
            warnings.push("No world tension was recorded in this branch.".to_owned());
        }
        if scanned.partial_sources > 0 {
            warnings.push(format!(
                "Reads whose threat sources were not all recorded: {}. Events they would name may be missing.",
                scanned.partial_sources
            ));
        }
        if scanned.aggregate_mismatches > 0 {
            warnings.push(format!(
                "Reads whose country contributions do not add up to their threat sources: {}.",
                scanned.aggregate_mismatches
            ));
        }
        if scanned.tension_mismatches > 0 {
            warnings.push(format!(
                "Reads whose world tension is not the game's cap of the contributions' sum: {}.",
                scanned.tension_mismatches
            ));
        }
        let unnamed = scanned
            .reads
            .iter()
            .filter(|read| read.unnamed_raw != "0")
            .count();
        if unnamed > 0 {
            warnings.push(format!(
                "Reads with a contribution from a country the recording never names: {unnamed}. It is counted in Other."
            ));
        }
        let history = branch_histories(source, branch, false, &[], cancelled)?;
        let mut countries = scanned.countries.clone();
        for country in &mut countries {
            country.identities = Some(history.countries.get(&country.tag).map_or_else(
                Vec::new,
                |holder| {
                    holder
                        .identities
                        .iter()
                        .filter(|identity| identity.slot == country.slot)
                        .cloned()
                        .collect()
                },
            ));
        }
        let unproved = countries
            .iter()
            .filter(|country| {
                country.runs.first().is_some_and(|run| {
                    !country.identities.as_ref().is_some_and(|identities| {
                        identities.iter().any(|identity| {
                            identity.first_hour <= run.first_hour
                                && run.first_hour <= identity.last_hour
                        })
                    })
                })
            })
            .count();
        if unproved > 0 {
            warnings.push(format!("{unproved} contributor tag/slot entries have no proven native holder at their first contribution read; their counts remain exact, while country looks stay unknown."));
        }
        let reply = TensionReply {
            branch_id: params.branch_id.clone(),
            tension_scale: TENSION_SCALE as u32,
            contribution_scale: cthreat_source::FIXED_POINT_SCALE as u32,
            ceiling_raw: cworld_threat::CLAMP_CEILING.to_string(),
            reads: scanned.reads.clone(),
            countries,
            events: scanned.events.clone(),
            warnings,
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
