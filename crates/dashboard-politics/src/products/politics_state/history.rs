//! Country political state, prepared on its own (family `politics_state`):
//! election schedules, governments in exile and balance-of-power membership,
//! as coalesced runs per country holder (its native canonical slot).
//!
//! Each part is observed at an hour that changes what it reads (the fields
//! of the rows it reads, and the identities naming its countries,
//! [`Part::reads`]) or that follows a gap; at every other hour it carries
//! its held values, so a change elsewhere, such as a war's casualties, costs
//! it nothing. Each observation is the part's whole state at that read, so a
//! value starts at the first read showing it and ends at the first read that
//! does not. A gap ends every value at its last read: nothing is carried
//! across it, and an unreadable table leaves its values unknown (no run),
//! never zero.
//!
//! - Elections: the recorded game predicate (`has_elections`, 1 or 0) for a
//!   country owning states (the game holds no others' elections), the
//!   stored signed interval and the next date from the shared native calendar
//!   step ([`country_people::next_election_date`]). A known "not held" and an
//!   unrecorded eligibility stay apart. An initial read establishes a
//!   schedule, never an election.
//! - Exiles: the exile cards' rule ([`country_people::exiles`]): a positive
//!   recorded host makes an exile; its host holder, legitimacy, manpower, cap,
//!   leader resources and its own supplying read hour are independent runs.
//!   The first read in exile is a bound, never the exile's start.
//! - Balances: records have no persistent identity beyond their stored
//!   position, so the history follows what each holder belongs to: per holder
//!   and balance identity, the record's value, active range and trending
//!   side, and the record's stored position at the read (an identity within
//!   that read only: holders sharing it at one hour share one record); per
//!   holder, the balance the game shows (the first record listing it, which
//!   needs the whole membership resolved). Two records listing one holder
//!   under the same identity keep the earlier, as the game reads it.
//! - Civil wars: each holder's stored civil-war rivals, one run per rival
//!   holder while the holder's own list names it, and apart from them the
//!   shared game predicate ([`dashboard_text::identity::CivilWars`], indexed
//!   once per observation) for every listed holder, and the game's existence
//!   check for every holder ever listed ([`super::civil_wars`]).
//!
//! Each part also keeps the hours its tables were read ([`Field::Known`]): a
//! value ending at an hour its part was not read ended unknown, not then.
//! Holders keep their tag runs; a country's history is every holder that
//! held its tag, whole, so a holder renamed keeps its history and a reused
//! tag never joins an unrelated holder.

use super::{BalanceRange, BalanceSide, Tags};
use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    invalid,
    prepared::series::{self, Accumulator, Reader},
    products::{
        LoadContext, Product,
        country_groups::HourSpan,
        country_people,
    },
    triggers::inputs::{Since, TableChanges, TableRead},
};
use dashboard_text::catalogue::View;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

/// The content catalogue a history's names read, where the mode shows any.
pub fn presentation_catalogue(
    source: &Source,
    params: &PoliticsHistoryParams,
    context: &LoadContext,
) -> Option<Result<Arc<Catalogue>, String>> {
    matches!(
        params.mode,
        PoliticsHistoryMode::Country | PoliticsHistoryMode::Balances
    )
    .then(|| {
        context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
            .map_err(|error| error.to_string())
    })
}

/// The family's preparation name and its query's product key.
pub const FAMILY: &str = "politics_state";
pub const QUERY: &str = "politics-state-history";
/// The tables the observer reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Politics,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::CountryCivilWars,
    TableId::CountryCounterInputs,
    TableId::CountryWars,
    TableId::WarRelations,
];

/// The holder slot [`Field::Known`] keys are filed under; no country has it.
const PART_SLOT: u32 = u32::MAX;

/// The parts of the political state read from their own tables.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Part {
    Elections,
    Exiles,
    Balances,
    CivilWars,
}

/// What names every part's countries: each holder's slot and tag.
fn identity_reads() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]),
        TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
    ]
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Elections, Part::Exiles, Part::Balances, Part::CivilWars];

    /// What the part reads of an hour, the identities naming its countries
    /// among it: an hour changing none of it since the hour before leaves
    /// the part as last observed.
    pub fn reads(self) -> &'static [TableRead] {
        static READS: LazyLock<[Vec<TableRead>; 4]> = LazyLock::new(|| {
            Part::ALL.map(|part| {
                let mut reads = identity_reads().to_vec();
                match part {
                    Part::Elections => reads.extend([
                        TableRead::fields(
                            TableId::Politics,
                            &["last_election", "has_elections", "election_interval", "next_election_cache"],
                        ),
                        TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
                    ]),
                    Part::Exiles => {
                        // The exile rows' own supplying reads, among every sliced table's.
                        let sliced = (TableId::CountryStabilityInputs.index() as u128) << 32;
                        reads.extend([
                            TableRead::fields(
                                TableId::CountryStabilityInputs,
                                &[
                                    "exile_host",
                                    "exile_legitimacy",
                                    "exile_leaders",
                                    "exile_manpower",
                                    "exile_manpower_cap",
                                ],
                            ),
                            TableRead::fields(TableId::SliceReads, &["sample_hour"])
                                .rows(sliced, sliced | u128::from(u32::MAX)),
                        ]);
                    }
                    Part::Balances => reads.extend([
                        TableRead::fields(
                            TableId::PowerBalances,
                            &["value_raw", "balance", "trending", "active_range"],
                        ),
                        TableRead::fields(TableId::PowerBalanceMembers, &["country"]),
                    ]),
                    Part::CivilWars => reads.extend(dashboard_text::identity::civil_war_reads()),
                }
                reads
            })
        });
        &READS[self as usize]
    }
}

pub(crate) fn values_section(branch: u64) -> String {
    format!("politics-state-{branch}")
}

/// What one series key holds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    /// 1 when the game holds elections, 0 when it does not.
    ElectionsHeld,
    /// 1 while the holder's schedule was read but whether it holds elections
    /// could not be: its eligibility is unknown, not denied.
    ElectionsRefused,
    ElectionInterval,
    NextElection,
    /// The stored last election's hour, where it is a date rather than an
    /// initializer the next date's cache stands in for.
    LastElection,
    /// The host's holder slot.
    ExileHost,
    ExileLegitimacy,
    ExileManpower,
    ExileCap,
    ExileLeaders,
    ExileRead,
    /// The fixed5 value of the record listing the holder under `balance`.
    BalanceValue(String),
    /// The index of the record's active range in [`Metadata::tokens`].
    BalanceRange(String),
    /// The index of the record's trending side in [`Metadata::tokens`].
    BalanceTrending(String),
    /// The stored position of the record listing the holder under `balance`.
    BalanceRecord(String),
    /// The key of the holder's [`Field::BalanceValue`] the game shows.
    BalanceShown,
    /// 1 while the holder's stored civil-war list names this rival holder.
    CivilWarRival(u32),
    /// 1 while the game's civil-war predicate holds for the holder.
    CivilWar,
    /// 1 while a holder ever listed in a civil war owns a state, the game's
    /// existence check; an hour that cannot say has no run.
    Exists,
    /// 1 at each hour the part's tables were read.
    Known(Part),
}

/// What the catalogue says about each key and holder.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    /// Each key's holder slot and field.
    pub fields: BTreeMap<u64, (u32, Field)>,
    /// Content tokens the range and trending runs index.
    pub tokens: Vec<String>,
    /// Each holder slot's tag from each hour it was first read with it.
    pub tags: BTreeMap<u32, Vec<(i32, String)>>,
    /// Named reasons, each counted once per observed hour it held.
    pub refusals: BTreeMap<String, u64>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    metadata: Metadata,
    #[serde(skip)]
    catalogue_pending: bool,
    last: Option<i32>,
    /// Each part's keys holding a value at its last observation.
    #[serde(default)]
    held: BTreeMap<Part, BTreeSet<u64>>,
    /// Every holder a civil-war list has named, whose existence the part follows.
    listed: BTreeSet<u32>,
    /// Each key once allocated, so an observation finds it without spelling
    /// its name; rebuilt by use after restore.
    #[serde(skip)]
    resolved: HashMap<(u32, Field), u64>,
    /// Each part observed, with its hour, in order.
    #[cfg(test)]
    #[serde(skip)]
    pub(crate) observed: Vec<(i32, Part)>,
}

/// One observed hour's values, by key.
type Observed = BTreeMap<u64, i128>;

impl Prepared {
    fn key(&mut self, slot: u32, field: Field) -> u64 {
        let id = (slot, field);
        if let Some(&key) = self.resolved.get(&id) {
            return key;
        }
        let name = serde_json::to_string(&id).expect("string keys");
        let key = match self.keys.get(&name) {
            Some(&key) => key,
            None => {
                let key = self.keys.len() as u64;
                self.keys.insert(name, key);
                self.metadata.fields.insert(key, id.clone());
                key
            }
        };
        self.resolved.insert(id, key);
        key
    }

    fn token(&mut self, token: &[u8]) -> Option<i128> {
        let token = String::from_utf8_lossy(installed_content_core::parser::c_string(token)).into_owned();
        if token.is_empty() {
            return None;
        }
        let index = match self.metadata.tokens.iter().position(|held| *held == token) {
            Some(index) => index,
            None => {
                self.metadata.tokens.push(token);
                self.metadata.tokens.len() - 1
            }
        };
        Some(index as i128)
    }

    fn refuse(&mut self, reason: impl Into<String>) {
        *self.metadata.refusals.entry(reason.into()).or_default() += 1;
    }

    fn tag(&mut self, tags: &Tags<'_>, slot: u32, hour: i32) {
        if let Some(tag) = tags.slot_tag(slot) {
            let runs = self.metadata.tags.entry(slot).or_default();
            if runs.last().is_none_or(|(_, last)| *last != tag) {
                runs.push((hour, tag));
            }
        }
    }

    /// One accepted hour: each part whose reads `changes` reach, and every
    /// part after a gap; the others carry.
    pub fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: &dyn TableChanges,
    ) -> io::Result<()> {
        self.values.advance(frame.hour)?;
        let gap = self
            .last
            .is_none_or(|last| last.checked_add(1) != Some(frame.hour));
        self.last = Some(frame.hour);
        let since = if gap {
            None
        } else {
            Since::hour_before(frame, changes)
        };
        let changed = |reads: &[TableRead]| {
            since
                .as_ref()
                .is_none_or(|since| reads.iter().any(|read| since.changed(read)))
        };
        let parts: Vec<Part> = Part::ALL
            .into_iter()
            .filter(|part| changed(part.reads()))
            .collect();
        let identity = changed(identity_reads().as_slice());
        self.observe_parts(frame, &parts, identity);
        Ok(())
    }

    /// One accepted hour observed whole, as after a gap.
    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.values.advance(frame.hour).unwrap();
        self.last = Some(frame.hour);
        self.observe_parts(frame, &Part::ALL, true);
    }

    /// `parts` at the hour, and every holder's tag where the identities
    /// changed, so a side's first read is known.
    fn observe_parts(&mut self, frame: &DecodedFrame, parts: &[Part], identity: bool) {
        if parts.is_empty() {
            return;
        }
        self.catalogue_pending = true;
        let tags = match Tags::new(frame) {
            Ok(tags) => Some(tags),
            Err(error) => {
                self.refuse(format!("country identities unavailable: {error}"));
                None
            }
        };
        if let Some(tags) = tags.as_ref().filter(|_| identity) {
            for holder in tags.holders() {
                self.tag(tags, holder, frame.hour);
            }
        }
        for &part in parts {
            #[cfg(test)]
            self.observed.push((frame.hour, part));
            let mut observed = Observed::new();
            if let Some(tags) = &tags {
                let read = match part {
                    Part::Elections => self.elections(frame, tags, &mut observed),
                    Part::Exiles => self.exiles(frame, tags, &mut observed),
                    Part::Balances => self.balances(frame, tags, &mut observed),
                    Part::CivilWars => self.civil_wars(frame, tags, &mut observed),
                };
                if read {
                    observed.insert(self.key(PART_SLOT, Field::Known(part)), 1);
                }
            }
            self.publish(part, frame.hour, observed);
        }
    }

    /// A part's whole state at the hour: each value read, and the end of
    /// each it held before and no longer does.
    fn publish(&mut self, part: Part, hour: i32, observed: Observed) {
        for (&key, &value) in &observed {
            self.values.observe(key, hour, Some(value), false);
        }
        let held = self.held.entry(part).or_default();
        for &key in held.iter().filter(|key| !observed.contains_key(key)) {
            self.values.observe(key, hour, None, false);
        }
        *held = observed.into_keys().collect();
    }

    /// Whether the part was read; the same holds for each part below.
    fn elections(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) -> bool {
        let rows = match frame.complete::<tables::Politics>() {
            Ok(rows) => rows,
            Err(error) => {
                self.refuse(format!("election schedules unavailable: {error}"));
                return false;
            }
        };
        // The game's daily politics, elections included, skip a country owning no states.
        let owned = match frame.complete::<tables::CountryCounterInputs>() {
            Ok(counters) => {
                let mut owned = HashMap::new();
                for counter in counters {
                    owned.entry(counter.country).or_insert(counter.owned_state_count);
                }
                Some(owned)
            }
            Err(error) => {
                self.refuse(format!("country existence unavailable: {error}"));
                None
            }
        };
        // A schedule whose country can't be named leaves the hour's schedules unknown, not ended.
        let mut read = true;
        for row in rows {
            let Some(slot) = tags.slot(row.country) else {
                self.refuse("an election schedule's country has no identity");
                read = false;
                continue;
            };
            let exists = owned
                .as_ref()
                .and_then(|owned| owned.get(&row.country))
                .map(|&count| count > 0);
            let held = match (row.has_elections.require("election eligibility").ok().copied(), exists) {
                (Some(false), _) | (Some(true), Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                (Some(true), None) => {
                    self.refuse("election eligibility: country existence not recorded");
                    None
                }
                (None, _) => {
                    self.refuse("election eligibility not recorded");
                    None
                }
            };
            let interval = row.election_interval.require("election interval").ok();
            for (field, value) in [
                (Field::ElectionsHeld, held.map(i128::from)),
                (Field::ElectionsRefused, held.is_none().then_some(1)),
                (
                    Field::ElectionInterval,
                    interval.map(|interval| i128::from(*interval)),
                ),
                (
                    Field::NextElection,
                    country_people::next_election_date(row).map(i128::from),
                ),
                (
                    Field::LastElection,
                    (!engine_native::cpolitical_status::needs_election_cache(row.last_election))
                        .then(|| i128::from(row.last_election)),
                ),
            ] {
                if let Some(value) = value {
                    observed.insert(self.key(slot, field), value);
                }
            }
        }
        read
    }

    fn exiles(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) -> bool {
        let rows = match frame.complete::<tables::CountryStabilityInputs>() {
            Ok(rows) => rows,
            Err(error) => {
                self.refuse(format!("exile status unavailable: {error}"));
                return false;
            }
        };
        let reads = frame.complete::<tables::SliceReads>().ok();
        let scalar = |value: &replay_data::rows::Recorded<Option<i64>>| {
            value.require("exile operand").ok().copied().flatten()
        };
        let small = |value: &replay_data::rows::Recorded<Option<i32>>| {
            value.require("exile operand").ok().copied().flatten()
        };
        // An exile is listed by its host: one whose host or country can't be named
        // leaves the hour's exiles unknown, never one fewer.
        let mut read = true;
        for row in rows {
            let Some(host) = small(&row.exile_host) else {
                self.refuse("an exile host is not recorded");
                read = false;
                continue;
            };
            if host <= 0 {
                continue;
            }
            let Some(slot) = tags.slot(row.country) else {
                self.refuse("a government in exile has no identity");
                read = false;
                continue;
            };
            let host = tags.slot(host as u32);
            if host.is_none() {
                self.refuse("an exile's host has no identity");
                read = false;
            }
            let sliced = reads.and_then(|reads| {
                replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
            });
            for (field, value) in [
                (Field::ExileHost, host.map(i128::from)),
                (
                    Field::ExileLegitimacy,
                    scalar(&row.exile_legitimacy).map(i128::from),
                ),
                (
                    Field::ExileManpower,
                    small(&row.exile_manpower).map(i128::from),
                ),
                (
                    Field::ExileCap,
                    small(&row.exile_manpower_cap).map(i128::from),
                ),
                (Field::ExileLeaders, small(&row.exile_leaders).map(i128::from)),
                (Field::ExileRead, sliced.map(i128::from)),
            ] {
                if let Some(value) = value {
                    observed.insert(self.key(slot, field), value);
                }
            }
        }
        read
    }

    fn balances(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) -> bool {
        let (records, members) = match (
            frame.complete::<tables::PowerBalances>(),
            frame.complete::<tables::PowerBalanceMembers>(),
        ) {
            (Ok(records), Ok(members)) => (records, members),
            (Err(error), _) | (_, Err(error)) => {
                self.refuse(format!("balance reads unavailable: {error}"));
                return false;
            }
        };
        let records: BTreeMap<u32, _> = records
            .iter()
            .map(|record| (record.ordinal, record))
            .collect();
        // Members in stored record order: the first record listing a holder
        // under an identity is the one the game reads.
        let mut ordered: Vec<_> = members.iter().collect();
        ordered.sort_by_key(|member| (member.record, member.ordinal));
        let mut seen = BTreeSet::new();
        let mut shown = BTreeMap::<u32, u64>::new();
        let mut unresolved = false;
        for member in ordered {
            let Some(slot) = tags.slot(member.country) else {
                unresolved = true;
                continue;
            };
            let Some(record) = records.get(&member.record) else {
                self.refuse("a balance member names a record that is not stored");
                continue;
            };
            let balance =
                String::from_utf8_lossy(installed_content_core::parser::c_string(&record.balance))
                    .into_owned();
            let key = self.key(slot, Field::BalanceValue(balance.clone()));
            shown.entry(slot).or_insert(key);
            if !seen.insert(key) {
                self.refuse("two records list one country under the same balance");
                continue;
            }
            observed.insert(key, i128::from(record.value_raw));
            observed.insert(
                self.key(slot, Field::BalanceRecord(balance.clone())),
                i128::from(record.ordinal),
            );
            if let Some(range) = self.token(&record.active_range) {
                observed.insert(self.key(slot, Field::BalanceRange(balance.clone())), range);
            }
            if let Some(side) = self.token(&record.trending) {
                observed.insert(self.key(slot, Field::BalanceTrending(balance)), side);
            }
        }
        if unresolved {
            self.refuse("a balance member has no identity, so no shown balance is known");
            return true;
        }
        for (slot, key) in shown {
            observed.insert(self.key(slot, Field::BalanceShown), i128::from(key));
        }
        true
    }

    /// Each holder's stored civil-war rivals, in the game's own lists: a
    /// list is not assumed to be answered by its rival's. Apart from them,
    /// the game's own predicate for every listed holder, its inputs indexed
    /// once for them all; a list naming a country that no longer exists
    /// does not make a civil war. And whether each holder ever listed
    /// exists, so a side's annexation is told from one it outlived.
    fn civil_wars(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) -> bool {
        let rows = match frame.complete::<tables::CountryCivilWars>() {
            Ok(rows) => rows,
            Err(error) => {
                self.refuse(format!("civil-war lists unavailable: {error}"));
                return false;
            }
        };
        let mut listed = BTreeSet::new();
        let mut read = true;
        for row in rows {
            let (Some(slot), Some(rival)) = (tags.slot(row.country), tags.slot(row.target)) else {
                // The lists are incomplete: what they name at this hour is unknown, not ended.
                self.refuse("a civil-war list names a country with no identity");
                read = false;
                continue;
            };
            observed.insert(self.key(slot, Field::CivilWarRival(rival)), 1);
            listed.extend([slot, rival]);
        }
        let predicate = dashboard_text::identity::CivilWars::new(frame);
        for &slot in &listed {
            match predicate
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|predicate| predicate.holds(slot))
            {
                Ok(true) => {
                    observed.insert(self.key(slot, Field::CivilWar), 1);
                }
                Ok(false) => {}
                Err(error) => {
                    self.refuse(format!("civil-war status unavailable: {error}"));
                    read = false;
                }
            }
        }
        self.listed.extend(listed);
        let mut unknown = false;
        let existing: Vec<u32> = self
            .listed
            .iter()
            .copied()
            .filter(|&slot| {
                match predicate
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|predicate| predicate.exists(slot))
                {
                    Ok(exists) => exists,
                    Err(_) => {
                        unknown = true;
                        false
                    }
                }
            })
            .collect();
        if unknown {
            self.refuse("civil-war side existence not recorded");
        }
        for slot in existing {
            observed.insert(self.key(slot, Field::Exists), 1);
        }
        read
    }

    pub fn bytes(&self) -> usize {
        self.values.bytes().saturating_add(if self.catalogue_pending {
            series::catalogue_bytes(&self.keys, &self.metadata)
        } else {
            0
        })
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.values
            .flush(writer, &values_section(branch), 0, cancelled)?;
        series::write_catalogue(writer, FAMILY, branch, &self.keys, &self.metadata)?;
        self.catalogue_pending = false;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PoliticsHistoryMode {
    /// One country's balances over the whole branch.
    Country,
    /// Every election schedule over the whole branch.
    Elections,
    /// Every government in exile over the whole branch.
    Exiles,
    /// Every holder's balances over the whole branch.
    Balances,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsHistoryParams {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    /// The country, for [`PoliticsHistoryMode::Country`].
    #[serde(default)]
    #[ts(optional)]
    pub tag: Option<String>,
    /// The window [`PoliticsHistoryMode::Exiles`] answers, both ends included.
    #[serde(default)]
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[serde(default)]
    #[ts(optional)]
    pub last_hour: Option<i32>,
}


/// Consecutive reads holding one value, as an exact decimal string.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: String,
}

/// A holder's tag from `hour` until its next entry.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderTag {
    pub hour: i32,
    pub tag: String,
}

/// One holder's election schedule over the branch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ElectionHistory {
    pub tags: Vec<HolderTag>,
    /// "1" when the game holds elections, "0" when it does not.
    pub held: Vec<PoliticsRun>,
    /// "1" while its schedule was read but its eligibility was not: unknown, not denied.
    pub refused: Vec<PoliticsRun>,
    /// Signed months.
    pub interval: Vec<PoliticsRun>,
    /// The next election's hour.
    pub next: Vec<PoliticsRun>,
    /// The term's start: the stored last election's hour, where it is a date.
    pub last: Vec<PoliticsRun>,
}

/// One holder's exile over the branch: each field's runs while a positive
/// host was read, each independent of the others.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExileHistory {
    pub tags: Vec<HolderTag>,
    /// The host's holder id.
    pub host: Vec<PoliticsRun>,
    /// Fixed5 legitimacy.
    pub legitimacy: Vec<PoliticsRun>,
    pub manpower: Vec<PoliticsRun>,
    pub cap: Vec<PoliticsRun>,
    pub leaders: Vec<PoliticsRun>,
    /// The hour the exile's resources were last read.
    pub read: Vec<PoliticsRun>,
}

/// One holder's balances over the branch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderBalances {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    pub balances: Vec<BalanceHistory>,
    /// The balance the game shows the holder, by identity token.
    pub shown: Vec<PoliticsRun>,
}

/// One balance identity a country belonged to. Range and trending runs
/// hold content tokens.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceHistory {
    pub token: String,
    pub name: Option<String>,
    pub left: Option<BalanceSide>,
    pub right: Option<BalanceSide>,
    /// The authored ranges in authored order; empty when content refuses.
    pub ranges: Vec<BalanceRange>,
    pub values: Vec<PoliticsRun>,
    /// The record's stored position: holders sharing it at an hour share that
    /// record. Not an identity across hours.
    pub record: Vec<PoliticsRun>,
    pub range: Vec<PoliticsRun>,
    pub trending: Vec<PoliticsRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsHistoryReply {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    /// Through the last prepared hour; later hours are still being prepared.
    pub last_hour: Option<i32>,
    pub tag: Option<String>,
    /// The hours the mode's part was read; others are unknown.
    pub known: Vec<HourSpan>,
    pub elections: Vec<ElectionHistory>,
    /// Content tokens to their in-game names, where they resolve.
    pub names: BTreeMap<String, String>,
    /// Every government in exile over the branch.
    pub exile_history: Vec<ExileHistory>,
    /// Each holder id's tags, for the holders runs name.
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    /// Every holder's balances over the branch; for a country, every holder
    /// that held its tag, whole.
    pub holder_balances: Vec<HolderBalances>,
    pub warnings: Vec<String>,
}

fn holder_tags(runs: &[(i32, String)]) -> Vec<HolderTag> {
    runs.iter()
        .map(|(hour, tag)| HolderTag {
            hour: *hour,
            tag: tag.clone(),
        })
        .collect()
}

/// A holder's balance runs by identity (value, range and trending), and the
/// runs of the balance shown, gathered over `spans`.
#[derive(Default)]
struct Gathered {
    balances: BTreeMap<String, [Vec<series::Run>; 4]>,
    shown: Vec<series::Run>,
}

impl Gathered {
    fn gather(
        &mut self,
        reader: &mut Reader<'_>,
        fields: &[(u64, &Field)],
        spans: &[(i32, i32)],
        through: i32,
    ) -> io::Result<()> {
        for &(id, field) in fields {
            let (balance, part) = match field {
                Field::BalanceValue(balance) => (balance, 0),
                Field::BalanceRange(balance) => (balance, 1),
                Field::BalanceTrending(balance) => (balance, 2),
                Field::BalanceRecord(balance) => (balance, 3),
                Field::BalanceShown => {
                    self.shown
                        .extend(clip(reader.runs(id, i32::MIN, through)?, spans, through));
                    continue;
                }
                _ => continue,
            };
            self.balances.entry(balance.clone()).or_default()[part]
                .extend(clip(reader.runs(id, i32::MIN, through)?, spans, through));
        }
        Ok(())
    }

    /// The balances in identity order, with names and authored presentation,
    /// and the shown runs by identity token.
    fn finish(
        self,
        metadata: &Metadata,
        presentation: &Presentation<'_>,
        names: &mut BTreeMap<String, String>,
        refused: &mut u64,
    ) -> (Vec<BalanceHistory>, Vec<PoliticsRun>) {
        let token_text = |value: i128| {
            usize::try_from(value)
                .ok()
                .and_then(|index| metadata.tokens.get(index))
                .cloned()
        };
        let sorted = |mut runs: Vec<series::Run>| {
            runs.sort_by_key(|run| run.first_hour);
            runs
        };
        let mut balances = Vec::new();
        for (token, [values, range, trending, record]) in self.balances {
            let (mut left, mut right, mut ranges) = presentation.authored(&token);
            presentation.name(&token, names, refused);
            present(presentation, names, refused, &mut left, &mut right, &mut ranges);
            let range = text_runs(sorted(range), token_text);
            let trending = text_runs(sorted(trending), token_text);
            for run in range.iter().chain(&trending) {
                presentation.name(&run.value, names, refused);
            }
            balances.push(BalanceHistory {
                name: names.get(&token).cloned(),
                values: text_runs(sorted(values), |value| Some(value.to_string())),
                record: text_runs(sorted(record), |value| Some(value.to_string())),
                range,
                trending,
                left,
                right,
                ranges,
                token,
            });
        }
        let shown = text_runs(sorted(self.shown), |value| {
            u64::try_from(value)
                .ok()
                .and_then(|id| metadata.fields.get(&id))
                .and_then(|(_, field)| match field {
                    Field::BalanceValue(balance) => Some(balance.clone()),
                    _ => None,
                })
        });
        (balances, shown)
    }
}

/// The hours `part`'s tables were read, `from..=through`.
pub(crate) fn known(
    reader: &mut Reader<'_>,
    metadata: &Metadata,
    part: Part,
    from: i32,
    through: i32,
) -> io::Result<Vec<HourSpan>> {
    let Some(&id) = metadata
        .fields
        .iter()
        .find_map(|(id, field)| (*field == (PART_SLOT, Field::Known(part))).then_some(id))
    else {
        return Ok(Vec::new());
    };
    Ok(reader
        .runs(id, from, through)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
        })
        .collect())
}

pub(crate) fn tag_at(runs: &[(i32, String)], hour: i32) -> Option<&str> {
    runs.iter()
        .rev()
        .find(|(from, _)| *from <= hour)
        .map(|(_, tag)| tag.as_str())
}

fn clip(runs: Vec<series::Run>, spans: &[(i32, i32)], through: i32) -> Vec<series::Run> {
    let mut out = Vec::new();
    for run in runs {
        for &(from, to) in spans {
            let first = run.first_hour.max(from);
            let last = run.last_hour.min(to).min(through);
            if first <= last {
                out.push(series::Run::new(first, last, run.value, run.break_before));
            }
        }
    }
    out.sort_by_key(|run| run.first_hour);
    out
}

fn text_runs(runs: Vec<series::Run>, text: impl Fn(i128) -> Option<String>) -> Vec<PoliticsRun> {
    runs.into_iter()
        .filter_map(|run| {
            Some(PoliticsRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                value: text(run.value)?,
            })
        })
        .collect()
}

/// Names and authored presentation from the recording's content stack.
struct Presentation<'a> {
    catalogue: Option<&'a Catalogue>,
    balances: Option<&'a installed_content_core::power_balance::Database>,
}
impl Presentation<'_> {
    fn name(&self, token: &str, names: &mut BTreeMap<String, String>, refused: &mut u64) {
        if token.is_empty() || names.contains_key(token) {
            return;
        }
        match self
            .catalogue
            .and_then(|catalogue| catalogue.localised(token, View::Plain))
        {
            Some(text) if !(text.contains('[') && text.contains(']')) => {
                names.insert(token.to_owned(), text);
            }
            _ if self.catalogue.is_some() => *refused += 1,
            _ => {}
        }
    }
    fn side(&self, token: &[u8]) -> BalanceSide {
        let token = String::from_utf8_lossy(installed_content_core::parser::c_string(token)).into_owned();
        BalanceSide {
            name: None,
            token,
        }
    }
    fn authored(
        &self,
        token: &str,
    ) -> (Option<BalanceSide>, Option<BalanceSide>, Vec<BalanceRange>) {
        let Some(balances) = self.balances else {
            return (None, None, Vec::new());
        };
        let definition = balances.definition(token.as_bytes());
        let ranges = balances
            .ranges(token.as_bytes())
            .unwrap_or_default()
            .iter()
            .map(|range| BalanceRange {
                token: String::from_utf8_lossy(installed_content_core::parser::c_string(&range.id))
                    .into_owned(),
                name: None,
                min: Some(range.min.to_string()),
                max: Some(range.max.to_string()),
            })
            .collect();
        (
            definition.map(|definition| self.side(&definition.left)),
            definition.map(|definition| self.side(&definition.right)),
            ranges,
        )
    }
}

/// Fills every side and range name from `names`, gathering them first.
fn present(
    presentation: &Presentation<'_>,
    names: &mut BTreeMap<String, String>,
    refused: &mut u64,
    left: &mut Option<BalanceSide>,
    right: &mut Option<BalanceSide>,
    ranges: &mut [BalanceRange],
) {
    for side in [left.as_mut(), right.as_mut()].into_iter().flatten() {
        presentation.name(&side.token, names, refused);
        side.name = names.get(&side.token).cloned();
    }
    for range in ranges {
        presentation.name(&range.token, names, refused);
        range.name = names.get(&range.token).cloned();
    }
}

/// The history `params` asks for, through `through`, the last prepared hour.
pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &PoliticsHistoryParams,
    through: Option<i32>,
    catalogue: Option<Result<Arc<Catalogue>, String>>,
    cancelled: &AtomicBool,
) -> io::Result<PoliticsHistoryReply> {
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid politics history branch"))?;
    let mut reply = PoliticsHistoryReply {
        branch_id: params.branch_id.clone(),
        mode: params.mode,
        last_hour: through,
        tag: params.tag.clone(),
        known: Vec::new(),
        elections: Vec::new(),
        names: BTreeMap::new(),
        exile_history: Vec::new(),
        holders: BTreeMap::new(),
        holder_balances: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        return Ok(reply);
    };
    let Some((_, metadata)) = series::read_catalogue::<Metadata>(store, key, FAMILY, branch)?
    else {
        return Ok(reply);
    };
    let mut reader = Reader::new(store, key, values_section(branch), cancelled)?;
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err(invalid("Politics history request cancelled"))
        } else {
            Ok(())
        }
    };
    let catalogue = match catalogue {
        Some(Ok(catalogue)) => Some(catalogue),
        Some(Err(error)) => {
            reply.warnings.push(format!("1 refused balance names: {error}"));
            None
        }
        None => None,
    };
    let effects = catalogue
        .as_deref()
        .map(|catalogue| catalogue.scripted_content().and_then(|content| content.effects()));
    if let Some(Err(error)) = &effects {
        reply
            .warnings
            .push(format!("1 refused balance ranges: {error}"));
    }
    let presentation = Presentation {
        catalogue: catalogue.as_deref(),
        balances: effects
            .as_ref()
            .and_then(|effects| effects.as_ref().ok())
            .and_then(|effects| effects.view(None).power_balances),
    };
    let mut by_slot = BTreeMap::<u32, Vec<(u64, &Field)>>::new();
    for (&id, (slot, field)) in &metadata.fields {
        by_slot.entry(*slot).or_default().push((id, field));
    }
    let part = match params.mode {
        PoliticsHistoryMode::Elections => Part::Elections,
        PoliticsHistoryMode::Exiles => Part::Exiles,
        PoliticsHistoryMode::Country | PoliticsHistoryMode::Balances => Part::Balances,
    };
    // The exile history is one window at a time: every exile's runs over a
    // whole branch outgrow a reply.
    let (from, through) = match params.mode {
        PoliticsHistoryMode::Exiles => match (params.first_hour, params.last_hour) {
            (Some(first), Some(last)) if first <= last => (first, last.min(through)),
            _ => return Err(invalid("An exile history names its window")),
        },
        _ => (i32::MIN, through),
    };
    let window = [(from, through)];
    reply.known = known(&mut reader, &metadata, part, from, through)?;
    let mut refused_names = 0_u64;
    match params.mode {
        PoliticsHistoryMode::Elections => {
            for (&slot, tags) in &metadata.tags {
                check()?;
                let mut history = ElectionHistory {
                    tags: holder_tags(tags),
                    held: Vec::new(),
                    refused: Vec::new(),
                    interval: Vec::new(),
                    next: Vec::new(),
                    last: Vec::new(),
                };
                for &(id, field) in by_slot.get(&slot).into_iter().flatten() {
                    let target = match field {
                        Field::ElectionsHeld => &mut history.held,
                        Field::ElectionsRefused => &mut history.refused,
                        Field::ElectionInterval => &mut history.interval,
                        Field::NextElection => &mut history.next,
                        Field::LastElection => &mut history.last,
                        _ => continue,
                    };
                    *target = text_runs(reader.runs(id, i32::MIN, through)?, |value| {
                        Some(value.to_string())
                    });
                }
                if !history.held.is_empty()
                    || !history.refused.is_empty()
                    || !history.interval.is_empty()
                {
                    reply.elections.push(history);
                }
            }
        }
        PoliticsHistoryMode::Country => {
            let tag = params
                .tag
                .as_deref()
                .ok_or_else(|| invalid("A country history names its country"))?;
            // Every holder that held the tag, whole: the caller picks the one
            // holding it at its hour and follows that holder through renames.
            for (&slot, tags) in &metadata.tags {
                check()?;
                if !tags.iter().any(|(_, held)| held == tag) {
                    continue;
                }
                let fields = by_slot.get(&slot).map(Vec::as_slice).unwrap_or_default();
                let mut gathered = Gathered::default();
                gathered.gather(&mut reader, fields, &window, through)?;
                let (balances, shown) =
                    gathered.finish(&metadata, &presentation, &mut reply.names, &mut refused_names);
                reply.holder_balances.push(HolderBalances {
                    holder: slot.to_string(),
                    tags: holder_tags(tags),
                    balances,
                    shown,
                });
            }
        }
        PoliticsHistoryMode::Balances => {
            for (&slot, fields) in &by_slot {
                check()?;
                if !fields.iter().any(|(_, field)| matches!(field, Field::BalanceValue(_))) {
                    continue;
                }
                let mut gathered = Gathered::default();
                gathered.gather(&mut reader, fields, &window, through)?;
                let (balances, shown) =
                    gathered.finish(&metadata, &presentation, &mut reply.names, &mut refused_names);
                reply.holder_balances.push(HolderBalances {
                    holder: slot.to_string(),
                    tags: holder_tags(metadata.tags.get(&slot).map(Vec::as_slice).unwrap_or_default()),
                    balances,
                    shown,
                });
            }
        }
        PoliticsHistoryMode::Exiles => {
            for (&slot, fields) in &by_slot {
                check()?;
                if !fields.iter().any(|(_, field)| **field == Field::ExileHost) {
                    continue;
                }
                let mut history = ExileHistory {
                    tags: holder_tags(metadata.tags.get(&slot).map(Vec::as_slice).unwrap_or_default()),
                    host: Vec::new(),
                    legitimacy: Vec::new(),
                    manpower: Vec::new(),
                    cap: Vec::new(),
                    leaders: Vec::new(),
                    read: Vec::new(),
                };
                for &(id, field) in fields {
                    let target = match field {
                        Field::ExileHost => &mut history.host,
                        Field::ExileLegitimacy => &mut history.legitimacy,
                        Field::ExileManpower => &mut history.manpower,
                        Field::ExileCap => &mut history.cap,
                        Field::ExileLeaders => &mut history.leaders,
                        Field::ExileRead => &mut history.read,
                        _ => continue,
                    };
                    *target = text_runs(reader.runs(id, from, through)?, |value| {
                        Some(value.to_string())
                    });
                }
                if history.host.is_empty() {
                    continue;
                }
                for run in &history.host {
                    if let Ok(host) = run.value.parse::<u32>()
                        && let Some(tags) = metadata.tags.get(&host)
                    {
                        reply
                            .holders
                            .entry(run.value.clone())
                            .or_insert_with(|| holder_tags(tags));
                    }
                }
                reply.exile_history.push(history);
            }
        }
    }
    if refused_names != 0 {
        reply.warnings.push(format!(
            "{refused_names} refused balance names: no plain localisation in the recorded content"
        ));
    }
    for (reason, count) in &metadata.refusals {
        reply
            .warnings
            .push(format!("{count} refused political state reads: {reason}"));
    }
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// The prepared history's product metadata; its query is routed by key.
pub struct PoliticsStateHistory;
impl Product for PoliticsStateHistory {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = PoliticsHistoryParams;
    type Reply = PoliticsHistoryReply;
    fn types() -> Vec<String> {
        vec![
            PoliticsHistoryParams::decl(),
            PoliticsHistoryMode::decl(),
            PoliticsRun::decl(),
            HolderTag::decl(),
            ElectionHistory::decl(),
            ExileHistory::decl(),
            BalanceHistory::decl(),
            HolderBalances::decl(),
            PoliticsHistoryReply::decl(),
        ]
    }
    fn validate(p: &PoliticsHistoryParams) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid politics history branch")?;
        match p.mode {
            PoliticsHistoryMode::Country => {
                dashboard_base::products::validate_tag(p.tag.as_deref().ok_or("Missing country")?)
            }
            PoliticsHistoryMode::Exiles => match (p.first_hour, p.last_hour) {
                (Some(first), Some(last)) if first <= last => Ok(()),
                _ => Err("An exile history names its window"),
            },
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
