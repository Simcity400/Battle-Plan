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
    country_history::{self, CountryIdentityRun},
    validate_countries,
};
use crate::{
    Source,
    metadata::{IdentityTrack, slot_read, validate_campaign},
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
pub(crate) const DAY_HOURS: i32 = 24;
pub const UNITS: u32 = SCALE as u32;
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

/// An [`ExperienceRead`] as exact integers; only replies format them.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct RawRead {
    pub first_hour: i32,
    pub(crate) last_hour: i32,
    pub read_hour: i32,
    pub starts_segment: bool,
    banked: [i64; 3],
    /// Banked at the read exactly a day earlier in this stretch.
    day_before: Option<[i64; 3]>,
    gain: Option<[i128; 3]>,
}

impl RawRead {
    pub fn values(&self) -> [[Option<i128>; 3]; 3] {
        [
            self.banked.map(|value| Some(i128::from(value))),
            self.gain.map_or([None; 3], |values| values.map(Some)),
            // Widen before subtracting two valid signed banked amounts.
            std::array::from_fn(|index| {
                self.day_before
                    .map(|previous| i128::from(self.banked[index]) - i128::from(previous[index]))
            }),
        ]
    }
}

impl From<&RawRead> for ExperienceRead {
    fn from(read: &RawRead) -> Self {
        let branches = |values: [Option<i128>; 3]| {
            Some(ExperienceChange {
                army_raw: values[0]?.to_string(),
                navy_raw: values[1]?.to_string(),
                air_raw: values[2]?.to_string(),
            })
        };
        let [_, gain, change] = read.values();
        let change = branches(change);
        Self {
            first_hour: read.first_hour,
            last_hour: read.last_hour,
            read_hour: read.read_hour,
            starts_segment: read.starts_segment,
            army_raw: read.banked[0].to_string(),
            navy_raw: read.banked[1].to_string(),
            air_raw: read.banked[2].to_string(),
            change_unknown: change.is_none().then_some(ChangeUnknown::NoMatchingRead),
            change,
            gain: branches(gain),
            gain_unknown: read.gain.is_none().then_some(GainUnknown::NoCompleteDay),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Seen {
    track: IdentityTrack,
    native_instance: Option<u32>,
    open: bool,
    recent: VecDeque<(i32, [i64; 3])>,
    gains: VecDeque<(i32, u32, [i64; 3])>,
    held: Option<RawRead>,
}

/// The walk's reads, as the reply shows them.
#[cfg(any(test, feature = "test-support"))]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tags: &[&str],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, ExperienceSeries>> {
    Ok(scan_reads(replay, branch, tags, cancelled)?
        .into_iter()
        .map(|(tag, reads)| {
            let series = ExperienceSeries {
                tag: tag.clone(),
                identities: None,
                reads: reads.iter().map(ExperienceRead::from).collect(),
            };
            (tag, series)
        })
        .collect())
}

/// One numeric walk; native country state survives a change of its recorded tag.
fn scan_reads(
    replay: &Replay,
    branch: u64,
    tags: &[&str],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<RawRead>>> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperience,
        TableId::CountryExperienceGains,
    ];
    let mut result: BTreeMap<String, Vec<RawRead>> = tags
        .iter()
        .map(|tag| ((*tag).to_owned(), Vec::new()))
        .collect();
    let mut observer = Scan::default();
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Experience request cancelled");
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
                    observer.step(decoded, replay.format, None, |tag, _, read| {
                        if let Some(reads) = result.get_mut(tag) {
                            if let Some(previous) = reads
                                .last_mut()
                                .filter(|old| old.first_hour == read.first_hour)
                            {
                                *previous = read;
                            } else {
                                reads.push(read);
                            }
                        }
                        Ok(())
                    })?;
                }
            }
            Ok(())
        },
    )?;
    Ok(result)
}

#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    #[serde(with = "crate::prepared_production::map_pairs")]
    seen: BTreeMap<(u32, u32), Seen>,
    published: BTreeMap<String, ((u32, u32), RawRead)>,
    hour: Option<i32>,
    active: BTreeSet<(String, (u32, u32))>,
    present: BTreeSet<(u32, u32)>,
    #[serde(skip)]
    native: Native,
    #[serde(skip)]
    pub removed: Vec<(String, (u32, u32))>,
}

#[derive(Default)]
struct Native {
    countries: Option<super::territory::CountryReads>,
    keys: Option<crate::metadata::CountryKeys>,
    country_rows: Option<Vec<replay::rows::CountryRow>>,
    links: std::sync::Arc<std::collections::HashMap<u32, u32>>,
    identities: BTreeMap<(u32, u32), String>,
    slots: BTreeMap<u32, Vec<(u32, u32)>>,
    removed: Vec<(String, (u32, u32))>,
    roster_changed: bool,
    banked_rows: Option<Vec<replay::rows::CountryExperienceRow>>,
    gain_rows: Option<Vec<replay::rows::CountryExperienceGainRow>>,
    banked: Option<BTreeMap<u32, Option<replay::rows::CountryExperienceRow>>>,
    gains: Option<BTreeMap<u32, Option<replay::rows::CountryExperienceGainRow>>>,
    clock: BTreeSet<u32>,
    #[cfg(any(test, feature = "test-support"))]
    pub work: crate::prepared::observer_tests::Work,
    #[cfg(any(test, feature = "test-support"))]
    clock_ticks: usize,
}
impl Native {
    fn refresh(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> BTreeSet<u32> {
        self.removed.clear();
        self.roster_changed = false;
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let mut joined = false;
        let links_changed = changed(TableId::CountryLinks);
        if links_changed {
            let links = frame.complete::<tables::CountryLinks>().ok();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += links.map_or(0, |rows| rows.len());
            }
            self.links =
                std::sync::Arc::new(links.map(crate::metadata::link_slots).unwrap_or_default());
        }
        let mut countries_changed = false;
        if changed(TableId::Countries) {
            let countries = frame.complete::<tables::Countries>().ok();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += countries.map_or(0, |rows| rows.len());
            }
            countries_changed = match (&self.keys, countries) {
                (Some(old), Some(rows)) => !old.same(rows),
                (old, rows) => old.is_some() != rows.is_some(),
            };
            if countries_changed {
                self.keys = countries.map(crate::metadata::CountryKeys::read);
                self.country_rows = self.keys.as_ref().map(|keys| {
                    keys.iter()
                        .map(|(id, canonical_slot, tag)| replay::rows::CountryRow {
                            id,
                            canonical_slot,
                            tag: tag.to_vec(),
                            ..Default::default()
                        })
                        .collect()
                });
            }
        }
        if countries_changed || links_changed {
            self.countries = self.country_rows.as_ref().map(|rows| {
                super::territory::CountryReads::from_slots(
                    frame,
                    rows,
                    std::sync::Arc::clone(&self.links),
                )
            });
            let mut identities = BTreeMap::new();
            for (tag, identity) in self.countries.iter().flat_map(|rows| rows.holders()) {
                if let Some(identity) = identity {
                    identities
                        .entry(*identity)
                        .and_modify(|tag| *tag = None)
                        .or_insert(Some(tag.clone()));
                }
            }
            let identities: BTreeMap<_, _> = identities
                .into_iter()
                .filter_map(|(id, tag)| tag.map(|tag| (id, tag)))
                .collect();
            self.removed.extend(
                self.identities
                    .iter()
                    .filter(|(id, tag)| identities.get(id) != Some(tag))
                    .map(|(id, tag)| (tag.clone(), *id)),
            );
            self.identities = identities;
            self.slots.clear();
            for identity in self.identities.keys() {
                self.slots.entry(identity.0).or_default().push(*identity);
            }
            joined = true;
        }
        let mut dirty = BTreeSet::new();
        self.roster_changed = joined;
        if joined {
            dirty.extend(self.identities.keys().map(|identity| identity.0));
        }
        let banked_changed = changed(TableId::CountryExperience);
        if banked_changed {
            let rows = frame.complete::<tables::CountryExperience>().ok();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            self.banked_rows = rows.map(|rows| rows.to_vec());
        }
        let gains_changed = changed(TableId::CountryExperienceGains);
        if gains_changed {
            let rows = frame.complete::<tables::CountryExperienceGains>().ok();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            self.gain_rows = rows.map(|rows| rows.to_vec());
        }
        if banked_changed || links_changed {
            let next = self.banked_rows.as_ref().map(|rows| {
                country_history::unique_by_slots(rows, &self.links, |row| row.country)
                    .rows
                    .into_iter()
                    .map(|(slot, row)| (slot, row.cloned()))
                    .collect::<BTreeMap<_, _>>()
            });
            dirty.extend(
                self.banked
                    .iter()
                    .chain(&next)
                    .flat_map(|rows| rows.keys())
                    .copied()
                    .filter(|slot| {
                        self.banked.as_ref().and_then(|rows| rows.get(slot))
                            != next.as_ref().and_then(|rows| rows.get(slot))
                    }),
            );
            self.banked = next;
        }
        if gains_changed || links_changed {
            let next = self.gain_rows.as_ref().map(|rows| {
                country_history::unique_by_slots(rows, &self.links, |row| row.country)
                    .rows
                    .into_iter()
                    .map(|(slot, row)| (slot, row.cloned()))
                    .collect::<BTreeMap<_, _>>()
            });
            dirty.extend(
                self.gains
                    .iter()
                    .chain(&next)
                    .flat_map(|rows| rows.keys())
                    .copied()
                    .filter(|slot| {
                        self.gains.as_ref().and_then(|rows| rows.get(slot))
                            != next.as_ref().and_then(|rows| rows.get(slot))
                    }),
            );
            self.clock = next
                .iter()
                .flat_map(|rows| rows.iter())
                .filter_map(|(slot, row)| {
                    row.as_ref().filter(|row| row.complete == 1).map(|_| *slot)
                })
                .collect();
            self.gains = next;
        }
        dirty
    }
}
impl Scan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn work(&self) -> crate::prepared::observer_tests::Work {
        self.native.work
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn clock_ticks(&self) -> usize {
        self.native.clock_ticks
    }

    pub fn step(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
        mut sink: impl FnMut(&str, (u32, u32), RawRead) -> io::Result<()>,
    ) -> io::Result<()> {
        let previous = self.hour;
        let adjacent = previous.and_then(|hour| hour.checked_add(1)) == Some(decoded.hour);
        self.hour = Some(decoded.hour);
        self.removed.clear();
        let dirty = self.native.refresh(decoded, changes);
        let read_hour = changes.map_or_else(
            || {
                slot_read(
                    format,
                    &decoded.metrics,
                    TableId::CountryExperience,
                    decoded.hour,
                )
            },
            |changes| changes.slot_read[TableId::CountryExperience.index()],
        );
        let force = changes.is_none_or(|changes| {
            changes.rows_changed[TableId::Countries.index()]
                && changes.rows_changed[TableId::CountryLinks.index()]
        }) || !adjacent;
        let selected: BTreeSet<_> = if force || read_hour {
            self.native.identities.keys().copied().collect()
        } else {
            dirty
                .iter()
                .chain(&self.native.clock)
                .flat_map(|slot| self.native.slots.get(slot).into_iter().flatten().copied())
                .collect()
        };
        for removed in &self.native.removed {
            if self.active.remove(removed) {
                self.removed.push(removed.clone());
            }
        }
        // Runtime joins are rebuilt after resume. Only current output membership
        // is reconciled then, never the retained historical `published` map.
        if force {
            let removed: Vec<_> = self
                .active
                .iter()
                .filter(|(tag, id)| self.native.identities.get(id) != Some(tag))
                .cloned()
                .collect();
            for row in removed {
                self.active.remove(&row);
                self.removed.push(row);
            }
        }
        let Self {
            seen,
            published,
            native,
            active,
            present,
            removed,
            ..
        } = self;
        let hour = decoded.hour;
        for identity in selected {
            let tag = &native.identities[&identity];
            let output = (tag.clone(), identity);
            let previous_output = previous.filter(|_| adjacent && active.contains(&output));
            let state = seen.entry(identity).or_default();
            if adjacent
                && present.contains(&identity)
                && let Some(previous) = previous
            {
                state.track.advance(previous);
            }
            let mut continues = state.track.step(Some(identity), hour);
            if !continues {
                state.native_instance = None;
            }
            let observed = native
                .gains
                .as_ref()
                .and_then(|rows| rows.get(&identity.0))
                .and_then(Option::as_ref);
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
                #[cfg(any(test, feature = "test-support"))]
                {
                    native.clock_ticks += 1;
                }
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
            let Some(rows) = native.banked.as_ref() else {
                state.open = false;
                if active.remove(&output) {
                    removed.push(output);
                }
                continue;
            };
            let Some(row) = rows.get(&identity.0).and_then(Option::as_ref) else {
                state.open = false;
                if active.remove(&output) {
                    removed.push(output);
                }
                continue;
            };
            if !read_hour {
                if state.open
                    && let Some(held) = &state.held
                {
                    if changes.is_none() || force || dirty.contains(&identity.0) {
                        publish(
                            published,
                            tag,
                            identity,
                            held,
                            hour,
                            previous_output,
                            &mut sink,
                        )?;
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            native.work.emitted += 1;
                        }
                    }
                    active.insert(output);
                } else if active.remove(&output) {
                    removed.push(output);
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
            let day_before = hour
                .checked_sub(DAY_HOURS)
                .and_then(|before| state.recent.iter().find(|(at, _)| *at == before))
                .map(|(_, previous)| *previous);
            let gain = (state.gains.len() == DAY_HOURS as usize).then(|| {
                state
                    .gains
                    .iter()
                    .fold([0_i128; 3], |mut sum, (_, _, values)| {
                        for index in 0..3 {
                            sum[index] += i128::from(values[index]);
                        }
                        sum
                    })
            });
            let held = RawRead {
                first_hour: hour,
                last_hour: hour,
                read_hour: hour,
                starts_segment: !state.open,
                banked,
                day_before,
                gain,
            };
            publish(
                published,
                tag,
                identity,
                &held,
                hour,
                previous_output,
                &mut sink,
            )?;
            #[cfg(any(test, feature = "test-support"))]
            {
                native.work.emitted += 1;
            }
            state.held = Some(held);
            state.recent.push_back((hour, banked));
            state.open = true;
            active.insert(output);
        }
        if native.roster_changed || force {
            *present = native.identities.keys().copied().collect();
        }
        Ok(())
    }
}

fn publish(
    published: &mut BTreeMap<String, ((u32, u32), RawRead)>,
    tag: &str,
    identity: (u32, u32),
    held: &RawRead,
    hour: i32,
    previous_hour: Option<i32>,
    sink: &mut impl FnMut(&str, (u32, u32), RawRead) -> io::Result<()>,
) -> io::Result<()> {
    let previous = published.get(tag);
    let same_identity = previous.is_some_and(|(seen, _)| *seen == identity);
    let mut read = if let Some((_, read)) = previous.filter(|(_, read)| {
        same_identity
            && read.read_hour == held.read_hour
            && (read.last_hour.checked_add(1) == Some(hour)
                || previous_hour.is_some_and(|previous| read.last_hour <= previous))
    }) {
        *read
    } else {
        RawRead {
            first_hour: hour,
            starts_segment: !same_identity || held.starts_segment && held.read_hour == hour,
            ..*held
        }
    };
    read.last_hour = hour;
    sink(tag, identity, read)?;
    published.insert(tag.to_owned(), (identity, read));
    Ok(())
}

#[derive(Default)]
struct Cache {
    entries: BTreeMap<(u64, String), Vec<RawRead>>,
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
            scan_reads(&source.replay(), branch, &missing, cancelled)?
        };
        let mut warnings = Vec::new();
        let series = tags
            .iter()
            .map(|tag| {
                let found = scanned.get(tag).or_else(|| cache.entries.get(&key(tag)));
                let selected: Vec<_> = history
                    .countries
                    .get(tag)
                    .map_or(&[][..], |country| country.identities.as_slice())
                    .iter()
                    .filter(|identity| identities.contains(&(identity.slot, identity.country_id)))
                    .cloned()
                    .collect();
                let reads: Vec<_> = found
                    .into_iter()
                    .flatten()
                    .flat_map(|read| {
                        let first = selected
                            .partition_point(|identity| identity.last_hour < read.first_hour);
                        selected[first..]
                            .iter()
                            .take_while(move |identity| identity.first_hour <= read.last_hour)
                            .filter_map(move |identity| {
                                let mut piece = *read;
                                piece.first_hour = piece.first_hour.max(identity.first_hour);
                                piece.last_hour = piece.last_hour.min(identity.last_hour);
                                (piece.first_hour <= piece.last_hour)
                                    .then(|| ExperienceRead::from(&piece))
                            })
                    })
                    .collect();
                if reads.is_empty() {
                    warnings.push(format!("{tag}: no recorded experience in this branch"));
                }
                ExperienceSeries {
                    tag: tag.clone(),
                    identities: Some(selected),
                    reads,
                }
            })
            .collect();
        for (tag, reads) in scanned {
            cache.entries.insert(key(&tag), reads);
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
