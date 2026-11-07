//! The one branch scan of every country's production output, shared by units
//! delivered and exact applied equipment-line IC. No product repeats the walk.

#[path = "production/rocket.rs"]
mod rocket;
pub(crate) use rocket::{RocketFold, RocketProduced, RocketRead, RocketTotals};

use replay_frame::frame::DecodedFrame;
use super::{self as equipment, Domain, Texts, VariantCatalogue};
use crate::products::LoadContext;
use crate::{
    Source, catalogue,
    metadata::{TaggedCountry, link_slot, slot_read, tagged_countries, validate_campaign},
};
use replay::campaign::Replay;
use replay_data::{rows::Recorded, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;


/// A country identity within one observation: its canonical slot, native id and
/// the observation (a restart begins another, whose totals start again).
pub(crate) type Identity = (u32, u32, u32);
/// What a country produced: a variant, and what the input was for.
pub(crate) type Entry = (u64, Purpose);

/// The countries production reads at an hour: each UTF-8 tag's (canonical slot, id) where
/// the links resolve a holder, the canonical slot each raw link id names, and the resolved
/// tags that are not UTF-8.
#[derive(Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CountryHolders {
    pub holders: BTreeMap<String, Option<(u32, u32)>>,
    pub slots: Arc<HashMap<u32, u32>>,
    pub unnamed: usize,
}

impl CountryHolders {
    /// `countries` resolved through `slots`, as [`crate::products::territory::CountryReads`]
    /// resolves them.
    pub fn of(countries: &[replay_data::rows::CountryRow], slots: Arc<HashMap<u32, u32>>) -> Self {
        let tagged = crate::metadata::tagged_countries_indexed(countries, &slots);
        let mut holders = BTreeMap::new();
        let mut unnamed = 0;
        for (tag, lookup) in tagged {
            match std::str::from_utf8(tag) {
                Ok(tag) => {
                    holders.insert(
                        tag.to_owned(),
                        lookup.found().map(|(country, slot)| (slot, country.id)),
                    );
                }
                Err(_) => unnamed += usize::from(matches!(lookup, TaggedCountry::Found(..))),
            }
        }
        Self {
            holders,
            slots,
            unnamed,
        }
    }

    /// What `reads` resolves, `unnamed` of its resolved tags not UTF-8.
    pub(crate) fn read(reads: &crate::products::territory::CountryReads, unnamed: usize) -> Self {
        Self {
            holders: reads.holders().clone(),
            slots: Arc::clone(reads.slots()),
            unnamed,
        }
    }
}

/// A table's rows in key order: a row frame's, or those a columnar walk keeps by key.
pub(crate) enum Rows<'a, R> {
    Slice(&'a [R]),
    Keyed(&'a BTreeMap<u128, R>),
}

impl<R> Clone for Rows<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for Rows<'_, R> {}

impl<'a, R> Rows<'a, R> {
    pub(crate) fn iter(self) -> RowsIter<'a, R> {
        match self {
            Self::Slice(rows) => RowsIter::Slice(rows.iter()),
            Self::Keyed(rows) => RowsIter::Keyed(rows.values()),
        }
    }
}

pub(crate) enum RowsIter<'a, R> {
    Slice(std::slice::Iter<'a, R>),
    Keyed(std::collections::btree_map::Values<'a, u128, R>),
}

impl<R> Clone for RowsIter<'_, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Slice(rows) => Self::Slice(rows.clone()),
            Self::Keyed(rows) => Self::Keyed(rows.clone()),
        }
    }
}

impl<'a, R> Iterator for RowsIter<'a, R> {
    type Item = &'a R;

    fn next(&mut self) -> Option<&'a R> {
        match self {
            Self::Slice(rows) => rows.next(),
            Self::Keyed(rows) => rows.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Slice(rows) => rows.size_hint(),
            Self::Keyed(rows) => rows.size_hint(),
        }
    }
}

impl<R> ExactSizeIterator for RowsIter<'_, R> {}

/// What a production tick's applied input was for, as the collector recorded
/// it. Input whose purpose was not recorded is its own entry (`Unknown`): a
/// reply split by purpose never attributes it to another purpose, and replies
/// that do not split count it with new production, as Equipment produced
/// does. Its magnitudes are kept as unproven evidence
/// ([`CostTotal::unproven_raw`]), which never cancels.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize, TS,
)]
#[derive(replay_spool::Spooled)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ProductionPurpose")]
pub enum Purpose {
    #[default]
    New,
    /// Ship refit production.
    Refit,
    /// Military production while its line converted stockpiled equipment
    /// into its design: conversion progress, including ticks that completed
    /// nothing, not a count of source units removed.
    Converted,
    /// The collector could not classify the tick, or the recording predates
    /// the purpose.
    Unknown,
}

impl Purpose {
    pub const ALL: [Self; 4] = [Self::New, Self::Refit, Self::Converted, Self::Unknown];

    pub const fn index(self) -> usize {
        self as usize
    }

    /// A row's recorded purpose.
    pub(crate) fn of(row: &replay_data::rows::ProductionLineOutputRow) -> Self {
        use replay_data::rows::OutputPurpose;
        match row.purpose() {
            OutputPurpose::New => Self::New,
            OutputPurpose::Refit => Self::Refit,
            OutputPurpose::Converted => Self::Converted,
            OutputPurpose::Unknown => Self::Unknown,
        }
    }
}

/// A country's total of one entry: its units and how many of its rows are not
/// counted exactly (the total is exact only at zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Total {
    pub units: i128,
    pub inexact: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProductionSpendingUnknown {
    NotRecorded,
    SpendingUnavailable,
    LostOutput,
    MissedPublication,
    RecordingGap,
    ObservationRestarted,
    ObservationMismatch,
    CountryChanged,
    RocketTableUnavailable,
    RocketInputInexact,
    RocketOwnerUnreadable,
    RocketDesignUnreadable,
    /// The value needs earlier observations than the prepared prefix it was
    /// answered from reaches; preparation supplies it later.
    Preparing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct CostTotal {
    pub raw: i128,
    pub reason: Option<ProductionSpendingUnknown>,
    /// The input of production ticks whose purpose was not recorded (only an
    /// Unknown entry's), as the sum of its magnitudes: it grows with each such
    /// input and never cancels.
    pub unproven_raw: i128,
}

/// One tag's reads of the output.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TagHistory {
    /// Each sample's read hour, ascending, and the last hour it holds.
    pub hours: Vec<i32>,
    pub last_hours: Vec<i32>,
    /// Whether a sample starts a new line.
    pub starts: Vec<bool>,
    pub identities: Vec<Identity>,
    pub(crate) presence: Vec<Option<bool>>,
    /// Each entry's total at the samples where it changed.
    #[serde(with = "crate::prepared_production::map_pairs")]
    pub changes: BTreeMap<Entry, Vec<(u32, Total)>>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    pub(crate) cost_changes: BTreeMap<Entry, Vec<(u32, CostTotal)>>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    pub(crate) current: BTreeMap<Entry, Total>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    pub(crate) cost_current: BTreeMap<Entry, CostTotal>,
    pub(crate) cost_taken: Option<(Identity, u64)>,
    /// The identity and totals version the last sample took.
    pub(crate) taken: Option<(Identity, u64)>,
    /// The identity and hour the last sample was extended to.
    pub(crate) last: Option<(Identity, i32)>,
}

/// Preparation's readers of a track's inventories, each taking its changes in turn.
pub(crate) const SPENT_READER: usize = 0;
pub(crate) const INVENTORY_READER: usize = 1;
pub(crate) const ROWS_READER: usize = 2;
const READERS: usize = 3;
/// A log longer than this is dropped, and every reader reads the inventories whole.
const EDITS_HELD: usize = 1 << 12;

/// The entries a track's inventories changed at, logged once for every reader: each takes
/// those logged since it last took them, or reads both inventories whole where they were
/// replaced. A reader that has not taken them yet reads them whole too.
#[derive(Clone, Debug)]
pub(crate) struct Edits {
    log: Vec<Entry>,
    taken: [usize; READERS],
    whole: [bool; READERS],
}

impl Default for Edits {
    fn default() -> Self {
        Self {
            log: Vec::new(),
            taken: [0; READERS],
            whole: [true; READERS],
        }
    }
}

impl Edits {
    /// `entry` changed in either inventory.
    pub(crate) fn note(&mut self, entry: Entry) {
        if self.log.len() >= EDITS_HELD {
            self.replaced();
            return;
        }
        self.log.push(entry);
    }

    /// Both inventories were replaced: every reader reads them whole.
    pub(crate) fn replaced(&mut self) {
        self.log.clear();
        self.taken = [0; READERS];
        self.whole = [true; READERS];
    }

    /// The entries changed since `reader` last took them, ascending and once each, in `out`;
    /// true when it must read the inventories whole instead.
    pub(crate) fn take(&mut self, reader: usize, out: &mut Vec<Entry>) -> bool {
        out.clear();
        out.extend_from_slice(&self.log[self.taken[reader]..]);
        self.taken[reader] = self.log.len();
        out.sort_unstable();
        out.dedup();
        if self.taken.iter().all(|&taken| taken == self.log.len()) {
            self.log.clear();
            self.taken = [0; READERS];
        }
        std::mem::take(&mut self.whole[reader])
    }
}

/// One branch's scan.
#[derive(Clone, Debug, Default)]
pub struct ScannedProduced {
    pub tags: BTreeMap<String, TagHistory>,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub variants: VariantCatalogue,
    /// Rows whose producer the links could not resolve to a tagged country.
    pub unattributed: usize,
    /// Rows the collector could not count exactly.
    pub inexact: usize,
    /// Rows that disappeared without their final count.
    pub lost: usize,
    /// Lines with output whose purpose was not recorded.
    pub unlisted: usize,
    /// Reads of the production lines without a readable output, and why.
    pub unread: usize,
    pub unread_reason: Option<String>,
    /// Tags that are not text.
    pub unnamed: usize,
    /// The hour from which each observation's totals are unknown, by observation: a
    /// frame that could hold an output publication could not be ingested (its block
    /// or frame unreadable, a read whose country, link or output table is not
    /// complete, or a row whose producer cannot be told once it holds units). The
    /// collector drops an ended line's rows after one written publication, so what
    /// such a publication held is lost for good: no total is resumed after it.
    pub tainted: BTreeMap<u32, i32>,
    /// Frames and reads that could not be ingested.
    pub missed: usize,
    pub(crate) spending_tainted: BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    pub(crate) rocket: RocketProduced,
}

impl ScannedProduced {
    /// Marks every total of the current observation unknown from `hour` on.
    fn taint(&mut self, observation: u32, hour: i32) {
        self.missed += 1;
        self.tainted.entry(observation).or_insert(hour);
        self.spending_taint(
            observation,
            hour,
            ProductionSpendingUnknown::MissedPublication,
        );
    }

    fn spending_taint(&mut self, observation: u32, hour: i32, reason: ProductionSpendingUnknown) {
        self.spending_tainted
            .entry(observation)
            .and_modify(|first| {
                if hour < first.0 {
                    *first = (hour, reason);
                }
            })
            .or_insert((hour, reason));
    }
}

struct RowState {
    identity: Option<Identity>,
    entry: Entry,
    /// Its purpose was recorded.
    proved: bool,
    units: i64,
    exact: bool,
    ended: bool,
    spent_raw: i64,
}

/// Adds a row's new count to its producer's total.
fn apply(
    totals: &mut HashMap<Identity, (u64, BTreeMap<Entry, Total>)>,
    version: &mut u64,
    state: &RowState,
    units: i64,
    exact: bool,
) {
    let Some(identity) = state.identity else {
        return;
    };
    if units == state.units && exact == state.exact {
        return;
    }
    *version += 1;
    let (changed, entries) = totals.entry(identity).or_default();
    *changed = *version;
    let total = entries.entry(state.entry).or_default();
    total.units += i128::from(units) - i128::from(state.units);
    total.inexact = total.inexact + u32::from(!exact) - u32::from(!state.exact);
}

fn apply_cost(
    totals: &mut HashMap<Identity, (u64, BTreeMap<Entry, CostTotal>)>,
    version: &mut u64,
    state: &RowState,
    delta: i128,
    reason: Option<ProductionSpendingUnknown>,
) -> io::Result<()> {
    let Some(identity) = state.identity else {
        return Ok(());
    };
    if delta == 0 && reason.is_none() {
        return Ok(());
    }
    *version += 1;
    let (changed, entries) = totals.entry(identity).or_default();
    *changed = *version;
    let total = entries.entry(state.entry).or_default();
    total.reason = total.reason.or(reason);
    if total.reason.is_none() {
        total.raw = total
            .raw
            .checked_add(delta)
            .ok_or_else(|| io::Error::other("Applied IC exceeds its exact integer range"))?;
        // Magnitudes, so corrections between lines never cancel the evidence.
        if !state.proved {
            total.unproven_raw = delta
                .checked_abs()
                .and_then(|delta| total.unproven_raw.checked_add(delta))
                .ok_or_else(|| io::Error::other("Applied IC exceeds its exact integer range"))?;
        }
    }
    Ok(())
}

/// Scans one branch once for every country.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedProduced> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantChanges,
        TableId::VariantModules,
        TableId::VariantUpgrades,
        TableId::ProductionLines,
        TableId::ProductionLineOutput,
        TableId::RocketProductionOutput,
    ];
    let cancel = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "Equipment produced request cancelled",
        )
    };
    let mut result = ScannedProduced::default();
    let mut rows: HashMap<(u64, u32), RowState> = HashMap::new();
    // Rows still expected at the next read, including independently known IC
    // when their delivery count is inexact.
    let mut live: HashSet<(u64, u32)> = HashSet::new();
    let mut totals: HashMap<Identity, (u64, BTreeMap<Entry, Total>)> = HashMap::new();
    let mut cost_totals: HashMap<Identity, (u64, BTreeMap<Entry, CostTotal>)> = HashMap::new();
    let mut seen_production: HashSet<Identity> = HashSet::new();
    let mut unlisted: HashSet<u64> = HashSet::new();
    let (mut version, mut observation) = (0u64, 0u32);
    let mut cost_version = 0;
    // The introduction ledger also authenticates exact zero in an empty
    // publication; absent suffixes must not become an old recording's zero.
    let mut spending_recorded = TableId::ProductionLineOutput
        .last_change()
        .appended
        .iter()
        .any(|field| field.name == "spent_raw" && replay.format.version() >= field.introduced);
    let mut last_frame = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .and_then(|start| start.first_hour.checked_sub(1));
    if let Some(gap) = replay
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch)
        .min_by_key(|gap| gap.first_hour)
    {
        result.spending_taint(0, gap.first_hour, ProductionSpendingUnknown::RecordingGap);
    }
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                // A block or frame that cannot be read may have held a publication.
                let Ok(frames) = frames else {
                    result.taint(observation, block.first_hour);
                    for track in result.tags.values_mut() {
                        track.last = None;
                    }
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let decoded = match Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) {
                        Ok(decoded) if validate_campaign(decoded, replay).is_ok() => decoded,
                        _ => {
                            result.taint(observation, frame.hour);
                            for track in result.tags.values_mut() {
                                track.last = None;
                            }
                            break;
                        }
                    };
                    let hour = frame.hour;
                    // Rocket rows retain every country/design total; they do not depend
                    // on a registered line or on the equipment census being readable.
                    result.rocket.observe(replay.format, decoded)?;
                    if let Some(last) = last_frame
                        && last.checked_add(1) != Some(hour)
                    {
                        result.spending_taint(
                            observation,
                            last.saturating_add(1),
                            ProductionSpendingUnknown::RecordingGap,
                        );
                    }
                    last_frame = Some(hour);
                    result.variants.observe(replay.format, decoded);
                    let read = slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::ProductionLineOutput,
                        decoded.hour,
                    );
                    // An hour the tables cannot tell extends nothing, so nothing continues across it.
                    let (Ok(countries), Ok(links), Ok(output)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::ProductionLineOutput>(),
                    ) else {
                        if read {
                            // The publication this read held, if any, is not ingested: whatever
                            // lines ended in it are lost, for a producer that cannot be told.
                            result.taint(observation, hour);
                            if decoded.complete::<tables::ProductionLines>().is_ok() {
                                result.unread += 1;
                            }
                            if result.unread_reason.is_none()
                                && let TableView::Unavailable { reason } =
                                    &*decoded.tables[TableId::ProductionLineOutput.index()]
                            {
                                result.unread_reason = Some(reason.clone());
                            }
                        }
                        for track in result.tags.values_mut() {
                            track.last = None;
                        }
                        continue;
                    };
                    let tagged: Vec<(String, Identity)> = tagged_countries(countries, links)
                        .into_iter()
                        .filter_map(|(tag, lookup)| {
                            let TaggedCountry::Found(country, slot) = lookup else {
                                return None;
                            };
                            match std::str::from_utf8(tag) {
                                Ok(tag) => Some((tag.to_owned(), (slot, country.id, observation))),
                                Err(_) => {
                                    result.unnamed += 1;
                                    None
                                }
                            }
                        })
                        .collect();
                    if !read {
                        // Carried: the last read still holds, only for the same identity.
                        let present: HashMap<&str, Identity> = tagged
                            .iter()
                            .map(|(tag, identity)| (tag.as_str(), *identity))
                            .collect();
                        for (tag, track) in &mut result.tags {
                            let continues = present.get(tag.as_str()).copied().filter(|identity| {
                                track.last.is_some_and(|(seen, at)| {
                                    seen == *identity && at.checked_add(1) == Some(hour)
                                })
                            });
                            track.last = continues.map(|identity| (identity, hour));
                            if continues.is_some()
                                && let Some(last) = track.last_hours.last_mut()
                            {
                                *last = hour;
                            }
                        }
                        continue;
                    }
                    result.reads.push(hour);
                    // A new observation restarts every count; totals never cross it.
                    if let Some(since) = output.iter().map(|row| row.since_hour).min() {
                        match result.observations.last() {
                            None => result.observations.push(since),
                            Some(current) if *current != since => {
                                result.observations.push(since);
                                observation += 1;
                                rows.clear();
                                live.clear();
                                totals.clear();
                                cost_totals.clear();
                                seen_production.clear();
                            }
                            Some(_) => {}
                        }
                    }
                    // Explicit suffix values also authenticate provisional source
                    // fixtures; empty publications require the introduction ledger.
                    spending_recorded |= output
                        .iter()
                        .any(|row| !matches!(row.spent_raw, Recorded::NotRecorded));
                    if !spending_recorded {
                        result.spending_taint(
                            observation,
                            hour,
                            ProductionSpendingUnknown::NotRecorded,
                        );
                    }
                    let observed: Vec<(String, Identity)> = tagged
                        .into_iter()
                        .map(|(tag, (slot, id, _))| (tag, (slot, id, observation)))
                        .collect();
                    let by_slot: HashMap<u32, Identity> = observed
                        .iter()
                        .map(|(_, identity)| (identity.0, *identity))
                        .collect();
                    let mut presence_unknown = false;
                    let census_complete =
                        if let Ok(lines) = decoded.complete::<tables::ProductionLines>() {
                            for line in lines {
                                if let Some(identity) = link_slot(links, line.country)
                                    .and_then(|slot| by_slot.get(&slot))
                                {
                                    seen_production.insert(*identity);
                                } else {
                                    presence_unknown = true;
                                }
                            }
                            true
                        } else {
                            false
                        };
                    let mut seen = HashSet::with_capacity(output.len());
                    for row in output {
                        let key = (row.line, row.ordinal);
                        seen.insert(key);
                        let purpose = Purpose::of(row);
                        let proved = purpose != Purpose::Unknown;
                        if !proved {
                            unlisted.insert(row.line);
                        }
                        let producer = link_slot(links, row.country)
                            .and_then(|slot| by_slot.get(&slot))
                            .copied();
                        let state = rows.entry(key).or_insert_with(|| {
                            if producer.is_none() {
                                result.unattributed += 1;
                            }
                            RowState {
                                identity: producer,
                                entry: (row.variant, purpose),
                                proved,
                                units: 0,
                                exact: true,
                                ended: false,
                                spent_raw: 0,
                            }
                        });
                        // A producer the links could not tell before may be told now, while
                        // nothing was delivered yet; units of a producer never told are lost to
                        // some country that cannot be named, so no total is known after them.
                        if state.identity.is_none() && state.units == 0 && state.exact {
                            state.identity = producer;
                        }
                        if let Some(identity) = state.identity {
                            seen_production.insert(identity);
                        } else {
                            presence_unknown = true;
                        }
                        if state.identity.is_none() && (row.units != 0 || !row.exact) {
                            result.taint(observation, hour);
                        }
                        if state.identity.is_none()
                            && !matches!(row.spent_raw, Recorded::Value(Some(0)))
                        {
                            result.spending_taint(
                                observation,
                                hour,
                                ProductionSpendingUnknown::SpendingUnavailable,
                            );
                        }
                        // Not exact once: a tick is left out of every later count.
                        let exact = state.exact && row.exact;
                        if state.exact && !exact {
                            result.inexact += 1;
                        }
                        // The observer authenticates the applied input's tuple. Census
                        // cost, delivered units and their exactness do not reprice it.
                        let (delta, spending_reason) = match row.spent_raw {
                            Recorded::Value(Some(value)) => {
                                (i128::from(value) - i128::from(state.spent_raw), None)
                            }
                            Recorded::Value(None) => {
                                (0, Some(ProductionSpendingUnknown::SpendingUnavailable))
                            }
                            Recorded::NotRecorded => {
                                (0, Some(ProductionSpendingUnknown::NotRecorded))
                            }
                        };
                        apply_cost(
                            &mut cost_totals,
                            &mut cost_version,
                            state,
                            delta,
                            spending_reason,
                        )?;
                        if let Recorded::Value(Some(value)) = row.spent_raw {
                            state.spent_raw = value;
                        }
                        apply(&mut totals, &mut version, state, row.units, exact);
                        state.units = row.units;
                        state.exact = exact;
                        state.ended |= row.ended;
                    }
                    // A row gone without its ended publication lost its final count.
                    for key in live.iter().filter(|key| !seen.contains(*key)) {
                        if let Some(state) = rows.get_mut(key) {
                            result.lost += usize::from(state.exact);
                            let units = state.units;
                            apply_cost(
                                &mut cost_totals,
                                &mut cost_version,
                                state,
                                0,
                                Some(ProductionSpendingUnknown::LostOutput),
                            )?;
                            apply(&mut totals, &mut version, state, units, false);
                            state.exact = false;
                        }
                    }
                    live = seen
                        .into_iter()
                        .filter(|key| rows.get(key).is_some_and(|state| !state.ended))
                        .collect();
                    // Each tag's sample: its country's totals at this read.
                    let present: HashSet<&str> =
                        observed.iter().map(|(tag, _)| tag.as_str()).collect();
                    for track in result
                        .tags
                        .iter_mut()
                        .filter(|(tag, _)| !present.contains(tag.as_str()))
                        .map(|(_, track)| track)
                    {
                        track.last = None;
                    }
                    for (tag, identity) in observed {
                        let track = result.tags.entry(tag).or_default();
                        let starts = !track.last.is_some_and(|(seen, at)| {
                            seen == identity && at.checked_add(1) == Some(hour)
                        });
                        let sample = u32::try_from(track.hours.len()).map_err(|_| {
                            io::Error::other(
                                "The production sample index exceeds its integer range",
                            )
                        })?;
                        track.hours.push(hour);
                        track.last_hours.push(hour);
                        track.starts.push(starts);
                        track.identities.push(identity);
                        track.presence.push(if seen_production.contains(&identity) {
                            Some(true)
                        } else if census_complete && !presence_unknown {
                            Some(false)
                        } else {
                            None
                        });
                        track.last = Some((identity, hour));
                        let (cost_changed, cost_entries) = cost_totals
                            .get(&identity)
                            .map_or((0, None), |(changed, entries)| (*changed, Some(entries)));
                        if track.cost_taken != Some((identity, cost_changed)) {
                            track.cost_taken = Some((identity, cost_changed));
                            let empty = BTreeMap::new();
                            let entries = cost_entries.unwrap_or(&empty);
                            let keys: BTreeSet<Entry> = entries
                                .keys()
                                .chain(track.cost_current.keys())
                                .copied()
                                .collect();
                            for entry in keys {
                                let now = entries.get(&entry).copied().unwrap_or_default();
                                if track.cost_current.get(&entry).copied().unwrap_or_default()
                                    != now
                                {
                                    track
                                        .cost_changes
                                        .entry(entry)
                                        .or_default()
                                        .push((sample, now));
                                }
                            }
                            track.cost_current = entries.clone();
                        }
                        let (changed, entries) = totals
                            .get(&identity)
                            .map_or((0, None), |(changed, entries)| (*changed, Some(entries)));
                        if track.taken == Some((identity, changed)) {
                            continue;
                        }
                        track.taken = Some((identity, changed));
                        let empty = BTreeMap::new();
                        let entries = entries.unwrap_or(&empty);
                        let keys: BTreeSet<Entry> = entries
                            .keys()
                            .chain(track.current.keys())
                            .copied()
                            .collect();
                        for entry in keys {
                            let now = entries.get(&entry).copied().unwrap_or_default();
                            if track.current.get(&entry).copied().unwrap_or_default() != now {
                                track.changes.entry(entry).or_default().push((sample, now));
                            }
                        }
                        track.current = entries.clone();
                    }
                }
            }
            Ok(())
        },
    )?;
    result.unlisted = unlisted.len();
    Ok(result)
}

/// A variant's type resolved through content.
#[derive(Clone, Debug)]
pub(crate) struct VariantType {
    /// The archetype key, or `UNRESOLVED_MARK` and its token (or its handle without one).
    pub key: String,
    /// Its place in the equipment database; none when unresolved.
    pub ordinal: Option<usize>,
    pub domain: Option<Domain>,
}

/// What a reply types and names with, from the recorded content stack: its
/// equipment types and its localisation. Either may be missing (no game folders,
/// or content that could not be read); then types stay unresolved and names
/// unknown, never assumed.
#[derive(Clone, Copy, Default)]
pub struct Named<'a> {
    pub units: Option<&'a installed_content_core::equipment::Database>,
    pub texts: Option<&'a dyn Texts>,
}

impl Named<'_> {
    /// A resolved type's localised name; none for an unresolved one.
    pub fn type_name(&self, key: &str) -> Option<String> {
        self.texts
            .filter(|_| !key.starts_with(equipment::UNRESOLVED_MARK))
            .and_then(|texts| texts.text(key))
    }
}

/// Each variant's type, why types could not be resolved, and how many variants
/// content does not type.
pub(crate) fn variant_types_for(
    catalogue: &VariantCatalogue,
    variants: impl IntoIterator<Item = u64>,
    named: Named,
) -> (HashMap<u64, VariantType>, BTreeSet<String>, usize) {
    let mut types = HashMap::new();
    let mut untyped = BTreeSet::new();
    let mut unresolved_count = 0;
    for variant in variants {
        let (kind, reason) = variant_type(catalogue.token(variant), variant, named);
        if let Some(reason) = reason {
            untyped.insert(reason);
            unresolved_count += 1;
        }
        types.insert(variant, kind);
    }
    (types, untyped, unresolved_count)
}

/// A variant's type from its recorded equipment token (empty without one),
/// and why content could not resolve it.
pub(crate) fn variant_type(
    token: &str,
    variant: u64,
    named: Named,
) -> (VariantType, Option<String>) {
    let resolved = match (named.units, token.is_empty()) {
        (_, true) => Err(format!(
            "variant {variant} has no recorded definition in this branch"
        )),
        (None, false) => Err(format!("{token}: content is unavailable")),
        (Some(units), false) => equipment::archetype_of(units, token),
    };
    match resolved {
        Ok((key, ordinal, domain)) => (
            VariantType {
                key,
                ordinal: Some(ordinal),
                domain: Some(domain),
            },
            None,
        ),
        // Kept apart under the shared mark (as the stockpile keys them): its
        // token, or its handle without one.
        Err(reason) => (
            VariantType {
                key: if token.is_empty() {
                    format!("{}{variant}", equipment::UNRESOLVED_MARK)
                } else {
                    format!("{}{token}", equipment::UNRESOLVED_MARK)
                },
                ordinal: None,
                domain: None,
            },
            Some(reason),
        ),
    }
}

/// How often a request waiting for another's scan of its branch checks whether it was cancelled.
const SCAN_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// Each branch's scan for this revision, and the branches a request is scanning now. The
/// lock is never held while scanning; a request missing a branch another is scanning
/// waits for that scan instead of starting its own.
struct Cache<T> {
    entries: BTreeMap<u64, Arc<T>>,
    scanning: BTreeSet<u64>,
    finished: Arc<std::sync::Condvar>,
    #[cfg(test)]
    waiting: usize,
}

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            scanning: BTreeSet::new(),
            finished: Arc::default(),
            #[cfg(test)]
            waiting: 0,
        }
    }
}

/// Frees a branch's scan when its scan ends in any way, so a waiter can take over.
struct Scanning<'a, T> {
    cache: &'a std::sync::Mutex<Cache<T>>,
    branch: u64,
}

impl<T> Drop for Scanning<'_, T> {
    fn drop(&mut self) {
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        cache.scanning.remove(&self.branch);
        cache.finished.notify_all();
    }
}

fn shared_scan<T>(
    cache: &std::sync::Mutex<Cache<T>>,
    branch: u64,
    cancelled: &AtomicBool,
    scan: impl FnOnce() -> io::Result<T>,
) -> io::Result<Arc<T>> {
    let mut held = cache.lock().unwrap_or_else(|error| error.into_inner());
    loop {
        if let Some(scanned) = held.entries.get(&branch) {
            return Ok(Arc::clone(scanned));
        }
        if held.scanning.insert(branch) {
            break;
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Equipment produced request cancelled",
            ));
        }
        let finished = Arc::clone(&held.finished);
        #[cfg(test)]
        {
            held.waiting += 1;
        }
        held = finished
            .wait_timeout(held, SCAN_WAIT)
            .unwrap_or_else(|error| error.into_inner())
            .0;
        #[cfg(test)]
        {
            held.waiting -= 1;
        }
    }
    drop(held);
    let scanning = Scanning { cache, branch };
    let scanned = Arc::new(scan()?);
    let mut held = cache.lock().unwrap_or_else(|error| error.into_inner());
    held.entries.insert(branch, Arc::clone(&scanned));
    drop(held);
    drop(scanning);
    Ok(scanned)
}

pub fn scanned(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedProduced>> {
    let state = source.state::<Cache<ScannedProduced>>();
    shared_scan(&state, branch, cancelled, || {
        scan(&source.replay(), branch, cancelled)
    })
}

#[cfg(test)]
mod scan_sharing_tests {
    use super::*;
    use std::sync::{Mutex, atomic::AtomicUsize, mpsc};

    fn waiting(cache: &Mutex<Cache<u32>>) -> usize {
        cache.lock().unwrap().waiting
    }

    #[test]
    fn concurrent_misses_share_one_scan() {
        let cache = Mutex::new(Cache::<u32>::default());
        let scans = AtomicUsize::new(0);
        let (release, released) = mpsc::channel::<()>();
        let never = AtomicBool::new(false);
        let (cache, scans, never) = (&cache, &scans, &never);
        std::thread::scope(|scope| {
            let first = job_priority::spawn_in(scope, move || {
                shared_scan(cache, 1, never, || {
                    scans.fetch_add(1, Ordering::SeqCst);
                    released.recv().unwrap();
                    Ok(7)
                })
            });
            while !cache.lock().unwrap().scanning.contains(&1) {
                std::thread::yield_now();
            }
            let second = job_priority::spawn_in(scope, move || {
                shared_scan(cache, 1, never, || {
                    scans.fetch_add(1, Ordering::SeqCst);
                    Ok(8)
                })
            });
            while waiting(cache) == 0 {
                std::thread::yield_now();
            }
            release.send(()).unwrap();
            assert_eq!(*first.join().unwrap().unwrap(), 7);
            assert_eq!(
                *second.join().unwrap().unwrap(),
                7,
                "the waiter reuses the first scan"
            );
        });
        assert_eq!(scans.load(Ordering::SeqCst), 1);
        assert!(cache.lock().unwrap().scanning.is_empty());
    }

    #[test]
    fn a_cancelled_scan_frees_its_branch_for_a_waiter() {
        let cache = Mutex::new(Cache::<u32>::default());
        let (release, released) = mpsc::channel::<()>();
        let never = AtomicBool::new(false);
        let (shared, never) = (&cache, &never);
        std::thread::scope(|scope| {
            let first = job_priority::spawn_in(scope, move || {
                shared_scan(shared, 1, never, || {
                    released.recv().unwrap();
                    Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
                })
            });
            while !cache.lock().unwrap().scanning.contains(&1) {
                std::thread::yield_now();
            }
            let second = job_priority::spawn_in(scope, move || shared_scan(shared, 1, never, || Ok(8)));
            while waiting(shared) == 0 {
                std::thread::yield_now();
            }
            release.send(()).unwrap();
            assert_eq!(
                first.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::Interrupted
            );
            assert_eq!(
                *second.join().unwrap().unwrap(),
                8,
                "the waiter takes the scan over"
            );
        });
        // A cancelled waiter stops waiting while another request scans.
        cache.lock().unwrap().scanning.insert(2);
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            shared_scan(&cache, 2, &cancelled, || Ok(9))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }
}

pub(crate) fn content(
    source: &Source,
    context: &LoadContext,
    warnings: &mut Vec<String>,
) -> (
    Option<Arc<catalogue::Catalogue>>,
    Option<Arc<installed_content_core::equipment::Database>>,
) {
    match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            let units = match equipment::units(source, &catalogue) {
                Ok(units) => {
                    warnings.extend(equipment::equipment_warnings(&units));
                    Some(units)
                }
                Err(error) => {
                    warnings.push(format!("Equipment types are unavailable: {error}"));
                    None
                }
            };
            (Some(catalogue), units)
        }
        Err(error) => {
            warnings.push(format!(
                "Equipment names and types are unavailable: {error}"
            ));
            (None, None)
        }
    }
}
