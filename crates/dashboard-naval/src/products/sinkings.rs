//! One cached branch scan for Ships sunk log and Naval losses. Equipment losses
//! can adopt this scan in a follow-up; its independent consumer is unchanged.
//! Its frames also build the branch's country histories, kept for every product.
//!
//! Each entry carries its event hour. Daily pruning deletes entries whose event
//! hour + NNavy.NAVAL_COMBAT_RESULT_TIMEOUT_YEARS * 0x2238 is <= now,
//! swap-erasing the last pointer into the removed position. Survivors are immutable
//! and order is immaterial: compare whole-entry multiplicities at consecutive
//! complete reads, not vector positions, partial keys or lifetime maxima.
//! Identical entries are legitimate. After a break, surviving multiset increases
//! can be recovered by their event hour. Events born and pruned entirely during
//! the break cannot be counted from this vector. Runtime retention identifies
//! possible loss spans. Scans of one branch take turns; a hit never waits on
//! another branch's scan.
//!
//! Convoy counts merge by (month, killer, owner) and add a signed i32 count.
//! Pruning removes months strictly older than current month -
//! NNavy.CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS, also by swap-erase.
//! There is no convoy event hour: deltas retain their bucket and supplying read.

use dashboard_base::{
    prepared::frame_changes::FrameChanges,
    products::{
        country_groups::HourSpan,
        country_history::{self, HistoryScan, Identity, RecordedHistories, observe_identity},
    },
    source::Source,
};
use dashboard_text::metadata::{
    IdentityTrack, LinkTags, TaggedCountry, branch_bounds, link_slot, slot_read, tagged_countries,
    validate_campaign,
};
use replay::container::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::SunkConvoyRow, rows::SunkShipEntry,
    rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::{
    frame::DecodedFrame, frame::FrameDecoder, frame::SharedView, frame::TableView,
    frame::ViewIdentity,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
mod preparation;
pub use preparation::Change;
use preparation::Pending;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "SavedShip", into = "SavedShip")]
pub struct ShipEvent {
    pub entry: SunkShipEntry,
    pub read_hour: i32,
    pub victim: Option<String>,
    pub attacker: Option<String>,
    /// Supplying-read fallback for a reference with no history at the event hour.
    pub victim_as_of: Option<i32>,
    pub attacker_as_of: Option<i32>,
}

/// Whole-entry signatures preserve multiplicity and the historical naming inputs
/// even after native retention prunes the entry from the decoder frame.
#[derive(Serialize, Deserialize)]
struct SavedShip {
    signature: RefusedShipKey,
    read_hour: i32,
    victim: Option<String>,
    attacker: Option<String>,
    victim_as_of: Option<i32>,
    attacker_as_of: Option<i32>,
}
impl From<ShipEvent> for SavedShip {
    fn from(event: ShipEvent) -> Self {
        let key = ShipKey::from(&event.entry);
        Self {
            signature: (key.text.map(|text| text.to_vec()), key.fields),
            read_hour: event.read_hour,
            victim: event.victim,
            attacker: event.attacker,
            victim_as_of: event.victim_as_of,
            attacker_as_of: event.attacker_as_of,
        }
    }
}
impl From<SavedShip> for ShipEvent {
    fn from(saved: SavedShip) -> Self {
        let (
            [
                ship_name,
                attribution_name,
                class_name,
                attribution_class_name,
            ],
            f,
        ) = saved.signature;
        Self {
            entry: SunkShipEntry {
                ship_name,
                attribution_name,
                class_name,
                attribution_class_name,
                victim_country: f[0] as u32,
                attacker_country: f[1] as u32,
                sink_hour: f[2] as u32 as i32,
                class_token: f[3] as u32,
                attribution_class_token: f[4] as u32,
                attribution_class_present: f[5] as u8,
                model_level: f[6] as u32 as i32,
                victim_equipment_variant: f[7],
                damage_source_reference: f[8],
                province: f[9] as u32,
                province_present: f[10] as u8,
                battle: f[11],
                cause_record: f[12] as u8,
                assisted_kill: f[13] as u8,
            },
            read_hour: saved.read_hour,
            victim: saved.victim,
            attacker: saved.attacker,
            victim_as_of: saved.victim_as_of,
            attacker_as_of: saved.attacker_as_of,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvoyEvent {
    pub month: i32,
    pub read_hour: i32,
    pub count: i64,
    pub owner: Option<String>,
    pub killer: Option<String>,
    pub baseline: bool,
    pub previous_read: Option<i32>,
    pub owner_raw: u32,
    pub killer_raw: u32,
}

/// One tag's native holder over consecutive recorded hours. Tag spelling is
/// presentation; the canonical slot and native country id own its tallies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountryIdentitySpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub identity: (u32, u32),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scanned {
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_ship_read: Option<i32>,
    pub first_convoy_read: Option<i32>,
    /// The first reads of the stretch its tallies count: the branch's first, until hours the
    /// recording lost begin another after them ([`ShipHistory::lost`]).
    pub stretch_ship_read: Option<i32>,
    pub stretch_convoy_read: Option<i32>,
    pub last_ship_read: Option<i32>,
    pub last_convoy_read: Option<i32>,
    pub ship_reads: Vec<i32>,
    pub convoy_reads: Vec<i32>,
    #[serde(skip)]
    pub ships: Vec<ShipEvent>,
    #[serde(skip)]
    pub convoys: Vec<ConvoyEvent>,
    pub ship_unread: Vec<HourSpan>,
    pub convoy_unread: Vec<HourSpan>,
    /// Potential unseen births start after the preceding read, before unread coverage.
    pub ship_breaks: Vec<HourSpan>,
    pub convoy_breaks: Vec<HourSpan>,
    pub country_candidates: BTreeMap<u32, BTreeSet<Identity>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub country_identities: BTreeMap<String, Vec<CountryIdentitySpan>>,
    pub tokens: BTreeMap<u32, String>,
    pub warnings: Vec<String>,
    pub gap_recoveries: usize,
    pub refused_entries: BTreeMap<String, usize>,
    #[serde(with = "dashboard_core::map_pairs")]
    pub uncertain_victims: BTreeMap<Identity, Vec<(i32, i32)>>,
    #[serde(with = "dashboard_core::map_pairs")]
    pub uncertain_attackers: BTreeMap<Identity, Vec<(i32, i32)>>,
    #[serde(with = "dashboard_core::map_pairs")]
    pub uncertain_convoy_owners: BTreeMap<Identity, Vec<(i32, i32)>>,
    #[serde(with = "dashboard_core::map_pairs")]
    pub uncertain_convoy_killers: BTreeMap<Identity, Vec<(i32, i32)>>,
    pub unresolved: BTreeSet<u32>,
    /// The branch's country histories, from the frames `scan_histories` reads.
    #[serde(skip)]
    pub histories: Arc<RecordedHistories>,
    #[serde(skip)]
    pub new_candidates: BTreeSet<Identity>,
    #[serde(skip)]
    pub new_tags: BTreeSet<String>,
    #[serde(skip)]
    pub current_holders: Vec<(String, Identity)>,
    #[serde(skip)]
    pub changed_references: BTreeSet<u32>,
}

/// Every recorded field participates, including attribution and presence flags.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ShipKey<'a> {
    text: [&'a [u8]; 4],
    fields: [u64; 14],
}
impl<'a> From<&'a SunkShipEntry> for ShipKey<'a> {
    fn from(e: &'a SunkShipEntry) -> Self {
        Self {
            text: [
                &e.ship_name,
                &e.attribution_name,
                &e.class_name,
                &e.attribution_class_name,
            ],
            fields: [
                u64::from(e.victim_country),
                u64::from(e.attacker_country),
                e.sink_hour as u32 as u64,
                u64::from(e.class_token),
                u64::from(e.attribution_class_token),
                u64::from(e.attribution_class_present),
                e.model_level as u32 as u64,
                e.victim_equipment_variant,
                e.damage_source_reference,
                u64::from(e.province),
                u64::from(e.province_present),
                e.battle,
                u64::from(e.cause_record),
                u64::from(e.assisted_kill),
            ],
        }
    }
}

pub(crate) type RefusedShipKey = ([Vec<u8>; 4], [u64; 14]);

#[derive(Default, Serialize, Deserialize)]
struct Tracker {
    #[serde(skip)]
    ships: Option<SharedView>,
    #[serde(with = "dashboard_core::map_pairs")]
    ship_counts: HashMap<RefusedShipKey, usize>,
    #[serde(skip)]
    dictionary: Option<ViewIdentity>,
    ship_open: bool,
    ship_future: Option<i32>,
    #[serde(with = "dashboard_core::map_pairs")]
    refused_ships: HashMap<RefusedShipKey, usize>,
    #[serde(with = "dashboard_core::map_pairs")]
    convoys: BTreeMap<(i32, u32, u32), i32>,
    convoy_open: bool,
    /// Whether the convoy rows changed since the last read: an equal read confirms only then.
    #[serde(skip)]
    convoys_changed: bool,
}

/// One hour's sunk ships, as a columnar walk reads them.
#[derive(replay_spool::Spooled)]
pub(crate) enum ShipRead {
    /// No slot read of the table's one row: whether it held exactly one row.
    Skipped { single: bool },
    /// A slot read: the entries first found, in order, and the new refusals by reason.
    Read {
        found: Vec<SunkShipEntry>,
        refused: Vec<(String, usize)>,
    },
}

/// One hour's sunk convoys as a walk of the recording's columns reads them.
#[derive(replay_spool::Spooled)]
pub(crate) enum ConvoyRead {
    /// No read of the complete rows: whether they were complete.
    Skipped { complete: bool },
    /// A read of rows equal to the last read's.
    Confirmed,
    /// The rows changed since the last read, or every row at a walk's first read.
    Read(Vec<SunkConvoyRow>),
}

/// The read compared last: its entries in order, counts taken and refused, and future dates.
#[derive(Clone, Default)]
#[derive(replay_spool::Spooled)]
pub(crate) struct ShipBase {
    entries: Vec<SunkShipEntry>,
    counts: HashMap<RefusedShipKey, usize>,
    refused: HashMap<RefusedShipKey, usize>,
    futures: BTreeMap<i32, usize>,
    /// The hour the read compared last refused its entries at.
    hour: Option<i32>,
}

/// An entry's count change among those taken, and the first position it gained at.
type Gained = (isize, Option<usize>);

impl ShipBase {
    /// Whether a read at `hour` refuses the entries last compared as that read did.
    pub(crate) fn settled(&self, hour: i32) -> bool {
        self.futures
            .first_key_value()
            .is_none_or(|(&future, _)| hour < future)
    }

    /// Compares a read at `hour`: the entries it first finds, in order, and new refusals.
    pub(crate) fn compare(
        &mut self,
        entries: Vec<SunkShipEntry>,
        hour: i32,
    ) -> (Vec<SunkShipEntry>, Vec<(String, usize)>) {
        let mut out = Scanned::default();
        let held = std::mem::take(&mut self.entries);
        if self.settled(hour) {
            self.compare_changed(&mut out, &held, &entries, hour);
        } else {
            let mut future = None;
            compare(
                &mut self.counts,
                &mut self.refused,
                &mut future,
                &mut out,
                &entries,
                hour,
            );
            self.futures.clear();
            for entry in entries.iter().filter(|entry| entry.sink_hour > hour) {
                *self.futures.entry(entry.sink_hour).or_default() += 1;
            }
        }
        self.entries = entries;
        self.hour = Some(hour);
        (
            out.ships.into_iter().map(|ship| ship.entry).collect(),
            out.refused_entries.into_iter().collect(),
        )
    }

    /// [`compare`] through the positions where `entries` differ from `held`, once settled.
    fn compare_changed(
        &mut self,
        out: &mut Scanned,
        held: &[SunkShipEntry],
        entries: &[SunkShipEntry],
        hour: i32,
    ) {
        let mut counted = HashMap::<ShipKey<'_>, Gained>::new();
        let mut refused = HashMap::<ShipKey<'_>, (isize, &'static str)>::new();
        for at in 0..held.len().max(entries.len()) {
            let (old, new) = (held.get(at), entries.get(at));
            if old == new {
                continue;
            }
            for (entry, step) in [(old, -1), (new, 1)] {
                let Some(entry) = entry else {
                    continue;
                };
                let key = ShipKey::from(entry);
                let Some(reason) = ship_refusal(entry, hour) else {
                    let gained = counted.entry(key).or_insert((0, None));
                    gained.0 += step;
                    if step > 0 && gained.1.is_none() {
                        gained.1 = Some(at);
                    }
                    continue;
                };
                refused.entry(key).or_insert((0, reason)).0 += step;
                if entry.sink_hour > hour {
                    let count = self.futures.entry(entry.sink_hour).or_default();
                    *count = count.saturating_add_signed(step);
                    if *count == 0 {
                        self.futures.remove(&entry.sink_hour);
                    }
                }
            }
        }
        // An entry held more often is first found as often more, at its first position.
        let mut found: Vec<(usize, &SunkShipEntry, usize)> = Vec::new();
        let mut earlier = Vec::new();
        for (key, (change, first)) in counted {
            let before = shift(&mut self.counts, owned(&key), change);
            if let (Ok(added @ 1..), Some(first)) = (usize::try_from(change), first) {
                // A copy held before may sit at a position left alone, ahead of it.
                if before > 0 {
                    earlier.push(found.len());
                }
                found.push((first, &entries[first], added));
            }
        }
        if let Some(end) = earlier.iter().map(|&at| found[at].0).max() {
            let wanted: HashMap<ShipKey<'_>, usize> = earlier
                .iter()
                .map(|&at| (ShipKey::from(found[at].1), at))
                .collect();
            for (position, entry) in entries[..end].iter().enumerate() {
                if let Some(&at) = wanted.get(&ShipKey::from(entry)) {
                    found[at].0 = found[at].0.min(position);
                }
            }
        }
        found.sort_unstable_by_key(|&(first, ..)| first);
        for (_, entry, added) in found {
            for _ in 0..added {
                out.ships.push(ShipEvent {
                    entry: entry.clone(),
                    read_hour: hour,
                    victim: None,
                    attacker: None,
                    victim_as_of: None,
                    attacker_as_of: None,
                });
            }
        }
        for (key, (change, reason)) in refused {
            shift(&mut self.refused, owned(&key), change);
            if let Ok(added @ 1..) = usize::try_from(change) {
                *out.refused_entries.entry(reason.into()).or_default() += added;
            }
        }
    }

    /// The bytes it holds on the heap, about.
    pub(crate) fn heap_bytes(&self) -> usize {
        let key = |(text, _): &RefusedShipKey| text.iter().map(Vec::capacity).sum::<usize>();
        // A hash table's buckets: at most seven eighths full, a control byte each.
        let table = |capacity: usize| capacity * 8 / 7 * (size_of::<(RefusedShipKey, usize)>() + 1);
        self.entries.capacity() * size_of::<SunkShipEntry>()
            + self
                .entries
                .iter()
                .map(|entry| {
                    [
                        &entry.ship_name,
                        &entry.attribution_name,
                        &entry.class_name,
                        &entry.attribution_class_name,
                    ]
                    .into_iter()
                    .map(Vec::capacity)
                    .sum::<usize>()
                })
                .sum::<usize>()
            + table(self.counts.capacity())
            + table(self.refused.capacity())
            + self
                .counts
                .keys()
                .chain(self.refused.keys())
                .map(key)
                .sum::<usize>()
            + self.futures.len() * size_of::<(i32, usize)>()
    }

    /// The entries compared last and their hour, for [`Self::restored`].
    pub(crate) fn saved(&self) -> SavedBase {
        SavedBase {
            entries: self
                .entries
                .iter()
                .map(|entry| {
                    let key = ShipKey::from(entry);
                    (key.text.map(<[u8]>::to_vec), key.fields)
                })
                .collect(),
            hour: self.hour,
        }
    }

    /// The base [`Self::saved`] wrote, its counts taken again from its entries.
    pub(crate) fn restored(saved: SavedBase) -> Self {
        let entries: Vec<SunkShipEntry> = saved
            .entries
            .into_iter()
            .map(|signature| {
                ShipEvent::from(SavedShip {
                    signature,
                    read_hour: 0,
                    victim: None,
                    attacker: None,
                    victim_as_of: None,
                    attacker_as_of: None,
                })
                .entry
            })
            .collect();
        let mut base = Self::default();
        if let Some(hour) = saved.hour {
            base.compare(entries, hour);
        }
        base
    }
}

/// A [`ShipBase`] as an operator's carry writes it.
#[derive(Serialize, Deserialize)]
pub(crate) struct SavedBase {
    entries: Vec<RefusedShipKey>,
    hour: Option<i32>,
}

/// `key`'s whole entry, owned.
fn owned(key: &ShipKey<'_>) -> RefusedShipKey {
    (key.text.map(<[u8]>::to_vec), key.fields)
}

/// Moves `key`'s count by `change`, dropping it at zero: the count before.
fn shift(counts: &mut HashMap<RefusedShipKey, usize>, key: RefusedShipKey, change: isize) -> usize {
    let before = counts.get(&key).copied().unwrap_or(0);
    match before.checked_add_signed(change).filter(|&after| after > 0) {
        Some(after) => {
            counts.insert(key, after);
        }
        None => {
            counts.remove(&key);
        }
    }
    before
}

/// Compares a whole read at `hour` with the counts of the read compared last.
fn compare(
    counts: &mut HashMap<RefusedShipKey, usize>,
    refused_before: &mut HashMap<RefusedShipKey, usize>,
    future: &mut Option<i32>,
    out: &mut Scanned,
    entries: &[SunkShipEntry],
    hour: i32,
) {
    let previous: HashMap<ShipKey<'_>, usize> = counts
        .iter()
        .map(|((text, fields), count)| {
            (
                ShipKey {
                    text: text.each_ref().map(|bytes| bytes.as_slice()),
                    fields: *fields,
                },
                *count,
            )
        })
        .collect();
    let mut now = HashMap::<ShipKey<'_>, (usize, &SunkShipEntry)>::new();
    *future = None;
    let mut refused = HashMap::<RefusedShipKey, usize>::new();
    for entry in entries {
        if let Some(reason) = ship_refusal(entry, hour) {
            let key = ShipKey::from(entry);
            let key = (key.text.map(|bytes| bytes.to_vec()), key.fields);
            let count = refused.entry(key.clone()).or_default();
            *count += 1;
            if *count > refused_before.get(&key).copied().unwrap_or(0) {
                *out.refused_entries.entry(reason.into()).or_default() += 1;
            }
            if entry.sink_hour > hour {
                *future =
                    Some(future.map_or(entry.sink_hour, |future| future.min(entry.sink_hour)));
            }
            continue;
        }
        now.entry(entry.into()).or_insert((0, entry)).0 += 1;
    }
    let held = now
        .iter()
        .map(|(key, (count, _))| ((key.text.map(|bytes| bytes.to_vec()), key.fields), *count))
        .collect();
    for entry in entries {
        let key = ShipKey::from(entry);
        let Some((count, entry)) = now.remove(&key) else {
            continue;
        };
        let added = count.saturating_sub(previous.get(&key).copied().unwrap_or(0));
        for _ in 0..added {
            out.ships.push(ShipEvent {
                entry: entry.clone(),
                read_hour: hour,
                victim: None,
                attacker: None,
                victim_as_of: None,
                attacker_as_of: None,
            });
        }
    }
    drop(now);
    drop(previous);
    *counts = held;
    *refused_before = refused;
}

fn append_span(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    if first > last {
        return;
    }
    let index = spans.partition_point(|s| i64::from(s.last_hour) + 1 < i64::from(first));
    let end = spans.partition_point(|s| i64::from(s.first_hour) <= i64::from(last) + 1);
    let first_hour = spans
        .get(index)
        .filter(|_| index < end)
        .map_or(first, |s| first.min(s.first_hour));
    let last_hour = end
        .checked_sub(1)
        .filter(|_| index < end)
        .map_or(last, |i| last.max(spans[i].last_hour));
    spans.splice(
        index..end,
        [HourSpan {
            first_hour,
            last_hour,
        }],
    );
}

impl Scanned {
    fn observe_tables(
        &mut self,
        track: &mut Tracker,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        tags: &HashMap<u32, String>,
        changes: Option<&FrameChanges>,
    ) {
        let hour = frame.hour;
        let view = &frame.tables[TableId::LexicalTokens.index()];
        if changes.map_or_else(
            || !track.dictionary.as_ref().is_some_and(|last| last.is(view)),
            |changes| changes.rows_changed[TableId::LexicalTokens.index()],
        ) && let Ok(rows) = frame.complete::<tables::LexicalTokens>()
        {
            track.dictionary = Some(view.identity());
            self.read_tokens(rows.iter().map(|row| (row.token, row.literal.as_slice())));
        }
        let ships = frame.complete::<tables::SunkShips>();
        if let Ok([_]) = ships
            && changes.map_or_else(
                || slot_read(format, &frame.metrics, TableId::SunkShips, hour),
                |changes| changes.slot_read[TableId::SunkShips.index()],
            )
        {
            track.ships(self, &frame.tables[TableId::SunkShips.index()], hour);
        } else {
            self.ships_unread(track, matches!(ships, Ok([_])), hour);
        }
        let convoys = frame.complete::<tables::SunkConvoys>();
        if let Some(changes) = changes {
            track.convoys_changed |= changes.rows_changed[TableId::SunkConvoys.index()];
        }
        if let Ok(rows) = convoys
            && changes.map_or_else(
                || slot_read(format, &frame.metrics, TableId::SunkConvoys, hour),
                |changes| {
                    changes.table_read[TableId::SunkConvoys.index()]
                        || changes.slot_read[TableId::SunkConvoys.index()]
                },
            )
        {
            // Supplying reads use the oracle's accepted-frame/cadence date.
            // Equal acquired rows advance that proof without another bucket walk.
            if self.first_convoy_read.is_some() && changes.is_some() && !track.convoys_changed {
                track.confirm_convoy_read(self, hour);
            } else {
                track.convoys(self, rows, hour, tags);
            }
        } else {
            self.convoys_unread(track, convoys.is_ok(), hour);
        }
    }

    /// [`Self::observe_tables`] of an hour a columnar walk read.
    fn observe_projected_tables(
        &mut self,
        track: &mut Tracker,
        hour: i32,
        tags: &HashMap<u32, String>,
        tokens: Option<&[(u32, Vec<u8>)]>,
        ships: &ShipRead,
        convoys: &ConvoyRead,
    ) {
        if let Some(rows) = tokens {
            self.read_tokens(
                rows.iter()
                    .map(|(token, literal)| (*token, literal.as_slice())),
            );
        }
        match ships {
            ShipRead::Read { found, refused } => {
                track.ship_read(self, hour);
                self.ships.extend(found.iter().map(|entry| ShipEvent {
                    entry: entry.clone(),
                    read_hour: hour,
                    victim: None,
                    attacker: None,
                    victim_as_of: None,
                    attacker_as_of: None,
                }));
                for (reason, count) in refused {
                    *self.refused_entries.entry(reason.clone()).or_default() += count;
                }
            }
            &ShipRead::Skipped { single } => self.ships_unread(track, single, hour),
        }
        match convoys {
            ConvoyRead::Read(rows) => track.convoys(self, rows, hour, tags),
            ConvoyRead::Confirmed => track.confirm_convoy_read(self, hour),
            &ConvoyRead::Skipped { complete } => self.convoys_unread(track, complete, hour),
        }
    }

    fn read_tokens<'a>(&mut self, rows: impl Iterator<Item = (u32, &'a [u8])>) {
        for (token, literal) in rows {
            if let Ok(text) = std::str::from_utf8(literal) {
                self.tokens.entry(token).or_insert_with(|| text.to_owned());
            }
        }
    }

    /// An hour without a slot read of the sunk ships' one row (`single`: the table held one).
    fn ships_unread(&mut self, track: &mut Tracker, single: bool, hour: i32) {
        if !single || !track.ship_open {
            append_span(&mut self.ship_unread, hour, hour);
            append_span(
                &mut self.ship_breaks,
                self.last_ship_read.map_or(hour, |h| h.saturating_add(1)),
                hour,
            );
            track.ship_open = false;
        }
    }

    /// An hour without a read of the complete sunk convoys (`complete`: they were complete).
    fn convoys_unread(&mut self, track: &mut Tracker, complete: bool, hour: i32) {
        if !complete || !track.convoy_open {
            append_span(&mut self.convoy_unread, hour, hour);
            append_span(
                &mut self.convoy_breaks,
                self.last_convoy_read.map_or(hour, |h| h.saturating_add(1)),
                hour,
            );
            track.convoy_open = false;
        }
    }
    pub fn metadata(&self) -> Self {
        Self {
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            first_ship_read: self.first_ship_read,
            first_convoy_read: self.first_convoy_read,
            stretch_ship_read: self.stretch_ship_read,
            stretch_convoy_read: self.stretch_convoy_read,
            last_ship_read: self.last_ship_read,
            last_convoy_read: self.last_convoy_read,
            ship_reads: Vec::new(),
            convoy_reads: Vec::new(),
            ships: Vec::new(),
            convoys: Vec::new(),
            ship_unread: self.ship_unread.clone(),
            convoy_unread: self.convoy_unread.clone(),
            ship_breaks: self.ship_breaks.clone(),
            convoy_breaks: self.convoy_breaks.clone(),
            country_candidates: self.country_candidates.clone(),
            segments: self.segments.clone(),
            country_identities: self.country_identities.clone(),
            tokens: self.tokens.clone(),
            warnings: self.warnings.clone(),
            gap_recoveries: self.gap_recoveries,
            refused_entries: self.refused_entries.clone(),
            uncertain_victims: self.uncertain_victims.clone(),
            uncertain_attackers: self.uncertain_attackers.clone(),
            uncertain_convoy_owners: self.uncertain_convoy_owners.clone(),
            uncertain_convoy_killers: self.uncertain_convoy_killers.clone(),
            unresolved: self.unresolved.clone(),
            histories: Arc::clone(&self.histories),
            new_candidates: BTreeSet::new(),
            new_tags: BTreeSet::new(),
            current_holders: Vec::new(),
            changed_references: BTreeSet::new(),
        }
    }

    pub fn identity(&self, tag: &str, hour: i32) -> Option<Identity> {
        let spans = self.country_identities.get(tag)?;
        let end = spans.partition_point(|span| span.first_hour <= hour);
        spans
            .get(end.checked_sub(1)?)
            .filter(|span| {
                hour <= span.last_hour
                    || spans
                        .get(end)
                        .is_some_and(|next| next.identity == span.identity)
            })
            .map(|span| span.identity)
    }

    fn missing(&mut self, track: &mut Tracker, first: i32, last: i32) {
        append_span(&mut self.ship_unread, first, last);
        append_span(&mut self.convoy_unread, first, last);
        append_span(
            &mut self.ship_breaks,
            self.last_ship_read.map_or(first, |h| h.saturating_add(1)),
            last,
        );
        append_span(
            &mut self.convoy_breaks,
            self.last_convoy_read.map_or(first, |h| h.saturating_add(1)),
            last,
        );
        track.ship_open = false;
        track.convoy_open = false;
    }
}

impl Tracker {
    fn ships(&mut self, out: &mut Scanned, view: &SharedView, hour: i32) {
        self.ship_read(out, hour);
        if self
            .ships
            .as_ref()
            .is_some_and(|last| SharedView::ptr_eq(last, view))
            && self.ship_future.is_none_or(|future| hour < future)
        {
            return;
        }
        compare(
            &mut self.ship_counts,
            &mut self.refused_ships,
            &mut self.ship_future,
            out,
            ship_entries(view),
            hour,
        );
        self.ships = Some(view.clone());
    }

    /// A slot read of the sunk ships' one row at `hour`, whatever it finds.
    fn ship_read(&mut self, out: &mut Scanned, hour: i32) {
        let last_read = out.last_ship_read;
        if !self.ship_open && last_read.is_some() {
            out.gap_recoveries += 1;
        }
        self.ship_open = true;
        out.first_ship_read.get_or_insert(hour);
        out.stretch_ship_read.get_or_insert(hour);
        out.last_ship_read = Some(hour);
        out.ship_reads.push(hour);
    }

    fn convoys(
        &mut self,
        out: &mut Scanned,
        rows: &[SunkConvoyRow],
        hour: i32,
        tags: &HashMap<u32, String>,
    ) {
        // A stretch's first read, the branch's or one after lost hours, is its baseline.
        let baseline = out.stretch_convoy_read.is_none();
        for row in rows {
            #[cfg(test)]
            work::WORK.with(|work| work.borrow_mut().convoys += 1);
            let last = self
                .convoys
                .entry((row.month, row.killer_country, row.owner_country))
                .or_default();
            // The bucket wraps as signed i32. Widen both signed endpoints
            // so summing deltas reproduces each captured i32 value.
            let count = i64::from(row.convoys) - i64::from(*last);
            *last = row.convoys;
            if count != 0 {
                out.convoys.push(ConvoyEvent {
                    month: row.month,
                    read_hour: hour,
                    count,
                    owner: tags.get(&row.owner_country).cloned(),
                    killer: tags.get(&row.killer_country).cloned(),
                    baseline,
                    previous_read: out.last_convoy_read,
                    owner_raw: row.owner_country,
                    killer_raw: row.killer_country,
                });
            }
        }
        self.confirm_convoy_read(out, hour);
    }
    fn confirm_convoy_read(&mut self, out: &mut Scanned, hour: i32) {
        self.convoys_changed = false;
        self.convoy_open = true;
        out.first_convoy_read.get_or_insert(hour);
        out.stretch_convoy_read.get_or_insert(hour);
        out.last_convoy_read = Some(hour);
        out.convoy_reads.push(hour);
    }
}

pub(crate) fn ship_entries(view: &SharedView) -> &[SunkShipEntry] {
    match &**view {
        TableView::Complete(TableRows::SunkShips(rows)) if rows.len() == 1 => &rows[0].entries,
        _ => &[],
    }
}

pub(crate) fn ship_refusal(entry: &SunkShipEntry, hour: i32) -> Option<&'static str> {
    if entry.sink_hour > hour {
        Some("SunkShips event hour is after its read")
    } else if entry.cause_record > 1
        || entry.assisted_kill > 1
        || entry.province_present > 1
        || entry.attribution_class_present > 1
    {
        Some("SunkShips boolean flag is above 1")
    } else {
        None
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct TagRun {
    first: i32,
    last: i32,
    tag: String,
    identity: Identity,
}

#[derive(Default, Serialize, Deserialize)]
struct CountryCache {
    links: LinkTags,
    holders: Vec<(String, Identity)>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    tags: HashMap<u32, String>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    empty: HashMap<u32, String>,
    /// The last hour with country tables, and the hour every holder's identity
    /// and tag run reach but are not yet written.
    hour: Option<i32>,
    extend_to: Option<i32>,
    #[serde(skip)]
    changed: bool,
}

impl CountryCache {
    /// Writes the deferred hours, as each would have extended every holder and tag run.
    fn extend(
        &mut self,
        out: &mut Scanned,
        history: &mut HashMap<u32, Vec<TagRun>>,
        identities: &mut HashMap<String, IdentityTrack>,
    ) {
        let Some(hour) = self.extend_to.take() else {
            return;
        };
        for (tag, identity) in &self.holders {
            identities
                .get_mut(tag)
                .expect("country identity")
                .step(Some(*identity), hour);
            out.segments
                .get_mut(tag)
                .and_then(|spans| spans.last_mut())
                .expect("country segment")
                .last_hour = hour;
            out.country_identities
                .get_mut(tag)
                .and_then(|spans| spans.last_mut())
                .expect("country identity span")
                .last_hour = hour;
        }
        for raw in self.tags.keys() {
            history
                .get_mut(raw)
                .and_then(|runs| runs.last_mut())
                .expect("country tag run")
                .last = hour;
        }
    }
}

pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scanned> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Naval sinkings cancelled",
        ));
    }
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SunkShips,
        TableId::SunkConvoys,
        TableId::LexicalTokens,
    ];
    let start = replay
        .branches
        .iter()
        .find(|b| b.id == branch)
        .ok_or_else(|| io::Error::other("Naval sinkings branch not found"))?;
    let (first_hour, last_hour) = branch_bounds(replay, start);
    let mut observer = ShipHistory::new(first_hour);
    let mut histories = HistoryScan::new(false, &[]);
    let mut blocks: Vec<_> = replay.index.iter().filter(|b| b.branch == branch).collect();
    blocks.sort_unstable_by_key(|b| b.first_hour);
    dashboard_core::blocks::scan_blocks(replay, blocks, &selected, cancelled, |blocks| {
        for (_, frames) in blocks {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Naval sinkings cancelled",
                ));
            }
            let Ok(frames) = frames else {
                continue;
            };
            let mut decoder = FrameDecoder::for_tables(&selected);
            for frame in frames {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Naval sinkings cancelled",
                    ));
                }
                let Ok(decoded) = decoder.advance(&frame) else {
                    break;
                };
                if validate_campaign(decoded, replay).is_err() {
                    break;
                }
                // Before the skips below: `scan_histories` reads exactly these frames.
                histories.observe(decoded);
                let hour = decoded.hour;
                if i64::from(hour) < observer.next.expect("sinking cursor")
                    || replay
                        .gaps
                        .iter()
                        .any(|g| g.branch == branch && g.first_hour <= hour && hour <= g.last_hour)
                {
                    continue;
                }
                observer.observe(replay.format, decoded);
            }
        }
        Ok(())
    })?;
    observer.finish(last_hour);
    observer.scan.histories = Arc::new(histories.finish());
    Ok(observer.scan)
}

fn countries<'a>(
    decoded: &DecodedFrame,
    out: &mut Scanned,
    history: &mut HashMap<u32, Vec<TagRun>>,
    identities: &mut HashMap<String, IdentityTrack>,
    cache: &'a mut CountryCache,
) -> &'a HashMap<u32, String> {
    let update = cache.links.update(decoded);
    countries_with(
        decoded.hour,
        update,
        || {
            (
                decoded
                    .complete::<tables::Countries>()
                    .expect("LinkTags complete countries"),
                decoded
                    .complete::<tables::CountryLinks>()
                    .expect("LinkTags complete links"),
            )
        },
        out,
        history,
        identities,
        cache,
    )
}

/// The country holders at `hour` once the link tags took `update`; `tables` gives the rows.
#[allow(clippy::too_many_arguments)]
fn countries_with<'a, 't>(
    hour: i32,
    update: io::Result<bool>,
    tables: impl FnOnce() -> (&'t [CountryRow], &'t [CountryLinkRow]),
    out: &mut Scanned,
    history: &mut HashMap<u32, Vec<TagRun>>,
    identities: &mut HashMap<String, IdentityTrack>,
    cache: &'a mut CountryCache,
) -> &'a HashMap<u32, String> {
    cache.changed = cache.hour.and_then(|last| last.checked_add(1)) != Some(hour);
    let Ok(changed) = update else {
        cache.changed |= cache.hour.is_some();
        return &cache.empty;
    };
    cache.changed |= changed;
    // The same holders an hour later continue every identity and tag run they
    // reached. Write that once, when the holders change, a break comes or the scan ends.
    if !changed && cache.hour.and_then(|last| last.checked_add(1)) == Some(hour) {
        cache.hour = Some(hour);
        cache.extend_to = Some(hour);
        return &cache.tags;
    }
    cache.extend(out, history, identities);
    cache.hour = Some(hour);
    if changed {
        let (rows, links) = tables();
        cache.holders.clear();
        let mut possible = HashMap::<u32, BTreeSet<Identity>>::new();
        for row in rows {
            possible
                .entry(row.canonical_slot)
                .or_default()
                .insert((row.canonical_slot, row.id));
        }
        for link in links {
            if let Some(holders) = possible.get(&link.canonical_slot) {
                let candidates = out.country_candidates.entry(link.id).or_default();
                for &holder in holders {
                    if candidates.insert(holder) {
                        out.new_candidates.insert(holder);
                        out.changed_references.insert(link.id);
                    }
                }
            }
        }
        let mut slots = HashMap::new();
        for (tag, country) in tagged_countries(rows, links) {
            if let TaggedCountry::Found(country, slot) = country
                && let Ok(tag) = std::str::from_utf8(tag)
            {
                let tag = tag.to_owned();
                if !out.segments.contains_key(&tag) {
                    out.new_tags.insert(tag.clone());
                }
                out.segments.entry(tag.clone()).or_default();
                out.country_identities.entry(tag.clone()).or_default();
                identities.entry(tag.clone()).or_default();
                cache.holders.push((tag.clone(), (slot, country.id)));
                slots.insert(slot, tag);
            }
        }
        cache.tags = links
            .iter()
            .filter_map(|link| {
                let tag = link_slot(links, link.id).and_then(|slot| slots.get(&slot))?;
                // Preserve tagged_countries' ambiguity semantics rather than
                // accepting a different row LinkTags could name under the slot.
                if cache.links.get(link.id) == Some(tag) {
                    Some((link.id, tag.clone()))
                } else {
                    None
                }
            })
            .collect();
    }
    for (tag, identity) in &cache.holders {
        let contiguous = observe_identity(
            out.segments.get_mut(tag).expect("country segments"),
            identities.get_mut(tag).expect("country identity"),
            *identity,
            hour,
        );
        let spans = out
            .country_identities
            .get_mut(tag)
            .expect("country identity spans");
        if contiguous {
            spans.last_mut().expect("country identity span").last_hour = hour;
        } else {
            spans.push(CountryIdentitySpan {
                first_hour: hour,
                last_hour: hour,
                identity: *identity,
            });
        }
    }
    for (raw, tag) in &cache.tags {
        let Some(identity) = out.identity(tag, hour) else {
            continue;
        };
        let runs = history.entry(*raw).or_default();
        if let Some(run) = runs.last_mut()
            && run.tag == *tag
            && run.identity == identity
            && run.last.checked_add(1) == Some(hour)
        {
            run.last = hour;
        } else {
            out.changed_references.insert(*raw);
            runs.push(TagRun {
                first: hour,
                last: hour,
                tag: tag.clone(),
                identity,
            });
        }
    }
    &cache.tags
}

/// Retained entries can predate country history. Name those only at the read
/// that supplied them; a later read must not fill an unresolved supplying read.
fn ship_attribution(
    raw: u32,
    hour: i32,
    read_hour: i32,
    history: &HashMap<u32, Vec<TagRun>>,
    candidates: &BTreeMap<u32, BTreeSet<Identity>>,
) -> (Option<String>, Option<i32>, BTreeSet<Identity>) {
    let (tag, possible) = attribution(raw, hour, history, candidates);
    if tag.is_none()
        && let Some(runs) = history.get(&raw)
        && runs.first().is_some_and(|run| hour < run.first)
    {
        let end = runs.partition_point(|run| run.first <= read_hour);
        if let Some(run) = end.checked_sub(1).and_then(|i| runs.get(i))
            && read_hour <= run.last
        {
            return (Some(run.tag.clone()), Some(read_hour), BTreeSet::new());
        }
    }
    (tag, None, possible)
}

fn resolve_ship(
    event: &mut ShipEvent,
    history: &HashMap<u32, Vec<TagRun>>,
    candidates: &BTreeMap<u32, BTreeSet<Identity>>,
) -> [BTreeSet<Identity>; 2] {
    let mut possible: [BTreeSet<Identity>; 2] = Default::default();
    for (index, (raw, tag, as_of)) in [
        (
            event.entry.victim_country,
            &mut event.victim,
            &mut event.victim_as_of,
        ),
        (
            event.entry.attacker_country,
            &mut event.attacker,
            &mut event.attacker_as_of,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (resolved, dated, holders) = ship_attribution(
            raw,
            event.entry.sink_hour,
            event.read_hour,
            history,
            candidates,
        );
        *tag = resolved;
        *as_of = dated;
        if index == 0 || event.entry.cause_record == 0 && event.entry.assisted_kill == 0 {
            possible[index] = holders;
        }
    }
    possible
}
fn resolve_convoy(
    event: &mut ConvoyEvent,
    history: &HashMap<u32, Vec<TagRun>>,
    candidates: &BTreeMap<u32, BTreeSet<Identity>>,
) -> [BTreeSet<Identity>; 2] {
    let mut possible: [BTreeSet<Identity>; 2] = Default::default();
    for (index, (raw, tag)) in [
        (event.owner_raw, &mut event.owner),
        (event.killer_raw, &mut event.killer),
    ]
    .into_iter()
    .enumerate()
    {
        let (resolved, holders) = attribution(raw, event.read_hour, history, candidates);
        *tag = resolved;
        possible[index] = holders;
    }
    possible
}

fn attribution(
    raw: u32,
    hour: i32,
    history: &HashMap<u32, Vec<TagRun>>,
    candidates: &BTreeMap<u32, BTreeSet<Identity>>,
) -> (Option<String>, BTreeSet<Identity>) {
    if raw == 0 {
        return (None, BTreeSet::new());
    }
    let runs = history.get(&raw).map(Vec::as_slice).unwrap_or(&[]);
    let end = runs.partition_point(|r| r.first <= hour);
    let before = end.checked_sub(1).and_then(|i| runs.get(i));
    let after = runs.get(end);
    let resolved = before.filter(|r| hour <= r.last).or_else(|| {
        before.filter(|left| {
            after.is_some_and(|right| left.tag == right.tag && left.identity == right.identity)
        })
    });
    if let Some(run) = resolved {
        return (Some(run.tag.clone()), BTreeSet::new());
    }
    let mut possible: BTreeSet<_> = before
        .into_iter()
        .chain(after)
        .map(|run| run.identity)
        .collect();
    if possible.is_empty() {
        possible.extend(candidates.get(&raw).into_iter().flatten().copied());
    }
    (None, possible)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryLoss {
    pub first_hour: i32,
    pub last_hour: i32,
    pub known_at: i32,
}

#[derive(Default)]
struct Cache {
    entries: Vec<(u64, Arc<Scanned>)>,
    /// One scan of a branch at a time, outside the cache lock.
    gates: BTreeMap<u64, Arc<Mutex<()>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retention {
    pub ships: bool,
    pub convoys: bool,
    pub ship_loss: Vec<HistoryLoss>,
    pub convoy_loss: Vec<HistoryLoss>,
    pub warnings: Vec<String>,
}

/// Runtime defines used by daily pruning. A missing numeric definition is
/// unknown, never the getter's default zero. Nonpositive windows can delete
/// changing histories before the next daily read.
pub fn retention(
    scan: &Scanned,
    catalogue: Option<&dashboard_text::catalogue::Catalogue>,
) -> Retention {
    RetentionDefines::read(catalogue).evaluate(scan)
}

/// These validated content operands are fixed for a configured branch. The
/// oracle and incremental observer use the same pruning arithmetic below.
#[derive(Clone)]
pub struct RetentionDefines {
    values: Option<[Result<i32, String>; 2]>,
}
impl RetentionDefines {
    pub fn read(catalogue: Option<&dashboard_text::catalogue::Catalogue>) -> Self {
        let values = catalogue.map(|catalogue| {
            #[cfg(test)]
            work::WORK.with(|work| work.borrow_mut().defines += 1);
            let defines = installed_content_core::defines::read(&catalogue.content.layers);
            [
                "NAVAL_COMBAT_RESULT_TIMEOUT_YEARS",
                "CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS",
            ]
            .map(|key| {
                defines.as_ref().map_err(ToString::to_string).and_then(|d| {
                    d.number("NNavy", key).map_err(|e| e.to_string())?;
                    d.int32("NNavy", key).map_err(|e| e.to_string())
                })
            })
        });
        Self { values }
    }
    fn value(&self, ship: bool) -> Option<i32> {
        self.values.as_ref()?[usize::from(!ship)]
            .as_ref()
            .ok()
            .copied()
            .filter(|value| *value > 0 && (!ship || value.checked_mul(0x2238).is_some()))
    }
    fn loss(&self, span: &HourSpan, ship: bool) -> Option<HistoryLoss> {
        #[cfg(test)]
        work::WORK.with(|work| work.borrow_mut().retention_spans += 1);
        let value = self.value(ship)?;
        if ship {
            let last = i64::from(span.last_hour) + 1 - i64::from(value) * 0x2238;
            (last >= i64::from(span.first_hour)).then_some(HistoryLoss {
                first_hour: span.first_hour,
                last_hour: last as i32,
                known_at: span.last_hour.saturating_add(1),
            })
        } else {
            let first = month(span.first_hour)?;
            let resume = month(span.last_hour.saturating_add(1))?;
            let last = resume - value - 1;
            (last >= first).then(|| HistoryLoss {
                first_hour: month_start(first).unwrap_or(span.first_hour),
                last_hour: month_start(last + 1)
                    .unwrap_or(span.last_hour)
                    .saturating_sub(1),
                known_at: span.last_hour.saturating_add(1),
            })
        }
    }
    fn warnings(&self, ship_losses: usize, convoy_losses: usize) -> Vec<String> {
        let Some(values) = &self.values else {
            return vec![
                "Naval retention defines are unavailable; completeness is unknown.".into(),
            ];
        };
        let mut warnings = Vec::new();
        for (index, key) in [
            "NAVAL_COMBAT_RESULT_TIMEOUT_YEARS",
            "CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS",
        ]
        .into_iter()
        .enumerate()
        {
            match &values[index] {
                Ok(_) if self.value(index == 0).is_some() => {
                    if index == 0 && ship_losses > 0 {
                        warnings.push(format!("{ship_losses} unread ship-history spans reach NDefines.NNavy.{key} retention; sinkings born and pruned wholly between complete reads cannot be counted."));
                    } else if index == 1 && convoy_losses > 0 {
                        warnings.push(format!("{convoy_losses} unread convoy-history spans exceed NDefines.NNavy.{key} retention; unseen pruned month buckets cannot be counted."));
                    }
                }
                Ok(_) => warnings.push(format!("NDefines.NNavy.{key} is nonpositive or outside supported retention arithmetic; completeness is unknown.")),
                Err(error) => warnings.push(format!("NDefines.NNavy.{key} is unavailable: {error}; completeness is unknown.")),
            }
        }
        warnings
    }
    pub fn evaluate(&self, scan: &Scanned) -> Retention {
        let ship_loss: Vec<_> = scan
            .ship_breaks
            .iter()
            .filter_map(|span| self.loss(span, true))
            .collect();
        let convoy_loss: Vec<_> = scan
            .convoy_breaks
            .iter()
            .filter_map(|span| self.loss(span, false))
            .collect();
        Retention {
            ships: self.value(true).is_some(),
            convoys: self.value(false).is_some(),
            warnings: self.warnings(ship_loss.len(), convoy_loss.len()),
            ship_loss,
            convoy_loss,
        }
    }
}

/// Only the trailing break can extend; completed breaks never change again.
pub struct RetentionProgress {
    pub defines: RetentionDefines,
    result: Retention,
    seen: [usize; 2],
    last: [Option<HourSpan>; 2],
    last_loss: [bool; 2],
}
impl RetentionProgress {
    pub fn new(defines: RetentionDefines) -> Self {
        let result = defines.evaluate(&Scanned::default());
        Self {
            defines,
            result,
            seen: [0; 2],
            last: [None, None],
            last_loss: [false; 2],
        }
    }
    pub fn update(&mut self, scan: &Scanned) -> &Retention {
        for (index, spans) in [&scan.ship_breaks, &scan.convoy_breaks]
            .into_iter()
            .enumerate()
        {
            if spans.len() == self.seen[index] && spans.last() == self.last[index].as_ref() {
                continue;
            }
            let losses = if index == 0 {
                &mut self.result.ship_loss
            } else {
                &mut self.result.convoy_loss
            };
            let first = if spans.len() < self.seen[index] {
                losses.clear();
                0
            } else if self.seen[index] > 0
                && spans.get(self.seen[index] - 1) != self.last[index].as_ref()
            {
                if self.last_loss[index] {
                    losses.pop();
                }
                self.seen[index] - 1
            } else {
                self.seen[index]
            };
            self.last_loss[index] = false;
            for (at, span) in spans.iter().enumerate().skip(first) {
                if let Some(loss) = self.defines.loss(span, index == 0) {
                    losses.push(loss);
                    self.last_loss[index] = at + 1 == spans.len();
                }
            }
            self.seen[index] = spans.len();
            self.last[index] = spans.last().cloned();
            self.result.warnings = self
                .defines
                .warnings(self.result.ship_loss.len(), self.result.convoy_loss.len());
        }
        &self.result
    }
}

/// Store zero-based month + ((hour - HOUR_EPOCH) / 0x2238) * 12,
/// exactly this absolute month encoding.
pub fn month(hour: i32) -> Option<i32> {
    let elapsed = i64::from(hour) - i64::from(engine_native::cdate::HOUR_EPOCH);
    if elapsed < 0 {
        return None;
    }
    let mut day = (elapsed / 24) % 365;
    let mut m = 0;
    while day >= i64::from(engine_native::cdate::MONTH_LENGTHS[m]) {
        day -= i64::from(engine_native::cdate::MONTH_LENGTHS[m]);
        m += 1;
    }
    i32::try_from(elapsed / (365 * 24) * 12 + m as i64).ok()
}

pub fn month_start(bucket: i32) -> Option<i32> {
    if bucket < 0 {
        return None;
    }
    let days = i64::from(bucket / 12) * 365
        + engine_native::cdate::MONTH_LENGTHS[..(bucket % 12) as usize]
            .iter()
            .map(|n| i64::from(*n))
            .sum::<i64>();
    i32::try_from(i64::from(engine_native::cdate::HOUR_EPOCH) + days * 24).ok()
}

/// Serialize cache construction too: simultaneous widgets share one scan.
pub fn cached(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scanned>> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Naval sinkings cancelled",
        ));
    }
    let state = source.state::<Cache>();
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let hit = |cache: &Cache| {
        cache
            .entries
            .iter()
            .find(|(id, _)| *id == branch)
            .map(|(_, scanned)| Arc::clone(scanned))
    };
    let gate = {
        let mut cache = lock();
        if let Some(scanned) = hit(&cache) {
            return Ok(scanned);
        }
        Arc::clone(cache.gates.entry(branch).or_default())
    };
    // A failed or cancelled scan publishes nothing and frees the next request.
    let _scan = gate.lock().unwrap_or_else(|e| e.into_inner());
    let found = hit(&lock());
    if let Some(scanned) = found {
        return Ok(scanned);
    }
    let mut scanned = scan(source.replay_ref(), branch, cancelled)?;
    scanned.histories = country_history::share(source, branch, scanned.histories);
    let scanned = Arc::new(scanned);
    let mut cache = lock();
    cache.entries.push((branch, Arc::clone(&scanned)));
    // Retain the last two scanned branches, matching Country factories.
    if cache.entries.len() > 2 {
        cache.entries.remove(0);
    }
    Ok(scanned)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub mod work;

/// Only attribution operands continue after a sinking row/title has been published.
#[derive(Serialize, Deserialize)]
struct OpenShip {
    victim: u32,
    attacker: u32,
    sink_hour: i32,
    read_hour: i32,
    primary: bool,
}
impl OpenShip {
    fn event(&self) -> ShipEvent {
        ShipEvent {
            entry: SunkShipEntry {
                victim_country: self.victim,
                attacker_country: self.attacker,
                sink_hour: self.sink_hour,
                cause_record: u8::from(!self.primary),
                ..Default::default()
            },
            read_hour: self.read_hour,
            victim: None,
            attacker: None,
            victim_as_of: None,
            attacker_as_of: None,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct OpenConvoy {
    month: i32,
    read_hour: i32,
    count: i64,
    baseline: bool,
    previous_read: Option<i32>,
    owner: u32,
    killer: u32,
}
impl OpenConvoy {
    fn event(&self) -> ConvoyEvent {
        ConvoyEvent {
            month: self.month,
            read_hour: self.read_hour,
            count: self.count,
            baseline: self.baseline,
            previous_read: self.previous_read,
            owner_raw: self.owner,
            killer_raw: self.killer,
            owner: None,
            killer: None,
        }
    }
}

/// The retained-vector scan shared by naval details, the sinking log and tallies.
/// Country attribution uses the same dated holders as the recording scan.
#[derive(Default, Serialize, Deserialize)]
pub struct ShipHistory {
    pub scan: Scanned,
    pub ship_indices: Vec<u32>,
    #[cfg(test)]
    open_ships: Vec<OpenShip>,
    #[cfg(test)]
    open_convoys: Vec<OpenConvoy>,
    next_ship_index: u32,
    track: Tracker,
    next: Option<i64>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    raw_history: HashMap<u32, Vec<TagRun>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    identities: HashMap<String, IdentityTrack>,
    countries: CountryCache,
    #[serde(with = "dashboard_core::map_pairs")]
    prepared_pending: BTreeMap<(bool, u64), Pending>,
    next_convoy_index: u64,
    #[serde(skip)]
    prepared_references: Option<BTreeMap<u32, BTreeSet<(bool, u64)>>>,
    #[cfg(test)]
    #[serde(skip)]
    preparing: bool,
}
impl ShipHistory {
    pub fn new(first_hour: i32) -> Self {
        Self {
            scan: Scanned {
                first_hour,
                last_hour: first_hour,
                ..Default::default()
            },
            next: Some(i64::from(first_hour)),
            ..Default::default()
        }
    }
    pub fn begin(&mut self, first: i32) {
        if self.next.is_none() {
            self.scan.first_hour = first;
            self.next = Some(i64::from(first));
        }
    }
    pub fn finish(&mut self, last_hour: i32) {
        #[cfg(test)]
        self.restore_open();
        self.countries
            .extend(&mut self.scan, &mut self.raw_history, &mut self.identities);
        if let Some(next) = self.next
            && next <= i64::from(last_hour)
        {
            self.scan.missing(&mut self.track, next as i32, last_hour);
            self.next = Some(i64::from(last_hour) + 1);
        }
        self.scan.last_hour = last_hour;
        self.scan.current_holders = if self.countries.hour == Some(last_hour) {
            self.countries.holders.clone()
        } else {
            Vec::new()
        };
        self.resolve();
    }
    fn resolve(&mut self) {
        let out = &mut self.scan;
        let raw_history = &self.raw_history;
        out.uncertain_victims.clear();
        out.uncertain_attackers.clear();
        out.uncertain_convoy_owners.clear();
        out.uncertain_convoy_killers.clear();
        out.unresolved.clear();
        out.warnings.clear();
        for event in &mut out.ships {
            #[cfg(test)]
            work::WORK.with(|work| work.borrow_mut().events += 1);
            let possible = resolve_ship(event, raw_history, &out.country_candidates);
            for ((raw, tag, uncertain), possible) in [
                (
                    event.entry.victim_country,
                    &mut event.victim,
                    &mut out.uncertain_victims,
                ),
                (
                    event.entry.attacker_country,
                    &mut event.attacker,
                    &mut out.uncertain_attackers,
                ),
            ]
            .into_iter()
            .zip(possible)
            {
                if raw != 0 && tag.is_none() {
                    out.unresolved.insert(raw);
                    for identity in possible {
                        uncertain
                            .entry(identity)
                            .or_default()
                            .push((event.entry.sink_hour, event.read_hour));
                    }
                }
            }
        }
        for event in &mut out.convoys {
            #[cfg(test)]
            work::WORK.with(|work| work.borrow_mut().events += 1);
            let possible = resolve_convoy(event, raw_history, &out.country_candidates);
            for ((raw, tag, uncertain), possible) in [
                (
                    event.owner_raw,
                    &mut event.owner,
                    &mut out.uncertain_convoy_owners,
                ),
                (
                    event.killer_raw,
                    &mut event.killer,
                    &mut out.uncertain_convoy_killers,
                ),
            ]
            .into_iter()
            .zip(possible)
            {
                if raw != 0 && tag.is_none() {
                    out.unresolved.insert(raw);
                    for identity in possible {
                        uncertain
                            .entry(identity)
                            .or_default()
                            .push((event.month, event.read_hour));
                    }
                }
            }
        }
        self.update_warnings();
    }
    fn update_warnings(&mut self) {
        let out = &mut self.scan;
        if out.gap_recoveries > 0 {
            out.warnings.push(format!("{} ship-history stretches resumed after missing coverage. Ships born and pruned wholly between reads cannot be counted when the break reaches retention.", out.gap_recoveries));
        }
        for (reason, count) in &out.refused_entries {
            out.warnings
                .push(format!("{count} naval entries refused: {reason}."));
        }
        if !out.unresolved.is_empty() {
            out.warnings.push(format!(
                "{} naval country references are unresolved.",
                out.unresolved.len()
            ));
        }
    }

    #[cfg(test)]
    fn restore_open(&mut self) {
        if self.scan.ships.is_empty() && !self.open_ships.is_empty() {
            self.scan.ships = self.open_ships.iter().map(OpenShip::event).collect();
        }
        if self.scan.convoys.is_empty() && !self.open_convoys.is_empty() {
            self.scan.convoys = self.open_convoys.iter().map(OpenConvoy::event).collect();
        }
    }
    #[cfg(test)]
    pub fn compact(&mut self) {
        let mut indices = Vec::new();
        self.open_ships.clear();
        self.open_convoys.clear();
        for (index, event) in std::mem::take(&mut self.ship_indices)
            .into_iter()
            .zip(std::mem::take(&mut self.scan.ships))
        {
            if (event.entry.victim_country != 0 && event.victim.is_none())
                || (event.entry.attacker_country != 0 && event.attacker.is_none())
            {
                indices.push(index);
                self.open_ships.push(OpenShip {
                    victim: event.entry.victim_country,
                    attacker: event.entry.attacker_country,
                    sink_hour: event.entry.sink_hour,
                    read_hour: event.read_hour,
                    primary: event.entry.cause_record == 0 && event.entry.assisted_kill == 0,
                });
            }
        }
        self.ship_indices = indices;
        for event in std::mem::take(&mut self.scan.convoys) {
            if (event.owner_raw != 0 && event.owner.is_none())
                || (event.killer_raw != 0 && event.killer.is_none())
            {
                self.open_convoys.push(OpenConvoy {
                    month: event.month,
                    read_hour: event.read_hour,
                    count: event.count,
                    baseline: event.baseline,
                    previous_read: event.previous_read,
                    owner: event.owner_raw,
                    killer: event.killer_raw,
                });
            }
        }
        self.scan.ship_reads.clear();
        self.scan.convoy_reads.clear();
    }

    pub fn resume(&mut self, frame: &DecodedFrame) {
        #[cfg(test)]
        self.restore_open();
        if self.scan.last_ship_read == Some(frame.hour)
            && frame.complete::<tables::SunkShips>().is_ok()
        {
            self.track.ships = Some(frame.tables[TableId::SunkShips.index()].clone());
        }
    }

    pub fn observe(&mut self, format: replay_data::format::FormatVersion, frame: &DecodedFrame) {
        self.observe_with_changes(format, frame, None);
    }
    fn observe_with_changes(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        changes: Option<&FrameChanges>,
    ) {
        self.begin_hour(frame.hour);
        let tags = countries(
            frame,
            &mut self.scan,
            &mut self.raw_history,
            &mut self.identities,
            &mut self.countries,
        );
        let ships = self.scan.ships.len();
        self.scan
            .observe_tables(&mut self.track, format, frame, tags, changes);
        self.end_hour(frame.hour, ships);
    }

    /// [`Self::observe_with_changes`] of an hour a columnar walk read.
    fn observe_projected(
        &mut self,
        hour: i32,
        countries: Option<Option<&ProjectedCountries>>,
        tokens: Option<&[(u32, Vec<u8>)]>,
        ships: &ShipRead,
        convoys: &ConvoyRead,
    ) {
        self.begin_hour(hour);
        let update = match countries {
            None => self.countries.links.update_projected(None, None),
            Some(None) => Ok(false),
            Some(Some(read)) => self
                .countries
                .links
                .update_projected(Some(&read.keys), Some(&read.links)),
        };
        let tags = countries_with(
            hour,
            update,
            || {
                let read = countries
                    .flatten()
                    .expect("rebuilt link tags read their country tables");
                (read.countries.as_slice(), read.link_rows.as_slice())
            },
            &mut self.scan,
            &mut self.raw_history,
            &mut self.identities,
            &mut self.countries,
        );
        let ships_before = self.scan.ships.len();
        self.scan
            .observe_projected_tables(&mut self.track, hour, tags, tokens, ships, convoys);
        self.end_hour(hour, ships_before);
    }

    fn begin_hour(&mut self, hour: i32) {
        self.scan.new_candidates.clear();
        self.scan.new_tags.clear();
        self.scan.changed_references.clear();
        #[cfg(test)]
        if !self.preparing {
            self.restore_open();
        }
        if self.next.is_none() {
            self.scan.first_hour = hour;
        }
        let first = *self.next.get_or_insert(i64::from(hour));
        if first < i64::from(hour) {
            self.scan.missing(&mut self.track, first as i32, hour - 1);
        }
        self.scan.last_hour = hour;
    }

    /// Numbers the ships the hour found past the first `ships`.
    fn end_hour(&mut self, hour: i32, ships: usize) {
        for _ in ships..self.scan.ships.len() {
            self.ship_indices.push(self.next_ship_index);
            self.next_ship_index = self
                .next_ship_index
                .checked_add(1)
                .expect("ship indices exceed the product's u32 row contract");
        }
        self.next = Some(i64::from(hour) + 1);
    }

    /// The entries a walk compared last become those the next read compares with.
    pub(crate) fn continue_from(&mut self, base: ShipBase) {
        self.track.ships = None;
        self.track.ship_future = base.futures.first_key_value().map(|(&hour, _)| hour);
        self.track.ship_counts = base.counts;
        self.track.refused_ships = base.refused;
    }

    /// Forgets its views, as a block read alone with a new decoder holds none.
    pub fn forget_views(&mut self) {
        self.track.ships = None;
        self.track.dictionary = None;
    }
}

/// A columnar walk's country tables, as the link tags and holders take them.
#[derive(replay_spool::Spooled)]
pub(crate) struct ProjectedCountries {
    pub(crate) keys: dashboard_text::metadata::CountryKeys,
    pub(crate) links: HashMap<u32, u32>,
    pub(crate) countries: Vec<CountryRow>,
    pub(crate) link_rows: Vec<CountryLinkRow>,
}

impl ProjectedCountries {
    /// The bytes it holds on the heap, about: each country's key and row, and each link.
    pub(crate) fn heap_bytes(&self) -> usize {
        let tags: usize = self.countries.iter().map(|row| row.tag.capacity()).sum();
        self.countries.capacity() * size_of::<CountryRow>()
            + self.countries.len() * size_of::<(u32, u32, Vec<u8>)>()
            + 2 * tags
            + self.links.capacity() * 8 / 7 * (size_of::<(u32, u32)>() + 1)
            + self.link_rows.capacity() * size_of::<CountryLinkRow>()
    }

    /// `countries` as (id, slot, tag) and `links` as (id, slot), each ascending by id.
    pub(crate) fn new(countries: Vec<(u32, u32, Vec<u8>)>, links: Vec<(u32, u32)>) -> Self {
        let rows = countries
            .iter()
            .map(|(id, slot, tag)| CountryRow {
                id: *id,
                canonical_slot: *slot,
                tag: tag.clone(),
                ..CountryRow::default()
            })
            .collect();
        let link_rows: Vec<CountryLinkRow> = links
            .iter()
            .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect();
        Self {
            keys: dashboard_text::metadata::CountryKeys::from_keys(countries),
            links: dashboard_text::metadata::link_slots(&link_rows),
            countries: rows,
            link_rows,
        }
    }
}
