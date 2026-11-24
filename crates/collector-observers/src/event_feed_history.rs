
pub(crate) use crate::script_event_observer::staging::Witness;
use engine_allocation as allocation;
use engine_events::event_feed_sites as native;
use engine_native_memory::Memory;
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

pub(crate) const WITNESS_ALLOCATION: &str =
    crate::script_event_observer::staging::WITNESS_ALLOCATION;
pub(crate) const WITNESS_ABANDONED: &str = crate::script_event_observer::staging::WITNESS_ABANDONED;
pub(crate) const WITNESS_RESET: &str = crate::script_event_observer::staging::WITNESS_RESET;

#[derive(Clone, Copy)]
struct Staged {
    token: u64,
    witness: Witness,
    consumed: bool,
}

pub const RECEIPT_STORAGE: &str = "Event feed publication receipts need prepared storage";

pub type Receipt = (u64, Option<bool>);

pub(crate) static RECOVERY: AtomicU64 = AtomicU64::new(1);
pub(crate) static RECOVERED: AtomicU64 = AtomicU64::new(0);
pub(crate) fn recovery_needed() {
    RECOVERY.fetch_add(1, Ordering::AcqRel);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerRegistry {
    Delayed,
    Decisions,
}
pub(crate) static REPEATED_OWNER_REFUSALS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
pub(crate) fn repeated_owner_refusal(registry: OwnerRegistry) {
    REPEATED_OWNER_REFUSALS[registry as usize].fetch_add(1, Ordering::Relaxed);
}

pub(crate) static CROSSED_SAMPLES: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
pub(crate) const REGISTRY_SAMPLE: usize = 2;
fn crossed_sample(sample: usize) {
    CROSSED_SAMPLES[sample].fetch_add(1, Ordering::Relaxed);
}
pub(crate) const OWNER_CROSSED: &str = "owner baseline crossed a native writer";
pub(crate) const OWNER_STORAGE: &str = "owner baseline needs prepared storage";

const OWNER_CONTROL_TOKEN: usize = if OWNER_STORAGE.len() > OWNER_CROSSED.len() {
    OWNER_STORAGE.len()
} else {
    OWNER_CROSSED.len()
};

const OWNER_SHORTAGES: [&str; 11] = [
    "decision owner certificates need prepared capacity",
    "decision pointer certificates need prepared capacity",
    "decision pointer certificates are full",
    "delayed-event pointer index needs prepared storage",
    OCCURRENCE_ALLOCATION,
    "event feed script token allocation failed",
    "event feed scope graph allocation failed",
    "event feed script scope allocation failed",
    "decision certificate allocation failed",
    "delayed-event identity allocation failed",
    "delayed-event census table allocation failed",
];
const OCCURRENCE_ALLOCATION: &str = "event feed occurrence allocation failed";

#[derive(Clone, Copy)]
pub(crate) struct Cut(u64);
impl Cut {
    pub(crate) fn take() -> Option<Self> {
        crate::event_feed_observer::writer_cut().map(Self)
    }
    pub(crate) fn holds(self) -> bool {
        crate::event_feed_observer::writer_cut_holds(self.0)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OwnerRefusal {
    pub reason: &'static str,
    pub counted: bool,
    pub retry: bool,
}

#[derive(Clone, Copy)]
enum Token {
    Native(engine_native_memory::msvc_string::Descriptor),
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
    pub fn reserve_with_headroom(&mut self, demand: SealDemand) -> Result<(), &'static str> {
        let headroom = |value: usize, floor: usize| value.saturating_add((value / 4).max(floor));
        self.reserve(SealDemand {
            events: headroom(demand.events, 64),
            tokens: headroom(demand.tokens, 1024),
            scopes: headroom(demand.scopes, 64),
        })
    }
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
    unpublished_from: usize,
    local: Option<Option<&'static str>>,
    sample: Option<Option<Cut>>,
    prepared: Option<u64>,
    staged: Vec<Option<Staged>>,
    staged_used: usize,
    staged_demand: usize,
    staged_epoch: u64,
    preparing_owner: bool,
    entry_demand: usize,
    owner_refusals: [Option<OwnerRefusal>; 2],
    owner_controls: usize,
}

impl Store {
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
    pub fn entry_capacity_demand(&self) -> usize {
        self.entries
            .len()
            .saturating_add(self.owner_controls)
            .saturating_add(1)
            .max(self.delayed.demand)
            .max(self.delayed.capacity())
            .max(self.entry_demand)
    }
    pub(crate) fn staged_demand(&self) -> usize {
        self.staged_demand
    }
    pub fn decision_capacity_demand(&self) -> usize {
        self.decisions.demand.max(self.decisions.capacity())
    }
    pub fn decision_owner_demand(&self) -> usize {
        self.decisions
            .owner_demand
            .max(self.decisions.owner_capacity())
    }
    pub fn decision_capacity(&self) -> usize {
        self.decisions.capacity()
    }
    pub fn decision_owner_capacity(&self) -> usize {
        self.decisions.owner_capacity()
    }
    pub fn prepare_decisions_sized(&mut self, slots: usize, owners: usize) -> io::Result<()> {
        self.decisions.prepare_sized(slots, owners)?;
        self.charge_storage();
        Ok(())
    }
    pub fn capacity_demand(&self) -> usize {
        self.entry_capacity_demand()
            .max(self.decisions.demand)
            .max(self.decisions.capacity())
            .max(self.staged_demand)
    }

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
    pub(crate) fn stage_witness(
        &mut self,
        token: u64,
        witness: Witness,
    ) -> Result<(), &'static str> {
        let needed = self.staged_used.saturating_add(1);
        self.staged_demand = self.staged_demand.max(needed);
        if needed > self.staged.len() {
            self.prepare_staged(needed.max(self.staged.len().saturating_mul(2)))
                .map_err(|_| WITNESS_ALLOCATION)?;
        }
        let slot = &mut self.staged[self.staged_used];
        *slot = Some(Staged {
            token,
            witness,
            consumed: false,
        });
        self.staged_used += 1;
        Ok(())
    }
    pub(crate) fn staged_epoch(&self) -> u64 {
        self.staged_epoch
    }
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

    pub fn reject(&mut self, reason: &'static str) -> &'static str {
        self.reject_many(reason, 1)
    }

    pub(crate) fn reject_many(&mut self, reason: &'static str, count: u64) -> &'static str {
        let _ = self.advance();
        self.count_rejected(count);
        if let Some(local) = self.local.as_mut() {
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

    pub(crate) fn owner_scope<T>(
        &mut self,
        registry: OwnerRegistry,
        apply: impl FnOnce(&mut Self) -> Result<T, &'static str>,
    ) -> Result<T, OwnerRefusal> {
        let outer = self.local.replace(None);
        self.sample = None;
        let result = apply(self);
        let scoped = self.local.take().flatten();
        let sample = self.sample.take();
        self.local = outer;
        let reason = match (result, scoped) {
            (Ok(value), None) => return Ok(value),
            (Err(reason), _) => reason,
            (Ok(_), Some(reason)) => reason,
        };
        let crossed =
            reason == OWNER_CROSSED || sample.is_some_and(|cut| cut.is_none_or(|cut| !cut.holds()));
        if crossed {
            crossed_sample(registry as usize);
        }
        let short = !crossed && OWNER_SHORTAGES.contains(&reason);
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
    pub(super) fn sample_holds(cut: Cut) -> bool {
        cut.holds() && crate::event_feed_observer::untaken_declines() == 0
    }
    pub(crate) fn certifies(cut: Cut) -> Result<(), &'static str> {
        if Self::sample_holds(cut) {
            Ok(())
        } else {
            Err(OWNER_CROSSED)
        }
    }
    pub(crate) fn sample(&mut self) -> Result<Cut, &'static str> {
        let cut = Cut::take();
        self.sample = Some(cut);
        cut.ok_or(OWNER_CROSSED)
    }
    pub(crate) fn watch(&mut self) {
        self.sample = Some(Cut::take());
    }

    fn revoke_retained_baselines(&mut self) {
        if self.registry_complete {
            self.registry_baseline = None;
            self.registry_complete = false;
            self.registry_state = None;
        }
        self.delayed.revoke_retained();
        self.decisions.revoke();
    }

    pub(crate) fn begin_recovery(&mut self, generation: u64, state: usize) -> Option<u64> {
        if self.failure.is_some() || self.observation(generation).is_err() {
            return None;
        }
        if self.registry_state? != state {
            self.reject("delayed-event census world certificate unavailable");
            return None;
        }
        self.decisions.reset(generation);
        Some(RECOVERY.load(Ordering::Acquire))
    }

    #[cfg(test)]
    pub(crate) fn recover_owner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: engine_native_memory::country_identity::Country,
    ) {
        let work = self.owner_work(memory, base, generation, state, hour, country);
        if !work.into_iter().any(|work| work) {
            return;
        }
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

    fn acquire(
        &mut self,
        generation: u64,
        event: impl FnOnce(u64) -> EventFeedEvent,
        proof: Option<Proof>,
    ) -> Result<Ticket, &'static str> {
        if let Err(reason) = self.observation(generation) {
            return Err(self.reject(reason));
        }
        if self.entries.len() >= self.entries.capacity().saturating_sub(self.owner_controls)
            && self
                .prepare_capacity(
                    self.entries
                        .len()
                        .max(self.owner_controls.saturating_add(1)),
                )
                .is_err()
        {
            return Err(self.reject(OCCURRENCE_ALLOCATION));
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

    pub fn script_known(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
    ) -> Result<(), &'static str> {
        let token = Token::Native(fact.token);
        self.script_token(memory, base, generation, hour, fact, token)
    }
    fn script_text(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let reserved = self.owner_controls;
        self.owner_controls = reserved.saturating_sub(1);
        let result = self.script_token(memory, base, generation, hour, fact, Token::Text(reason));
        if result.is_err() {
            self.owner_controls = reserved;
        }
        result
    }
    fn script_token(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        fact: ScriptFact,
        token: Token,
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
            if !entry_room(self) {
                self.prepare_capacity(
                    self.entries
                        .len()
                        .max(self.owner_controls.saturating_add(1)),
                )
                .map_err(|_| OCCURRENCE_ALLOCATION)?;
            }
            let mut token_end = self
                .tokens_used
                .checked_add(token.len())
                .ok_or("event feed script token demand overflow")?;
            let token_demand =
                token_end.saturating_add(self.owner_controls.saturating_mul(OWNER_CONTROL_TOKEN));
            self.token_demand = self.token_demand.max(token_demand);
            let token_result = if token_demand > self.script_tokens.len()
                && self
                    .prepare_scripts(
                        token_demand.max(self.script_tokens.len().saturating_mul(2)),
                        self.script_scopes.len(),
                        self.scope_nodes.len(),
                    )
                    .is_err()
            {
                Err("event feed script token allocation failed")
            } else {
                self.script_tokens
                    .get_mut(self.tokens_used..token_end)
                    .ok_or("event feed script tokens need prepared storage")
                    .and_then(|output| token.copy(memory, output))
            };
            let token_unrecorded = if let Err(reason) = token_result {
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
                let mut grown = true;
                while graph.failure.is_some() && graph.required > self.scope_nodes.len() {
                    if self
                        .prepare_scripts(
                            self.script_tokens.len(),
                            self.script_scopes.len(),
                            graph.required.max(self.scope_nodes.len().saturating_mul(2)),
                        )
                        .is_err()
                    {
                        grown = false;
                        break;
                    }
                    graph = engine_events::cevent_scope::copy_graph(
                        memory,
                        base,
                        root,
                        &mut self.scope_nodes,
                        &mut self.scope_index,
                    );
                }
                self.graph_demand = self.graph_demand.max(graph.required);
                if let Some(reason) = graph.failure {
                    self.reject(if grown {
                        reason
                    } else {
                        "event feed scope graph allocation failed"
                    });
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
            if scope_end > self.script_scopes.len()
                && self
                    .prepare_scripts(
                        self.script_tokens.len(),
                        scope_end.max(self.script_scopes.len().saturating_mul(2)),
                        self.scope_nodes.len(),
                    )
                    .is_err()
            {
                self.reject("event feed script scope allocation failed");
                scope_unrecorded = true;
                graph_len = 0;
                scope_end = self.scopes_used;
            }
            let quiet = crate::country_input_changes::names_quiet();
            let mut world = None;
            for (output, node) in self.script_scopes[self.scopes_used..scope_end]
                .iter_mut()
                .zip(&self.scope_nodes[..graph_len])
            {
                let value = node.operands;
                let country = if value.country as i32 > 0 {
                    world
                        .get_or_insert_with(|| native::provenance::World::new(memory, base).ok())
                        .as_ref()
                        .and_then(|world| world.country_and_object(value.country).ok())
                } else {
                    None
                };
                *output = EventFeedScope {
                    country_attempted: true,
                    country_id: country.map(|(identity, _)| identity.id),
                    country_names: quiet
                        .and(country)
                        .and_then(|(_, object)| country_names(memory, object)),
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
            if quiet.is_none() || crate::country_input_changes::names_quiet() != quiet {
                for output in &mut self.script_scopes[self.scopes_used..scope_end] {
                    output.country_names = None;
                }
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

    pub fn script_instance(&mut self, generation: u64, frame: usize, instance: u32) {
        if self.observation(generation).is_err() {
            return;
        }
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
        let entry = &mut self.entries[ticket.slot];
        let published = entry.state == State::PublishedPending;
        let proof = entry.proof.take().unwrap();
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

    pub fn acknowledged(&mut self, publication: u64) {
        self.receipts(&[(publication, Some(true))]);
    }
    pub fn discarded(&mut self, publication: u64) {
        self.receipts(&[(publication, Some(false))]);
    }
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
        self.unpublished_from = 0;
        if self
            .handoff
            .is_some_and(|(handoff, _)| handoff == publication)
        {
            self.handoff = None;
        }
        self.invalidate("Event feed hourly snapshot discarded before output");
    }
    pub fn receipt_room(&self) -> Option<usize> {
        let len = self.publications.len();
        (len == self.publications.capacity()).then(|| len.saturating_mul(2).max(len + 1))
    }
    pub fn install_receipts(&mut self, spare: &mut Vec<u64>) {
        if self.publications.len() == self.publications.capacity()
            && spare.capacity() > self.publications.len()
        {
            spare.clear();
            spare.extend_from_slice(&self.publications);
            std::mem::swap(&mut self.publications, spare);
        }
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
        self.prepare_boundary::<engine_native_memory::memory::LiveMemory>(boundary, None)?;
        self.seal(output, boundary)
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
        self.seal(output, boundary)
    }
    pub fn prepare_boundary<M: Memory>(
        &mut self,
        boundary: OwnerBoundary,
        registry: Option<(&M, usize)>,
    ) -> Result<(), &'static str> {
        self.prepared = None;
        self.reset_staged();
        if self.outstanding() == 0 && self.retained.is_none() {
            self.entries.retain(|entry| entry.state != State::Published);
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
    pub fn seal(
        &mut self,
        output: &mut Snapshot,
        mut boundary: OwnerBoundary,
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
        let unpublished = self.unpublished_from;
        let demand = self.seal_demand()?;
        output.required = demand.events;
        output.reserve(demand)?;
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
        allocation::reserve(&mut self.publications, 1).map_err(|_| RECEIPT_STORAGE)?;
        self.advance()?;
        if boundary_refusal {
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

fn country_names(
    memory: &impl Memory,
    country: usize,
) -> Option<[replay_data::event_feed_rows::EventFeedText; 3]> {
    use engine_native::ccountry::{ADJECTIVE, DEFINITE_NAME, NAME};
    let text = |offset: usize| {
        let descriptor =
            engine_native_memory::msvc_string::Descriptor::read(memory, country.checked_add(offset)?)
                .ok()?;
        replay_data::event_feed_rows::EventFeedText::filled(descriptor.len, |output| {
            descriptor.copy(memory, output).is_ok()
        })
    };
    Some([text(NAME)?, text(DEFINITE_NAME)?, text(ADJECTIVE)?])
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

pub struct Fact {
    pub source: EventFeedSource,
    pub actor: native::CountryIdentity,
    pub target: native::CountryIdentity,
    pub cause: EventFeedCause,
}

pub struct ScriptFact {
    pub source: EventFeedSource,
    pub actor: native::CountryIdentity,
    pub target: native::CountryIdentity,
    pub cause: EventFeedCause,
    pub token: engine_native_memory::msvc_string::Descriptor,
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
