//! Every country's recorded agency, without a player/AI admission gate.
//!
//! The agency screen shows recorded active and maximum counts, rather than
//! the slowly growing recruitment admission ceiling. The capture's
//! `unknown_f0` preserves the computed maximum bits. Defence uses the
//! recorded value and the AGENCY_DEFENSE_LEVEL template's fixed5 LEVEL argument.
//!
//! Agencies are hourly. Equal consecutive reads compress, but missing rows,
//! failed tables, gaps and identity changes end a run. Membership counts have
//! their own daily read date and never replace the screen's recorded active count.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::HourSpan,
    country_history::{self, Identity, RecordedHistories, observe_identity, unique_by_slot},
    operative_rows::membership_read,
    validate_tag,
};
use crate::{
    Source,
    catalogue::plain_text,
    metadata::{IdentityTrack, link_slot},
};
use replay::{
    container::Replay,
    rows::{IntelligenceAgencyRow, OperativeMembershipRow, TableId, tables},
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

#[cfg(test)]
use crate::metadata::validate_campaign;
#[cfg(test)]
use replay::frame::FrameDecoder;
#[cfg(test)]
use std::collections::VecDeque;

pub(crate) mod prepared;

#[cfg(test)]
mod oracle;
#[cfg(test)]
use oracle::scan;

pub struct AgencyComparison;

#[cfg(test)]
const CACHED_BRANCHES: usize = 2;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum AgencyRankField {
    Defense,
    Active,
    Maximum,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyTop {
    pub count: u32,
    pub field: AgencyRankField,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyComparisonParams {
    pub branch_id: String,
    /// Empty means all countries; tags are never restricted by human status.
    pub tags: Vec<String>,
    #[serde(default)]
    pub include_uncreated: bool,
    pub top: Option<AgencyTop>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyUpgradeItem {
    /// This read's array position, preserving repeated definitions and all levels
    /// in the recorded upgrade list.
    pub ordinal: u32,
    pub definition_pointer: String,
    pub branch_pointer: String,
    /// The upgrade's branch and definition have separate lexical tokens.
    /// A token alone is ambiguous.
    pub branch_token: Option<String>,
    pub token: Option<String>,
    /// Raw lexical IDs survive a missing recorded token-name dictionary.
    pub branch_token_id: Option<u32>,
    pub token_id: Option<u32>,
    pub level: i32,
    /// Capture authentication status; refused entries retain their raw values.
    pub identity_status: u8,
    pub identity_refusal: Option<String>,
    pub name: Option<String>,
    pub content_reason: Option<String>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyUpgrades {
    pub read_hour: Option<i32>,
    /// A failed or incomplete acquired-list read is distinct from absent capture.
    pub unavailable: Option<String>,
    /// Mathematical sum of max(0, signed level), unknown for an incomplete or
    /// unauthenticated list. This inventory sum omits the maximum-count formula's
    /// created gate, i32 wrapping, base and slot calculation.
    pub nonnegative_level_sum: Option<String>,
    pub items: Vec<AgencyUpgradeItem>,
}

/// [`AgencyUpgrades`] as scanned. A run whose acquired list did not change
/// shares the previous run's items instead of copying them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ScannedUpgrades {
    read_hour: Option<i32>,
    unavailable: Option<String>,
    nonnegative_level_sum: Option<String>,
    items: Arc<[AgencyUpgradeItem]>,
}

/// Acquired entries are separate from elapsed creation/upgrade work.
fn upgrades_at_read(row: &AgencyProjection) -> AgencyUpgrades {
    row.upgrades.as_ref().map_or(
        AgencyUpgrades {
            read_hour: None,
            unavailable: None,
            nonnegative_level_sum: None,
            items: Vec::new(),
        },
        |read| AgencyUpgrades {
            read_hour: read.read_hour,
            unavailable: read.unavailable.clone(),
            nonnegative_level_sum: read.nonnegative_level_sum.clone(),
            items: read.items.to_vec(),
        },
    )
}

fn acquired_read(
    frame: &replay::frame::DecodedFrame,
    rows: &[&replay::rows::AgencyUpgradeLevelRow],
    unavailable: Option<String>,
) -> AgencyUpgrades {
    let mut read = AgencyUpgrades {
        read_hour: Some(frame.hour),
        unavailable: None,
        nonnegative_level_sum: None,
        items: Vec::new(),
    };
    read.unavailable = unavailable;
    for row in rows {
        let raw = super::intelligence_capture::upgrade(frame, row);
        if raw.identity_reason.is_some() {
            read.unavailable.get_or_insert_with(|| {
                "Acquired agency levels contain unauthenticated identities".into()
            });
        }
        read.items.push(AgencyUpgradeItem {
            ordinal: raw.ordinal,
            definition_pointer: raw.definition_pointer,
            branch_pointer: raw.branch_pointer,
            branch_token: raw.branch_literal,
            token: raw.upgrade_literal,
            branch_token_id: raw.branch_token,
            token_id: raw.upgrade_token,
            level: raw.level,
            identity_status: raw.identity_status,
            identity_refusal: raw.identity_reason,
            name: None,
            content_reason: None,
            icon: None,
        });
    }
    read.items.sort_by_key(|item| item.ordinal);
    if read
        .items
        .iter()
        .enumerate()
        .any(|(ordinal, item)| item.ordinal as usize != ordinal)
    {
        read.unavailable
            .get_or_insert_with(|| "Acquired agency list has missing or repeated ordinals".into());
    }
    if read.unavailable.is_none() {
        let sum: u128 = read
            .items
            .iter()
            .map(|item| item.level.max(0) as u128)
            .sum();
        read.nonnegative_level_sum = Some(sum.to_string());
    }
    read
}

fn upgrades_unrecorded() -> Option<String> {
    Some("Not recorded: this recording has no acquired agency upgrade levels; upgrade progress measures work on the current creation or upgrade.".into())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyValue {
    pub created: bool,
    /// Created names are literal recorded bytes, including an empty name.
    /// Uncreated names use the content-localised default.
    pub name: Option<String>,
    pub defense_raw: String,
    /// The full native content-localised text, retained for the tooltip.
    pub defense_text: Option<String>,
    /// Only LEVEL's complete formatted output.
    pub defense_value: Option<String>,
    pub active_count: i32,
    pub maximum: i32,
    pub upgrades: AgencyUpgrades,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyEntry {
    pub first_hour: i32,
    pub last_hour: i32,
    /// First supplying read; every hour in this run independently reconfirms
    /// the agency scalars. The operative list retains its separate daily date.
    pub read_hour: i32,
    pub value: AgencyValue,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencySeries {
    pub tag: String,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    pub entries: Vec<AgencyEntry>,
    pub rosters: Vec<AgencyRosterRun>,
}

/// Active-list counts carry independently from agency-screen scalars. Their
/// latest supplying read is in the reply's operative_read_hours, not repeated
/// for each country or incorporated into the agency-screen run identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyRosterRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Supplying census at this run's start; an alias may start between reads.
    pub read_hour: i32,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyComparisonReply {
    pub branch_id: String,
    /// False only when the branch contains no readable agency table at all.
    pub available: bool,
    /// Complete country and agency tables at these hours. A missing country
    /// row inside them is still missing, never a zero-valued agency.
    pub observed: Vec<HourSpan>,
    pub operative_read_hours: Vec<i32>,
    pub series: Vec<AgencySeries>,
    /// Label from the screen's AGENCY_DEFENSE_LEVEL template.
    pub defense_label: Option<String>,
    /// One recording-level reason, rather than repeating it at every agency read.
    pub upgrades_unrecorded: Option<String>,
    pub warnings: Vec<String>,
}

/// Only agency-screen values participate in run identity. Daily updates change
/// work progress and recruitment admission independently; neither appears in
/// the active/maximum display and neither is stored here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct AgencyProjection {
    created: bool,
    name: Vec<u8>,
    defense_raw: i64,
    active_count: i32,
    unknown_f0: u32,
    upgrades: Option<ScannedUpgrades>,
}

impl From<&IntelligenceAgencyRow> for AgencyProjection {
    fn from(row: &IntelligenceAgencyRow) -> Self {
        let created = row.created != 0;
        Self {
            created,
            // The content default is used until the agency is created.
            name: if created {
                row.name.clone()
            } else {
                Vec::new()
            },
            defense_raw: row.defense_raw,
            active_count: row.active_count,
            unknown_f0: row.unknown_f0,
            upgrades: None,
        }
    }
}

impl AgencyProjection {
    fn matches(&self, row: &IntelligenceAgencyRow) -> bool {
        let created = row.created != 0;
        self.created == created
            && (!created || self.name == row.name)
            && self.defense_raw == row.defense_raw
            && self.active_count == row.active_count
            && self.unknown_f0 == row.unknown_f0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RawEntry {
    first_hour: i32,
    last_hour: i32,
    read_hour: i32,
    row: AgencyProjection,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Track {
    entries: Vec<RawEntry>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    last_value: Option<(Identity, i32)>,
    roster: Option<(Identity, i32, u32)>,
    rosters: Vec<AgencyRosterRun>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct HeldRoster {
    hour: i32,
    read_hour: i32,
    count: u32,
}

#[derive(Default, Serialize, Deserialize)]
struct ScannedAgencies {
    tracks: BTreeMap<String, Track>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    held_rosters: HashMap<Identity, HeldRoster>,
    available: bool,
    observed: Vec<HourSpan>,
    operative_read_hours: Vec<i32>,
    unresolved_rows: usize,
    untext_tags: usize,
    unsupported_rosters: BTreeMap<u8, usize>,
    warnings: BTreeMap<String, usize>,
    #[cfg(test)]
    #[serde(skip)]
    native_countries: super::operative_rows::CountryInputs,
    #[serde(skip)]
    agency_slots: Option<(HashMap<u32, Option<usize>>, usize)>,
    #[serde(skip)]
    acquired: Option<(HashMap<u32, ScannedUpgrades>, ScannedUpgrades)>,
}

impl From<AgencyUpgrades> for ScannedUpgrades {
    fn from(read: AgencyUpgrades) -> Self {
        Self {
            read_hour: read.read_hour,
            unavailable: read.unavailable,
            nonnegative_level_sum: read.nonnegative_level_sum,
            items: read.items.into(),
        }
    }
}

impl ScannedAgencies {
    fn observe_upgrades(&mut self, hour: i32, tag: &str, upgrades: impl Into<ScannedUpgrades>) {
        let upgrades = upgrades.into();
        let Some(track) = self.tracks.get_mut(tag) else {
            return;
        };
        let Some((entry, earlier)) = track
            .entries
            .split_last_mut()
            .filter(|(entry, _)| entry.last_hour == hour)
        else {
            return;
        };
        if entry.row.upgrades.as_ref().is_some_and(|previous| {
            previous.unavailable == upgrades.unavailable
                && previous.nonnegative_level_sum == upgrades.nonnegative_level_sum
                && (Arc::ptr_eq(&previous.items, &upgrades.items)
                    || previous.items == upgrades.items)
        }) {
            return;
        }
        // A new scalar run usually keeps its predecessor's list: share it.
        let items = match entry
            .row
            .upgrades
            .as_ref()
            .or_else(|| earlier.last()?.row.upgrades.as_ref())
        {
            Some(previous)
                if Arc::ptr_eq(&previous.items, &upgrades.items)
                    || previous.items == upgrades.items =>
            {
                Arc::clone(&previous.items)
            }
            _ => upgrades.items,
        };
        let upgrades = ScannedUpgrades {
            read_hour: upgrades.read_hour,
            unavailable: upgrades.unavailable,
            nonnegative_level_sum: upgrades.nonnegative_level_sum,
            items,
        };
        if entry.first_hour == hour {
            entry.row.upgrades = Some(upgrades);
        } else {
            let mut next = entry.clone();
            entry.last_hour = hour - 1;
            next.first_hour = hour;
            next.last_hour = hour;
            next.read_hour = hour;
            next.row.upgrades = Some(upgrades);
            track.entries.push(next);
        }
    }

    fn note_roster_read(&mut self, hour: i32) {
        if self.operative_read_hours.last() != Some(&hour) {
            self.operative_read_hours.push(hour);
        }
    }

    fn observe(
        &mut self,
        hour: i32,
        tag: &str,
        identity: Identity,
        row: Option<&IntelligenceAgencyRow>,
        roster: Option<Option<u32>>,
    ) {
        let held = match roster {
            Some(value) => value.map(|count| HeldRoster {
                hour,
                read_hour: hour,
                count,
            }),
            None => self.held_rosters.get(&identity).copied().and_then(|held| {
                (held.hour.checked_add(1) == Some(hour)).then_some(HeldRoster { hour, ..held })
            }),
        };
        if let Some(held) = held {
            self.held_rosters.insert(identity, held);
        } else {
            self.held_rosters.remove(&identity);
        }
        // Avoid allocating the owned tag on every country-hour.
        let track = match self.tracks.get_mut(tag) {
            Some(track) => track,
            None => self.tracks.entry(tag.into()).or_default(),
        };
        if !observe_identity(
            &mut track.segments,
            &mut track.last_identity,
            identity,
            hour,
        ) {
            track.roster = None;
        }
        let previous_roster = track.roster;
        track.roster = held.map(|held| (identity, hour, held.count));
        if let Some(held) = held {
            let count = held.count;
            let follows = previous_roster.is_some_and(|(seen, at, previous)| {
                seen == identity && at.checked_add(1) == Some(hour) && previous == count
            });
            if follows {
                track.rosters.last_mut().expect("previous roster").last_hour = hour;
            } else {
                track.rosters.push(AgencyRosterRun {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour: held.read_hour,
                    count,
                });
            }
        }
        let Some(row) = row else {
            track.last_value = None;
            return;
        };
        let follows = track
            .last_value
            .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
        let equal = follows
            && track
                .entries
                .last()
                .is_some_and(|entry| entry.row.matches(row));
        if equal {
            track.entries.last_mut().expect("previous value").last_hour = hour;
        } else {
            let row = AgencyProjection::from(row);
            track.entries.push(RawEntry {
                first_hour: hour,
                last_hour: hour,
                read_hour: hour,
                row,
            });
        }
        track.last_value = Some((identity, hour));
    }
}

/// Scalar active_count is the active-list count. Recruitment pools are
/// separate memberships: roster_kind 0 is that active list, not all entries an
/// operative can have. The recruitment vectors are projected as kinds 1..3
/// by engine::coperative_leader::walk. Capture retains active membership, so a
/// captured operative still counts in the active/maximum display;
/// free-operatives is different.
fn roster_counts(
    rows: &[OperativeMembershipRow],
    links: &[replay::rows::CountryLinkRow],
) -> (HashMap<u32, u32>, usize, BTreeMap<u8, usize>) {
    let mut counts = HashMap::new();
    let mut unresolved = 0;
    let mut unsupported = BTreeMap::new();
    for row in rows {
        match row.roster_kind {
            0 => {}
            1..=3 => continue,
            other => {
                *unsupported.entry(other).or_default() += 1;
                unresolved += 1;
                continue;
            }
        }
        match link_slot(links, row.roster_country) {
            Some(slot) => {
                *counts.entry(slot).or_insert(0) += 1;
            }
            None => unresolved += 1,
        }
    }
    (counts, unresolved, unsupported)
}

impl ScannedAgencies {
    #[allow(clippy::type_complexity)]
    fn observe_frame_countries(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        replay: &Replay,
        inputs: &super::operative_rows::CountryInputs,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<(BTreeMap<String, Identity>, usize, BTreeMap<u8, usize>)> {
        let mut active = BTreeMap::new();
        let mut roster_unresolved = 0;
        let mut roster_unsupported = BTreeMap::new();
        let roster_read = membership_read(frame);
        let memberships = frame.complete::<tables::OperativeMemberships>().ok();
        if roster_read && memberships.is_some() {
            self.note_roster_read(frame.hour);
        }
        let Some((countries, links)) = inputs.complete() else {
            return Ok((active, roster_unresolved, roster_unsupported));
        };
        self.held_rosters
            .retain(|_, held| held.hour.checked_add(1) == Some(frame.hour));
        let rows = frame.complete::<tables::IntelligenceAgencies>().ok();
        self.available |= rows.is_some();
        if rows.is_some() {
            match self.observed.last_mut() {
                Some(span) if span.last_hour.checked_add(1) == Some(frame.hour) => {
                    span.last_hour = frame.hour
                }
                _ => self.observed.push(HourSpan {
                    first_hour: frame.hour,
                    last_hour: frame.hour,
                }),
            }
        }
        if self.agency_slots.is_none()
            || inputs.changed
            || changes
                .is_none_or(|changes| changes.rows_changed[TableId::IntelligenceAgencies.index()])
        {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| {
                work.row_visits += rows.map_or(0, |rows| rows.len())
            });
            let by_slot = unique_by_slot(rows.into_iter().flatten(), links, |row| row.country);
            let slots = by_slot
                .rows
                .into_iter()
                .map(|(slot, row)| {
                    (
                        slot,
                        row.map(|row| {
                            rows.expect("complete agency rows")
                                .binary_search_by_key(&row.country, |row| row.country)
                                .expect("agency native row key")
                        }),
                    )
                })
                .collect();
            self.agency_slots = Some((
                slots,
                by_slot.unresolved_rows + by_slot.duplicate_rows + by_slot.ambiguous_slots,
            ));
        }
        self.unresolved_rows += self.agency_slots.as_ref().expect("agency current slots").1;
        let roster = if roster_read {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| {
                work.row_visits += memberships.map_or(0, |rows| rows.len())
            });
            memberships.map(|rows| roster_counts(rows, links))
        } else {
            None
        };
        if let Some((_, unresolved, unsupported)) = &roster {
            self.unresolved_rows += unresolved;
            roster_unresolved = *unresolved;
            roster_unsupported = unsupported.clone();
            for (kind, count) in unsupported {
                *self.unsupported_rosters.entry(*kind).or_default() += count;
            }
        }
        if self.acquired.is_none()
            || inputs.changed
            || changes.is_none_or(|changes| {
                [TableId::AgencyUpgradeLevels, TableId::LexicalTokens]
                    .iter()
                    .any(|table| changes.rows_changed[table.index()])
            })
        {
            let mut acquired: HashMap<u32, Vec<&replay::rows::AgencyUpgradeLevelRow>> =
                HashMap::new();
            let mut acquired_unavailable = frame
                .complete::<tables::AgencyUpgradeLevels>()
                .err()
                .map(|error| {
                    format!("Acquired agency levels incomplete or unavailable: {error:?}")
                });
            let levels = super::recorded_rows::<tables::AgencyUpgradeLevels>(frame)?;
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.row_visits += levels.len());
            for row in levels {
                if let Some(slot) = link_slot(links, row.country) {
                    acquired.entry(slot).or_default().push(row);
                } else {
                    acquired_unavailable.get_or_insert_with(|| {
                        "Acquired agency entry has an unresolved country link".into()
                    });
                }
            }
            let project = |rows: &[&replay::rows::AgencyUpgradeLevelRow]| {
                let read = acquired_read(frame, rows, acquired_unavailable.clone());
                ScannedUpgrades {
                    read_hour: read.read_hour,
                    unavailable: read.unavailable,
                    nonnegative_level_sum: read.nonnegative_level_sum,
                    items: read.items.into(),
                }
            };
            let default = project(&[]);
            let acquired = acquired
                .into_iter()
                .map(|(slot, rows)| (slot, project(&rows)))
                .collect();
            self.acquired = Some((acquired, default));
        }
        for (tag, lookup) in &inputs.tagged {
            let Some((index, slot)) = *lookup else {
                continue;
            };
            let country = &countries[index];
            let Ok(tag) = std::str::from_utf8(tag) else {
                self.untext_tags += 1;
                continue;
            };
            let known_identity = inputs.aliases.get(tag.as_bytes()) == Some(&1)
                && inputs
                    .slots
                    .get(&slot)
                    .is_some_and(|row| row.id == country.id);
            let count = if !known_identity || memberships.is_none() {
                Some(None)
            } else if let Some((counts, unresolved, _)) = &roster {
                // A complete world roster proves an empty active list, except
                // when a nonempty raw country reference could name this slot.
                Some((*unresolved == 0).then(|| counts.get(&slot).copied().unwrap_or(0)))
            } else {
                None
            };
            active.insert(tag.to_owned(), (slot, country.id));
            self.observe(
                frame.hour,
                tag,
                (slot, country.id),
                self.agency_slots
                    .as_ref()
                    .expect("agency current slots")
                    .0
                    .get(&slot)
                    .copied()
                    .flatten()
                    .map(|index| &rows.expect("complete agency rows")[index]),
                count,
            );
            if TableId::AgencyUpgradeLevels
                .last_change()
                .present(replay.format.version())
            {
                let (acquired, default) = self.acquired.as_ref().expect("current acquired levels");
                let mut read = acquired.get(&slot).unwrap_or(default).clone();
                read.read_hour = Some(frame.hour);
                self.observe_upgrades(frame.hour, tag, read);
            }
        }
        Ok((active, roster_unresolved, roster_unsupported))
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Agency comparison request cancelled",
        ))
    } else {
        Ok(())
    }
}

/// Created agencies keep even an empty stored name; otherwise the content's
/// AGENCY_NAME key supplies the name.
fn agency_name(row: &AgencyProjection, default: impl FnOnce() -> Option<String>) -> Option<String> {
    if row.created {
        std::str::from_utf8(&row.name)
            .ok()
            .and_then(|name| plain_text(name).ok())
    } else {
        default()
    }
}

fn value(
    entry: &RawEntry,
    default: impl FnOnce() -> Option<String>,
    defense: impl FnOnce(i64) -> (Option<String>, Option<String>),
) -> AgencyValue {
    let row = &entry.row;
    let (defense_text, defense_value) = defense(row.defense_raw);
    AgencyValue {
        created: row.created,
        name: agency_name(row, default),
        defense_raw: row.defense_raw.to_string(),
        defense_text,
        defense_value,
        active_count: row.active_count,
        // The recorded bits are displayed as a signed integer.
        maximum: row.unknown_f0 as i32,
        upgrades: upgrades_at_read(row),
    }
}

fn rank(entry: &RawEntry, field: AgencyRankField) -> i64 {
    match field {
        AgencyRankField::Defense => entry.row.defense_raw,
        AgencyRankField::Active => i64::from(entry.row.active_count),
        AgencyRankField::Maximum => i64::from(entry.row.unknown_f0 as i32),
    }
}

#[cfg(test)]
#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, Arc<ScannedAgencies>)>,
}

#[cfg(test)]
fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedAgencies>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, result)) = scans.entries.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(result));
    }
    let result = Arc::new(scan(&source.replay(), branch, cancelled)?);
    scans.entries.push_back((branch, Arc::clone(&result)));
    while scans.entries.len() > CACHED_BRANCHES {
        scans.entries.pop_front();
    }
    Ok(result)
}

#[cfg(test)]
fn requested_series(
    scanned: &ScannedAgencies,
    params: &AgencyComparisonParams,
    default: impl Fn() -> Option<String>,
    defense: impl Fn(i64) -> (Option<String>, Option<String>),
    cancelled: &AtomicBool,
) -> io::Result<Vec<AgencySeries>> {
    project_series(scanned, params, None, default, defense, cancelled)
}

fn project_series(
    scanned: &ScannedAgencies,
    params: &AgencyComparisonParams,
    selection: Option<(&RecordedHistories, &[String])>,
    default: impl Fn() -> Option<String>,
    defense: impl Fn(i64) -> (Option<String>, Option<String>),
    cancelled: &AtomicBool,
) -> io::Result<Vec<AgencySeries>> {
    let tags: BTreeSet<_> = params.tags.iter().collect();
    let candidates: BTreeMap<String, Vec<&RawEntry>> = scanned
        .tracks
        .iter()
        .filter(|(tag, _)| tags.is_empty() || tags.contains(tag))
        .map(|(tag, track)| {
            (
                tag.clone(),
                track
                    .entries
                    .iter()
                    .filter(|entry| params.include_uncreated || entry.row.created)
                    .collect(),
            )
        })
        .filter(|(_, entries): &(_, Vec<&RawEntry>)| !entries.is_empty())
        .collect();
    let memberships = params
        .top
        .as_ref()
        .map(|top| {
            crate::ranking::top_membership(
                &candidates,
                top.count as usize,
                |entry| (entry.first_hour, entry.last_hour),
                |entry| Some(rank(entry, top.field)),
                cancelled,
            )
        })
        .transpose()?;
    let mut result = Vec::new();
    // The content default is resolved only for uncreated agencies.
    let default_name = std::cell::LazyCell::new(default);
    let mut defense_texts = HashMap::new();
    let selected =
        selection.map(|(history, tags)| country_history::selected_identities(history, tags));
    for (tag, entries) in candidates {
        check_cancelled(cancelled)?;
        let membership = memberships.as_ref().and_then(|all| all.get(&tag)).cloned();
        if memberships.is_some() && membership.is_none() {
            continue;
        }
        let mut series = AgencySeries {
            segments: scanned.tracks[&tag].segments.clone(),
            rosters: scanned.tracks[&tag].rosters.clone(),
            tag,
            membership,
            entries: Vec::new(),
        };
        if let Some((history, _)) = selection {
            let spans: Vec<_> = history
                .countries
                .get(&series.tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| {
                    selected
                        .as_ref()
                        .unwrap()
                        .contains(&(identity.slot, identity.country_id))
                })
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect();
            if spans.is_empty() {
                continue;
            }
            let bounds = |span: &HourSpan| (span.first_hour, span.last_hour);
            let set_bounds = |span: &mut HourSpan, first, last| {
                span.first_hour = first;
                span.last_hour = last;
            };
            series.membership = Some(match &series.membership {
                Some(membership) => clip_runs(membership, &spans, bounds, set_bounds, cancelled)?,
                None => spans.clone(),
            });
            series.segments = clip_runs(&series.segments, &spans, bounds, set_bounds, cancelled)?;
            series.rosters = clip_runs(
                &series.rosters,
                &spans,
                |run| (run.first_hour, run.last_hour),
                |run, first, last| {
                    run.first_hour = first;
                    run.last_hour = last;
                },
                cancelled,
            )?;
        }
        let mut append = |entry: &&RawEntry, first_hour, last_hour| -> io::Result<()> {
            let item = AgencyEntry {
                first_hour,
                last_hour,
                read_hour: entry.read_hour,
                value: value(
                    entry,
                    || (*default_name).clone(),
                    |_| {
                        defense_texts
                            .entry(entry.row.defense_raw)
                            .or_insert_with(|| defense(entry.row.defense_raw))
                            .clone()
                    },
                ),
            };
            series.entries.push(item);
            Ok(())
        };
        if let Some(spans) = &series.membership {
            crate::ranking::for_each_piece(
                &entries,
                spans,
                |entry| (entry.first_hour, entry.last_hour),
                cancelled,
                &mut append,
            )?;
        } else {
            for entry in &entries {
                append(entry, entry.first_hour, entry.last_hour)?;
            }
        }
        result.push(series);
    }
    Ok(result)
}

fn measured(value: &impl Serialize) -> io::Result<usize> {
    crate::transport::json_len(value, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        io::Error::other(
            "Agency comparison exceeds one complete reply's budget; no countries were omitted.",
        )
    })
}

fn clip_runs<T: Clone>(
    runs: &[T],
    spans: &[HourSpan],
    bounds: impl Fn(&T) -> (i32, i32),
    set_bounds: impl Fn(&mut T, i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Vec<T>> {
    let mut clipped = Vec::new();
    crate::ranking::for_each_piece(runs, spans, bounds, cancelled, |run, first, last| {
        let mut run = run.clone();
        set_bounds(&mut run, first, last);
        clipped.push(run);
        Ok(())
    })?;
    Ok(clipped)
}

fn reply(
    source: &Source,
    params: &AgencyComparisonParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scanned: &ScannedAgencies,
    history: Option<&RecordedHistories>,
) -> io::Result<AgencyComparisonReply> {
    let mut projection = params.clone();
    if let Some(history) = &history {
        projection.tags = country_history::selected_aliases(history, &params.tags);
        if projection.tags.is_empty() {
            projection.tags.clone_from(&params.tags);
        }
    }
    let mut warnings: Vec<_> = scanned
        .warnings
        .iter()
        .map(|(warning, count)| format!("{warning} ({count})"))
        .collect();
    let content = match context
        .content()
        .and_then(|options| super::intelligence_content::agency_content(source, options))
    {
        Ok(content) => Some(content),
        Err(error) => {
            warnings.push(format!("Agency localisation is unavailable: {error}"));
            None
        }
    };
    if scanned.unresolved_rows > 0 {
        warnings.push(format!("{} agency or operative membership rows do not name one resolved country; their values are unknown.", scanned.unresolved_rows));
    }
    if scanned.untext_tags > 0 {
        warnings.push(format!(
            "{} country-hours have non-text tags; they cannot be named.",
            scanned.untext_tags
        ));
    }
    for (kind, count) in &scanned.unsupported_rosters {
        warnings.push(format!("{count} operative memberships use unsupported roster_kind {kind}; their roster counts are unknown."));
    }
    let mut series = project_series(
        scanned,
        &projection,
        history.map(|history| (history, params.tags.as_slice())),
        || {
            content
                .as_ref()
                .and_then(|content| content.default_agency_name())
        },
        |raw| {
            let text = content
                .as_ref()
                .and_then(|content| content.defense_text(raw));
            let value = content
                .as_ref()
                .zip(text.as_deref())
                .and_then(|(content, text)| content.defense_value(text));
            (text, value)
        },
        cancelled,
    )?;
    for item in series
        .iter_mut()
        .flat_map(|series| &mut series.entries)
        .flat_map(|entry| &mut entry.value.upgrades.items)
    {
        let name = match (
            content.as_ref(),
            item.branch_token.as_deref(),
            item.token.as_deref(),
        ) {
            (_, _, _) if item.identity_refusal.is_some() => {
                Err("Acquired upgrade identity refused".into())
            }
            (Some(content), Some(branch), Some(token)) => content.upgrade_name(branch, token),
            (None, _, _) => Err("Acquired upgrade content unavailable".into()),
            _ => Err("Acquired upgrade branch or definition lexical name not recorded".into()),
        };
        match name {
            Ok(name) => item.name = Some(name),
            Err(reason) => item.content_reason = Some(reason),
        }
    }
    let mut upgrade_refusals = BTreeMap::new();
    for item in series
        .iter()
        .flat_map(|series| &series.entries)
        .flat_map(|entry| &entry.value.upgrades.items)
    {
        for reason in [&item.identity_refusal, &item.content_reason]
            .into_iter()
            .flatten()
        {
            *upgrade_refusals.entry(reason).or_insert(0_usize) += 1;
        }
    }
    warnings.extend(
        upgrade_refusals
            .into_iter()
            .map(|(reason, count)| format!("{reason} ({count} acquired-upgrade observations).")),
    );
    let unnamed = series
        .iter()
        .flat_map(|series| &series.entries)
        .filter(|entry| entry.value.created && entry.value.name.is_none())
        .count();
    if unnamed > 0 {
        warnings.push(format!("{unnamed} agency name runs have unsupported non-text bytes or controls; their names are unknown."));
    }
    let defense_label = content.as_ref().and_then(|content| content.defense_label());
    if let Some(content) = content {
        warnings.extend(content.warnings.iter().cloned());
        warnings.extend(content.agency_refusal_lines());
    }
    let reply = AgencyComparisonReply {
        branch_id: params.branch_id.clone(),
        available: scanned.available,
        observed: scanned.observed.clone(),
        operative_read_hours: scanned.operative_read_hours.clone(),
        series,
        defense_label,
        upgrades_unrecorded: (!TableId::AgencyUpgradeLevels
            .last_change()
            .present(source.replay().format.version()))
        .then(upgrades_unrecorded)
        .flatten(),
        warnings,
    };
    measured(&reply)?;
    Ok(reply)
}

impl Product for AgencyComparison {
    const KEY: &'static str = "agency-comparison";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AgencyComparisonParams;
    type Reply = AgencyComparisonReply;

    fn types() -> Vec<String> {
        vec![
            AgencyComparisonParams::decl(),
            AgencyRankField::decl(),
            AgencyTop::decl(),
            AgencyUpgradeItem::decl(),
            AgencyUpgrades::decl(),
            AgencyValue::decl(),
            AgencyEntry::decl(),
            AgencySeries::decl(),
            AgencyRosterRun::decl(),
            AgencyComparisonReply::decl(),
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
            .as_ref()
            .is_some_and(|top| !params.tags.is_empty() || top.count == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        let history = (!params.tags.is_empty())
            .then(|| country_history::branch_histories(source, branch, false, &[], cancelled))
            .transpose()?;
        let scanned = scanned_branch(source, branch, cancelled)?;
        reply(
            source,
            params,
            context,
            cancelled,
            &scanned,
            history.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(country: u32, defense_raw: i64) -> IntelligenceAgencyRow {
        IntelligenceAgencyRow {
            country,
            defense_raw,
            created: 1,
            name: b"Recorded agency".to_vec(),
            active_count: 2,
            unknown_f0: 7,
            admission_ceiling: 3,
            ..Default::default()
        }
    }

    fn params() -> AgencyComparisonParams {
        AgencyComparisonParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            include_uncreated: false,
            top: None,
        }
    }

    #[test]
    fn agency_comparison_manual_picks_follow_all_holders_and_exclude_unrelated_aliases() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        let frames = [
            (0, ["PRE", "OTHER", "C"]),
            (1, ["X", "OTHER", "C"]),
            (2, ["Y", "X", "C"]),
            (4, ["Y", "X", "C"]),
            (5, ["Z", "X", "Y"]),
        ]
        .into_iter()
        .map(|(offset, tags)| {
            let mut frame = source(1, 16 + offset, Some(1));
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                [7, 9, 11]
                    .into_iter()
                    .zip(tags)
                    .enumerate()
                    .map(|(index, (id, tag))| CountryRow {
                        id,
                        canonical_slot: index as u32 + 1,
                        tag: tag.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    [7, 9, 11]
                        .into_iter()
                        .enumerate()
                        .map(|(index, id)| CountryLinkRow {
                            id,
                            canonical_slot: index as u32 + 1,
                        })
                        .collect(),
                ));
            *frame.tables[TableId::IntelligenceAgencies.index()] =
                TableView::Complete(TableRows::IntelligenceAgencies(
                    [7, 9, 11]
                        .into_iter()
                        .map(|id| row(id, i64::from(id)))
                        .collect(),
                ));
            frame
        })
        .collect();
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "holders".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
            &cancelled,
        )
        .unwrap();
        let mut request = params();
        request.tags = vec!["X".into()];
        let reply = AgencyComparison::load(
            &source,
            &request,
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(
            reply
                .series
                .iter()
                .map(|series| series.tag.as_str())
                .collect::<Vec<_>>(),
            ["OTHER", "PRE", "X", "Y", "Z"]
        );
        let y = reply
            .series
            .iter()
            .find(|series| series.tag == "Y")
            .unwrap();
        assert_eq!(
            y.entries
                .iter()
                .map(|entry| (
                    entry.first_hour,
                    entry.last_hour,
                    entry.value.defense_raw.as_str()
                ))
                .collect::<Vec<_>>(),
            [(18, 18, "7"), (20, 20, "7")]
        );
        assert_eq!(
            y.membership.as_ref().unwrap(),
            &[
                HourSpan {
                    first_hour: 18,
                    last_hour: 18
                },
                HourSpan {
                    first_hour: 20,
                    last_hour: 20
                }
            ]
        );
        assert_eq!(
            reply
                .series
                .iter()
                .find(|series| series.tag == "PRE")
                .unwrap()
                .entries[0]
                .first_hour,
            16
        );
        assert!(reply.series.iter().all(|series| {
            series
                .entries
                .iter()
                .all(|entry| entry.value.defense_raw != "11")
        }));
    }

    fn agency_source(
        operation_files: &[(&str, &[u8])],
    ) -> (Source, LoadContext, crate::catalogue::tests::Folder) {
        use crate::products::ContentOptions;
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{LexicalTokenRow, OperationRow, TableRows},
        };
        let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
        let localisation = "\u{feff}l_english:\n MOD_OPERATION:0 \"Custom operation\"\n AGENCY_NAME:0 \"Custom agency\"\n AGENCY_DEFENSE_LEVEL:0 \"Protection: $LEVEL$ / $LEVEL$\"\n";
        for (path, bytes) in operation_files
            .iter()
            .copied()
            .chain([("localisation/test_l_english.yml", localisation.as_bytes())])
        {
            let file = folder.0.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let layers = installed_content::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        let catalogue = crate::catalogue::tests::layers_catalogue(layers);
        let options = ContentOptions {
            options: installed_content::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.clone(),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
        };
        let mut first = source(1, 16, Some(1));
        let mut agency = row(11, 125_000);
        agency.created = 0;
        *first.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![agency]));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(Vec::new()));
        first.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        first.metrics.table_ns[TableId::Operations.index()] = 1;
        *first.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(
            [70, 71]
                .into_iter()
                .map(|definition_token| OperationRow {
                    id: u64::from(definition_token),
                    owner: 11,
                    target: 11,
                    definition_token,
                    ..Default::default()
                })
                .collect(),
        ));
        *first.tables[TableId::OperationAssignments.index()] =
            TableView::Complete(TableRows::OperationAssignments(Vec::new()));
        *first.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 70,
                    literal: b"mod_operation".to_vec(),
                },
                LexicalTokenRow {
                    token: 71,
                    literal: b"AGENCY_NAME".to_vec(),
                },
            ]));
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first])]).unwrap());
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "agency-operation-notes".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        crate::catalogue::set_test_catalogue(&source, &options, Arc::new(catalogue));
        (
            source,
            LoadContext {
                content: Some(options),
            },
            folder,
        )
    }

    #[test]
    fn acquired_levels_preserve_duplicates_and_signed_operands_then_split_changed_reads() {
        use replay::rows::AgencyUpgradeLevelRow;
        let observed = crate::test_fixtures::source(1, 10, Some(1));
        let mut rows: Vec<_> = [-3, 2, 3]
            .into_iter()
            .enumerate()
            .map(|(ordinal, level)| AgencyUpgradeLevelRow {
                country: 1,
                ordinal: ordinal as u32,
                definition_pointer: 91,
                branch_pointer: 81,
                upgrade_token: Some(71),
                branch_token: Some(72),
                level,
                identity_status: engine::agency_upgrade_levels::AUTHENTICATED,
            })
            .collect();
        let mut read = acquired_read(&observed, &rows.iter().collect::<Vec<_>>(), None);
        assert_eq!(
            read.items.iter().map(|item| item.level).collect::<Vec<_>>(),
            [-3, 2, 3]
        );
        assert_eq!(read.nonnegative_level_sum.as_deref(), Some("5"));
        let mut scanned = ScannedAgencies::default();
        let agency = row(1, 10);
        for hour in [10, 11] {
            scanned.observe(hour, "SYN", (1, 1), Some(&agency), None);
            read.read_hour = Some(hour);
            scanned.observe_upgrades(hour, "SYN", read.clone());
        }
        rows[2].level = 7;
        read = acquired_read(&observed, &rows.iter().collect::<Vec<_>>(), None);
        read.read_hour = Some(12);
        scanned.observe(12, "SYN", (1, 1), Some(&agency), None);
        scanned.observe_upgrades(12, "SYN", read.clone());
        let entries = &scanned.tracks["SYN"].entries;
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.first_hour, entry.last_hour))
                .collect::<Vec<_>>(),
            [(10, 11), (12, 12)]
        );
        assert_eq!(
            entries[0].row.upgrades.as_ref().unwrap().read_hour,
            Some(10)
        );
        assert_eq!(
            entries[1]
                .row
                .upgrades
                .as_ref()
                .unwrap()
                .nonnegative_level_sum
                .as_deref(),
            Some("9")
        );
        // A changed scalar starts a run that shares the unchanged list.
        let mut moved = agency.clone();
        moved.active_count += 1;
        scanned.observe(13, "SYN", (1, 1), Some(&moved), None);
        scanned.observe_upgrades(13, "SYN", read);
        let entries = &scanned.tracks["SYN"].entries;
        assert!(Arc::ptr_eq(
            &entries[1].row.upgrades.as_ref().unwrap().items,
            &entries[2].row.upgrades.as_ref().unwrap().items
        ));
        rows[1].identity_status = 255;
        let refused = acquired_read(&observed, &rows.iter().collect::<Vec<_>>(), None);
        assert!(refused.nonnegative_level_sum.is_none());
        assert_eq!(refused.items.len(), 3);
        assert!(refused.items[1].identity_refusal.is_some());
        let failed = acquired_read(&observed, &[], Some("synthetic failed read".into()));
        assert!(failed.items.is_empty());
        assert!(failed.nonnegative_level_sum.is_none());
        assert_eq!(failed.unavailable.as_deref(), Some("synthetic failed read"));
    }

    #[test]
    fn agency_comparison_neither_shows_nor_reads_operation_definitions() {
        use super::super::intelligence_content::intelligence_content;
        use crate::products::espionage_log::EspionageLog;
        let (good, good_context, _good_folder) = agency_source(&[(
            "common/operations/known.txt",
            b"mod_operation = { name = MOD_OPERATION icon = mod_sprite }",
        )]);
        let (bad, bad_context, bad_folder) = agency_source(&[
            (
                "common/operations/known.txt",
                b"mod_operation = { name = MOD_OPERATION icon = mod_sprite on_start = { free_operative = { operative = custom_name } } }",
            ),
            (
                "common/operations/misread.txt",
                b"misread_operation = { name = { unused = yes } }",
            ),
            (
                "common/operations/unsettled.txt",
                b"unsettled_operation = { operation_target = { target_array = custom_array } }",
            ),
            ("common/operations/unparsable.txt", b"\x80 = {}"),
            (
                "common/operations/collision.txt",
                b"AGENCY_NAME = { name = MOD_OPERATION icon = AGENCY_DEFENSE_LEVEL }",
            ),
            ("common/operations/unreadable.txt", b"unused_operation = {}"),
        ]);
        let cancelled = AtomicBool::new(false);
        let agency_params = AgencyComparisonParams {
            include_uncreated: true,
            ..params()
        };
        let baseline =
            AgencyComparison::load(&good, &agency_params, &good_context, &cancelled).unwrap();
        assert_eq!(baseline.series.len(), 1);
        assert_eq!(
            baseline.series[0].entries[0].value.name.as_deref(),
            Some("Custom agency")
        );
        let expected = serde_json::to_value(&baseline).unwrap();
        let malformed =
            AgencyComparison::load(&bad, &agency_params, &bad_context, &cancelled).unwrap();
        assert_eq!(serde_json::to_value(&malformed).unwrap(), expected);
        let agency = super::super::intelligence_content::agency_content(
            &bad,
            bad_context.content().unwrap(),
        )
        .unwrap();
        assert!(agency.definitions.operations.is_empty());
        assert_eq!(
            agency
                .catalogue
                .content
                .layers
                .derived::<crate::catalogue::CachedResult<installed_content::intelligence::Intelligence>>()
                .loads(),
            0,
            "agency comparison must not read operation definitions"
        );
        // The explicit operation read below classifies on_start effects through
        // the installation's effect registry; agency comparison above needs none.
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let content = intelligence_content(&bad, bad_context.content().unwrap()).unwrap();
        for kind in [
            installed_content::DiagnosticKind::Misread,
            installed_content::DiagnosticKind::Unsupported,
            installed_content::DiagnosticKind::Unsettled,
            installed_content::DiagnosticKind::Refused,
        ] {
            assert!(
                content.definitions.notes.iter().any(|note| note.0 == kind),
                "{:?}",
                content.definitions.notes
            );
        }
        assert!(content.definitions.notes.iter().any(|(kind, note)| {
            *kind == installed_content::DiagnosticKind::Refused && note.contains("unparsable.txt")
        }));
        let espionage = EspionageLog::load(
            &bad,
            &crate::products::espionage_log::EspionageLogParams {
                branch_id: "1".into(),
                window: None,
            },
            &bad_context,
            &cancelled,
        )
        .unwrap();
        for warning in &content.definitions.warnings {
            assert!(espionage.warnings.contains(warning), "{warning}");
        }
        // Keep the inventory, but make a file unreadable and replace the
        // catalogue so a cached full intelligence read cannot mask an I/O error.
        // A cloned resolved stack shares its immutable derived databases. Bind a
        // fresh inventory before removing the file so the new read reaches I/O.
        let layers = installed_content::ResolvedLayers::from_folders(&[(&bad_folder.0, &[][..])]).unwrap();
        std::fs::remove_file(bad_folder.0.join("common/operations/unreadable.txt")).unwrap();
        let catalogue = crate::catalogue::tests::layers_catalogue(layers);
        crate::catalogue::set_test_catalogue(
            &bad,
            bad_context.content().unwrap(),
            Arc::new(catalogue),
        );
        let unreadable =
            AgencyComparison::load(&bad, &agency_params, &bad_context, &cancelled).unwrap();
        assert_eq!(serde_json::to_value(&unreadable).unwrap(), expected);
        let rebound_agency = super::super::intelligence_content::agency_content(
            &bad,
            bad_context.content().unwrap(),
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&agency, &rebound_agency));
        assert!(intelligence_content(&bad, bad_context.content().unwrap()).is_err());
    }

    #[test]
    fn agency_comparison_omits_operation_notes_and_keeps_agency_refusals_after_espionage() {
        use crate::products::espionage_log::{EspionageLog, EspionageLogParams};
        let (source, context, _folder) = agency_source(&[(
            "common/operations/art.txt",
            b"mod_operation = { name = MOD_OPERATION icon = AGENCY_DEFENSE_LEVEL } AGENCY_NAME = { name = MOD_OPERATION icon = AGENCY_NAME }",
        )]);
        let cancelled = AtomicBool::new(false);
        let operation_params = EspionageLogParams {
            branch_id: "1".into(),
            window: None,
        };
        let agency_params = AgencyComparisonParams {
            include_uncreated: true,
            ..params()
        };
        let espionage =
            EspionageLog::load(&source, &operation_params, &context, &cancelled).unwrap();
        assert_eq!(espionage.types.len(), 2);
        for operation in &espionage.types {
            assert_eq!(operation.name.as_deref(), Some("Custom operation"));
            assert!(
                operation
                    .icon_missing
                    .as_ref()
                    .unwrap()
                    .contains("artwork AGENCY_")
            );
        }
        assert!(
            espionage
                .warnings
                .iter()
                .any(|warning| warning.contains("AGENCY_NAME"))
        );
        assert!(
            espionage
                .warnings
                .iter()
                .any(|warning| warning.contains("AGENCY_DEFENSE_LEVEL"))
        );
        let reply_after_artwork =
            AgencyComparison::load(&source, &agency_params, &context, &cancelled).unwrap();
        assert_eq!(
            reply_after_artwork.warnings,
            [
                "1 texts are not shown: defence template must render LEVEL once to separate its label and number; rendered 2 occurrences; AGENCY_DEFENSE_LEVEL"
            ]
        );
    }

    #[test]
    fn agency_comparison_gaps_missing_rows_and_identity_changes_stay_missing() {
        let mut scanned = ScannedAgencies::default();
        let agency = row(1, 25_000);
        scanned.note_roster_read(10);
        scanned.observe(10, "SYN", (1, 1), Some(&agency), Some(Some(2)));
        scanned.observe(11, "SYN", (1, 1), None, None);
        scanned.observe(12, "SYN", (1, 1), Some(&agency), None);
        // Hour 13 is a gap. A later equal read cannot bridge it.
        scanned.observe(14, "SYN", (1, 1), Some(&agency), None);
        scanned.observe(15, "SYN", (1, 2), Some(&agency), None);
        let series = requested_series(
            &scanned,
            &params(),
            || None,
            |_| (None, None),
            &AtomicBool::new(false),
        )
        .unwrap();
        let entries = &series[0].entries;
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.first_hour, entry.last_hour))
                .collect::<Vec<_>>(),
            [(10, 10), (12, 12), (14, 14), (15, 15)]
        );
        assert_eq!(
            series[0].rosters,
            [AgencyRosterRun {
                first_hour: 10,
                last_hour: 12,
                read_hour: 10,
                count: 2
            }]
        );
        assert_eq!(scanned.operative_read_hours, [10]);
        assert_eq!(entries[3].value.defense_raw, "25000");
        assert_eq!(entries[3].value.defense_text, None);
        assert_eq!(entries[3].value.defense_value, None);
    }

    #[test]
    fn agency_comparison_never_created_uses_localised_default_not_stored_name() {
        let mut agency = row(1, 0);
        agency.created = 0;
        agency.name = b"Stale stored bytes".to_vec();
        assert_eq!(
            agency_name(&AgencyProjection::from(&agency), || Some(
                "Agence du mod".into()
            )),
            Some("Agence du mod".into())
        );
        agency.created = 1;
        agency.name.clear();
        assert_eq!(
            agency_name(&AgencyProjection::from(&agency), || Some("Wrong".into())),
            Some(String::new())
        );
        let value = value(
            &RawEntry {
                first_hour: 10,
                last_hour: 10,
                read_hour: 10,
                row: AgencyProjection::from(&agency),
            },
            || None,
            |_| (Some("Localised zero: 0.00".into()), Some("0.00".into())),
        );
        assert_eq!(value.maximum, 7);
        assert_eq!(value.defense_text.as_deref(), Some("Localised zero: 0.00"));
        assert_eq!(value.defense_value.as_deref(), Some("0.00"));
        assert_eq!(value.upgrades.nonnegative_level_sum, None);
        assert!(upgrades_unrecorded().unwrap().starts_with("Not recorded:"));
    }

    #[test]
    fn agency_comparison_daily_work_and_admission_changes_keep_one_displayed_run() {
        let mut scanned = ScannedAgencies::default();
        let mut agency = row(1, 25_000);
        for hour in 0..(12 * 365 * 24) {
            agency.upgrade_progress_raw = i64::from(hour / 24) * 100_000;
            agency.admission_ceiling = hour / 24;
            let roster = (hour % 24 == 0).then_some(Some(2));
            if roster.is_some() {
                scanned.note_roster_read(hour);
            }
            scanned.observe(hour, "SYN", (1, 1), Some(&agency), roster);
        }
        let entries = &scanned.tracks["SYN"].entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].first_hour, entries[0].last_hour),
            (0, 12 * 365 * 24 - 1)
        );
        assert_eq!(
            scanned.tracks["SYN"].rosters,
            [AgencyRosterRun {
                first_hour: 0,
                last_hour: 12 * 365 * 24 - 1,
                read_hour: 0,
                count: 2
            }]
        );
        assert_eq!(
            scanned.operative_read_hours,
            (0..12 * 365).map(|day| day * 24).collect::<Vec<_>>()
        );
        // The same independently dated history for 5,000 synthetic countries
        // stays bounded; dates are shared, not repeated in each country's runs.
        let track = scanned.tracks.remove("SYN").unwrap();
        for country in 0..5000 {
            scanned
                .tracks
                .insert(format!("SYN_{country}"), track.clone());
        }
        let reply = AgencyComparisonReply {
            branch_id: "1".into(),
            available: true,
            observed: vec![HourSpan {
                first_hour: 0,
                last_hour: 12 * 365 * 24 - 1,
            }],
            operative_read_hours: scanned.operative_read_hours.clone(),
            series: requested_series(
                &scanned,
                &params(),
                || None,
                |_| {
                    (
                        Some("Synthetic protection: 0.25".into()),
                        Some("0.25".into()),
                    )
                },
                &AtomicBool::new(false),
            )
            .unwrap(),
            defense_label: Some("Synthetic protection".into()),
            upgrades_unrecorded: upgrades_unrecorded(),
            warnings: Vec::new(),
        };
        assert_eq!(reply.series.len(), 5000);
        assert_eq!(reply.defense_label.as_deref(), Some("Synthetic protection"));
        assert!(measured(&reply).unwrap() < MAX_PRODUCT_REPLY_BYTES);
    }

    #[test]
    fn agency_comparison_request_accepts_two_tags_and_u32_max_top_and_deduplicates_selection() {
        let mut request = params();
        request.tags = vec!["SYN".into(), "SYN".into()];
        assert!(AgencyComparison::validate(&request).is_ok());
        let mut scanned = ScannedAgencies::default();
        scanned.observe(1, "SYN", (1, 1), Some(&row(1, 0)), None);
        assert_eq!(
            requested_series(
                &scanned,
                &request,
                || None,
                |_| (None, None),
                &AtomicBool::new(false)
            )
            .unwrap()
            .len(),
            1
        );
        request.top = Some(AgencyTop {
            count: 1,
            field: AgencyRankField::Defense,
        });
        assert_eq!(
            AgencyComparison::validate(&request),
            Err("Invalid top countries")
        );
        request.tags.clear();
        request.top.as_mut().unwrap().count = u32::MAX;
        assert!(AgencyComparison::validate(&request).is_ok());
        request.top.as_mut().unwrap().count = 0;
        assert_eq!(
            AgencyComparison::validate(&request),
            Err("Invalid top countries")
        );
        request.top.as_mut().unwrap().count = 5_000;
        assert!(AgencyComparison::validate(&request).is_ok());
    }

    #[test]
    fn agency_comparison_refuses_the_shared_complete_reply_budget() {
        let mut scanned = ScannedAgencies::default();
        scanned.observe(1, "SYN", (1, 1), Some(&row(1, 0)), None);
        let mut reply = AgencyComparisonReply {
            branch_id: "1".into(),
            available: true,
            observed: vec![HourSpan {
                first_hour: 1,
                last_hour: 1,
            }],
            operative_read_hours: Vec::new(),
            series: requested_series(
                &scanned,
                &params(),
                || None,
                |_| (Some("x".repeat(MAX_PRODUCT_REPLY_BYTES)), None),
                &AtomicBool::new(false),
            )
            .unwrap(),
            defense_label: None,
            upgrades_unrecorded: upgrades_unrecorded(),
            warnings: Vec::new(),
        };
        let length = crate::transport::json_len(&reply, usize::MAX).unwrap();
        let text = reply.series[0].entries[0]
            .value
            .defense_text
            .as_mut()
            .unwrap();
        text.truncate(text.len() - (length - MAX_PRODUCT_REPLY_BYTES));
        assert_eq!(measured(&reply).unwrap(), MAX_PRODUCT_REPLY_BYTES);
        reply.series[0].entries[0]
            .value
            .defense_text
            .as_mut()
            .unwrap()
            .push('x');
        let error = measured(&reply).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Agency comparison exceeds one complete reply's budget")
        );
    }

    #[test]
    fn agency_comparison_unused_default_name_is_not_resolved() {
        let mut scanned = ScannedAgencies::default();
        scanned.observe(1, "SYN", (1, 1), Some(&row(1, 0)), None);
        let series = requested_series(
            &scanned,
            &params(),
            || panic!("created agencies must not resolve an unused default name"),
            |_| (None, None),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            series[0].entries[0].value.name.as_deref(),
            Some("Recorded agency")
        );
    }

    #[test]
    fn agency_comparison_localises_each_defense_value_and_default_name_once() {
        use std::cell::Cell;
        let mut scanned = ScannedAgencies::default();
        for country in 1..=5000 {
            let mut agency = row(country, i64::from(country % 2) * 100_000);
            agency.created = 0;
            scanned.observe(
                1,
                &format!("SYN_{country}"),
                (country, country),
                Some(&agency),
                None,
            );
        }
        let default_calls = Cell::new(0);
        let defense_calls = Cell::new(0);
        let mut request = params();
        request.include_uncreated = true;
        let series = requested_series(
            &scanned,
            &request,
            || {
                default_calls.set(default_calls.get() + 1);
                Some("Localised default".into())
            },
            |raw| {
                defense_calls.set(defense_calls.get() + 1);
                (
                    Some(format!("Localised defense: {raw}")),
                    Some(raw.to_string()),
                )
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(series.len(), 5000);
        assert_eq!(default_calls.get(), 1);
        assert_eq!(defense_calls.get(), 2);
        assert!(series.iter().all(|series| {
            let value = &series.entries[0].value;
            value.defense_value.as_deref() == Some(value.defense_raw.as_str())
        }));
        assert!(
            series
                .iter()
                .all(|series| series.entries[0].value.name.as_deref() == Some("Localised default"))
        );
    }

    #[test]
    fn agency_comparison_5000_countries_top_n_and_complete_response_budget() {
        let mut scanned = ScannedAgencies::default();
        for country in 1..=5000 {
            let agency = row(country, i64::from(country) * 100_000);
            scanned.observe(
                10,
                &format!("SYN_{country:04}"),
                (country, country),
                Some(&agency),
                None,
            );
            scanned.observe(
                11,
                &format!("SYN_{country:04}"),
                (country, country),
                Some(&agency),
                None,
            );
        }
        let cancelled = AtomicBool::new(false);
        let all =
            requested_series(&scanned, &params(), || None, |_| (None, None), &cancelled).unwrap();
        assert_eq!(all.len(), 5000);
        assert!(measured(&all).unwrap() < MAX_PRODUCT_REPLY_BYTES);
        assert!(
            all.iter()
                .all(|series| series.entries.len() == 1 && series.entries[0].last_hour == 11)
        );
        let mut top_params = params();
        top_params.top = Some(AgencyTop {
            count: 3,
            field: AgencyRankField::Defense,
        });
        let top =
            requested_series(&scanned, &top_params, || None, |_| (None, None), &cancelled).unwrap();
        assert_eq!(
            top.iter()
                .map(|series| series.tag.as_str())
                .collect::<Vec<_>>(),
            ["SYN_4998", "SYN_4999", "SYN_5000"]
        );
        assert_eq!(
            top[0].membership.as_ref().unwrap()[0],
            HourSpan {
                first_hour: 10,
                last_hour: 11
            }
        );
    }

    #[test]
    fn agency_comparison_off_read_aliases_keep_native_census_dates_and_refuse_broken_coverage() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        // The wire codec refuses duplicate canonical slots. Exercise their
        // identity guard directly, then scan the supported unavailable census.
        let duplicate_slots: Vec<_> = [7, 9]
            .into_iter()
            .map(|id| CountryRow {
                id,
                canonical_slot: 1,
                tag: format!("SYN_{id}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let duplicate_links: Vec<_> = [7, 9]
            .into_iter()
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: 1,
            })
            .collect();
        let (slots, unresolved) =
            super::super::operative_rows::country_rows_partial(&duplicate_slots, &duplicate_links);
        assert!(slots.is_empty());
        assert_eq!(unresolved, [7, 9]);
        for missing_scalar in [false, true] {
            for duplicate_tag in [false, true] {
                let frames = [
                    (15, 7, "X"),
                    (16, 7, "Y"),
                    (17, 7, "X"),
                    (18, 7, "Y"),
                    (19, 7, "X"),
                    (20, 7, "Y"),
                    (21, 9, "Y"),
                    (22, 9, "Z"),
                    (24, 7, "Z"),
                    (25, 7, "Z"),
                    (26, 7, "Z"),
                    (27, 7, "Z"),
                    (28, 7, "Z"),
                ]
                .into_iter()
                .map(|(hour, id, tag)| {
                    let mut frame = source(1, hour, Some(1));
                    let mut countries = vec![CountryRow {
                        id,
                        canonical_slot: 1,
                        tag: tag.as_bytes().to_vec(),
                        ..Default::default()
                    }];
                    if hour == 26 {
                        countries.push(CountryRow {
                            id: 9,
                            canonical_slot: 2,
                            tag: b"Z".to_vec(),
                            ..Default::default()
                        });
                    }
                    let links = countries
                        .iter()
                        .map(|country| CountryLinkRow {
                            id: country.id,
                            canonical_slot: country.canonical_slot,
                        })
                        .collect();
                    *frame.tables[TableId::Countries.index()] = if hour == 26 && !duplicate_tag {
                        TableView::Unavailable {
                            reason: "synthetic duplicate-slot country census refused".into(),
                        }
                    } else {
                        TableView::Complete(TableRows::Countries(countries))
                    };
                    *frame.tables[TableId::CountryLinks.index()] =
                        TableView::Complete(TableRows::CountryLinks(links));
                    let mut agency = row(id, 125_000);
                    agency.active_count = 7;
                    *frame.tables[TableId::IntelligenceAgencies.index()] =
                        if missing_scalar && hour == 15 {
                            TableView::Unavailable {
                                reason: "synthetic source-alias scalar missing".into(),
                            }
                        } else {
                            TableView::Complete(TableRows::IntelligenceAgencies(vec![agency]))
                        };
                    *frame.tables[TableId::AgencyUpgradeLevels.index()] =
                        TableView::Complete(TableRows::AgencyUpgradeLevels(Vec::new()));
                    *frame.tables[TableId::OperativeMemberships.index()] = if hour == 18 {
                        TableView::Unavailable {
                            reason: "synthetic due census refused".into(),
                        }
                    } else {
                        TableView::Complete(TableRows::OperativeMemberships(
                            (1..=2)
                                .map(|id| OperativeMembershipRow {
                                    id,
                                    roster_country: 7,
                                    ..Default::default()
                                })
                                .collect(),
                        ))
                    };
                    frame.metrics.table_ns[TableId::OperativeMemberships.index()] =
                        u64::from(matches!(hour, 15 | 18 | 20 | 25 | 28));
                    frame.metrics.table_ns[TableId::WingCumulative.index()] = 1;
                    frame
                })
                .collect();
                let cancelled = AtomicBool::new(false);
                let replay =
                    Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
                let scanned = scan(&replay, 1, &cancelled).unwrap();
                assert_eq!(scanned.operative_read_hours, [15, 20, 25, 28]);
                let counts = |tag: &str| {
                    scanned.tracks[tag]
                        .rosters
                        .iter()
                        .map(|run| (run.first_hour, run.last_hour, run.read_hour, run.count))
                        .collect::<Vec<_>>()
                };
                assert_eq!(counts("X"), [(15, 15, 15, 2), (17, 17, 15, 2)]);
                assert_eq!(counts("Y"), [(16, 16, 15, 2), (20, 20, 20, 2)]);
                assert_eq!(counts("Z"), [(25, 25, 25, 2), (28, 28, 28, 2)]);
                let source = Source::from_replay(
                    crate::SourceKey {
                        source_id: "held-agency-alias".into(),
                        revision: "1".into(),
                    },
                    "synthetic.battleplan".into(),
                    replay,
                    &cancelled,
                )
                .unwrap();
                let mut request = params();
                request.tags = vec!["X".into()];
                let reply = AgencyComparison::load(
                    &source,
                    &request,
                    &LoadContext { content: None },
                    &cancelled,
                )
                .unwrap();
                let y = reply
                    .series
                    .iter()
                    .find(|series| series.tag == "Y")
                    .unwrap();
                assert_eq!(
                    y.rosters
                        .iter()
                        .map(|run| (run.first_hour, run.read_hour, run.count))
                        .collect::<Vec<_>>(),
                    [(16, 15, 2), (20, 20, 2)]
                );
                assert!(y.entries.iter().all(|entry| entry.value.active_count == 7));
                assert!(
                    reply
                        .series
                        .iter()
                        .flat_map(|series| &series.rosters)
                        .all(|run| ![18, 19, 21, 22, 24, 26, 27].contains(&run.first_hour))
                );
                assert!(measured(&reply).unwrap() < MAX_PRODUCT_REPLY_BYTES);
            }
        }
    }

    #[test]
    fn agency_comparison_held_scheduled_and_peer_frames_do_not_redate_roster_reads() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{frame::TableView, rows::TableRows};
        let member = OperativeMembershipRow {
            id: 77,
            roster_country: 11,
            roster_kind: 0,
            ..Default::default()
        };
        let mut first = source(1, 15, Some(1));
        *first.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![row(11, 125_000)]));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![member.clone()]));
        first.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        let mut scheduled = first.clone();
        scheduled.hour = 16;
        scheduled.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        assert!(replay::cadence::is_refresh(
            TableId::OperativeMemberships,
            scheduled.hour
        ));
        let mut peer = scheduled.clone();
        peer.hour = 17;
        peer.metrics.table_ns[TableId::WingCumulative.index()] = 1;
        assert_eq!(
            replay::cadence::cadence(TableId::WingCumulative),
            replay::cadence::cadence(TableId::OperativeMemberships)
        );
        let mut refreshed = first.clone();
        refreshed.hour = 18;
        *refreshed.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![
                member.clone(),
                OperativeMembershipRow { id: 78, ..member },
            ]));
        let mut held = refreshed.clone();
        held.hour = 19;
        held.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![first, scheduled, peer, refreshed, held])],
        )
        .unwrap();
        let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(scanned.operative_read_hours, [15, 18]);
        assert_eq!(
            scanned.tracks["GER"].rosters,
            [
                AgencyRosterRun {
                    first_hour: 15,
                    last_hour: 17,
                    read_hour: 15,
                    count: 1
                },
                AgencyRosterRun {
                    first_hour: 18,
                    last_hour: 19,
                    read_hour: 18,
                    count: 2
                },
            ]
        );
        // The unavailable hourly upgrade input retains a dated diagnostic.
        assert_eq!(
            scanned.tracks["GER"]
                .entries
                .iter()
                .map(|entry| (
                    entry.first_hour,
                    entry.last_hour,
                    entry.read_hour,
                    entry.row.active_count,
                ))
                .collect::<Vec<_>>(),
            (15..=19)
                .map(|hour| (hour, hour, hour, 2))
                .collect::<Vec<_>>()
        );
        for entry in &scanned.tracks["GER"].entries {
            let upgrades = entry.row.upgrades.as_ref().unwrap();
            assert_eq!(upgrades.read_hour, Some(entry.read_hour));
            assert!(upgrades.unavailable.as_ref().unwrap().contains(&format!(
                "hour {} AgencyUpgradeLevels is unavailable",
                entry.read_hour
            )));
        }
    }

    #[test]
    fn agency_comparison_replay_reads_5000_countries_and_refuses_gaps_without_zeroes() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        let mut first = source(1, 16, Some(1));
        *first.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5000)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("SYN_{id:04}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *first.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=5000)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *first.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(
                (1..=5000)
                    .map(|id| row(id, i64::from(id) * 100_000))
                    .collect(),
            ));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(Vec::new()));
        first.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        let mut missing = first.clone();
        missing.hour = 17;
        missing.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        *missing.tables[TableId::IntelligenceAgencies.index()] = TableView::Unavailable {
            reason: "synthetic missing read".into(),
        };
        let mut later = first.clone();
        later.hour = 19;
        later.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        *later.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![row(1, 0)]));
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, missing, later])])
                .unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "agency-synthetic".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        let reply = AgencyComparison::load(
            &source,
            &AgencyComparisonParams {
                tags: (1..=5000).map(|id| format!("SYN_{id:04}")).collect(),
                ..params()
            },
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(reply.series.len(), 5000);
        assert_eq!(reply.defense_label, None);
        assert_eq!(
            reply.observed,
            [
                HourSpan {
                    first_hour: 16,
                    last_hour: 16
                },
                HourSpan {
                    first_hour: 19,
                    last_hour: 19
                }
            ]
        );
        let first = &reply.series[0];
        assert_eq!(first.entries.len(), 2);
        assert_eq!(
            first.rosters,
            [AgencyRosterRun {
                first_hour: 16,
                last_hour: 17,
                read_hour: 16,
                count: 0
            }]
        );
        assert_eq!(reply.operative_read_hours, [16]);
        assert_eq!(first.entries[1].value.defense_raw, "0");
        assert_eq!(first.entries[1].value.defense_value, None);
        assert_eq!(reply.series[1].entries.len(), 1);
        assert!(measured(&reply).unwrap() < MAX_PRODUCT_REPLY_BYTES);
    }

    #[test]
    fn agency_comparison_partial_rosters_stay_unknown_and_refuse_by_kind() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        let mut first = source(1, 16, Some(1));
        *first.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]));
        *first.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]));
        *first.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![row(1, 0)]));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![
                OperativeMembershipRow {
                    id: 1,
                    roster_country: 1,
                    roster_kind: 0,
                    ..Default::default()
                },
                OperativeMembershipRow {
                    id: 2,
                    roster_country: 99,
                    roster_kind: 0,
                    ..Default::default()
                },
                OperativeMembershipRow {
                    id: 3,
                    roster_country: 1,
                    roster_kind: 4,
                    ..Default::default()
                },
            ]));
        first.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first])]).unwrap());
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "agency-partial".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        let reply = AgencyComparison::load(
            &source,
            &params(),
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        let value = &reply.series[0].entries[0].value;
        assert_eq!((value.active_count, value.maximum), (2, 7));
        assert!(reply.series[0].rosters.is_empty());
        assert_eq!(reply.operative_read_hours, [16]);
        assert!(reply.warnings.iter().any(|warning| {
            warning.starts_with("1 operative memberships use unsupported roster_kind 4")
        }));
        assert!(
            reply
                .warnings
                .iter()
                .any(|warning| warning.starts_with("2 agency or operative membership rows"))
        );
    }

    #[test]
    fn agency_comparison_corrupt_blocks_report_the_missing_history() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![source(1, 16, Some(1))])])
                .unwrap();
        let mut bytes = replay.bytes().to_vec();
        let block = &replay.index[0];
        // The directory CRC guards every section; a damaged last byte only
        // fails the section it lands in, which need not be one this scan reads.
        bytes[(block.offset + 4) as usize] ^= 0x40;
        let damaged = Replay::open(bytes).unwrap();
        let scanned = scan(&damaged, 1, &AtomicBool::new(false)).unwrap();
        assert!(!scanned.available);
        assert!(scanned.observed.is_empty());
        assert_eq!(
            scanned
                .warnings
                .get("Unread replay block; agency history has a gap"),
            Some(&1)
        );
    }
}
