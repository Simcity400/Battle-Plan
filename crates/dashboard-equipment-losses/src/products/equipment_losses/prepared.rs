//! Shared all-country loss observations, sparse exact values and native-dated ship events.
//!
//! Each segment's lens folds every country on the walk's workers ([`lens`]); the emit binds the
//! countries whose state the lens began from and splices what the lens made of them once per
//! segment. Each hour's facts are queued as a step; a drain folds the queued steps over the
//! countries no lens holds, in contiguous shards, each country keeping its own series at the
//! branch's hours. A flush publishes every country's series as the one section. A leaf's change
//! is an empty event at its hour: its total is the key's value there.
use super::*;
use super::columnar::{self, Drains, Facts, Input, Metadata, Records, btree_bytes};
#[cfg(test)]
use dashboard_base::prepared::series::Run;
use dashboard_base::prepared::{
    coverage,
    prefix::Prefix,
    series::{self, Accumulator, Reader},
};
use serde_json::{Value, json};
use std::{mem::size_of, sync::Arc};

pub(super) mod lens;

type Identity = (u32, u32);

/// A key the catalogue names once its step is folded: its step, whether it is a leaf, its
/// country or order of making, and its key in its country's series.
type Name = ((u64, u8, u64, u64), Identity, Option<LeafKey>);

#[derive(Serialize, Deserialize)]
struct Country {
    scanned: Scanned,
    state: State,
    #[serde(with = "dashboard_core::map_pairs")]
    totals: BTreeMap<LeafKey, i128>,
    /// Each leaf's key in `values`; the country's own is 0.
    #[serde(with = "dashboard_core::map_pairs")]
    leaves: BTreeMap<LeafKey, u64>,
    /// The published key of each of `values`' keys.
    names: Vec<u64>,
    values: Accumulator,
    last_read: Option<String>,
    #[serde(skip)]
    read_key: Option<(Option<Coverage>, Option<String>, u8)>,
    /// Whether the steps folded so far hold it current, and found.
    #[serde(skip)]
    current: Option<bool>,
    /// The unresolved sets it last took in, by read: one handed on again adds nothing.
    #[serde(skip)]
    taken_unresolved: [Option<Arc<BTreeSet<u32>>>; 4],
    #[serde(skip)]
    membership: Option<census::Membership>,
    /// The first step it takes.
    #[serde(skip)]
    born: u64,
    /// The queued ticks its series took.
    #[serde(skip)]
    ticked: usize,
    /// Whether the segment's lens folds it: the emit's drains pass it by.
    #[serde(skip)]
    lensed: bool,
    /// In a lens, what it made of the segment's hours.
    #[serde(skip)]
    out: Option<Box<lens::Out>>,
}

/// The existing naming and uncertainty inputs contain changes, not hourly values.
#[derive(Deserialize)]
struct CountryMetadata {
    scanned: Scanned,
    #[serde(with = "dashboard_core::map_pairs")]
    leaves: BTreeMap<LeafKey, u64>,
}

/// [`CountryMetadata`] as a flush writes it: its country's spans, and its leaves' published keys.
#[derive(Serialize)]
struct CountryMetadataRef<'a> {
    scanned: &'a Scanned,
    #[serde(with = "dashboard_core::map_pairs")]
    leaves: BTreeMap<LeafKey, u64>,
}

#[derive(Serialize, Deserialize)]
struct ShipLoss {
    sink_hour: i32,
    variant: Option<u32>,
    delta_raw: String,
}

/// A call every country's series takes in turn: an hour's advance, or a gap's end.
#[derive(Clone, Copy)]
enum Tick {
    Advance(i32),
    Through(i32),
}

/// What the countries take of an hour, or of the hours a gap missed.
struct Step {
    index: u64,
    kind: Kind,
}

enum Kind {
    Hour(Box<Taken>),
    /// Hours `first..=last` no item gave, after the lend-lease read `lend_read`.
    Missing {
        first: i32,
        last: i32,
        lend_read: Option<i32>,
    },
}

/// An hour's facts, and the countries it selects.
struct Taken {
    at: i32,
    interrupted: bool,
    /// Whether both identity tables were complete: a country they did not hold is absent.
    complete: bool,
    /// The countries current from this hour, each found or unresolved.
    current: Option<Arc<BTreeMap<Identity, bool>>>,
    memberships: Option<Memberships>,
    /// Countries selected beside the slots' found ones: those current before or after it.
    listed: Vec<Identity>,
    /// Whether it selects every country, or every found one.
    every: bool,
    every_found: bool,
    /// Slots, ascending, whose found countries a membership change selects.
    slots: Vec<u32>,
    lend_process: bool,
    /// The lend-lease read before this hour.
    lend_read: Option<i32>,
    metadata: Arc<Metadata>,
    drains: Arc<Drains>,
    records: Arc<Vec<(u32, Records)>>,
    unresolved: [Arc<BTreeSet<u32>>; 4],
    /// The ticks through this hour's advance.
    tick: usize,
}

/// An hour's change of the countries' division memberships.
struct Memberships {
    /// Every membership is unknown before `changes`.
    reset: bool,
    changes: Vec<(u32, Option<census::Membership>)>,
}

/// Steps since the last drain, with the ticks the series take through them and the countries
/// they made, in order.
#[derive(Default)]
struct Queue {
    steps: Vec<Step>,
    ticks: Vec<Tick>,
    made: Vec<(u64, Identity)>,
    /// The bytes the steps own past the vectors, each shared input counted once.
    kept: usize,
    /// The shared inputs of the hour queued last.
    shared: Option<(Arc<Metadata>, Arc<Drains>)>,
}

impl Taken {
    /// The bytes it owns, itself included, apart from its shared inputs.
    fn heap(&self) -> usize {
        let current = self
            .current
            .as_ref()
            .map_or(0, |current| {
                // Its own Arc allocation, even when empty, as well as its nodes.
                columnar::ARC
                    + size_of::<BTreeMap<Identity, bool>>()
                    + btree_bytes::<Identity, bool>(current.len())
            });
        let memberships = self.memberships.as_ref().map_or(0, |memberships| {
            memberships.changes.capacity() * size_of::<(u32, Option<census::Membership>)>()
        });
        let records: usize = self
            .records
            .iter()
            .map(|(_, records)| records.heap())
            .sum();
        let unresolved: usize = self
            .unresolved
            .iter()
            .map(|slots| btree_bytes::<u32, ()>(slots.len()))
            .sum();
        size_of::<Self>()
            + current
            + memberships
            + self.listed.capacity() * size_of::<Identity>()
            + self.slots.capacity() * size_of::<u32>()
            + self.records.capacity() * size_of::<(u32, Records)>()
            + records
            + unresolved
    }
}

impl Queue {
    /// Queues hour `taken` as step `index`; a shared input is new where it differs from the last.
    fn hour(&mut self, index: u64, taken: Taken) {
        let (metadata, drains) = (&taken.metadata, &taken.drains);
        let before = self.shared.as_ref();
        let mut kept = taken.heap();
        if before.is_none_or(|(old, _)| !Arc::ptr_eq(old, metadata)) {
            kept += metadata.heap();
            if before.is_none_or(|(old, _)| !Arc::ptr_eq(&old.drain, &metadata.drain)) {
                kept += columnar::drain_heap(&metadata.drain);
            }
            if before.is_none_or(|(old, _)| !Arc::ptr_eq(&old.land, &metadata.land)) {
                kept += columnar::land_heap(&metadata.land);
            }
        }
        if before.is_none_or(|(_, old)| !Arc::ptr_eq(old, drains)) {
            kept += drains.heap();
        }
        self.kept += kept;
        self.shared = Some((Arc::clone(metadata), Arc::clone(drains)));
        self.steps.push(Step {
            index,
            kind: Kind::Hour(Box::new(taken)),
        });
    }

    /// The bytes it holds.
    fn held(&self) -> usize {
        self.kept
            + self.steps.capacity() * size_of::<Step>()
            + self.ticks.capacity() * size_of::<Tick>()
            + self.made.capacity() * size_of::<(u64, Identity)>()
    }
}


/// What a shard's fold made: its countries' new leaves, by step, and the designs that lost.
#[derive(Default)]
struct Folded {
    fresh: Vec<(u64, Identity, u64, LeafKey)>,
    variants: BTreeSet<u32>,
}

/// A shard's fold failure, at the step and country where serial folding would meet it.
struct Failed {
    at: (u64, Identity),
    error: io::Error,
}

/// Steps a drain takes at most.
const QUEUED: usize = 256;
/// Bytes queued steps keep past their items' release: four of the run's 8 MiB output chunks.
const QUEUED_BYTES: usize = 32 << 20;
/// Fold work, in countries' weights, past which it is shared out over spare cores.
const SHARD_WORK: usize = 2048;
/// The least work a shard is given.
const SHARD_LEAST: usize = 256;

#[derive(Serialize, Deserialize)]
pub struct Prepared {
    next: i64,
    interrupted: bool,
    keys: BTreeMap<String, u64>,
    #[serde(with = "dashboard_core::map_pairs")]
    countries: BTreeMap<Identity, Country>,
    absent: Scanned,
    absent_state: State,
    variants: VariantCatalogue,
    loss_variants: BTreeSet<u32>,
    variant_counts: BTreeMap<u64, usize>,
    /// The hours every country's series take: their batch is published with its bounds.
    clock: Accumulator,
    lend_read: Option<i32>,
    #[serde(skip)]
    steps: u64,
    #[serde(skip)]
    current: Arc<BTreeMap<Identity, bool>>,
    #[serde(skip)]
    memberships: Option<BTreeMap<u32, Option<census::Membership>>>,
    /// Whether the last observed hour lacked complete Countries or CountryLinks.
    #[serde(skip)]
    identities_incomplete: bool,
    #[serde(skip)]
    queue: Queue,
    /// The most the queued steps held at once.
    #[serde(skip)]
    queued_peak: usize,
    /// In a segment's lens, what it begins its countries from; None in the emit.
    #[serde(skip)]
    lens: Option<Box<lens::Mode>>,
    /// The segment a lens folds the bound countries of, until it hands them back.
    #[serde(skip)]
    span: Option<Box<lens::Span>>,
    /// Keys made since the catalogue last named them, and how many countries were made.
    #[serde(skip)]
    names: Vec<Name>,
    #[serde(skip)]
    made: u64,
}

fn state(format: replay_data::format::FormatVersion) -> State {
    State {
        land_source: LandSource::CombatLog,
        include_all_destruction: true,
        destruction_coverage_not_recorded: format
            .field_gaps(TableId::EquipmentDestruction)
            .next()
            .is_some(),
        ..Default::default()
    }
}
fn key(keys: &mut BTreeMap<String, u64>, value: &impl Serialize) -> io::Result<u64> {
    let next = keys.len() as u64;
    Ok(*keys.entry(serde_json::to_string(value)?).or_insert(next))
}

/// Takes the last lend-lease read into a country's exchange dates where it was read since.
fn exchanges_read(country: &mut Country, read: Option<i32>) {
    if !country
        .scanned
        .unread_active
        .contains(&LossRecord::LendLease)
        && let Some(read) = read
    {
        country.state.exchanges_read = Some(
            country
                .state
                .exchanges_read
                .map_or(read, |seen| seen.max(read)),
        );
    }
}

/// Takes `country`'s series through `ticks[..through]`, consecutive advances as one span.
fn catch_up(country: &mut Country, ticks: &[Tick], through: usize) -> io::Result<()> {
    let mut at = country.ticked;
    while at < through {
        match ticks[at] {
            Tick::Advance(first) => {
                let mut last = first;
                at += 1;
                while at < through {
                    match ticks[at] {
                        Tick::Advance(next) if last.checked_add(1) == Some(next) => {
                            last = next;
                            at += 1;
                        }
                        _ => break,
                    }
                }
                country.values.advance_span(first, last)?;
            }
            Tick::Through(hour) => {
                country.values.through(hour);
                at += 1;
            }
        }
    }
    country.ticked = country.ticked.max(through);
    Ok(())
}

/// Runs `step` on contiguous shares of `items` by `weigh`, over spare cores once the work
/// passes a bound, and gives back what each gave in order.
fn shared<T: Send, R: Send>(
    items: &mut [T],
    weigh: impl Fn(&T) -> usize,
    step: impl Fn(&mut [T]) -> R + Sync,
) -> Vec<R> {
    let total: usize = items.iter().map(&weigh).sum();
    let workers = std::thread::available_parallelism()
        .map_or(1, |cores| (cores.get() / 2).max(1))
        .min(total / SHARD_LEAST)
        .min(items.len());
    if total < SHARD_WORK || workers <= 1 {
        return vec![step(items)];
    }
    let share = total.div_ceil(workers);
    let mut chunks = Vec::with_capacity(workers);
    let mut rest = items;
    while !rest.is_empty() {
        let mut end = rest.len();
        if chunks.len() + 1 < workers {
            let mut filled = 0;
            end = 0;
            while end < rest.len() && filled < share {
                filled += weigh(&rest[end]);
                end += 1;
            }
        }
        let (chunk, tail) = std::mem::take(&mut rest).split_at_mut(end);
        chunks.push(chunk);
        rest = tail;
    }
    let context = replay_frame::cpu::Context::capture();
    let memory = engine_allocation::labels::current();
    let step = &step;
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let context = context.clone();
                job_priority::spawn_in(scope, move || {
                    let _memory = engine_allocation::labels::resume(memory);
                    let _cpu = context.enter("equipment_losses fold");
                    step(chunk)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

/// The countries of `shard` holding `slot`.
fn holding(shard: &[(Identity, &mut Country)], slot: u32) -> std::ops::Range<usize> {
    let first = shard.partition_point(|(identity, _)| identity.0 < slot);
    let last = shard.partition_point(|(identity, _)| identity.0 <= slot);
    first..last
}

/// The countries of `shard` an hour selects, ascending.
fn chosen(shard: &[(Identity, &mut Country)], taken: &Taken, index: u64) -> Vec<usize> {
    let born = |at: usize| shard[at].1.born <= index && !shard[at].1.lensed;
    if taken.every {
        return (0..shard.len()).filter(|&at| born(at)).collect();
    }
    let found = |at: usize| born(at) && shard[at].1.current == Some(true);
    let mut chosen = Vec::new();
    if taken.every_found || taken.lend_process {
        chosen.extend((0..shard.len()).filter(|&at| {
            found(at) && (taken.every_found || !shard[at].1.state.exchanges.is_empty())
        }));
    }
    let (Some(lowest), Some(highest)) = (shard.first(), shard.last()) else {
        return chosen;
    };
    let range = (lowest.0.0, highest.0.0);
    let mut slots = |slots: &mut dyn Iterator<Item = u32>| {
        for slot in slots
            .skip_while(|slot| *slot < range.0)
            .take_while(|slot| *slot <= range.1)
        {
            chosen.extend(holding(shard, slot).filter(|&at| found(at)));
        }
    };
    slots(&mut taken.drains.slots.iter().map(|item| item.0));
    slots(&mut taken.records.iter().map(|item| item.0));
    slots(&mut taken.slots.iter().copied());
    for identity in &taken.listed {
        if let Ok(at) = shard.binary_search_by_key(identity, |(identity, _)| *identity)
            && born(at)
        {
            chosen.push(at);
        }
    }
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}

/// Folds the queued steps over a shard of the countries, ascending by identity, and takes
/// each country's series through the queue's ticks.
fn fold(shard: &mut [(Identity, &mut Country)], queue: &Queue) -> Result<Folded, Failed> {
    let _cpu = replay_frame::cpu::scope_phase("equipment_losses:fold");
    let mut folded = Folded::default();
    for step in &queue.steps {
        let index = step.index;
        match &step.kind {
            Kind::Missing {
                first,
                last,
                lend_read,
            } => {
                // A lens's country meets no gap: its lens stops short of one, but where lost.
                for (_, country) in shard
                    .iter_mut()
                    .filter(|(_, country)| country.born <= index && !country.lensed)
                {
                    country.scanned.advance(first.saturating_sub(1));
                    exchanges_read(country, *lend_read);
                    country.scanned.missing(*first, *last, LandSource::CombatLog);
                }
            }
            Kind::Hour(taken) => {
                if let Some(current) = &taken.current {
                    for (identity, country) in shard.iter_mut() {
                        if country.born <= index {
                            country.current = current.get(&*identity).copied();
                        }
                    }
                }
                if let Some(memberships) = &taken.memberships {
                    if memberships.reset {
                        for (_, country) in shard.iter_mut() {
                            if country.born <= index {
                                country.membership = None;
                            }
                        }
                    }
                    for &(slot, membership) in &memberships.changes {
                        for at in holding(shard, slot) {
                            let country = &mut shard[at].1;
                            if country.born <= index {
                                country.membership = membership;
                            }
                        }
                    }
                }
                for at in chosen(shard, taken, index) {
                    let (identity, country) = &mut shard[at];
                    let identity = *identity;
                    take(identity, country, taken, index, &queue.ticks, &mut folded).map_err(
                        |error| Failed {
                            at: (index, identity),
                            error,
                        },
                    )?;
                }
                // A lens leaves ships to the emit: their counts are kept from the branch's start.
                for (slot, records) in taken.records.iter() {
                    let Some(ships) = records.ships.as_deref() else {
                        continue;
                    };
                    for at in holding(shard, *slot) {
                        let (identity, country) = &mut shard[at];
                        if country.lensed && country.born <= index && country.current == Some(true)
                        {
                            let identity = *identity;
                            sunk(country, taken.at, ships, &mut folded).map_err(|error| Failed {
                                at: (index, identity),
                                error,
                            })?;
                        }
                    }
                }
            }
        }
    }
    // Without ticks (a lens's queue) no country's place in them moved from the start.
    let ticked = if queue.ticks.is_empty() { &mut [][..] } else { shard };
    for (identity, country) in ticked.iter_mut() {
        let identity = *identity;
        catch_up(country, &queue.ticks, queue.ticks.len()).map_err(|error| Failed {
            at: (u64::MAX, identity),
            error,
        })?;
        country.ticked = 0;
    }
    Ok(folded)
}

/// A bound country's ships read at `hour`: each count past the last seen sank anew.
fn sunk(country: &mut Country, hour: i32, ships: &[ShipRead], folded: &mut Folded) -> io::Result<()> {
    let mut scanned = Scanned::default();
    ship_counts(&mut scanned, &mut country.state, ships);
    for ((_, variant), events) in scanned.events {
        if let Some(variant) = variant {
            folded.variants.insert(variant);
        }
        for (sink_hour, delta) in events {
            country.values.event(
                0,
                hour,
                &ShipLoss {
                    sink_hour,
                    variant,
                    delta_raw: delta.to_string(),
                },
            )?;
        }
    }
    Ok(())
}

/// One country's hour: its spans, its reads' amounts, and their publication.
fn take(
    identity: Identity,
    country: &mut Country,
    taken: &Taken,
    index: u64,
    ticks: &[Tick],
    folded: &mut Folded,
) -> io::Result<()> {
    catch_up(country, ticks, taken.tick)?;
    let at = taken.at;
    // A lens's first take of a country it began from the restart leaves the dates it read
    // lend-lease to the emit, which knows whether that record was read the hour before.
    let begun = country
        .out
        .as_deref_mut()
        .is_some_and(|out| out.begun && !std::mem::replace(&mut out.taken, true));
    if !begun {
        exchanges_read(country, taken.lend_read);
    }
    country.scanned.advance(at.saturating_sub(1));
    let selected = match country.current {
        Some(true) => Selection::Found(identity.0),
        Some(false) => Selection::Unresolved,
        None if taken.complete => Selection::Absent,
        None => Selection::Unresolved,
    };
    let mut drained: &[Drained] = &[];
    country_metadata_with(
        &mut country.scanned,
        &mut country.state,
        at,
        &taken.metadata.table,
        selected,
        taken.metadata.reads,
        true,
        Some(taken.metadata.land.country(identity.0, country.membership)),
        &taken.metadata.drain,
        |_, _| {
            drained = taken.drains.of(identity.0)?;
            Ok(())
        },
    )?;
    if matches!(selected, Selection::Found(_)) {
        let records = taken
            .records
            .binary_search_by_key(&identity.0, |item| item.0)
            .ok()
            .map(|at| &taken.records[at].1);
        let (scanned, state) = (&mut country.scanned, &mut country.state);
        let mut lens = country.out.as_deref_mut();
        if lens.is_none()
            && let Some(ships) = records.and_then(|records| records.ships.as_deref())
        {
            ship_counts(scanned, state, ships);
        }
        if let Some(records) = records {
            match &mut lens {
                Some(out) => convoy_rows(scanned, state, at, &records.convoys, |key, count| {
                    out.first_convoy(at, key, count)
                }),
                None => convoy_rows(scanned, state, at, &records.convoys, |_, _| {}),
            }
        }
        // A country holding exchanges takes every lend-lease read, to see them end.
        if taken.lend_process
            && (records.is_some_and(|records| !records.lend.is_empty())
                || !taken.unresolved[2].is_empty()
                || !state.exchanges.is_empty())
        {
            let pools = records.map_or(&[][..], |records| records.lend.as_slice());
            match &mut lens {
                Some(out) => out.lend(scanned, state, at, pools),
                None => lend_lease_rows(scanned, state, at, pools),
            }
        }
        if let Some(records) = records {
            match &mut lens {
                Some(out) => combat_rows(scanned, state, at, &records.combat, |key, amount| {
                    out.first_log(at, key, amount)
                }),
                None => combat_rows(scanned, state, at, &records.combat, |_, _| {}),
            }
        }
        // The sets only grow: one this country took in already adds nothing.
        for (read, unresolved) in taken.unresolved.iter().enumerate() {
            let held = &mut country.taken_unresolved[read];
            if held.as_ref().is_some_and(|held| Arc::ptr_eq(held, unresolved)) {
                continue;
            }
            let into = if read < 3 {
                &mut scanned.unresolved
            } else {
                &mut scanned.combat_unresolved
            };
            into.extend(unresolved.iter().copied());
            *held = Some(Arc::clone(unresolved));
        }
    }
    publish(identity, country, at, taken.interrupted, drained, index, folded)
}

/// Publishes a country's hour into its series: its read's key, and each leaf it changed.
fn publish(
    identity: Identity,
    country: &mut Country,
    hour: i32,
    interrupted: bool,
    drains: &[Drained],
    index: u64,
    folded: &mut Folded,
) -> io::Result<()> {
    let coverage = country.scanned.coverage.last().map(|span| span.coverage);
    let reason = country
        .scanned
        .destruction_coverage
        .last()
        .and_then(|span| span.reason.as_deref());
    let unread = country
        .scanned
        .unread
        .iter()
        .filter(|(_, spans)| spans.last().is_some_and(|span| span.last_hour == hour))
        .fold(0_u8, |mask, (record, _)| mask | (1 << (*record as u8)));
    let mut changed_read = false;
    if country.read_key.as_ref().is_none_or(|held| {
        held.0 != coverage || held.1.as_deref() != reason || held.2 != unread
    }) {
        let read = serde_json::to_string(&(
            coverage,
            reason,
            country
                .scanned
                .unread
                .keys()
                .filter(|record| unread & (1 << (**record as u8)) != 0)
                .collect::<Vec<_>>(),
        ))?;
        changed_read = country.last_read.as_ref() != Some(&read);
        country.last_read = Some(read);
        country.read_key = Some((coverage, reason.map(str::to_owned), unread));
    }
    if let Some(out) = country.out.as_deref_mut() {
        if changed_read {
            out.versions.push(hour);
            if out.first_read.is_none() {
                out.first_read = country.last_read.clone().map(|read| (hour, read));
            }
        }
        return out.publish(
            &mut country.scanned,
            &mut country.totals,
            &mut country.leaves,
            hour,
            drains,
        );
    }
    let version = country
        .values
        .state(0)
        .and_then(|state| state.last)
        .map_or(0, |run| run.value)
        + i128::from(changed_read);
    if changed_read || interrupted {
        country.values.observe(0, hour, Some(version), false);
    }
    let events = std::mem::take(&mut country.scanned.events);
    let mut amounts = Vec::with_capacity(events.len() + drains.len());
    for (leaf, events) in events {
        if let Some(variant) = leaf.1 {
            folded.variants.insert(variant);
        }
        if leaf.0 == LossSource::SunkShips {
            for (sink_hour, delta) in events {
                country.values.event(
                    0,
                    hour,
                    &ShipLoss {
                        sink_hour,
                        variant: leaf.1,
                        delta_raw: delta.to_string(),
                    },
                )?;
            }
            continue;
        }
        let mut delta = 0;
        for (at, amount) in events {
            if at != hour {
                return Err(io::Error::other(format!(
                    "Equipment loss cause {:?} has an unsupported backdated hour {at}",
                    leaf.0
                )));
            }
            delta += amount;
        }
        amounts.push((leaf, delta));
    }
    for &(cause, variant, delta) in drains {
        folded.variants.insert(variant);
        amounts.push((
            (LossSource::Destruction(cause), Some(variant)),
            i128::from(delta),
        ));
    }
    amounts.sort_unstable_by_key(|item| item.0);
    let mut changed = Vec::with_capacity(amounts.len());
    for (leaf, delta) in amounts {
        *country.totals.entry(leaf).or_default() += delta;
        let next = country.leaves.len() as u64 + 1;
        if let std::collections::btree_map::Entry::Vacant(entry) = country.leaves.entry(leaf) {
            entry.insert(next);
            folded.fresh.push((index, identity, next, leaf));
        }
        if changed.last() != Some(&leaf) {
            changed.push(leaf);
        }
    }
    let history = interrupted.then(|| country.leaves.keys().copied().collect::<Vec<_>>());
    let observed = history.as_deref().unwrap_or(&changed);
    for &leaf in observed {
        let key = country.leaves[&leaf];
        let value = country.totals[&leaf];
        let changed = changed.binary_search(&leaf).is_ok();
        country
            .values
            .observe_boundary(key, hour, Some(value), false, changed);
        if changed {
            country.values.event_raw(key, hour, Vec::new())?;
        }
    }
    Ok(())
}

impl Prepared {
    pub fn new(replay: &Replay, branch: u64, first: i32) -> Self {
        let first = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map_or(first, |start| branch_bounds(replay, start).0);
        Self::starting(replay.format, first)
    }

    /// A branch's state whose hours, gaps included, start at `first`.
    pub fn starting(format: replay_data::format::FormatVersion, first: i32) -> Self {
        Self {
            next: i64::from(first),
            interrupted: false,
            keys: BTreeMap::new(),
            countries: BTreeMap::new(),
            absent: Scanned {
                format,
                ..Default::default()
            },
            absent_state: state(format),
            variants: Default::default(),
            loss_variants: BTreeSet::new(),
            variant_counts: BTreeMap::new(),
            clock: Accumulator::default(),
            lend_read: None,
            steps: 0,
            current: Arc::default(),
            memberships: None,
            identities_incomplete: false,
            queue: Queue::default(),
            queued_peak: 0,
            lens: None,
            span: None,
            names: Vec::new(),
            made: 0,
        }
    }

    /// The most bytes its queued steps held at once.
    pub fn queued_peak(&self) -> usize {
        self.queued_peak
    }

    pub fn projector(&self, replay: &Replay) -> io::Result<super::columnar::Projector> {
        super::columnar::Projector::new(replay)
    }

    /// Takes up what the segment's lens hands on, then queues the hour's facts as a step.
    pub fn observe_projected(&mut self, hour: super::columnar::Hour) -> io::Result<()> {
        let _cpu =
            replay_frame::cpu::scope("equipment_losses.columnar.emit", Some("equipment_losses"));
        let at = hour.hour;
        let begun = match hour.lens.map(|lens| *lens) {
            Some(lens::Handed::Begun(begun)) => Some(begun),
            Some(lens::Handed::Part(part)) => {
                self.splice(part)?;
                None
            }
            None => None,
        };
        match hour.input {
            Facts::Read(input) => {
                if let Some(begun) = &begun {
                    self.begin(begun, at)?;
                }
                if self.observe_input(at, &input)? {
                    self.variants
                        .observe_hour(at, input.names, |_| input.variants);
                }
                if let Some(begun) = begun {
                    self.bind(begun);
                }
            }
            Facts::Other => {
                if i64::from(at) >= self.next {
                    self.forget_runtime();
                }
                return Ok(());
            }
            Facts::Lens => return Ok(()),
        }
        let held = self.queue.held();
        self.queued_peak = self.queued_peak.max(held);
        if self.queue.steps.len() >= QUEUED || held >= QUEUED_BYTES {
            self.drain()?;
        }
        Ok(())
    }

    /// Queues an hour's facts as a step, after what the branch itself keeps of it: its hours,
    /// identities, memberships and the absent country's spans; whether it was the branch's next.
    fn observe_input(&mut self, at: i32, input: &Input) -> io::Result<bool> {
        if i64::from(at) < self.next {
            return Ok(false);
        }
        let interrupted = self.interrupted || self.next < i64::from(at);
        if self.next < i64::from(at) {
            self.finish_block(at - 1);
        }
        if self.lens.is_none() {
            self.clock.advance(at)?;
            self.queue.ticks.push(Tick::Advance(at));
        }
        self.absent.advance(at.saturating_sub(1));
        let index = self.steps;
        self.steps += 1;
        let mut listed = Vec::new();
        let mut current = None;
        if let Some(identities) = &input.identities {
            listed.extend(self.current.keys().copied());
            let mut now = BTreeMap::new();
            for &(identity, resolved) in identities {
                if resolved {
                    if !self.countries.contains_key(&identity) {
                        self.make(identity, input.format, index);
                    }
                    now.entry(identity)
                        .and_modify(|found| *found = false)
                        .or_insert(true);
                } else if self.countries.contains_key(&identity) {
                    now.insert(identity, false);
                }
            }
            listed.extend(now.keys().copied());
            let now = Arc::new(now);
            current = Some(Arc::clone(&now));
            self.current = now;
        }
        let mut slots = Vec::new();
        let mut memberships = None;
        if input.memberships_whole || !input.membership_complete {
            if let Some(old) = &self.memberships {
                slots.extend(old.keys().copied());
            }
            self.memberships = input.membership_complete.then(BTreeMap::new);
            memberships = Some(Memberships {
                reset: true,
                changes: Vec::new(),
            });
        }
        if input.membership_complete {
            let held = self.memberships.get_or_insert_with(BTreeMap::new);
            let mut changes = Vec::new();
            for &(slot, membership) in &input.memberships {
                if held.insert(slot, membership) != Some(membership) {
                    slots.push(slot);
                    changes.push((slot, membership));
                }
            }
            if !changes.is_empty() {
                memberships
                    .get_or_insert(Memberships {
                        reset: false,
                        changes: Vec::new(),
                    })
                    .changes = changes;
            }
        }
        slots.sort_unstable();
        slots.dedup();
        let every = input.refresh || interrupted || !input.complete || self.identities_incomplete;
        self.identities_incomplete = !input.complete;
        let absent = if input.complete {
            Selection::Absent
        } else {
            Selection::Unresolved
        };
        country_metadata_with(
            &mut self.absent,
            &mut self.absent_state,
            at,
            &input.metadata.table,
            absent,
            input.metadata.reads,
            true,
            None,
            &input.metadata.drain,
            |_, _| Ok(()),
        )?;
        let lend_read = self.lend_read;
        if input.lend_read {
            self.lend_read = Some(at);
        }
        let tick = self.queue.ticks.len();
        self.queue.hour(
            index,
            Taken {
                at,
                interrupted,
                complete: input.complete,
                current,
                memberships,
                listed,
                every,
                every_found: input.unresolved.iter().any(|raw| !raw.is_empty()),
                slots,
                lend_process: input.lend_process,
                lend_read,
                metadata: Arc::clone(&input.metadata),
                drains: Arc::clone(&input.drains),
                records: Arc::clone(&input.records),
                unresolved: input.unresolved.clone(),
                tick,
            },
        );
        self.next = i64::from(at) + 1;
        self.interrupted = false;
        Ok(true)
    }

    /// A country first resolved at step `index`, going on from the absent country's spans.
    fn make(&mut self, identity: Identity, format: replay_data::format::FormatVersion, index: u64) {
        let mut continuation = state(format);
        continuation
            .destruction_withheld
            .clone_from(&self.absent_state.destruction_withheld);
        let membership = self
            .memberships
            .as_ref()
            .and_then(|memberships| memberships.get(&identity.0).copied().flatten());
        let mut scanned = self.absent.clone();
        let mut out = None;
        let mut lensed = false;
        if let Some(mode) = &self.lens {
            out = Some(Box::new(mode.begin(identity, index, &mut continuation)));
        } else if let Some(span) = self
            .span
            .as_deref()
            .filter(|span| span.newborns() && index > span.index)
        {
            // The lens begins it from its absent country, as the emit's stood before the segment.
            scanned = span.absent.clone();
            continuation.destruction_withheld.clone_from(&span.withheld);
            lensed = true;
        }
        self.countries.insert(
            identity,
            Country {
                scanned,
                state: continuation,
                totals: BTreeMap::new(),
                leaves: BTreeMap::new(),
                names: Vec::new(),
                values: self.clock.follower(),
                last_read: None,
                read_key: None,
                current: None,
                taken_unresolved: Default::default(),
                membership,
                born: index,
                ticked: self.queue.ticks.len(),
                lensed,
                out,
            },
        );
        self.queue.made.push((index, identity));
    }

    /// Folds the queued steps over every country, and names the keys they made in the order
    /// they were made.
    fn drain(&mut self) -> io::Result<()> {
        let queue = std::mem::take(&mut self.queue);
        if queue.ticks.is_empty() && queue.steps.is_empty() {
            return Ok(());
        }
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.fold", None);
        let mut countries: Vec<(Identity, &mut Country)> = self
            .countries
            .iter_mut()
            .map(|(identity, country)| (*identity, country))
            .collect();
        let mut folded = Vec::new();
        let mut failed: Option<Failed> = None;
        // A lens folds on its walk's worker; the emit shares its countries out over spare cores.
        let shards = if self.lens.is_some() {
            vec![fold(&mut countries, &queue)]
        } else {
            shared(
                &mut countries,
                |(_, country)| 1 + country.leaves.len() * usize::from(!country.lensed),
                |shard| fold(shard, &queue),
            )
        };
        for shard in shards {
            match shard {
                Ok(shard) => folded.push(shard),
                Err(error) => {
                    if failed.as_ref().is_none_or(|first| error.at < first.at) {
                        failed = Some(error);
                    }
                }
            }
        }
        if let Some(failed) = failed {
            return Err(failed.error);
        }
        // A lens names nothing: the emit numbers what it hands on.
        if self.lens.is_some() {
            return Ok(());
        }
        // Keys are numbered as they were first met: by step, a step's new countries first.
        for &(index, identity) in &queue.made {
            self.names
                .push(((index, 0, self.made, 0), identity, None));
            self.made += 1;
        }
        for shard in folded {
            self.loss_variants.extend(shard.variants);
            self.names
                .extend(shard.fresh.into_iter().map(|(index, identity, local, leaf)| {
                    let packed = u64::from(identity.0) << 32 | u64::from(identity.1);
                    ((index, 1, packed, local), identity, Some(leaf))
                }));
        }
        // A bound country's keys are met with the segment's hand-over, which names them all.
        if !self.span.as_deref().is_some_and(lens::Span::live) {
            self.name()?;
        }
        Ok(())
    }

    /// Names the keys made since, in the order they were first met.
    fn name(&mut self) -> io::Result<()> {
        let mut names = std::mem::take(&mut self.names);
        names.sort_unstable_by_key(|item| item.0);
        for ((_, _, _, local), identity, leaf) in names {
            let name = match leaf {
                None => key(&mut self.keys, &("country", identity))?,
                Some(leaf) => key(&mut self.keys, &("loss", identity, leaf))?,
            };
            let country = self
                .countries
                .get_mut(&identity)
                .expect("named loss country");
            if country.names.len() as u64 != local {
                return Err(io::Error::other("Equipment loss keys out of order"));
            }
            country.names.push(name);
        }
        Ok(())
    }

    fn forget_runtime(&mut self) {
        self.current = Arc::default();
        self.memberships = None;
        self.identities_incomplete = false;
    }

    /// Hours through `last` were lost from the recording, after those its last flush published:
    /// they are missing, and every country's totals restart after them.
    pub fn lost(&mut self, last: i32) -> io::Result<()> {
        self.drain()?;
        self.abandon()?;
        self.finish_block(last);
        self.drain()?;
        for country in self.countries.values_mut() {
            for total in country.totals.values_mut() {
                *total = 0;
            }
        }
        Ok(())
    }

    pub fn finish_block(&mut self, last: i32) {
        if self.next > i64::from(last) {
            return;
        }
        let first = self.next as i32;
        self.absent.advance(first.saturating_sub(1));
        self.variants.forget(first);
        self.absent.missing(first, last, LandSource::CombatLog);
        self.queue.steps.push(Step {
            index: self.steps,
            kind: Kind::Missing {
                first,
                last,
                lend_read: self.lend_read,
            },
        });
        self.steps += 1;
        self.clock.through(last);
        self.queue.ticks.push(Tick::Through(last));
        self.next = i64::from(last) + 1;
        self.interrupted = true;
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("equipment_losses:series_flush");
        self.drain()?;
        self.abandon()?;
        if let Ok(last) = i32::try_from(self.next - 1) {
            self.absent.advance(last);
            for country in self.countries.values_mut() {
                country.scanned.advance(last);
                exchanges_read(country, self.lend_read);
            }
        }
        self.clock.flush_with(
            self.countries
                .values_mut()
                .map(|country| (&mut country.values, country.names.as_slice())),
            writer,
            &format!("equipment-loss-values-{branch}"),
            5,
            cancelled,
        )?;
        let mut countries: Vec<&mut Country> = self.countries.values_mut().collect();
        let written = shared(
            &mut countries,
            |country| 1 + country.leaves.len(),
            |shard| {
                shard
                    .iter()
                    .map(|country| {
                        dashboard_text::metadata::check_cancel(cancelled)?;
                        let leaves = country
                            .leaves
                            .iter()
                            .map(|(&leaf, &local)| (leaf, country.names[local as usize]))
                            .collect();
                        let json = serde_json::to_vec(&CountryMetadataRef {
                            scanned: &country.scanned,
                            leaves,
                        })?;
                        Ok((country.names[0], series::compress(&json, 3)?))
                    })
                    .collect::<io::Result<Vec<_>>>()
            },
        )
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;
        for (actor, bytes) in written.into_iter().flatten() {
            series::write_compressed(
                writer,
                &format!("equipment-loss-country-{branch}-{actor}"),
                0,
                &bytes,
            )?;
        }
        for &variant in &self.loss_variants {
            let handle = u64::from(variant) << 32 | u64::from(VARIANT_TYPE);
            let Some(versions) = self.variants.versions.get(&handle) else {
                continue;
            };
            if self.variant_counts.get(&handle) == Some(&versions.len()) {
                continue;
            }
            let actor = key(&mut self.keys, &("variant", handle))?;
            series::write_reply(
                writer,
                &format!("equipment-loss-variant-{branch}-{actor}"),
                0,
                versions,
            )?;
            self.variant_counts.insert(handle, versions.len());
        }
        series::write_catalogue(writer, "equipment-loss", branch, &self.keys, &())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    selection: LossesParams,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    reads: Vec<i32>,
    head_hour: i32,
    detail_hour: i32,
}

/// Reached values come from runs; until complete, later sinking observations
/// and type tokens can revise any prefix hour.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreparedLossesReply {
    pub branch_id: String,
    pub tag: String,
    pub land_source: LandSource,
    pub land_lower_bound: Option<String>,
    pub amount_scale: u32,
    pub causes: Vec<LossCause>,
    pub designs: Vec<LossDesign>,
    pub land_coverage: Vec<CoverageSpan>,
    pub destruction_coverage: Vec<DestructionCoverageSpan>,
    pub destruction_counted_from: Option<i32>,
    pub retained_months: Option<i32>,
    pub warnings: Vec<String>,
    /// The first hour of each stretch after hours the recording lost: every total restarts
    /// there, and a total at a later hour counts from the last of them before it.
    pub restarts: Vec<i32>,
    #[serde(flatten)]
    pub prefix: Prefix,
    pub approximate: Vec<dashboard_base::products::country_groups::HourSpan>,
    pub prepared_reads: Vec<i32>,
    pub prepared_head_hour: i32,
    pub prepared_detail_hour: i32,
    pub prepared_from: f64,
    pub prepared_to: f64,
    pub prepared_width: u32,
}

/// The total of leaf `actor` at `hour`, an hour it changed: its value there.
fn total_at(reader: &mut Reader<'_>, actor: u64, hour: i32) -> io::Result<i128> {
    reader
        .value_at(actor, hour)?
        .map(|run| run.value)
        .ok_or_else(|| dashboard_core::invalid("A prepared loss change has no total"))
}

fn sampled_events(
    reader: &mut Reader<'_>,
    actor: u64,
    from: f64,
    to: f64,
    width: u32,
    reads: &[i32],
) -> io::Result<Vec<(i32, i128)>> {
    let summary = reader.summary(
        actor,
        (from.floor() as i32, to.ceil() as i32),
        (from, to, width),
        5,
    )?;
    let mut hours: BTreeSet<i32> = reads.iter().copied().collect();
    // Native cause uncertainty starts at its first loss, even when that
    // amount is neither viewport extremum nor a reached read.
    if let Some(first) = reader
        .events(&[actor], i32::MIN, i32::MAX, 0, 1)?
        .rows
        .first()
    {
        hours.insert(first.hour);
    }
    if let Some(runs) = summary.runs {
        hours.extend(runs.into_iter().map(|run| run.first_hour));
    } else {
        for bucket in summary.buckets.into_iter().flatten() {
            hours.extend([
                bucket.first_hour,
                bucket.min_hour,
                bucket.max_hour,
                bucket.last_value_hour,
            ]);
        }
    }
    // Palette weights use the complete requested counting interval's signed
    // peak, independently of the chart viewport.
    let first = reads.iter().min().copied().unwrap_or(from.floor() as i32);
    let last = reads.iter().max().copied().unwrap_or(to.ceil() as i32);
    for bucket in reader
        .summary(
            actor,
            (first, last),
            (f64::from(first), f64::from(last), 1),
            5,
        )?
        .buckets
        .into_iter()
        .flatten()
    {
        hours.extend([bucket.min_hour, bucket.max_hour]);
    }
    let mut samples = BTreeMap::new();
    for hour in hours {
        if let Some((changed, _)) = reader.latest_event(actor, hour)? {
            samples.insert(changed, total_at(reader, actor, changed)?);
        }
    }
    // Each stretch after lost hours publishes totals counted from its own start.
    let (mut previous, mut stretch) = (0, None);
    Ok(samples
        .into_iter()
        .map(|(hour, value)| {
            let since = coverage::since(super::FAMILY, hour);
            if since != stretch {
                (previous, stretch) = (0, since);
            }
            let delta = value - previous;
            previous = value;
            (hour, delta)
        })
        .collect())
}

/// Each leaf's running totals restarted at `restarts`, the first hours of the stretches after
/// hours the recording lost: a total at a stretch's first hour counts from there, never across.
fn restart_totals(causes: &mut [LossCause], restarts: &[i32]) -> io::Result<()> {
    if restarts.is_empty() {
        return Ok(());
    }
    let parse = |point: &LossPoint| {
        point
            .total_raw
            .parse::<i128>()
            .map_err(|_| dashboard_core::invalid("Invalid prepared loss amount"))
    };
    for leaf in causes.iter_mut().flat_map(|cause| cause.leaves.iter_mut()) {
        let mut points = Vec::with_capacity(leaf.points.len() + restarts.len());
        // The running total summed from the branch's start, its stretch's start, and the total
        // shown last.
        let (mut offset, mut last, mut shown) = (0_i128, 0_i128, 0_i128);
        let mut next = restarts.iter().copied().peekable();
        let zero = |hour: i32| LossPoint {
            hour,
            total_raw: "0".into(),
        };
        for point in &leaf.points {
            let total = parse(point)?;
            while let Some(since) = next.next_if(|&since| since <= point.hour) {
                offset = last;
                if since < point.hour && shown != 0 {
                    points.push(zero(since));
                }
                shown = 0;
            }
            last = total;
            shown = total - offset;
            points.push(LossPoint {
                hour: point.hour,
                total_raw: shown.to_string(),
            });
        }
        for since in next {
            if shown != 0 {
                points.push(zero(since));
            }
            shown = 0;
        }
        leaf.points = points;
    }
    Ok(())
}

pub fn query(
    source: &Source,
    raw: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(raw.clone())?;
    EquipmentLosses::validate(&params.selection).map_err(dashboard_core::invalid)?;
    if !params.reads.contains(&params.head_hour)
        || !params.reads.contains(&params.detail_hour)
        || params.reads.windows(2).any(|hours| hours[0] >= hours[1])
    {
        return Err(dashboard_core::invalid(
            "Invalid prepared equipment loss reached hours",
        ));
    }
    let selected = params.selection.country.ok_or_else(|| {
        dashboard_core::invalid("Choose a recorded native country for equipment losses")
    })?;
    let branch = params.selection.branch();
    let origin_ready = validate_origin(source, &params.selection, context, cancelled)?;
    let coverage = dashboard_base::prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        "equipment-losses",
        branch,
        None,
        cancelled,
    )?
    .coverage;
    // Until the selected holder's origin is witnessed, none of its totals
    // can be published even if the losses family is ahead of identities.
    let coverage = if origin_ready {
        coverage
    } else {
        dashboard_base::prepared::prefix::extent(source, branch, None)?
    };
    let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
    // Every loss leaf is a cumulative signed total: before coverage completes,
    // a read after the prepared prefix answers at the prefix's last hour, the
    // viewport ends there, and the later hours are pending.
    let requested = params.reads.iter().copied().chain([to.ceil() as i32]).max();
    let prefix = Prefix::new(source, branch, coverage, requested);
    let mut params = params;
    let (from, to) = through_prefix(&mut params, (from, to), &prefix);
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let mut scanned = if origin_ready {
        read_scanned(
            &store,
            &source_key,
            branch,
            (selected.slot, selected.country_id),
            (from, to, width),
            &params.reads,
            source.replay_ref().format,
            cancelled,
        )?
        .unwrap_or_default()
    } else {
        Scanned::default()
    };
    scanned.format = source.replay_ref().format;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let mut reply = reply_with_content(source, &params.selection, context, &scanned)?;
    let restarts = coverage::restarts(super::FAMILY, i32::MIN, i32::MAX);
    restart_totals(&mut reply.causes, &restarts)?;
    let result = PreparedLossesReply {
        branch_id: reply.branch_id,
        tag: reply.tag,
        land_source: reply.land_source,
        land_lower_bound: reply.land_lower_bound,
        amount_scale: reply.amount_scale,
        causes: reply.causes,
        designs: reply.designs,
        land_coverage: reply.coverage,
        destruction_coverage: reply.destruction_coverage,
        destruction_counted_from: reply.destruction_counted_from,
        retained_months: reply.retained_months,
        warnings: reply.warnings,
        restarts,
        approximate: prefix.revisable(source, branch),
        prefix,
        prepared_reads: params.reads,
        prepared_head_hour: params.head_hour,
        prepared_detail_hour: params.detail_hour,
        prepared_from: from,
        prepared_to: to,
        prepared_width: width,
    };
    dashboard_base::products::check_reply_budget(&result)?;
    serde_json::to_value(result).map_err(io::Error::from)
}

/// The request a reply answers from `prefix`: before coverage completes, each
/// reached hour after the prefix reads its last hour, and the viewport ends
/// there. The reads stay ordered and hold the head and detail hours.
fn through_prefix(params: &mut Params, (from, to): (f64, f64), prefix: &Prefix) -> (f64, f64) {
    if prefix.coverage.complete {
        return (from, to);
    }
    let last = prefix.coverage.last_hour;
    params.reads = params.reads.iter().map(|&hour| hour.min(last)).collect();
    params.reads.dedup();
    params.head_hour = params.head_hour.min(last);
    params.detail_hour = params.detail_hour.min(last);
    let to = to.min(f64::from(last));
    (from.min(to), to)
}

fn validate_origin(
    source: &Source,
    selection: &LossesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<bool> {
    let selected = selection.country.ok_or_else(|| {
        dashboard_core::invalid("Choose a recorded native country for equipment losses")
    })?;
    let branch = selection.branch();
    let history = match dashboard_base::products::country_history::prepared_branch_histories(
        source,
        branch,
        context.content.as_ref(),
        cancelled,
    )? {
        Some(history) => history,
        None => {
            let Some(frame) = dashboard_base::prepared::query::progressive::frame_at(
                source,
                branch,
                selected.origin_hour,
                &[
                    replay_data::rows::TableId::Campaign,
                    replay_data::rows::TableId::Countries,
                    replay_data::rows::TableId::CountryLinks,
                ],
                cancelled,
            )?
            else {
                return Ok(false);
            };
            let mut scan = dashboard_base::products::country_history::HistoryScan::new(false, &[]);
            scan.observe(&frame);
            Arc::new(scan.finish())
        }
    };
    let witnessed = history
        .countries
        .get(&selection.tag)
        .is_some_and(|history| {
            history.identities.iter().any(|identity| {
                identity.first_hour <= selected.origin_hour
                    && selected.origin_hour <= identity.last_hour
                    && identity.slot == selected.slot
                    && identity.country_id == selected.country_id
            })
        });
    if !witnessed {
        return Err(dashboard_core::refused(
            "The selected native country identity is not recorded at its origin",
        ));
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn read_scanned(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    identity: Identity,
    (from, to, width): (f64, f64, u32),
    reads: &[i32],
    format: replay_data::format::FormatVersion,
    cancelled: &AtomicBool,
) -> io::Result<Option<Scanned>> {
    let Some((keys, ())) =
        series::read_catalogue::<()>(store, source_key, "equipment-loss", branch)?
    else {
        return Ok(None);
    };

    let Some(&country) = keys.get(&serde_json::to_string(&("country", identity))?) else {
        return Ok(None);
    };
    let Some(mut metadata) = series::read_reply::<CountryMetadata>(
        store,
        source_key,
        &format!("equipment-loss-country-{branch}-{country}"),
        0,
    )?
    else {
        return Ok(None);
    };
    let (from, to, width) = series::viewport(Some(from), Some(to), Some(width))?;
    // A leaf sends its summary's sampled hours, not its buckets: the reply is measured whole.
    series::summary_budget(from, to, width, 1)?;
    metadata.scanned.format = format;
    let mut reader = Reader::new(
        store,
        source_key,
        format!("equipment-loss-values-{branch}"),
        cancelled,
    )?;
    for (&leaf, &actor) in &metadata.leaves {
        metadata.scanned.events.insert(
            leaf,
            sampled_events(&mut reader, actor, from, to, width, reads)?,
        );
    }
    // The sparse ship key is ordered by observation. Native sinking dates
    // are applied by the existing points builder, even after prefix commits.
    let mut ships = Reader::new(
        store,
        source_key,
        format!("equipment-loss-values-{branch}"),
        cancelled,
    )?;
    let mut cursor = 0;
    loop {
        let page = ships.events(&[country], i32::MIN, i32::MAX, cursor, 1024)?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(dashboard_core::invalid(
                "Prepared ship loss page made no progress",
            ));
        }
        for row in page.rows {
            let ship: ShipLoss = serde_json::from_slice(&row.row)?;
            let delta = ship
                .delta_raw
                .parse::<i128>()
                .map_err(|_| dashboard_core::invalid("Invalid prepared ship loss amount"))?;
            metadata
                .scanned
                .events
                .entry((LossSource::SunkShips, ship.variant))
                .or_default()
                .push((ship.sink_hour, delta));
        }
        let Some(next) = page.cursor else {
            break;
        };
        cursor = next;
    }
    for variant in metadata
        .scanned
        .events
        .keys()
        .filter_map(|(_, variant)| *variant)
        .collect::<BTreeSet<_>>()
    {
        let handle = u64::from(variant) << 32 | u64::from(VARIANT_TYPE);
        if let Some(&actor) = keys.get(&serde_json::to_string(&("variant", handle))?)
            && let Some(versions) = series::read_reply(
                store,
                source_key,
                &format!("equipment-loss-variant-{branch}-{actor}"),
                0,
            )?
        {
            metadata.scanned.variants.versions.insert(handle, versions);
        }
    }
    // Metadata is replaced as preparation advances. An earlier requested head
    // must not acquire a gap or unread interval observed only after that head.
    let last = reads.iter().copied().max().unwrap_or(to.ceil() as i32);
    let scanned = &mut metadata.scanned;
    scanned.last_hour = scanned.last_hour.map(|hour| hour.min(last));
    scanned.coverage.retain_mut(|span| {
        span.last_hour = span.last_hour.min(last);
        span.first_hour <= span.last_hour
    });
    scanned.destruction_coverage.retain_mut(|span| {
        span.last_hour = span.last_hour.min(last);
        span.first_hour <= span.last_hour
    });
    scanned.destruction_unknown.retain_mut(|span| {
        span.last_hour = span.last_hour.min(last);
        span.first_hour <= span.last_hour
    });
    for spans in scanned.unread.values_mut() {
        spans.retain_mut(|span| {
            span.last_hour = span.last_hour.min(last);
            span.first_hour <= span.last_hour
        });
    }
    scanned.ended.retain(|exchange| exchange.from <= last);
    scanned.destruction_counted_from = scanned
        .destruction_counted_from
        .filter(|hour| *hour <= last);
    scanned.events.retain(|_, events| {
        events.retain(|(hour, _)| *hour <= last);
        !events.is_empty()
    });
    for versions in scanned.variants.versions.values_mut() {
        versions.retain(|version| version.first_hour <= last);
    }
    Ok(Some(metadata.scanned))
}

pub fn series_query(
    source: &Source,
    product: &str,
    raw: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let selection: LossesParams = serde_json::from_value(raw["family"]["selection"].clone())?;
    EquipmentLosses::validate(&selection).map_err(dashboard_core::invalid)?;
    if raw["branchId"].as_str() != Some(selection.branch_id.as_str()) {
        return Err(dashboard_core::invalid(
            "Equipment loss selection branch does not match its request",
        ));
    }
    if !validate_origin(source, &selection, context, cancelled)? {
        return Ok(Value::Null);
    }
    let identity = selection.country.ok_or_else(|| {
        dashboard_core::invalid("Choose a recorded native country for equipment losses")
    })?;
    let branch = selection.branch();
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let Some((keys, ())) =
        series::read_catalogue::<()>(&store, &source_key, "equipment-loss", branch)?
    else {
        return Ok(Value::Null);
    };
    let Some(&country) = keys.get(&serde_json::to_string(&(
        "country",
        (identity.slot, identity.country_id),
    ))?) else {
        return Ok(Value::Null);
    };
    let Some(metadata) = series::read_reply::<CountryMetadata>(
        &store,
        &source_key,
        &format!("equipment-loss-country-{branch}-{country}"),
        0,
    )?
    else {
        return Ok(Value::Null);
    };
    let wanted: Option<Vec<LeafKey>> = raw
        .get("leaves")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?;
    let actors: Vec<_> = wanted
        .as_ref()
        .map_or_else(
            || metadata.leaves.values().copied().collect::<Vec<_>>(),
            |leaves| {
                leaves
                    .iter()
                    .filter_map(|leaf| metadata.leaves.get(leaf))
                    .copied()
                    .collect()
            },
        )
        .into_iter()
        .chain(std::iter::once(country))
        .collect();
    let mut reader = Reader::new(
        &store,
        &source_key,
        format!("equipment-loss-values-{branch}"),
        cancelled,
    )?;
    match product {
        "series-next-change" => {
            let navigation: series::Navigation = serde_json::from_value(raw.clone())?;
            let origin = navigation.origin()?;
            let forward = matches!(
                navigation.key,
                series::MeasureKey::Home | series::MeasureKey::ArrowRight
            );
            let candidate = series::navigate(
                &mut [(
                    &mut reader,
                    actors.into_iter().map(|actor| (actor, None)).collect(),
                )],
                &navigation,
            )?;
            let mut reached = candidate.filter(|hour| {
                if forward {
                    *hour > origin
                } else {
                    *hour < origin
                }
            });
            let mut cursor = 0;
            loop {
                let page = reader.events(&[country], i32::MIN, i32::MAX, cursor, 1024)?;
                if page
                    .cursor
                    .is_some_and(|next| next <= cursor || page.rows.is_empty())
                {
                    return Err(dashboard_core::invalid(
                        "Prepared ship loss page made no progress",
                    ));
                }
                for row in page.rows {
                    let ship: ShipLoss = serde_json::from_slice(&row.row)?;
                    if wanted.as_ref().is_some_and(|leaves| {
                        !leaves.contains(&(LossSource::SunkShips, ship.variant))
                    }) {
                        continue;
                    }
                    let hour = f64::from(ship.sink_hour);
                    if hour < navigation.first_hour
                        || hour > navigation.last_hour
                        || if forward {
                            hour <= origin
                        } else {
                            hour >= origin
                        }
                    {
                        continue;
                    }
                    reached = Some(reached.map_or(hour, |before| {
                        if forward {
                            before.min(hour)
                        } else {
                            before.max(hour)
                        }
                    }));
                }
                let Some(next) = page.cursor else {
                    break;
                };
                cursor = next;
            }
            Ok(json!({"hour": reached.or(candidate)}))
        }
        "events-page" => {
            let (from, to, cursor, size) = series::event_window(
                raw["from"].as_f64(),
                raw["to"].as_f64(),
                raw["cursor"].as_str(),
                raw["pageSize"]
                    .as_u64()
                    .and_then(|size| u32::try_from(size).ok()),
            )?;
            let page = reader.events(&actors, from, to, cursor, size)?;
            let mut rows = Vec::with_capacity(page.rows.len());
            for row in page.rows {
                // A leaf's change holds nothing: its point is its total there.
                let point = if row.key == country {
                    serde_json::from_slice::<serde_json::Value>(&row.row)?
                } else {
                    let total = total_at(&mut reader, row.key, row.hour)?;
                    json!({"hour": row.hour, "totalRaw": total.to_string()})
                };
                rows.push(json!({"key":row.key.to_string(), "hour":row.hour, "ordinal":row.ordinal.to_string(), "row":point}));
            }
            Ok(
                json!({"firstOrdinal":page.first.to_string(), "total":page.total.to_string(), "cursor":page.cursor.map(|cursor| cursor.to_string()),
                "rows": rows}),
            )
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(
                raw["from"].as_f64(),
                raw["to"].as_f64(),
                raw["width"]
                    .as_u64()
                    .and_then(|width| u32::try_from(width).ok()),
            )?;
            let mut lines = Vec::new();
            for actor in actors {
                lines.push(json!({"key":actor.to_string(), "summary":reader.event_clusters(actor, from, to, width)?}));
            }
            Ok(json!({"series":lines}))
        }
        "series-state" | "series-summary" => {
            let reads: Vec<i32> = raw["reads"].as_array().map_or_else(Vec::new, |reads| {
                reads
                    .iter()
                    .filter_map(|read| {
                        read["hour"]
                            .as_i64()
                            .and_then(|hour| i32::try_from(hour).ok())
                    })
                    .collect()
            });
            let mut reads = reads;
            let head = reads
                .first()
                .copied()
                .or_else(|| raw["to"].as_f64().map(|hour| hour.floor() as i32))
                .ok_or_else(|| dashboard_core::invalid("Missing equipment loss hour"))?;
            reads.push(head);
            reads.sort_unstable();
            reads.dedup();
            query(
                source,
                &json!({"selection":selection, "from":raw["from"].as_f64().unwrap_or(f64::from(head)), "to":raw["to"].as_f64().unwrap_or(f64::from(head)),
                "width":raw["width"].as_u64().unwrap_or(1), "reads":reads, "headHour":head, "detailHour":head}),
                context,
                cancelled,
            )
        }
        _ => Err(dashboard_core::invalid(
            "Unknown equipment-loss series query",
        )),
    }
}

#[cfg(test)]
mod columnar_parity;
#[cfg(test)]
mod tests;
