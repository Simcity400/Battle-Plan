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
    /// Complete gain runs, dated by their first hour. The last run holds
    /// through the scan's frontier until an observed change closes it.
    gains: VecDeque<(i32, [i64; 3])>,
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

/// The Countries fields a country's keys read besides its id, the row key
/// ([`crate::metadata::CountryKeys`]).
static COUNTRY_KEY_FIELDS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    crate::prepared::frame_changes::field_mask(TableId::Countries, &["canonical_slot", "tag"])
});

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
    /// The last complete gains read, held as the frame's own view.
    gain_rows: Option<replay::frame::SharedView>,
    /// The raw country ids `links` resolves to each slot.
    link_countries: BTreeMap<u32, Vec<u32>>,
    banked: Option<BTreeMap<u32, Option<replay::rows::CountryExperienceRow>>>,
    gains: Option<BTreeMap<u32, Option<replay::rows::CountryExperienceGainRow>>>,
    #[cfg(any(test, feature = "test-support"))]
    pub work: crate::prepared::observer_tests::Work,
    #[cfg(any(test, feature = "test-support"))]
    gain_updates: usize,
}
/// Replace the joins of changed owners only. Every raw alias of an affected
/// slot participates, so duplicate rows leave that slot unknown as in the
/// whole-table join. Returns the changed slots and rows visited.
fn apply_experience_rows<R: Clone + PartialEq>(
    rows: &[R],
    (changed, deleted): (&[replay::rows::DeltaRow], &[u128]),
    links: &std::collections::HashMap<u32, u32>,
    link_countries: &BTreeMap<u32, Vec<u32>>,
    held: &mut BTreeMap<u32, Option<R>>,
    country: fn(&R) -> u32,
    shift: u32,
) -> (BTreeSet<u32>, usize) {
    let slots: BTreeSet<u32> = changed
        .iter()
        .map(|row| row.key)
        .chain(deleted.iter().copied())
        .filter_map(|key| links.get(&((key >> shift) as u32)).copied())
        .collect();
    let mut visits = changed.len();
    let mut dirty = BTreeSet::new();
    for slot in slots {
        let mut found = Vec::new();
        for &raw in link_countries.get(&slot).into_iter().flatten() {
            let first = rows.partition_point(|row| country(row) < raw);
            let last = rows.partition_point(|row| country(row) <= raw);
            found.extend(&rows[first..last]);
        }
        visits += found.len();
        let next = match found.as_slice() {
            [] => None,
            [row] => Some(Some((*row).clone())),
            _ => Some(None),
        };
        if held.get(&slot) == next.as_ref() {
            continue;
        }
        dirty.insert(slot);
        match next {
            Some(row) => {
                held.insert(slot, row);
            }
            None => {
                held.remove(&slot);
            }
        }
    }
    (dirty, visits)
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
            self.link_countries.clear();
            for (&raw, &slot) in self.links.iter() {
                self.link_countries.entry(slot).or_default().push(raw);
            }
        }
        let mut countries_changed = false;
        if changed(TableId::Countries) {
            let countries = frame.complete::<tables::Countries>().ok();
            // No country deleted or added and no key field changed: the keys
            // are the hour before's without comparing every country.
            let kept = self.keys.is_some()
                && countries
                    .zip(self.country_rows.as_ref())
                    .is_some_and(|(rows, held)| rows.len() == held.len())
                && changes
                    .and_then(|changes| changes.rows(TableId::Countries))
                    .is_some_and(|(changed, deleted)| {
                        deleted.is_empty()
                            && changed
                                .iter()
                                .all(|row| row.mask & *COUNTRY_KEY_FIELDS == 0)
                    });
            #[cfg(any(test, feature = "test-support"))]
            if !kept {
                self.work.row_visits += countries.map_or(0, |rows| rows.len());
            }
            countries_changed = !kept
                && match (&self.keys, countries) {
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
        let mut banked_applied = false;
        if banked_changed {
            let rows = frame.complete::<tables::CountryExperience>().ok();
            if !links_changed
                && let (Some(rows), Some(delta), Some(held)) = (
                    rows,
                    changes.and_then(|changes| changes.rows(TableId::CountryExperience)),
                    self.banked.as_mut(),
                )
            {
                let (updated, visits) = apply_experience_rows(
                    rows,
                    delta,
                    &self.links,
                    &self.link_countries,
                    held,
                    |row| row.country,
                    0,
                );
                dirty.extend(updated);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += visits;
                }
                let _ = visits;
                banked_applied = true;
            } else {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.map_or(0, |rows| rows.len());
                }
            }
        }
        let gains_changed = changed(TableId::CountryExperienceGains);
        let mut gains_applied = false;
        if gains_changed {
            let rows = frame.complete::<tables::CountryExperienceGains>().ok();
            // With the links unchanged and the hour before's gains held, only
            // the slots of the countries whose rows the hour's recorded
            // changes name are joined again.
            if !links_changed
                && let (Some(rows), Some(changed)) = (
                    rows,
                    changes.and_then(|changes| changes.rows(TableId::CountryExperienceGains)),
                )
                && self.gains.is_some()
            {
                let (updated, visits) = apply_experience_rows(
                    rows,
                    changed,
                    &self.links,
                    &self.link_countries,
                    self.gains.as_mut().expect("held gains"),
                    |row| row.country,
                    32,
                );
                dirty.extend(updated);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += visits;
                }
                let _ = visits;
                gains_applied = true;
            } else {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.map_or(0, |rows| rows.len());
                }
            }
            self.gain_rows = rows
                .is_some()
                .then(|| frame.tables[TableId::CountryExperienceGains.index()].clone());
        }
        if !banked_applied && (banked_changed || links_changed) {
            let rows = frame.complete::<tables::CountryExperience>().ok();
            let next = rows.map(|rows| {
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
        if !gains_applied && (gains_changed || links_changed) {
            let gain_rows = self.gain_rows.as_deref().and_then(|view| match view {
                replay::frame::TableView::Complete(rows) => {
                    rows.rows::<tables::CountryExperienceGains>()
                }
                _ => None,
            });
            let next = gain_rows.map(|rows| {
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

    /// Gain runs visited at changed inputs or banked reads.
    #[cfg(any(test, feature = "test-support"))]
    pub fn gain_updates(&self) -> usize {
        self.native.gain_updates
    }

    pub fn step(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
        mut sink: impl FnMut(&str, (u32, u32), RawRead) -> io::Result<()>,
    ) -> io::Result<()> {
        let _cpu = ::replay::cpu::scope_phase("experience:observe");
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
                    native.gain_updates += 1;
                }
                let values = [gained.army_raw, gained.navy_raw, gained.air_raw];
                if state.gains.back().is_none_or(|(_, held)| *held != values) {
                    state.gains.push_back((hour, values));
                }
                let first = hour.saturating_sub(DAY_HOURS - 1);
                while state.gains.get(1).is_some_and(|(at, _)| *at <= first) {
                    state.gains.pop_front();
                }
                if let Some((at, _)) = state.gains.front_mut() {
                    *at = (*at).max(first);
                }
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
            let gain = state
                .gains
                .front()
                .filter(|(first, _)| {
                    i64::from(hour) - i64::from(*first) + 1 == i64::from(DAY_HOURS)
                })
                .map(|_| {
                    let mut sum = [0_i128; 3];
                    let mut end = i64::from(hour) + 1;
                    for &(first, values) in state.gains.iter().rev() {
                        let hours = end - i64::from(first);
                        for index in 0..3 {
                            sum[index] += i128::from(values[index]) * i128::from(hours);
                        }
                        end = i64::from(first);
                    }
                    sum
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

impl Scan {
    /// Hours `first..=last` following the last stepped hour, with no changed
    /// inputs or banked read. Open gain runs already hold through these hours;
    /// the next changed input or read visits their countries.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> bool {
        let _cpu = ::replay::cpu::scope_phase("experience:quiet");
        if self.hour.and_then(|hour| hour.checked_add(1)) != Some(first) || last < first {
            return false;
        }
        self.hour = Some(last);
        self.removed.clear();
        self.native.removed.clear();
        self.native.roster_changed = false;
        true
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

/// The tables a progressive experience sample reads.
pub const SAMPLE_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryExperience,
    TableId::CountryExperienceGains,
];

/// The measures a progressive sample answers: each branch's banked
/// experience, a daily read held until the next. Daily gain and change per
/// day need every hourly interval of a day, or a read exactly a day earlier,
/// so only preparation answers them.
pub const SAMPLED_MEASURES: [&str; 3] = ["army-banked", "navy-banked", "air-banked"];

/// A progressive sampler of banked experience, stepping its own scan through
/// the hours it is given in order as preparation does: an hour answers each
/// requested country's bank as its last read at or before the hour holds it,
/// dated by that read ([`READ_HOUR`]). An hour that does not follow the last
/// one given starts afresh, so a held read never crosses hours the sampler
/// did not see, and an hour before the first read it saw answers nothing.
///
/// [`READ_HOUR`]: crate::prepared_measure::query::READ_HOUR
pub struct Sampler {
    format: replay::format::FormatVersion,
    scan: std::cell::RefCell<(Scan, Option<i32>)>,
}

struct BankedCountry {
    key: String,
    identity: (u32, u32),
    hour: i32,
    banked: [i64; 3],
}
struct BankedBlock(Vec<BankedCountry>);

impl Sampler {
    /// Exact cached folds hold all banks of the selected native holders, so
    /// switching bank or showing all three does not observe their blocks again.
    pub fn cache_keys(requested: &[(String, String)]) -> Vec<(String, String)> {
        let countries: BTreeSet<_> = requested.iter().map(|(key, _)| key.clone()).collect();
        let mut keys: Vec<_> = countries
            .into_iter()
            .flat_map(|key| {
                SAMPLED_MEASURES
                    .into_iter()
                    .chain([crate::prepared_measure::query::READ_HOUR])
                    .map(move |field| (key.clone(), field.to_owned()))
            })
            .collect();
        keys.sort();
        keys
    }
    /// All three banks share one compact observation of a block's first fresh
    /// supplying read. Selection and measure only project this observation.
    pub fn first_block(
        source: &Source,
        block: &replay::container::BlockIndex,
        requested: &[(String, String)],
        cancelled: &AtomicBool,
    ) -> io::Result<crate::prepared::query::progressive::Samples> {
        use crate::prepared::query::{observations, progressive};
        use crate::prepared_measure::query::READ_HOUR;
        let cache = observations::scope(source, "country-experience:first-read", None);
        let selected: Vec<_> = SAMPLE_TABLES.iter().map(|table| table.index()).collect();
        let observation = cache.get(
            block,
            &selected,
            cancelled,
            |value: &Option<BankedBlock>| {
                std::mem::size_of::<Option<BankedBlock>>()
                    + value.as_ref().map_or(0, |value| {
                        value.0.capacity() * std::mem::size_of::<BankedCountry>()
                            + value
                                .0
                                .iter()
                                .map(|country| country.key.capacity())
                                .sum::<usize>()
                    })
            },
            || {
                let replay = source.replay_ref();
                let frames = replay.read_block_selected(block, &[])?;
                let Some(read) = frames.iter().find(|frame| {
                    slot_read(
                        replay.format,
                        &frame.metrics,
                        TableId::CountryExperience,
                        frame.hour,
                    ) && !replay.gaps.iter().any(|gap| {
                        gap.branch == block.branch
                            && gap.first_hour <= frame.hour
                            && frame.hour <= gap.last_hour
                    })
                }) else {
                    return Ok(None);
                };
                let Some(frame) = progressive::frame_at(
                    source,
                    block.branch,
                    read.hour,
                    SAMPLE_TABLES,
                    cancelled,
                )?
                else {
                    return Ok(None);
                };
                let mut countries = Vec::new();
                Scan::default().step(&frame, replay.format, None, |tag, identity, read| {
                    countries.push(BankedCountry {
                        key: {
                            let mut key = serde_json::to_string(&(tag, identity.0, identity.1))?;
                            key.shrink_to_fit();
                            key
                        },
                        identity,
                        hour: read.read_hour,
                        banked: read.banked,
                    });
                    Ok(())
                })?;
                countries.shrink_to_fit();
                Ok(Some(BankedBlock(countries)))
            },
        )?;
        let mut samples = progressive::Samples::new();
        let wanted: BTreeSet<_> = requested.iter().cloned().collect();
        let wanted_countries: BTreeSet<_> = requested.iter().map(|(key, _)| key.as_str()).collect();
        for country in observation
            .as_ref()
            .as_ref()
            .into_iter()
            .flat_map(|block| &block.0)
        {
            for (index, measure) in SAMPLED_MEASURES.iter().enumerate() {
                let key = (country.key.clone(), (*measure).to_owned());
                if wanted.contains(&key) {
                    samples.insert(
                        key,
                        (
                            Some(country.identity),
                            Some(i128::from(country.banked[index])),
                        ),
                    );
                }
            }
            let key = (country.key.clone(), READ_HOUR.to_owned());
            if wanted.contains(&key) || wanted_countries.contains(country.key.as_str()) {
                samples.insert(
                    key,
                    (Some(country.identity), Some(i128::from(country.hour))),
                );
            }
        }
        for key in requested {
            samples.entry(key.clone()).or_insert((None, None));
        }
        Ok(samples)
    }
    pub fn new(format: replay::format::FormatVersion) -> Self {
        Self {
            format,
            scan: std::cell::RefCell::new((Scan::default(), None)),
        }
    }

    /// The banked measures of `requested` (series keys: the JSON of tag,
    /// slot and country id, with a measure) at `frame`'s hour, each with its
    /// country's supplying read hour; every country's when none are requested.
    pub fn sample(
        &self,
        frame: &replay::frame::DecodedFrame,
        requested: Option<&[(String, String)]>,
    ) -> io::Result<crate::prepared::query::progressive::Samples> {
        use crate::prepared_measure::query::READ_HOUR;
        // Each requested key, and its country's read hour, by recorded tag.
        let mut keys = BTreeSet::new();
        for (key, measure) in requested.into_iter().flatten() {
            if measure != READ_HOUR && !SAMPLED_MEASURES.contains(&measure.as_str()) {
                return Err(crate::invalid("Unknown experience sample measure"));
            }
            keys.insert((key.clone(), measure.clone()));
            keys.insert((key.clone(), READ_HOUR.to_owned()));
        }
        let mut wanted: BTreeMap<String, Vec<((u32, u32), String, String)>> = BTreeMap::new();
        for (key, measure) in &keys {
            let (tag, slot, id): (String, u32, u32) = serde_json::from_str(key)
                .map_err(|_| crate::invalid("Invalid experience sample key"))?;
            wanted
                .entry(tag)
                .or_default()
                .push(((slot, id), key.clone(), measure.clone()));
        }
        let mut result = crate::prepared::query::progressive::Samples::new();
        let mut state = self.scan.borrow_mut();
        let (scan, last) = &mut *state;
        if last.and_then(|hour| hour.checked_add(1)) != Some(frame.hour) {
            *scan = Scan::default();
        }
        *last = Some(frame.hour);
        scan.step(frame, self.format, None, |tag, identity, read| {
            let banked = read.values()[0];
            let mut answer = |key: String, measure: &str| {
                let value = match measure {
                    READ_HOUR => Some(i128::from(read.read_hour)),
                    _ => SAMPLED_MEASURES
                        .iter()
                        .position(|sampled| *sampled == measure)
                        .and_then(|index| banked[index]),
                };
                result.insert((key, measure.to_owned()), (Some(identity), value));
            };
            match requested {
                None => {
                    let key = serde_json::to_string(&(tag, identity.0, identity.1))?;
                    for measure in SAMPLED_MEASURES.into_iter().chain([READ_HOUR]) {
                        answer(key.clone(), measure);
                    }
                }
                Some(_) => {
                    for (at, key, measure) in wanted.get(tag).into_iter().flatten() {
                        if *at == identity {
                            answer(key.clone(), measure);
                        }
                    }
                }
            }
            Ok(())
        })?;
        for key in keys {
            result.entry(key).or_insert((None, None));
        }
        Ok(result)
    }
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
