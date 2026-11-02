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
    prepared::query::native,
};
use engine_events::ccountry_experience_status::SCALE;
use replay::container::Replay;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::FrameDecoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryExperience;
/// A change compares reads one day apart, rather than partial-day accumulators.
pub const DAY_HOURS: i32 = 24;
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
    /// This country's preceding day lacks at least one complete hourly interval.
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

impl ExperienceRead {
    /// A read held over `first_hour..=last_hour`, from its banked, gain and change values by
    /// branch as [`RawRead::values`] gives them; None without every banked value.
    pub fn from_values(
        (first_hour, last_hour): (i32, i32),
        read_hour: i32,
        starts_segment: bool,
        [banked, gain, change]: [[Option<i128>; 3]; 3],
    ) -> Option<Self> {
        let branches = |values: [Option<i128>; 3]| {
            Some(ExperienceChange {
                army_raw: values[0]?.to_string(),
                navy_raw: values[1]?.to_string(),
                air_raw: values[2]?.to_string(),
            })
        };
        let banked = branches(banked)?;
        let change = branches(change);
        let gain = branches(gain);
        Some(Self {
            first_hour,
            last_hour,
            read_hour,
            starts_segment,
            army_raw: banked.army_raw,
            navy_raw: banked.navy_raw,
            air_raw: banked.air_raw,
            change_unknown: change.is_none().then_some(ChangeUnknown::NoMatchingRead),
            change,
            gain_unknown: gain.is_none().then_some(GainUnknown::NoCompleteDay),
            gain,
        })
    }
}

impl From<&RawRead> for ExperienceRead {
    fn from(read: &RawRead) -> Self {
        Self::from_values(
            (read.first_hour, read.last_hour),
            read.read_hour,
            read.starts_segment,
            read.values(),
        )
        .expect("a read holds every banked value")
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Seen {
    track: IdentityTrack,
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
                    if [TableId::CountryExperience, TableId::CountryExperienceGains]
                        .into_iter()
                        .any(|table| native::left_to_native(decoded, table))
                    {
                        return Err(io::Error::other(native::unprepared("Experience")));
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

/// What the scan reads of an hour, joined by canonical slot: the countries holding a tag and
/// their rows, from row frames ([`Scan`]) or from a walk's columns.
pub trait Joins {
    /// Each country holding exactly one tag, by its canonical slot and id.
    fn holders(&self) -> &BTreeMap<(u32, u32), Arc<str>>;
    /// The holders at canonical `slot`, ascending.
    fn at(&self, slot: u32) -> &[(u32, u32)];
    /// Army, navy and air banked in `slot`'s one CountryExperience row.
    fn banked(&self, slot: u32) -> Option<[i64; 3]>;
    /// Army, navy and air gained in `slot`'s one CountryExperienceGains row, when that row
    /// holds a complete interval.
    fn gained(&self, slot: u32) -> Option<[i64; 3]>;
}

/// What an hour moved of the joins the scan reads.
pub struct Moved<'a> {
    /// The slots whose rows or holders may have changed, ascending: every one that did.
    pub dirty: &'a [u32],
    /// The tags and countries that stopped holding each other.
    pub removed: &'a [(Arc<str>, (u32, u32))],
    /// The holders were joined anew.
    pub roster: bool,
    /// Every holder is visited and the current outputs reconciled with the joins.
    pub rebuilt: bool,
    /// CountryExperience's daily read is dated at this hour.
    pub read: bool,
}

/// The scan's state from hour to hour, whatever joins feed it.
#[derive(Default, Serialize, Deserialize)]
pub struct Reads {
    #[serde(with = "crate::prepared_production::map_pairs")]
    seen: BTreeMap<(u32, u32), Seen>,
    published: BTreeMap<Arc<str>, ((u32, u32), RawRead)>,
    hour: Option<i32>,
    active: BTreeSet<(Arc<str>, (u32, u32))>,
    present: BTreeSet<(u32, u32)>,
    #[serde(skip)]
    removed: Vec<(Arc<str>, (u32, u32))>,
    #[serde(skip)]
    selected: Vec<(u32, u32)>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    emitted: usize,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    gain_updates: usize,
}

impl Reads {
    /// The outputs the last hour stepped took out: their tag and country.
    pub fn removed(&self) -> &[(Arc<str>, (u32, u32))] {
        &self.removed
    }

    /// Steps `hour`: each read the hour publishes goes to `sink` with its tag and country.
    pub fn step<J: Joins>(
        &mut self,
        joins: &J,
        hour: i32,
        moved: Moved<'_>,
        mut sink: impl FnMut(&Arc<str>, (u32, u32), RawRead) -> io::Result<()>,
    ) -> io::Result<()> {
        let previous = self.hour;
        let adjacent = previous.and_then(|hour| hour.checked_add(1)) == Some(hour);
        self.hour = Some(hour);
        self.removed.clear();
        let force = moved.rebuilt || !adjacent;
        let holders = joins.holders();
        let mut selected = std::mem::take(&mut self.selected);
        selected.clear();
        if force || moved.read {
            selected.extend(holders.keys().copied());
        } else {
            for &slot in moved.dirty {
                selected.extend_from_slice(joins.at(slot));
            }
            selected.sort_unstable();
            selected.dedup();
        }
        for removed in moved.removed {
            if self.active.remove(removed) {
                self.removed.push(removed.clone());
            }
        }
        // Runtime joins are rebuilt after resume. Only current output membership
        // is reconciled then, never the retained historical `published` map.
        if force {
            let gone: Vec<_> = self
                .active
                .iter()
                .filter(|(tag, id)| holders.get(id) != Some(tag))
                .cloned()
                .collect();
            for row in gone {
                self.active.remove(&row);
                self.removed.push(row);
            }
        }
        let visited = self.visit(
            joins,
            &selected,
            (hour, previous, adjacent, force),
            &moved,
            &mut sink,
        );
        self.selected = selected;
        visited?;
        if moved.roster || force {
            self.present = holders.keys().copied().collect();
        }
        Ok(())
    }

    fn visit<J: Joins>(
        &mut self,
        joins: &J,
        selected: &[(u32, u32)],
        (hour, previous, adjacent, force): (i32, Option<i32>, bool, bool),
        moved: &Moved<'_>,
        sink: &mut impl FnMut(&Arc<str>, (u32, u32), RawRead) -> io::Result<()>,
    ) -> io::Result<()> {
        let holders = joins.holders();
        let Self {
            seen,
            published,
            active,
            present,
            removed,
            ..
        } = &mut *self;
        #[cfg(any(test, feature = "test-support"))]
        let (mut emitted, mut gain_updates) = (0, 0);
        for &identity in selected {
            let tag = &holders[&identity];
            let output = (Arc::clone(tag), identity);
            let previous_output = previous.filter(|_| adjacent && active.contains(&output));
            let state = seen.entry(identity).or_default();
            if adjacent
                && present.contains(&identity)
                && let Some(previous) = previous
            {
                state.track.advance(previous);
            }
            let continues = state.track.step(Some(identity), hour);
            if !continues {
                state.gains.clear();
            }
            if let Some(values) = joins.gained(identity.0) {
                #[cfg(any(test, feature = "test-support"))]
                {
                    gain_updates += 1;
                }
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
            let Some(banked) = joins.banked(identity.0) else {
                state.open = false;
                if active.remove(&output) {
                    removed.push(output);
                }
                continue;
            };
            if !moved.read {
                if state.open
                    && let Some(held) = &state.held
                {
                    if force || moved.dirty.binary_search(&identity.0).is_ok() {
                        publish(published, tag, identity, held, hour, previous_output, sink)?;
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            emitted += 1;
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
            publish(published, tag, identity, &held, hour, previous_output, sink)?;
            #[cfg(any(test, feature = "test-support"))]
            {
                emitted += 1;
            }
            state.held = Some(held);
            state.recent.push_back((hour, banked));
            state.open = true;
            active.insert(output);
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.emitted += emitted;
            self.gain_updates += gain_updates;
        }
        Ok(())
    }
}

/// The scan of row frames: the joins of each frame's tables, and the reads they step.
#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    reads: Reads,
    #[serde(skip)]
    native: Native,
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
    country_rows: Option<Vec<replay_data::rows::CountryRow>>,
    links: std::sync::Arc<std::collections::HashMap<u32, u32>>,
    identities: BTreeMap<(u32, u32), Arc<str>>,
    slots: BTreeMap<u32, Vec<(u32, u32)>>,
    removed: Vec<(Arc<str>, (u32, u32))>,
    roster_changed: bool,
    /// The last complete gains read, held as the frame's own view.
    gain_rows: Option<replay_frame::frame::SharedView>,
    /// The raw country ids `links` resolves to each slot.
    link_countries: BTreeMap<u32, Vec<u32>>,
    banked: Option<BTreeMap<u32, Option<replay_data::rows::CountryExperienceRow>>>,
    gains: Option<BTreeMap<u32, Option<replay_data::rows::CountryExperienceGainRow>>>,
    #[cfg(any(test, feature = "test-support"))]
    pub work: crate::prepared::observer_tests::Work,
}

impl Joins for Native {
    fn holders(&self) -> &BTreeMap<(u32, u32), Arc<str>> {
        &self.identities
    }

    fn at(&self, slot: u32) -> &[(u32, u32)] {
        self.slots.get(&slot).map_or(&[][..], Vec::as_slice)
    }

    fn banked(&self, slot: u32) -> Option<[i64; 3]> {
        let row = self.banked.as_ref()?.get(&slot)?.as_ref()?;
        Some([row.army_raw, row.navy_raw, row.air_raw])
    }

    fn gained(&self, slot: u32) -> Option<[i64; 3]> {
        let row = self.gains.as_ref()?.get(&slot)?.as_ref()?;
        (row.complete == 1).then_some([row.army_raw, row.navy_raw, row.air_raw])
    }
}

/// Replace the joins of changed owners only. Every raw alias of an affected
/// slot participates, so duplicate rows leave that slot unknown as in the
/// whole-table join. Returns the changed slots and rows visited.
fn apply_experience_rows<R: Clone + PartialEq>(
    rows: &[R],
    (changed, deleted): (&[replay_data::rows::DeltaRow], &[u128]),
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
        frame: &replay_frame::frame::DecodedFrame,
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
                        .map(|(id, canonical_slot, tag)| replay_data::rows::CountryRow {
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
                        .or_insert(Some(Arc::<str>::from(tag.as_str())));
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
                replay_frame::frame::TableView::Complete(rows) => {
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
        let mut work = self.native.work;
        work.emitted = self.reads.emitted;
        work
    }

    /// Gain runs visited at changed inputs or banked reads.
    #[cfg(any(test, feature = "test-support"))]
    pub fn gain_updates(&self) -> usize {
        self.reads.gain_updates
    }

    pub fn step(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        format: replay_data::format::FormatVersion,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
        mut sink: impl FnMut(&str, (u32, u32), RawRead) -> io::Result<()>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("experience:observe");
        let dirty: Vec<u32> = self.native.refresh(decoded, changes).into_iter().collect();
        let read = changes.map_or_else(
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
        let rebuilt = changes.is_none_or(|changes| {
            changes.rows_changed[TableId::Countries.index()]
                && changes.rows_changed[TableId::CountryLinks.index()]
        });
        let Self { reads, native } = self;
        reads.step(
            &*native,
            decoded.hour,
            Moved {
                dirty: &dirty,
                removed: &native.removed,
                roster: native.roster_changed,
                rebuilt,
                read,
            },
            |tag, identity, read| sink(&**tag, identity, read),
        )
    }
}

fn publish(
    published: &mut BTreeMap<Arc<str>, ((u32, u32), RawRead)>,
    tag: &Arc<str>,
    identity: (u32, u32),
    held: &RawRead,
    hour: i32,
    previous_hour: Option<i32>,
    sink: &mut impl FnMut(&Arc<str>, (u32, u32), RawRead) -> io::Result<()>,
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
    published.insert(Arc::clone(tag), (identity, read));
    Ok(())
}

#[derive(Default)]
struct Cache {
    entries: BTreeMap<(u64, String), Vec<RawRead>>,
}

impl Product for CountryExperience {
    const KEY: &'static str = "country-experience";
    const CONTENT_DEPENDENT: bool = false;
    const SERVED: Option<&'static crate::prepared::served::Plan> = Some(&served::PLAN);
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

pub mod served;
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
