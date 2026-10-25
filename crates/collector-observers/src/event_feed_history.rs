//! Owned native observations and pointer-free hourly publications with exact receipts.

pub(crate) use crate::script_event_observer::staging::Witness;
use engine_allocation as allocation;
use engine_events::event_feed_sites as native;
use engine_native::Memory;
use replay_data::{
    event_feed_rows::{
        EventFeedCause, EventFeedCountry, EventFeedCounts, EventFeedCoverage, EventFeedEvent,
        EventFeedHistoryRow, EventFeedScope, EventFeedScript, EventFeedScriptChange,
        EventFeedSource,
    },
    rows::Recorded,
};
use std::{
    io,
    mem::size_of,
    sync::atomic::{AtomicU64, Ordering},
};

/// More detour witnesses staged in one hour than prepared: refused by name,
/// never grown on a callback.
pub(crate) const WITNESS_STORAGE: &str = crate::script_event_observer::staging::WITNESS_STORAGE;
/// Witnesses a frame abandoned on an unwind, dropped at the next boundary.
pub(crate) const WITNESS_ABANDONED: &str = crate::script_event_observer::staging::WITNESS_ABANDONED;
/// An open detour's witnesses a boundary reset before it published.
pub(crate) const WITNESS_RESET: &str = crate::script_event_observer::staging::WITNESS_RESET;

/// One latch's witness, tagged with its frame token. The arena bumps within
/// an hour and resets at the boundary, like the token and scope arenas; a
/// publisher marks its frame's witnesses consumed.
#[derive(Clone, Copy)]
struct Staged {
    token: u64,
    witness: Witness,
    consumed: bool,
}

/// A seal whose receipt room could not be grown off the lease.
pub const RECEIPT_STORAGE: &str = "Event feed publication receipts need prepared storage";

/// Publication outcome: accepted, discarded, or still in flight.
pub type Receipt = (u64, Option<bool>);

/// Bumped whenever an owner certificate is retired, or an owner the next
/// census would baseline is refused. Constant work, so native callbacks may
/// bump it.
pub(crate) static RECOVERY: AtomicU64 = AtomicU64::new(1);
/// The latest epoch a complete census visited every live owner in.
pub(crate) static RECOVERED: AtomicU64 = AtomicU64::new(0);
pub(crate) fn recovery_needed() {
    RECOVERY.fetch_add(1, Ordering::AcqRel);
}
/// The registries an owner refusal belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerRegistry {
    Delayed,
    Decisions,
}
/// Since load: refusals of an owner already refused in the same loss epoch.
/// They emit no marker; uncertified owners still request recovery.
pub(crate) static REPEATED_OWNER_REFUSALS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
pub(crate) fn repeated_owner_refusal(registry: OwnerRegistry) {
    REPEATED_OWNER_REFUSALS[registry as usize].fetch_add(1, Ordering::Relaxed);
}

/// Since load: baseline samples a native writer crossed, indexed by
/// [`OwnerRegistry`] then [`REGISTRY_SAMPLE`]. None is certified.
pub(crate) static CROSSED_SAMPLES: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
pub(crate) const REGISTRY_SAMPLE: usize = 2;
fn crossed_sample(sample: usize) {
    CROSSED_SAMPLES[sample].fetch_add(1, Ordering::Relaxed);
}
/// An owner baseline a native writer crossed: refused, and retried at the
/// next visit rather than recorded as a failed attempt.
pub(crate) const OWNER_CROSSED: &str = "owner baseline crossed a native writer";
/// A recovery baseline never grows storage under the shared-store lease. It
/// raises the demand presize prepares, and is retried at the next visit.
pub(crate) const OWNER_STORAGE: &str = "owner baseline needs prepared storage";

/// Each owed registry may need one local storage/crossed refusal. Successful
/// baselines and ordinary occurrences leave its entry and reason bytes alone.
const OWNER_CONTROL_TOKEN: usize = if OWNER_STORAGE.len() > OWNER_CROSSED.len() {
    OWNER_STORAGE.len()
} else {
    OWNER_CROSSED.len()
};

/// Prepared indexes out of room, or private off-lease baseline growth failed.
/// These remain storage pressure, never a failed native-content attempt.
const OWNER_SHORTAGES: [&str; 10] = [
    "decision owner certificates need prepared capacity",
    "decision pointer certificates need prepared capacity",
    "decision pointer certificates are full",
    "delayed-event pointer index needs prepared storage",
    "event registry baseline occurrence allocation failed",
    "event registry baseline token allocation failed",
    "event registry baseline graph allocation failed",
    "event registry baseline scope allocation failed",
    "decision baseline certificate allocation failed",
    "delayed-event baseline identity allocation failed",
];

/// A baseline sample's writer cut, taken inside a worker lease. It holds
/// while no native writer has attempted the store or run a bracketed
/// original since it was taken, and none is in flight. Nothing waits for a
/// native writer: a crossed sample is refused instead.
#[derive(Clone, Copy)]
pub(crate) struct Cut(u64);
impl Cut {
    /// `None` while a native writer is in flight.
    pub(crate) fn take() -> Option<Self> {
        crate::event_feed_observer::writer_cut().map(Self)
    }
    pub(crate) fn holds(self) -> bool {
        crate::event_feed_observer::writer_cut_holds(self.0)
    }
}

/// An owner baseline's refusal, kept with that owner.
#[derive(Clone, Copy)]
pub(crate) struct OwnerRefusal {
    pub reason: &'static str,
    /// A rejection was already counted inside the baseline.
    pub counted: bool,
    /// Retry acquisition at the next census boundary.
    pub retry: bool,
}

/// A recorded token: the native writer's string, or a capture refusal's
/// static reason. Neither allocates on a callback.
#[derive(Clone, Copy)]
enum Token {
    Native(engine_native::msvc_string::Descriptor),
    Text(&'static str),
}
impl Token {
    fn len(self) -> usize {
        match self {
            Self::Native(descriptor) => descriptor.len,
            Self::Text(text) => text.len(),
        }
    }
    fn copy(self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        match self {
            Self::Native(descriptor) => descriptor.copy(memory, output),
            Self::Text(text) => {
                output
                    .get_mut(..text.len())
                    .ok_or("string output too short")?
                    .copy_from_slice(text.as_bytes());
                Ok(())
            }
        }
    }
}

/// The observed world boundary and interval coverage.
#[derive(Clone, Copy)]
pub struct OwnerBoundary {
    pub generation: u64,
    pub read_hour: i32,
    pub coverage: EventFeedCoverage,
    pub failure: Option<&'static str>,
}

#[derive(Debug)]
#[repr(C)]
pub struct Ticket {
    slot: usize,
    sequence: u64,
}

/// A failed completion retains its ticket; it cannot retry settlement.
#[derive(Debug)]
pub struct Retained {
    pub ticket: Ticket,
    pub reason: &'static str,
}

enum Proof {
    War(native::WarPending),
    Annex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    PublishedPending,
    Known,
    Failed,
    Published,
}

struct Entry {
    event: EventFeedEvent,
    proof: Option<Proof>,
    state: State,
    publication: Option<u64>,
    dispatch_frame: usize,
    dispatch_scope: usize,
    dispatch_definition: usize,
}

/// Pointer-free hourly copy. Its allocation and later encoding are off-thread.
pub struct Snapshot {
    pub boundary: OwnerBoundary,
    counts: Option<EventFeedCounts>,
    outstanding: u64,
    events: Vec<EventFeedEvent>,
    script_tokens: Vec<u8>,
    script_scopes: Vec<EventFeedScope>,
    publication: u64,
    pub required: usize,
}
impl Snapshot {
    pub fn new(capacity: usize) -> Self {
        Self {
            boundary: OwnerBoundary {
                generation: 0,
                read_hour: 0,
                coverage: EventFeedCoverage::Unavailable,
                failure: Some("Event feed not drained"),
            },
            counts: None,
            outstanding: 0,
            events: allocation::owned_vec(allocation::vec(capacity)),
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
            publication: 0,
            required: 0,
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.events.capacity() * size_of::<EventFeedEvent>()
            + self.script_tokens.capacity()
            + self.script_scopes.capacity() * size_of::<EventFeedScope>()
    }
    pub fn grow(&mut self, capacity: usize) -> io::Result<()> {
        let additional = capacity.saturating_sub(self.events.len());
        allocation::reserve_exact(&mut self.events, additional)
    }
    /// Capacity for a seal of `demand`.
    pub fn reserve(&mut self, demand: SealDemand) -> Result<(), &'static str> {
        self.grow(demand.events)
            .map_err(|_| "Event feed snapshot allocation failed")?;
        let tokens = demand.tokens.saturating_sub(self.script_tokens.len());
        allocation::reserve_exact(&mut self.script_tokens, tokens)
            .map_err(|_| "event feed token snapshot allocation failed")?;
        let scopes = demand.scopes.saturating_sub(self.script_scopes.len());
        allocation::reserve_exact(&mut self.script_scopes, scopes)
            .map_err(|_| "event feed scope snapshot allocation failed")
    }
    /// Off the shared-store lease: room for a measured seal, with headroom
    /// for facts callbacks add before the sealing lease.
    pub fn reserve_with_headroom(&mut self, demand: SealDemand) -> Result<(), &'static str> {
        let headroom = |value: usize, floor: usize| value.saturating_add((value / 4).max(floor));
        self.reserve(SealDemand {
            events: headroom(demand.events, 64),
            tokens: headroom(demand.tokens, 1024),
            scopes: headroom(demand.scopes, 64),
        })
    }
    /// Callbacks the store declined while the seal held it refuse this
    /// output and count in its rejections.
    pub(crate) fn declined_during_seal(&mut self, declined: u64) {
        self.boundary.coverage = EventFeedCoverage::Unavailable;
        self.boundary
            .failure
            .get_or_insert(crate::event_feed_observer::OBSERVATION_OVERLAP);
        self.counts = self.counts.and_then(|mut counts| {
            counts.rejected = counts.rejected.checked_add(declined)?;
            Some(counts)
        });
    }
    pub fn refuse(&mut self, generation: u64, hour: i32, reason: &'static str) {
        self.events.clear();
        self.script_tokens.clear();
        self.script_scopes.clear();
        self.boundary = OwnerBoundary {
            generation,
            read_hour: hour,
            coverage: EventFeedCoverage::Unavailable,
            failure: Some(reason),
        };
        self.counts = None;
        self.outstanding = 0;
        self.publication = 0;
    }
    pub fn publication(&self) -> u64 {
        self.publication
    }
    pub fn prepare_pack(&self) -> io::Result<Prepared> {
        let failure = self
            .boundary
            .failure
            .map(|reason| allocation::copy_vec(reason.as_bytes()))
            .transpose()?;
        let mut row = EventFeedHistoryRow {
            script_tokens: allocation::copy_vec(&self.script_tokens)?,
            script_scopes: allocation::copy_vec(&self.script_scopes)?,
            generation: self.boundary.generation,
            read_hour: self.boundary.read_hour,
            coverage: self.boundary.coverage,
            counts: self.counts,
            outstanding: self.outstanding,
            failure,
            script_refusals: if self.boundary.coverage == EventFeedCoverage::Unavailable {
                replay_data::event_feed_rows::DECISION_FAMILY_REFUSALS
            } else {
                0
            },
            events: allocation::copy_vec(&self.events)?,
        };
        // Off the hourly boundary: the pack stores each distinct token once.
        row.intern_tokens()?;
        Ok(Prepared {
            row,
            revision: self.publication,
        })
    }
}

#[derive(Debug)]
pub struct Prepared {
    row: EventFeedHistoryRow,
    revision: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct DetachLosses {
    pub unrecorded_tail_events: u64,
    pub unrecorded_tail_revisions: u64,
    pub failed_batches: u64,
}
impl Prepared {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(row: EventFeedHistoryRow) -> Self {
        Self { row, revision: 0 }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.row.events.capacity() * size_of::<EventFeedEvent>()
            + self.row.script_tokens.capacity()
            + self.row.script_scopes.capacity() * size_of::<EventFeedScope>()
            + self.row.failure.as_ref().map_or(0, Vec::capacity)
    }

    pub fn row(&self) -> &EventFeedHistoryRow {
        &self.row
    }
    pub fn publication(&self) -> u64 {
        self.revision
    }
}

pub struct Store {
    delayed: delayed::Index,
    pub(crate) decisions: decisions::Index,
    entries: Vec<Entry>,
    script_tokens: Vec<u8>,
    tokens_used: usize,
    script_scopes: Vec<EventFeedScope>,
    scopes_used: usize,
    scope_nodes: Vec<engine_events::cevent_scope::Node>,
    scope_index: Vec<engine_events::cevent_scope::IndexSlot>,
    pub token_demand: usize,
    pub scope_demand: usize,
    pub graph_demand: usize,
    charge: crate::raw_memory::Charge,
    generation: Option<u64>,
    observing: bool,
    next_sequence: u64,
    revision: u64,
    accepted_revision: u64,
    /// Ascending pending seals, including accounting-only output; grown off-lease.
    publications: Vec<u64>,
    failed_batches: u64,
    counts: EventFeedCounts,
    counts_valid: bool,
    first_interval: bool,
    failure: Option<&'static str>,
    failure_revision: u64,
    retained: Option<&'static str>,
    handoff: Option<(u64, EventFeedCoverage)>,
    registry_baseline: Option<u64>,
    registry_complete: bool,
    pub(crate) registry_state: Option<usize>,
    /// Every entry before it has a publication, so callbacks and the seal
    /// visit only the unpublished tail.
    unpublished_from: usize,
    /// Inside a worker owner baseline: the first rejection's reason.
    local: Option<Option<&'static str>>,
    /// Inside a worker owner baseline: prepared storage ran short. Owner
    /// baselines never grow it under the lease.
    local_short: bool,
    /// The owner baseline sample being read: its writer cut, or `None` when
    /// it began while a native writer was in flight.
    sample: Option<Option<Cut>>,
    /// The world a boundary was prepared in, awaiting its seal.
    prepared: Option<u64>,
    /// Detour witnesses staged this hour ([`Staged`]), a bump arena reset at
    /// the boundary.
    staged: Vec<Option<Staged>>,
    staged_used: usize,
    staged_demand: usize,
    /// Bumped by every arena reset, so a detour open across a boundary
    /// knows its witnesses were dropped.
    staged_epoch: u64,
    /// Only the country worker's privately owned recovery transaction may
    /// grow while reading an owner. It never shares the observer lease.
    preparing_owner: bool,
    entry_demand: usize,
    owner_refusals: [Option<OwnerRefusal>; 2],
    /// Unspent local-refusal controls, replenished from the live census only
    /// after a complete publication budget fits. Never a native-data limit.
    owner_controls: usize,
}

impl Store {
    /// Seed from all retained branches and truncated prefixes, off-thread.
    pub fn new(last_sequence: Option<u64>) -> Result<Self, &'static str> {
        let next_sequence = match last_sequence {
            Some(sequence) => sequence
                .checked_add(1)
                .ok_or("Event feed sequence exhausted")?,
            None => 0,
        };
        Ok(Self {
            delayed: Default::default(),
            decisions: Default::default(),
            entries: Vec::new(),
            script_tokens: Vec::new(),
            tokens_used: 0,
            script_scopes: Vec::new(),
            scopes_used: 0,
            scope_nodes: Vec::new(),
            scope_index: Vec::new(),
            token_demand: 0,
            scope_demand: 0,
            graph_demand: 0,
            charge: Default::default(),
            generation: None,
            observing: false,
            next_sequence,
            revision: 0,
            accepted_revision: 0,
            publications: Vec::new(),
            failed_batches: 0,
            counts: EventFeedCounts::default(),
            counts_valid: true,
            first_interval: true,
            failure: None,
            failure_revision: 0,
            retained: None,
            handoff: None,
            registry_baseline: None,
            registry_complete: false,
            registry_state: None,
            unpublished_from: 0,
            local: None,
            local_short: false,
            sample: None,
            prepared: None,
            staged: Vec::new(),
            staged_used: 0,
            staged_demand: 0,
            staged_epoch: 0,
            preparing_owner: false,
            entry_demand: 0,
            owner_refusals: [None; 2],
            owner_controls: 0,
        })
    }

    /// Grow off-thread from live counts or observed pressure.
    pub fn prepare_capacity(&mut self, additional: usize) -> io::Result<()> {
        allocation::reserve_exact(&mut self.entries, additional)?;
        self.charge_storage();
        Ok(())
    }
    fn charge_storage(&mut self) {
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.entries)
                + crate::raw_memory::vector_bytes(&self.script_tokens)
                + crate::raw_memory::vector_bytes(&self.script_scopes)
                + crate::raw_memory::vector_bytes(&self.scope_nodes)
                + crate::raw_memory::vector_bytes(&self.scope_index),
            index_vectors: self.delayed.bytes() + self.decisions.bytes(),
            auxiliary_vectors: crate::raw_memory::vector_bytes(&self.staged),
            ..Default::default()
        });
    }
    /// Prepare only staging storage off-thread. Sizes are demands, not limits.
    pub fn prepare_scripts(
        &mut self,
        tokens: usize,
        scopes: usize,
        graph: usize,
    ) -> io::Result<()> {
        fn prepare<T: Default + Clone>(buffer: &mut Vec<T>, len: usize) -> io::Result<()> {
            if len > buffer.len() {
                allocation::reserve_exact(buffer, len - buffer.len())?;
                buffer.resize(len, T::default());
            }
            Ok(())
        }
        prepare(&mut self.script_tokens, tokens)?;
        prepare(&mut self.script_scopes, scopes)?;
        prepare(&mut self.scope_nodes, graph)?;
        prepare(
            &mut self.scope_index,
            graph
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(allocation::oom)?,
        )?;
        self.charge_storage();
        Ok(())
    }
    pub fn script_capacity(&self) -> [usize; 3] {
        [
            self.script_tokens.len(),
            self.script_scopes.len(),
            self.scope_nodes.len(),
        ]
    }
    pub fn restore_counts(&mut self, counts: EventFeedCounts) {
        self.counts = counts;
        if counts.acquired != counts.settled || counts.failed > counts.settled {
            self.counts_valid = false;
            self.retained = Some("retained Event feed acquisitions have no settlement receipt");
        }
    }
    pub fn capacity(&self) -> usize {
        self.entries.capacity()
    }
    pub fn capacity_demand(&self) -> usize {
        self.entries
            .len()
            .saturating_add(self.owner_controls)
            .saturating_add(1)
            .max(self.delayed.demand)
            .max(self.delayed.capacity())
            .max(self.decisions.demand)
            .max(self.decisions.capacity())
            .max(self.staged_demand)
            .max(self.entry_demand)
    }

    /// Prepared alongside the decision index: every detour witness is a
    /// decision or activation one.
    pub(crate) fn prepare_staged(&mut self, capacity: usize) -> io::Result<()> {
        let len = self.staged.len();
        if capacity > len {
            allocation::reserve_exact(&mut self.staged, capacity - len)?;
            self.staged.resize(capacity, None);
        }
        Ok(())
    }
    pub(crate) fn witness_capacity(&self) -> usize {
        self.staged.len()
    }
    /// A latch appends its witness. Demand rises even when it does not fit,
    /// so the next hour is prepared larger; never grows on the callback.
    pub(crate) fn stage_witness(
        &mut self,
        token: u64,
        witness: Witness,
    ) -> Result<(), &'static str> {
        self.staged_demand = self.staged_demand.max(self.staged_used.saturating_add(1));
        let slot = self
            .staged
            .get_mut(self.staged_used)
            .ok_or(WITNESS_STORAGE)?;
        *slot = Some(Staged {
            token,
            witness,
            consumed: false,
        });
        self.staged_used += 1;
        Ok(())
    }
    /// The arena's current epoch, recorded by a detour at entry.
    pub(crate) fn staged_epoch(&self) -> u64 {
        self.staged_epoch
    }
    /// The frame's next unconsumed witness in native order, marked consumed.
    /// A frame entered in an earlier `epoch` had its witnesses reset by a
    /// boundary: that is refused by name, never an empty publication.
    pub(crate) fn take_staged(
        &mut self,
        token: u64,
        epoch: Option<u64>,
    ) -> Result<Option<Witness>, &'static str> {
        if epoch.is_some_and(|epoch| epoch != self.staged_epoch) {
            return Err(WITNESS_RESET);
        }
        for slot in &mut self.staged[..self.staged_used] {
            if let Some(staged) = slot
                && staged.token == token
                && !staged.consumed
            {
                staged.consumed = true;
                return Ok(Some(staged.witness));
            }
        }
        Ok(None)
    }
    /// Reset the arena at a boundary. Witnesses no publisher consumed were
    /// abandoned by an unwinding detour: drop them and refuse their hour.
    fn reset_staged(&mut self) {
        let abandoned = self.staged[..self.staged_used]
            .iter()
            .filter(|slot| slot.is_some_and(|staged| !staged.consumed))
            .count() as u64;
        self.staged_used = 0;
        self.staged_epoch = self.staged_epoch.wrapping_add(1);
        if abandoned != 0 {
            self.reject_many(WITNESS_ABANDONED, abandoned);
        }
    }
    #[cfg(test)]
    pub(crate) fn staged_live(&self) -> usize {
        self.staged[..self.staged_used]
            .iter()
            .filter(|slot| slot.is_some_and(|staged| !staged.consumed))
            .count()
    }

    fn advance(&mut self) -> Result<(), &'static str> {
        let Some(revision) = self.revision.checked_add(1) else {
            self.retained.get_or_insert("Event feed revision exhausted");
            return Err("Event feed revision exhausted");
        };
        self.revision = revision;
        Ok(())
    }

    /// Revoke observation before an unobserved mutation. Concurrent native
    /// escape uses the observer's atomic fault flag instead of this store.
    pub fn invalidate(&mut self, reason: &'static str) {
        let _ = self.advance();
        self.observing = false;
        self.registry_baseline = None;
        self.registry_complete = false;
        self.registry_state = None;
        self.delayed.invalidate();
        self.decisions.invalidate();
        self.failure.get_or_insert(reason);
        self.failure_revision = self.revision;
    }

    /// Count an operation whose inputs or storage could not be retained.
    pub fn reject(&mut self, reason: &'static str) -> &'static str {
        self.reject_many(reason, 1)
    }

    /// A declined observation owns no ticket. Retain its exact loss count
    /// until the boundary publishes the refused interval.
    pub(crate) fn reject_many(&mut self, reason: &'static str, count: u64) -> &'static str {
        let _ = self.advance();
        self.count_rejected(count);
        if let Some(local) = self.local.as_mut() {
            // One owner's baseline: its refusal marker names the reason, and
            // no other certificate or interval is affected.
            local.get_or_insert(reason);
            return reason;
        }
        self.revoke_retained_baselines();
        self.failure.get_or_insert(reason);
        self.failure_revision = self.revision;
        reason
    }

    fn count_rejected(&mut self, count: u64) {
        if let Some(rejected) = self.counts.rejected.checked_add(count) {
            self.counts.rejected = rejected;
        } else {
            self.counts_valid = false;
            self.retained
                .get_or_insert("Event feed rejected count exhausted");
        }
    }

    /// A worker owner baseline of `registry`. Its rejections count but stay
    /// with that owner; the refusal carries the first reason, whether a
    /// rejection was already counted, and whether it is retried at the next
    /// visit. Retention and observation loss stay global. Nothing inside
    /// grows prepared storage unless this is the private off-lease transaction.
    ///
    /// Every exit that does not certify checks the sample's writer cut
    /// first: a sample a writer crossed is retried whatever error or
    /// rejection it ended in, since a concurrent writer can make any read
    /// fail or differ. An unbroken failure keeps its native reason.
    pub(crate) fn owner_scope<T>(
        &mut self,
        registry: OwnerRegistry,
        apply: impl FnOnce(&mut Self) -> Result<T, &'static str>,
    ) -> Result<T, OwnerRefusal> {
        let outer = self.local.replace(None);
        let outer_short = std::mem::replace(&mut self.local_short, false);
        self.sample = None;
        let result = apply(self);
        let scoped = self.local.take().flatten();
        let short = std::mem::replace(&mut self.local_short, outer_short);
        let sample = self.sample.take();
        self.local = outer;
        let reason = match (result, scoped) {
            (Ok(value), None) if !short => return Ok(value),
            (Err(reason), _) => reason,
            (Ok(_), Some(reason)) => reason,
            (Ok(_), None) => OWNER_STORAGE,
        };
        let crossed =
            reason == OWNER_CROSSED || sample.is_some_and(|cut| cut.is_none_or(|cut| !cut.holds()));
        if crossed {
            crossed_sample(registry as usize);
        }
        // A prepared index short of room is a storage shortage too.
        let short = !crossed && (short || OWNER_SHORTAGES.contains(&reason));
        Err(OwnerRefusal {
            reason: if crossed {
                OWNER_CROSSED
            } else if short {
                OWNER_STORAGE
            } else {
                reason
            },
            counted: scoped.is_some(),
            retry: true,
        })
    }
    /// Only the private off-lease transaction or a non-owner worker baseline
    /// may grow storage.
    fn grows(&self, growing: bool) -> bool {
        self.preparing_owner || (growing && self.local.is_none())
    }
    /// Prepared storage ran short: an owner baseline is retried.
    fn short(&mut self) {
        if self.local.is_some() {
            self.local_short = true;
        }
    }
    /// A baseline sample certifies only across an unbroken writer cut, with
    /// no decline recorded in this interval: such a decline is a lost
    /// writer.
    pub(super) fn sample_holds(cut: Cut) -> bool {
        cut.holds() && crate::event_feed_observer::untaken_declines() == 0
    }
    /// The certifying marker of an owner baseline follows only a sample
    /// that holds.
    pub(crate) fn certifies(cut: Cut) -> Result<(), &'static str> {
        if Self::sample_holds(cut) {
            Ok(())
        } else {
            Err(OWNER_CROSSED)
        }
    }
    /// An owner baseline's start, before its first native read: no sample
    /// while a native writer is in flight. The cut stays with the store
    /// until the owner scope ends.
    pub(crate) fn sample(&mut self) -> Result<Cut, &'static str> {
        let cut = Cut::take();
        self.sample = Some(cut);
        cut.ok_or(OWNER_CROSSED)
    }
    /// A certified owner's scalar acquisition, before its native read. It
    /// certifies nothing, so a writer in flight refuses nothing, but a read
    /// error under a crossed cut is retried rather than failed.
    pub(crate) fn watch(&mut self) {
        self.sample = Some(Cut::take());
    }

    fn revoke_retained_baselines(&mut self) {
        // A previously complete census must be acquired again after a lost
        // writer interval: the reducer cannot carry it through that gap.
        if self.registry_complete {
            self.registry_baseline = None;
            self.registry_complete = false;
            self.registry_state = None;
        }
        self.delayed.revoke_retained();
        // No native writer was missed: pointer attributions stay exact.
        self.decisions.revoke();
    }

    /// Census begin on the existing worker. Recovery is receipt-qualified:
    /// nothing is read while a loss or refusal awaits its receipt, or before
    /// the boundary's registry baseline established this world.
    pub(crate) fn begin_recovery(&mut self, generation: u64, state: usize) -> Option<u64> {
        if self.failure.is_some() || self.observation(generation).is_err() {
            return None;
        }
        if self.registry_state? != state {
            self.reject("delayed-event census world certificate unavailable");
            return None;
        }
        // Prepared index resets run on this worker, never on a writer.
        self.decisions.reset(generation);
        Some(RECOVERY.load(Ordering::Acquire))
    }

    /// One live country's visit: certified owners read nothing more than
    /// their scalar political balance; uncertified owners retry exact baselines.
    #[cfg(test)]
    pub(crate) fn recover_owner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: engine_native::ccountry::Country,
    ) {
        let work = self.owner_work(memory, base, generation, state, hour, country);
        if !work.into_iter().any(|work| work) {
            return;
        }
        // Publish a completed registry before reading the other one: a
        // later crossed sample cannot withdraw its preceding certificate.
        for registry in [OwnerRegistry::Decisions, OwnerRegistry::Delayed] {
            if !work[registry as usize] {
                continue;
            }
            let mut selected = [false; 2];
            selected[registry as usize] = true;
            match recovery::PreparedOwner::read(
                memory, base, generation, state, hour, country, selected,
            ) {
                Ok(mut plan) => {
                    if let Err(reason) =
                        self.publish_owner(memory, base, generation, hour, &mut plan)
                    {
                        self.reject(reason);
                    }
                }
                Err(_) => {
                    self.refuse_owner_preparation(memory, base, generation, hour, country, selected)
                }
            }
        }
    }

    /// A census that visited every live country without an interval loss
    /// recovered every owner certificate retired before its begin.
    pub(crate) fn finish_recovery(&self, generation: u64, epoch: u64, complete: bool) {
        if complete && self.failure.is_none() && self.observation(generation).is_ok() {
            RECOVERED.fetch_max(epoch, Ordering::AcqRel);
        }
    }

    fn observation(&self, generation: u64) -> Result<(), &'static str> {
        if !self.observing || self.generation != Some(generation) || self.retained.is_some() {
            return Err("Event feed world observation unavailable");
        }
        Ok(())
    }

    /// Retain an event. With a proof it stays pending until its native call
    /// settles; without one it is settled and known at once.
    fn acquire(
        &mut self,
        generation: u64,
        event: impl FnOnce(u64) -> EventFeedEvent,
        proof: Option<Proof>,
    ) -> Result<Ticket, &'static str> {
        if let Err(reason) = self.observation(generation) {
            return Err(self.reject(reason));
        }
        if self.entries.len() >= self.entries.capacity().saturating_sub(self.owner_controls) {
            self.short();
            return Err(self.reject("Event feed callback storage needs growth"));
        }
        let Some(next) = self.next_sequence.checked_add(1) else {
            self.retained.get_or_insert("Event feed sequence exhausted");
            return Err(self.reject("Event feed sequence exhausted"));
        };
        let Some(acquired) = self.counts.acquired.checked_add(1) else {
            self.counts_valid = false;
            self.retained
                .get_or_insert("Event feed acquired count exhausted");
            return Err(self.reject("Event feed acquired count exhausted"));
        };
        let known = proof.is_none();
        let Some(settled) = self.counts.settled.checked_add(u64::from(known)) else {
            self.counts_valid = false;
            self.retained
                .get_or_insert("Event feed settlement count exhausted");
            return Err(self.reject("Event feed settlement count exhausted"));
        };
        self.advance().map_err(|reason| self.reject(reason))?;
        let ticket = Ticket {
            slot: self.entries.len(),
            sequence: self.next_sequence,
        };
        self.entries.push(Entry {
            event: event(ticket.sequence),
            proof,
            state: if known { State::Known } else { State::Pending },
            publication: None,
            dispatch_frame: 0,
            dispatch_scope: 0,
            dispatch_definition: 0,
        });
        self.next_sequence = next;
        self.counts.acquired = acquired;
        self.counts.settled = settled;
        Ok(ticket)
    }

    /// An event a provenance hook proved at its own native boundary
    /// ([`crate::war_provenance`]): settled when acquired.
    pub fn known(&mut self, generation: u64, hour: i32, fact: Fact) -> Result<(), &'static str> {
        self.acquire(
            generation,
            |sequence| EventFeedEvent {
                script: None,
                sequence,
                generation,
                hour,
                source: fact.source,
                actor: country(fact.actor),
                target: country(fact.target),
                cause: Recorded::Value(fact.cause),
            },
            None,
        )
        .map(drop)
    }

    /// Acquire the one script occurrence the native writer reached. Every
    /// payload destination is prepared; no callback allocation or queue walk.
    pub fn script_known(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
    ) -> Result<(), &'static str> {
        self.script_mode(memory, base, generation, hour, fact, false)
    }
    /// Initial baseline acquisition on the existing boundary worker only.
    fn script_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
    ) -> Result<(), &'static str> {
        self.script_mode(memory, base, generation, hour, fact, true)
    }
    fn script_mode(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
        growing: bool,
    ) -> Result<(), &'static str> {
        let token = Token::Native(fact.token);
        self.script_token(memory, base, generation, hour, fact, token, growing)
    }
    /// A capture refusal marker whose token is the static reason.
    #[allow(
        clippy::too_many_arguments,
        reason = "The refusal reason travels beside the usual occurrence operands."
    )]
    fn script_text(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
        reason: &'static str,
        growing: bool,
    ) -> Result<(), &'static str> {
        // A refusal alone may spend one control. Repeated refusals have
        // already returned before here; a failed emission spends nothing.
        let reserved = self.owner_controls;
        self.owner_controls = reserved.saturating_sub(1);
        let result = self.script_token(
            memory,
            base,
            generation,
            hour,
            fact,
            Token::Text(reason),
            growing,
        );
        if result.is_err() {
            self.owner_controls = reserved;
        }
        result
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "The token source is separate from the fact's native descriptor."
    )]
    fn script_token(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
        token: Token,
        growing: bool,
    ) -> Result<(), &'static str> {
        let result = (|| {
            self.observation(generation)?;
            let entry_room = |store: &Self| {
                store.entries.len()
                    < store
                        .entries
                        .capacity()
                        .saturating_sub(store.owner_controls)
            };
            if self.grows(growing) && !entry_room(self) {
                self.prepare_capacity(
                    self.entries
                        .len()
                        .max(self.owner_controls.saturating_add(1)),
                )
                .map_err(|_| "event registry baseline occurrence allocation failed")?;
            }
            if !entry_room(self) {
                self.short();
                return Err("Event feed callback storage needs growth");
            }
            let mut token_end = self
                .tokens_used
                .checked_add(token.len())
                .ok_or("event feed script token demand overflow")?;
            let token_demand =
                token_end.saturating_add(self.owner_controls.saturating_mul(OWNER_CONTROL_TOKEN));
            self.token_demand = self.token_demand.max(token_demand);
            if self.grows(growing) && token_demand > self.script_tokens.len() {
                self.prepare_scripts(
                    token_demand.max(self.script_tokens.len().saturating_mul(2)),
                    self.script_scopes.len(),
                    self.scope_nodes.len(),
                )
                .map_err(|_| "event registry baseline token allocation failed")?;
            }
            if token_demand > self.script_tokens.len() {
                self.short();
            }
            let token_result = if token_demand > self.script_tokens.len() {
                Err("event feed script tokens need prepared storage")
            } else {
                self.script_tokens
                    .get_mut(self.tokens_used..token_end)
                    .ok_or("event feed script tokens need prepared storage")
                    .and_then(|output| token.copy(memory, output))
            };
            let token_unrecorded = if let Err(reason) = token_result {
                // A refusal without its reason would not name what it refuses.
                if let Token::Text(_) = token {
                    return Err(reason);
                }
                self.reject(reason);
                token_end = self.tokens_used;
                true
            } else {
                false
            };
            let mut scope_unrecorded = false;
            let mut graph_len = if let Some(root) = fact.scope {
                let mut graph = engine_events::cevent_scope::copy_graph(
                    memory,
                    base,
                    root,
                    &mut self.scope_nodes,
                    &mut self.scope_index,
                );
                while self.grows(growing)
                    && graph.failure.is_some()
                    && graph.required > self.scope_nodes.len()
                {
                    self.prepare_scripts(
                        self.script_tokens.len(),
                        self.script_scopes.len(),
                        graph.required.max(self.scope_nodes.len().saturating_mul(2)),
                    )
                    .map_err(|_| "event registry baseline graph allocation failed")?;
                    graph = engine_events::cevent_scope::copy_graph(
                        memory,
                        base,
                        root,
                        &mut self.scope_nodes,
                        &mut self.scope_index,
                    );
                }
                self.graph_demand = self.graph_demand.max(graph.required);
                if graph.failure.is_some() && graph.required > self.scope_nodes.len() {
                    self.short();
                }
                if let Some(reason) = graph.failure {
                    self.reject(reason);
                    scope_unrecorded = true;
                    0
                } else {
                    graph.len
                }
            } else {
                0
            };
            let mut scope_end = self
                .scopes_used
                .checked_add(graph_len)
                .ok_or("event feed script scope demand overflow")?;
            self.scope_demand = self.scope_demand.max(scope_end);
            if self.grows(growing) && scope_end > self.script_scopes.len() {
                self.prepare_scripts(
                    self.script_tokens.len(),
                    scope_end.max(self.script_scopes.len().saturating_mul(2)),
                    self.scope_nodes.len(),
                )
                .map_err(|_| "event registry baseline scope allocation failed")?;
            }
            if scope_end > self.script_scopes.len() {
                self.short();
                self.reject("event feed script scopes need prepared storage");
                scope_unrecorded = true;
                graph_len = 0;
                scope_end = self.scopes_used;
            }
            for (output, node) in self.script_scopes[self.scopes_used..scope_end]
                .iter_mut()
                .zip(&self.scope_nodes[..graph_len])
            {
                let value = node.operands;
                *output = EventFeedScope {
                    country: value.country,
                    random: value.random,
                    bindings: value.bindings,
                    state: value.state,
                    references: value.references,
                    strategic_region: value.strategic_region,
                    combat_context_unrecorded: value.combat_context_unrecorded,
                    target_storage_unrecorded: value.target_storage_unrecorded,
                    tooltip_storage_unrecorded: value.tooltip_storage_unrecorded,
                };
            }
            let word =
                |value| u32::try_from(value).map_err(|_| "event feed script arena ID overflow");
            let script = EventFeedScript {
                token_start: word(self.tokens_used)?,
                token_len: word(token_end - self.tokens_used)?,
                token_unrecorded,
                scope_unrecorded,
                scope_start: word(self.scopes_used)?,
                scope_len: word(graph_len)?,
                ..fact.script
            };
            Ok((token_end, scope_end, script))
        })();
        let (token_end, scope_end, script) = result.map_err(|reason| self.reject(reason))?;
        let ticket = self.acquire(
            generation,
            |sequence| EventFeedEvent {
                sequence,
                generation,
                hour,
                source: fact.source,
                actor: country(fact.actor),
                target: country(fact.target),
                cause: Recorded::Value(fact.cause),
                script: Some(script),
            },
            None,
        )?;
        self.entries[ticket.slot].dispatch_frame = fact.dispatch_frame;
        self.entries[ticket.slot].dispatch_scope = fact.scope.unwrap_or(0);
        self.entries[ticket.slot].dispatch_definition = fact.dispatch_definition;
        self.tokens_used = token_end;
        self.scopes_used = scope_end;
        Ok(())
    }

    /// The native pending lookup/registration has supplied this dispatch's
    /// actual instance. Nested dispatches have distinct native frames.
    pub fn script_instance(&mut self, generation: u64, frame: usize, instance: u32) {
        if self.observation(generation).is_err() {
            return;
        }
        // Published receipts can retain hours of entries; none is a dispatch.
        if let Some(entry) = self.entries[self.unpublished_from..]
            .iter_mut()
            .rev()
            .find(|entry| {
                entry.dispatch_frame == frame
                    && entry.event.generation == generation
                    && entry.publication.is_none()
            })
            && let Some(script) = entry.event.script.as_mut()
        {
            script.instance = Some(instance);
            entry.dispatch_frame = 0;
        }
    }

    pub fn registered_instance(
        &mut self,
        generation: u64,
        scope: usize,
        definition: usize,
        instance: u32,
    ) {
        if self.observation(generation).is_err() {
            return;
        }
        if let Some(entry) = self.entries[self.unpublished_from..]
            .iter_mut()
            .rev()
            .find(|entry| {
                entry.dispatch_frame != 0
                    && entry.dispatch_scope == scope
                    && entry.event.generation == generation
                    && entry.publication.is_none()
                    && entry.dispatch_definition == definition
            })
            && let Some(script) = entry.event.script.as_mut()
        {
            script.instance = Some(instance);
            entry.dispatch_frame = 0;
        }
    }

    /// War call observation retains inputs before the original and its reentrant
    /// destruction before the relation replacement store.
    pub fn war_before(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        frame: native::WarFrame,
        cause: EventFeedCause,
    ) -> Result<Ticket, &'static str> {
        self.observation(generation)
            .map_err(|reason| self.reject(reason))?;
        let pending =
            native::war_before(memory, base, frame).map_err(|reason| self.reject(reason))?;
        let operands = pending.operands();
        self.acquire(
            generation,
            |sequence| event(sequence, generation, operands, cause),
            Some(Proof::War(pending)),
        )
    }

    /// Copy actor/FROM before authored dispatch.
    pub fn annex_before(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        frame: native::AnnexFrame,
        cause: EventFeedCause,
    ) -> Result<Ticket, &'static str> {
        self.observation(generation)
            .map_err(|reason| self.reject(reason))?;
        let operands =
            native::annex_before(memory, base, frame).map_err(|reason| self.reject(reason))?;
        self.acquire(
            generation,
            |sequence| event(sequence, generation, operands, cause),
            Some(Proof::Annex),
        )
    }

    fn entry(&self, ticket: &Ticket) -> Option<&Entry> {
        self.entries
            .get(ticket.slot)
            .filter(|entry| entry.event.sequence == ticket.sequence)
    }

    /// Retain the entry, proof, ticket and native flight after uncertainty.
    pub fn uncertain(&mut self, ticket: &Ticket, reason: &'static str) {
        let _ = self.advance();
        self.failure.get_or_insert(reason);
        self.failure_revision = self.revision;
        self.retained.get_or_insert(reason);
        if self
            .entry(ticket)
            .is_none_or(|entry| !matches!(entry.state, State::Pending | State::PublishedPending))
        {
            self.retained = Some("Event feed uncertain ticket ownership differs");
        }
    }

    /// # Safety
    /// This call returned normally in the same world, with fresh guarded reads.
    pub unsafe fn normal_returned(
        &mut self,
        ticket: Ticket,
        generation: u64,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), Retained> {
        let reason = if !self.observing || self.generation != Some(generation) {
            Some("Event feed world invalidated before settlement")
        } else if self.entry(&ticket).is_none_or(|entry| {
            !matches!(entry.state, State::Pending | State::PublishedPending)
                || entry.proof.is_none()
                || entry.event.generation != generation
        }) {
            Some("Event feed pending ticket ownership differs")
        } else if self.retained.is_some() {
            Some("Event feed native completion remains uncertain")
        } else if !self.counts_valid
            || self.counts.settled == u64::MAX
            || self.counts.failed == u64::MAX
        {
            Some("Event feed settlement count exhausted")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.uncertain(&ticket, reason);
            return Err(Retained { ticket, reason });
        }
        if let Err(reason) = self.advance() {
            self.uncertain(&ticket, reason);
            return Err(Retained { ticket, reason });
        }
        // A failed post read cannot retry its moved proof.
        let entry = &mut self.entries[ticket.slot];
        let published = entry.state == State::PublishedPending;
        let proof = entry.proof.take().unwrap();
        // Only the operands are proved again; the cause is the entry's own.
        let cause = entry.event.cause;
        let result = match proof {
            Proof::War(pending) => {
                native::war_returned(memory, base, pending).map(|operands| EventFeedEvent {
                    script: None,
                    cause,
                    ..event(ticket.sequence, generation, operands, Default::default())
                })
            }
            Proof::Annex => Ok(entry.event),
        };
        self.counts.settled += 1;
        match result {
            Ok(proved) if proved == entry.event => {
                entry.state = if published {
                    State::Published
                } else {
                    State::Known
                };
            }
            Ok(_) => {
                entry.state = State::Failed;
                self.counts.failed += 1;
                self.failure
                    .get_or_insert("Event feed postproof operands differ");
                self.failure_revision = self.revision;
            }
            Err(reason) => {
                entry.state = State::Failed;
                self.counts.failed += 1;
                self.failure.get_or_insert(reason);
                self.failure_revision = self.revision;
            }
        }
        Ok(())
    }

    pub fn outstanding(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.state, State::Pending | State::PublishedPending))
            .count() as u64
    }

    /// Original returned normally after invalidation. Settle without touching
    /// the old native proof (relation replacement can reenter destruction).
    pub fn world_returned(&mut self, ticket: Ticket) -> Result<(), Retained> {
        if self
            .entry(&ticket)
            .is_none_or(|entry| !matches!(entry.state, State::Pending | State::PublishedPending))
            || self.retained.is_some()
            || !self.counts_valid
            || self.counts.settled == u64::MAX
            || self.counts.failed == u64::MAX
        {
            let reason = "Event feed invalidated completion ownership differs";
            self.uncertain(&ticket, reason);
            return Err(Retained { ticket, reason });
        }
        self.invalidate("Event feed native world changed during original");
        self.counts.settled += 1;
        self.counts.failed += 1;
        let entry = &mut self.entries[ticket.slot];
        entry.proof = None;
        entry.state = State::Failed;
        Ok(())
    }

    /// Swap prepared storage at a ticket-free hourly boundary. The caller
    /// returns the old allocation to the spool; nothing is freed here.
    pub fn install_capacity(&mut self, spare: &mut Self) {
        if self.outstanding() == 0
            && spare.entries.capacity() >= self.entries.capacity()
            && spare.script_tokens.len() >= self.script_tokens.len()
            && spare.script_scopes.len() >= self.script_scopes.len()
            && spare.scope_nodes.len() >= self.scope_nodes.len()
            && spare.delayed.covers(&self.delayed)
            && spare.decisions.covers(&self.decisions)
            && spare.staged.len() >= self.staged.len()
        {
            spare.entries.clear();
            spare.entries.append(&mut self.entries);
            std::mem::swap(&mut self.entries, &mut spare.entries);
            std::mem::swap(&mut self.charge, &mut spare.charge);
            spare.script_tokens[..self.tokens_used]
                .copy_from_slice(&self.script_tokens[..self.tokens_used]);
            spare.script_scopes[..self.scopes_used]
                .copy_from_slice(&self.script_scopes[..self.scopes_used]);
            std::mem::swap(&mut self.script_tokens, &mut spare.script_tokens);
            std::mem::swap(&mut self.script_scopes, &mut spare.script_scopes);
            std::mem::swap(&mut self.scope_nodes, &mut spare.scope_nodes);
            std::mem::swap(&mut self.scope_index, &mut spare.scope_index);
            self.delayed.install(&mut spare.delayed);
            self.decisions.install(&mut spare.decisions);
            spare.staged[..self.staged_used].copy_from_slice(&self.staged[..self.staged_used]);
            std::mem::swap(&mut self.staged, &mut spare.staged);
        }
    }

    /// Only the hourly consumer touches live history. Output acknowledges an
    /// immutable publication number, so newer callbacks cannot stale its receipt.
    pub fn acknowledged(&mut self, publication: u64) {
        self.receipts(&[(publication, Some(true))]);
    }
    pub fn discarded(&mut self, publication: u64) {
        self.receipts(&[(publication, Some(false))]);
    }
    /// Strictly ascending receipts; one allocation-free entry pass per batch.
    pub fn receipts(&mut self, receipts: &[Receipt]) {
        let pending = &self.publications;
        let received = |publication: u64| {
            pending.binary_search(&publication).ok()?;
            let index = receipts
                .binary_search_by_key(&publication, |receipt| receipt.0)
                .ok()?;
            receipts[index].1
        };
        if !receipts
            .iter()
            .any(|&(publication, _)| received(publication).is_some())
        {
            return;
        }
        // Different publications touch different entries.
        for entry in &mut self.entries {
            match entry.publication.and_then(received) {
                Some(true) => {
                    entry.state = match entry.state {
                        State::Known | State::Failed => State::Published,
                        State::Pending if matches!(entry.proof, Some(Proof::Annex)) => {
                            State::PublishedPending
                        }
                        state => state,
                    };
                }
                Some(false)
                    if !matches!(entry.state, State::Published | State::PublishedPending) =>
                {
                    entry.publication = None;
                }
                _ => {}
            }
        }
        let mut previous = None;
        for &(publication, outcome) in receipts {
            let Some(accepted) = outcome else {
                continue;
            };
            if previous == Some(publication)
                || self.publications.binary_search(&publication).is_err()
            {
                continue;
            }
            previous = Some(publication);
            if accepted {
                self.accept_publication(publication);
            } else {
                self.discard_publication(publication);
            }
        }
        self.publications.retain(|&publication| {
            receipts
                .binary_search_by_key(&publication, |receipt| receipt.0)
                .map_or(true, |index| receipts[index].1.is_none())
        });
    }
    fn accept_publication(&mut self, publication: u64) {
        self.accepted_revision = self.accepted_revision.max(publication);
        if self
            .handoff
            .is_some_and(|(handoff, _)| handoff == publication)
        {
            self.first_interval |= self.handoff.unwrap().1 == EventFeedCoverage::Unavailable;
            if self.retained.is_none() && self.failure_revision <= publication {
                self.failure = None;
            }
            self.handoff = None;
        }
    }
    fn discard_publication(&mut self, publication: u64) {
        self.failed_batches += 1;
        // The next seal publishes these entries again.
        self.unpublished_from = 0;
        if self
            .handoff
            .is_some_and(|(handoff, _)| handoff == publication)
        {
            self.handoff = None;
        }
        self.invalidate("Event feed hourly snapshot discarded before output");
    }
    /// The receipt room the next seal needs, when it has none.
    pub fn receipt_room(&self) -> Option<usize> {
        let len = self.publications.len();
        (len == self.publications.capacity()).then(|| len.saturating_mul(2).max(len + 1))
    }
    /// Adopt off-lease room; the caller frees the replaced storage off-lease.
    pub fn install_receipts(&mut self, spare: &mut Vec<u64>) {
        if self.publications.len() == self.publications.capacity()
            && spare.capacity() > self.publications.len()
        {
            spare.clear();
            spare.extend_from_slice(&self.publications);
            std::mem::swap(&mut self.publications, spare);
        }
    }
    #[cfg(test)]
    fn reserve_receipt(&mut self) -> Result<(), &'static str> {
        allocation::reserve(&mut self.publications, 1).map_err(|_| RECEIPT_STORAGE)
    }
    pub fn unacknowledged(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| {
                entry.publication.is_some()
                    && !matches!(entry.state, State::Published | State::PublishedPending)
            })
            .count()
    }
    #[cfg(test)]
    fn unaccepted_output(&self) -> bool {
        self.revision != self.accepted_revision
            || self.unacknowledged() != 0
            || self.entries.iter().any(|entry| {
                entry.publication.is_none() && (entry.fact() || entry.state == State::Failed)
            })
    }
    /// Decode/spool have joined and receipts have drained. Never acknowledge a
    /// tail or invent its supplying hour; only unresolved ownership pins detach.
    pub fn detach_losses(&self) -> Result<DetachLosses, &'static str> {
        if self.outstanding() != 0 || self.retained.is_some() {
            return Err("Event feed owner has unsettled native ownership");
        }
        if !self.publications.is_empty() || self.unacknowledged() != 0 {
            return Err("Event feed output receipt remains uncertain");
        }
        Ok(DetachLosses {
            unrecorded_tail_events: self.entries.iter().filter(|entry| entry.fact()).count() as u64,
            unrecorded_tail_revisions: self.revision - self.accepted_revision,
            failed_batches: self.failed_batches,
        })
    }
    #[cfg(test)]
    pub fn snapshot(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
    ) -> Result<(), &'static str> {
        self.prepare_boundary::<engine_native::memory::LiveMemory>(boundary, None)?;
        self.reserve_receipt()?;
        self.seal(output, boundary, false)
    }
    #[cfg(test)]
    pub fn snapshot_growing(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
    ) -> Result<(), &'static str> {
        self.prepare_boundary::<engine_native::memory::LiveMemory>(boundary, None)?;
        self.seal(output, boundary, true)
    }
    #[cfg(test)]
    pub fn snapshot_with_registry<M: Memory>(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
        memory: &M,
        base: usize,
    ) -> Result<(), &'static str> {
        self.prepare_boundary(boundary, Some((memory, base)))?;
        self.seal(output, boundary, true)
    }
    /// The boundary worker's first half: retire acknowledged history, admit
    /// the observed world and acquire the registry baseline. Owner census
    /// visits may follow before [`Self::seal`] publishes the same hour.
    pub fn prepare_boundary<M: Memory>(
        &mut self,
        boundary: OwnerBoundary,
        registry: Option<(&M, usize)>,
    ) -> Result<(), &'static str> {
        self.prepared = None;
        // Every detour completed on its writer's thread before this
        // boundary; a witness still here was abandoned on an unwind.
        self.reset_staged();
        // No outstanding ticket can still refer to an index that retain moves.
        if self.outstanding() == 0 && self.retained.is_none() {
            self.entries.retain(|entry| entry.state != State::Published);
            // Unacknowledged and newly reached entries survive together.
            // Compact their local spans even during a continuous event stream;
            // waiting for an empty hour would grow arenas for the whole game.
            let mut tokens = 0;
            let mut scopes = 0;
            for entry in &mut self.entries {
                if let Some(script) = entry.event.script.as_mut() {
                    let source = script.token_start as usize;
                    let len = script.token_len as usize;
                    self.script_tokens.copy_within(source..source + len, tokens);
                    script.token_start = tokens as u32;
                    tokens += len;
                    let source = script.scope_start as usize;
                    let len = script.scope_len as usize;
                    self.script_scopes.copy_within(source..source + len, scopes);
                    script.scope_start = scopes as u32;
                    scopes += len;
                }
            }
            self.tokens_used = tokens;
            self.scopes_used = scopes;
            self.unpublished_from = self
                .entries
                .iter()
                .position(|entry| entry.publication.is_none())
                .unwrap_or(self.entries.len());
        }
        if self.generation != Some(boundary.generation) {
            if self.outstanding() != 0 || self.generation.is_some() && self.retained.is_some() {
                return Err("Event feed prior world has unsettled native ownership");
            }
            if self.unacknowledged() != 0 {
                return Err("Event feed prior world awaits output acceptance");
            }
            let discarded = self
                .entries
                .iter()
                .filter(|entry| entry.publication.is_none() && entry.fact())
                .count();
            self.counts.rejected = self
                .counts
                .rejected
                .checked_add(discarded as u64)
                .ok_or("Event feed rejected count exhausted")?;
            self.entries.clear();
            self.unpublished_from = 0;
            self.tokens_used = 0;
            self.scopes_used = 0;
            self.owner_controls = 0;
            self.generation = Some(boundary.generation);
            self.registry_state = None;
            self.first_interval = true;
            if discarded != 0 {
                self.failure
                    .get_or_insert("Event feed unsampled prior-world events discarded");
            }
            self.failure_revision = self.revision;
            self.handoff = None;
        }
        self.observing = self.retained.is_none();
        self.delayed.reset(boundary.generation);
        // A bootstrap interval may have refused pre-observation calls.
        // Start the single retained census only after that refusal has
        // an output receipt; never acquire a baseline we must invalidate
        // in the same publication. Missing writer coverage also waits;
        // a healthy first interval can still publish a fresh baseline.
        // This registry baseline still grows storage under the worker
        // lease: it predates owner recovery and is small. Owner baselines
        // never grow.
        if let Some((memory, base)) = registry
            && self.failure.is_none()
            && boundary.coverage == EventFeedCoverage::Complete
            && let Err(reason) =
                self.registry_baseline(memory, base, boundary.generation, boundary.read_hour)
        {
            self.reject(reason);
        }
        self.prepared = Some(boundary.generation);
        Ok(())
    }
    /// What sealing now would publish: unpublished facts, their tokens and
    /// scopes. Read-only, so the output can grow off the shared-store lease.
    pub fn seal_demand(&self) -> Result<SealDemand, &'static str> {
        self.entries[self.unpublished_from..]
            .iter()
            .filter(|entry| entry.publication.is_none() && entry.fact())
            .try_fold(SealDemand::default(), |demand, entry| {
                let script = entry.event.script;
                Ok(SealDemand {
                    events: demand.events + 1,
                    tokens: demand
                        .tokens
                        .checked_add(script.map_or(0, |script| script.token_len as usize))
                        .ok_or("event feed token snapshot demand overflow")?,
                    scopes: demand
                        .scopes
                        .checked_add(script.map_or(0, |script| script.scope_len as usize))
                        .ok_or("event feed scope snapshot demand overflow")?,
                })
            })
    }
    /// The boundary worker's second half: publish everything reached since
    /// the previous seal, including the prepared hour's census markers. In
    /// production output and receipt room grow off-lease; shortfalls refuse the hour.
    pub fn seal(
        &mut self,
        output: &mut Snapshot,
        mut boundary: OwnerBoundary,
        growing: bool,
    ) -> Result<(), &'static str> {
        if self.prepared.take() != Some(boundary.generation)
            || self.generation != Some(boundary.generation)
        {
            return Err("Event feed boundary seal differs from its prepared world");
        }
        let outstanding = self.outstanding();
        boundary.failure = self
            .retained
            .or(self.failure)
            .or(boundary.failure)
            .or_else(|| (outstanding != 0).then_some("Event feed acquired events unsettled"))
            .or_else(|| (!self.counts_valid).then_some("Event feed accounting exhausted"));
        // Every entry before this one already has a publication.
        let unpublished = self.unpublished_from;
        let demand = self.seal_demand()?;
        output.required = demand.events;
        if growing {
            output.reserve(demand)?;
        }
        if output.required > output.events.capacity() {
            return Err("Event feed hourly storage needs growth");
        }
        if demand.tokens > output.script_tokens.capacity()
            || demand.scopes > output.script_scopes.capacity()
        {
            return Err("Event feed script hourly storage needs growth");
        }
        if self.entries[unpublished..].iter().any(|entry| {
            entry.publication.is_none() && entry.fact() && entry.event.hour > boundary.read_hour
        }) {
            return Err("Event feed event is later than its supplying hour");
        }
        boundary.coverage = if boundary.failure.is_some() {
            EventFeedCoverage::Unavailable
        } else if self.first_interval || boundary.coverage == EventFeedCoverage::Partial {
            EventFeedCoverage::Partial
        } else {
            boundary.coverage
        };
        // Partial coverage preserves only a baseline carried by this exact
        // publication. The first interval can supply one; an older retained
        // certificate cannot survive a later forwarder/input gap.
        let fresh_registry = |change| {
            self.entries[unpublished..].iter().any(|entry| {
                entry.publication.is_none()
                    && entry.fact()
                    && entry
                        .event
                        .script
                        .is_some_and(|script| script.change == Some(change))
            })
        };
        let partial_without_baseline = boundary.coverage == EventFeedCoverage::Partial
            && self.registry_complete
            && !(fresh_registry(EventFeedScriptChange::PendingReady)
                && fresh_registry(EventFeedScriptChange::FiredReady));
        let mut boundary_refusal = false;
        if boundary.coverage == EventFeedCoverage::Unavailable || partial_without_baseline {
            self.revoke_retained_baselines();
            if self.failure.is_none() {
                self.failure = boundary.failure.or(partial_without_baseline
                    .then_some("Event feed interval writer coverage incomplete"));
                boundary_refusal = self.failure.is_some();
            }
        }
        if growing {
            allocation::reserve(&mut self.publications, 1).map_err(|_| RECEIPT_STORAGE)?;
        }
        if self.publications.len() == self.publications.capacity() {
            return Err(RECEIPT_STORAGE);
        }
        self.advance()?;
        if boundary_refusal {
            // An older in-flight publication cannot acknowledge this loss.
            self.failure_revision = self.revision;
        }
        self.publications.push(self.revision);
        output.events.clear();
        output.script_tokens.clear();
        output.script_scopes.clear();
        for entry in &mut self.entries[unpublished..] {
            if entry.publication.is_none() && entry.fact() {
                let mut event = entry.event;
                if let Some(script) = event.script.as_mut() {
                    let token_start = script.token_start as usize;
                    let scope_start = script.scope_start as usize;
                    script.token_start = u32::try_from(output.script_tokens.len())
                        .map_err(|_| "event feed token snapshot ID overflow")?;
                    script.scope_start = u32::try_from(output.script_scopes.len())
                        .map_err(|_| "event feed scope snapshot ID overflow")?;
                    output.script_tokens.extend_from_slice(
                        &self.script_tokens[token_start..token_start + script.token_len as usize],
                    );
                    output.script_scopes.extend_from_slice(
                        &self.script_scopes[scope_start..scope_start + script.scope_len as usize],
                    );
                }
                output.events.push(event);
                entry.publication = Some(self.revision);
            } else if entry.state == State::Failed && entry.publication.is_none() {
                entry.publication = Some(self.revision);
            }
        }
        // Skip what this seal published; a war proof awaiting its settlement
        // keeps itself and every later entry in the unpublished tail.
        self.unpublished_from += self.entries[unpublished..]
            .iter()
            .position(|entry| entry.publication.is_none())
            .unwrap_or(self.entries.len() - unpublished);
        output.boundary = boundary;
        output.counts = self.counts_valid.then_some(self.counts);
        output.outstanding = outstanding;
        output.publication = self.revision;
        self.track_handoff(self.revision, boundary.coverage);
        self.first_interval = boundary.coverage == EventFeedCoverage::Unavailable;
        Ok(())
    }

    /// Keep the first publication covering the current failure, not an
    /// older healthy receipt or a refusal from an earlier loss epoch.
    fn track_handoff(&mut self, publication: u64, coverage: EventFeedCoverage) {
        if publication >= self.failure_revision
            && self.handoff.is_none_or(|(pending, previous)| {
                pending < self.failure_revision
                    || pending == publication
                        && coverage == EventFeedCoverage::Unavailable
                        && previous != EventFeedCoverage::Unavailable
            })
        {
            self.handoff = Some((publication, coverage));
        }
    }

    /// Callbacks the store declined while the seal still held it happened
    /// before the next interval began. They refuse the sealed publication,
    /// whose receipt ends the loss, and retire every certificate a missed
    /// writer may have broken. Observation continues: nothing after the
    /// seal's release was missed.
    pub(crate) fn seal_declined(&mut self, sealed: u64, declined: u64) {
        let _ = self.advance();
        self.registry_baseline = None;
        self.registry_complete = false;
        self.registry_state = None;
        self.delayed.invalidate();
        self.decisions.invalidate();
        self.count_rejected(declined);
        self.failure
            .get_or_insert(crate::event_feed_observer::OBSERVATION_OVERLAP);
        self.failure_revision = self.failure_revision.max(sealed);
        self.track_handoff(sealed, EventFeedCoverage::Unavailable);
    }
}

#[cfg(test)]
impl Store {
    pub(crate) fn failure_for_test(&self) -> Option<&'static str> {
        self.failure
    }
    pub(crate) fn rejected_for_test(&self) -> u64 {
        self.counts.rejected
    }
}

/// A seal's output demand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SealDemand {
    pub events: usize,
    pub tokens: usize,
    pub scopes: usize,
}

impl Entry {
    fn fact(&self) -> bool {
        self.state == State::Known
            || self.state == State::Pending && matches!(self.proof.as_ref(), Some(Proof::Annex))
    }
}

fn country(identity: native::CountryIdentity) -> EventFeedCountry {
    EventFeedCountry {
        reference: identity.reference,
        id: identity.id,
        instance: identity.instance,
    }
}

fn event(
    sequence: u64,
    generation: u64,
    operands: native::Operands,
    cause: EventFeedCause,
) -> EventFeedEvent {
    EventFeedEvent {
        script: None,
        sequence,
        generation,
        hour: operands.hour,
        source: match operands.source {
            native::Source::Declaration => EventFeedSource::Declaration,
            native::Source::Join => EventFeedSource::Join,
            native::Source::CivilWarWarTransfer => EventFeedSource::CivilWarWarTransfer,
            native::Source::Annex => EventFeedSource::Annex,
        },
        actor: country(operands.actor),
        target: country(operands.target),
        cause: Recorded::Value(cause),
    }
}

/// An event a provenance hook proved: its kind, countries and cause.
pub struct Fact {
    pub source: EventFeedSource,
    pub actor: native::CountryIdentity,
    pub target: native::CountryIdentity,
    pub cause: EventFeedCause,
}

/// Acquisition-local operands; descriptors and pointers never leave this call.
pub struct ScriptFact {
    pub source: EventFeedSource,
    pub actor: native::CountryIdentity,
    pub target: native::CountryIdentity,
    pub cause: EventFeedCause,
    pub token: engine_native::msvc_string::Descriptor,
    pub scope: Option<usize>,
    pub dispatch_frame: usize,
    pub dispatch_definition: usize,
    pub script: EventFeedScript,
}

pub(crate) mod decisions;
pub(crate) mod delayed;
pub(crate) mod recovery;
pub(crate) mod registry;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod write_behind_tests;
