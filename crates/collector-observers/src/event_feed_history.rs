//! Owned native observations and pointer-free hourly publications with exact receipts.

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
use std::{io, mem::size_of};

// Spool recycles one captured frame before completing its prepared write.
pub const PUBLICATION_CAPACITY: usize = collector_extended_support::hourly::BUFFERS + 1;

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
        let row = EventFeedHistoryRow {
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
        row.validate()?;
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
    // Accounting-only snapshots also need an exact receipt for uncertain writes.
    publications: [Option<u64>; PUBLICATION_CAPACITY],
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
            publications: [None; PUBLICATION_CAPACITY],
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
            .saturating_add(1)
            .max(self.delayed.demand)
            .max(self.delayed.capacity())
            .max(self.decisions.demand)
            .max(self.decisions.capacity())
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
        self.revoke_retained_baselines();
        let _ = self.advance();
        self.failure.get_or_insert(reason);
        self.failure_revision = self.revision;
        if let Some(rejected) = self.counts.rejected.checked_add(count) {
            self.counts.rejected = rejected;
        } else {
            self.counts_valid = false;
            self.retained
                .get_or_insert("Event feed rejected count exhausted");
        }
        reason
    }

    fn revoke_retained_baselines(&mut self) {
        // A failed census remains an attempted census, never a polling retry.
        // A previously complete census must be acquired again after a lost
        // writer interval: the reducer cannot carry it through that gap.
        if self.registry_complete {
            self.registry_baseline = None;
            self.registry_complete = false;
            self.registry_state = None;
        }
        self.delayed.revoke_retained();
        self.decisions.invalidate();
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
        if self.entries.len() == self.entries.capacity() {
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
        let result = (|| {
            self.observation(generation)?;
            if growing && self.entries.len() == self.entries.capacity() {
                self.prepare_capacity(self.entries.len().max(1))
                    .map_err(|_| "event registry baseline occurrence allocation failed")?;
            }
            if self.entries.len() == self.entries.capacity() {
                return Err("Event feed callback storage needs growth");
            }
            let mut token_end = self
                .tokens_used
                .checked_add(fact.token.len)
                .ok_or("event feed script token demand overflow")?;
            self.token_demand = self.token_demand.max(token_end);
            if growing && token_end > self.script_tokens.len() {
                self.prepare_scripts(
                    token_end.max(self.script_tokens.len().saturating_mul(2)),
                    self.script_scopes.len(),
                    self.scope_nodes.len(),
                )
                .map_err(|_| "event registry baseline token allocation failed")?;
            }
            let token_result = self
                .script_tokens
                .get_mut(self.tokens_used..token_end)
                .ok_or("event feed script tokens need prepared storage")
                .and_then(|token| fact.token.copy(memory, token));
            let token_unrecorded = if let Err(reason) = token_result {
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
                while growing && graph.failure.is_some() && graph.required > self.scope_nodes.len()
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
            if growing && scope_end > self.script_scopes.len() {
                self.prepare_scripts(
                    self.script_tokens.len(),
                    scope_end.max(self.script_scopes.len().saturating_mul(2)),
                    self.scope_nodes.len(),
                )
                .map_err(|_| "event registry baseline scope allocation failed")?;
            }
            if scope_end > self.script_scopes.len() {
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
        if let Some(entry) = self.entries.iter_mut().rev().find(|entry| {
            entry.dispatch_frame == frame
                && entry.event.generation == generation
                && entry.publication.is_none()
        }) && let Some(script) = entry.event.script.as_mut()
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
        if let Some(entry) = self.entries.iter_mut().rev().find(|entry| {
            entry.dispatch_frame != 0
                && entry.dispatch_scope == scope
                && entry.event.generation == generation
                && entry.publication.is_none()
                && entry.dispatch_definition == definition
        }) && let Some(script) = entry.event.script.as_mut()
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
        }
    }

    /// Only the hourly consumer touches live history. Output acknowledges an
    /// immutable publication number, so newer callbacks cannot stale its receipt.
    pub fn acknowledged(&mut self, publication: u64) {
        let Some(pending) = self
            .publications
            .iter_mut()
            .find(|pending| **pending == Some(publication))
        else {
            return;
        };
        *pending = None;
        self.accepted_revision = self.accepted_revision.max(publication);
        for entry in &mut self.entries {
            if entry.publication == Some(publication) {
                entry.state = match entry.state {
                    State::Known | State::Failed => State::Published,
                    State::Pending if matches!(entry.proof, Some(Proof::Annex)) => {
                        State::PublishedPending
                    }
                    state => state,
                };
            }
        }
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
    pub fn discarded(&mut self, publication: u64) {
        let Some(pending) = self
            .publications
            .iter_mut()
            .find(|pending| **pending == Some(publication))
        else {
            return;
        };
        *pending = None;
        self.failed_batches += 1;
        for entry in &mut self.entries {
            if entry.publication == Some(publication)
                && !matches!(entry.state, State::Published | State::PublishedPending)
            {
                entry.publication = None;
            }
        }
        if self
            .handoff
            .is_some_and(|(handoff, _)| handoff == publication)
        {
            self.handoff = None;
        }
        self.invalidate("Event feed hourly snapshot discarded before output");
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
        if self.publications.iter().any(Option::is_some) || self.unacknowledged() != 0 {
            return Err("Event feed output receipt remains uncertain");
        }
        Ok(DetachLosses {
            unrecorded_tail_events: self.entries.iter().filter(|entry| entry.fact()).count() as u64,
            unrecorded_tail_revisions: self.revision - self.accepted_revision,
            failed_batches: self.failed_batches,
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
    ) -> Result<(), &'static str> {
        self.snapshot_mode(output, boundary, false, |_| Ok(()))
    }
    pub fn snapshot_growing(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
    ) -> Result<(), &'static str> {
        self.snapshot_mode(output, boundary, true, |_| Ok(()))
    }
    pub fn snapshot_with_registry(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        self.snapshot_mode(output, boundary, true, |store| {
            // A bootstrap interval may have refused pre-observation calls.
            // Start the single retained census only after that refusal has
            // an output receipt; never acquire a baseline we must invalidate
            // in the same publication. Missing writer coverage also waits;
            // a healthy first interval can still publish a fresh baseline.
            if store.failure.is_some() || boundary.coverage != EventFeedCoverage::Complete {
                return Ok(());
            }
            if let Err(reason) =
                store.registry_baseline(memory, base, boundary.generation, boundary.read_hour)
            {
                store.reject(reason);
            }
            Ok(())
        })
    }
    fn snapshot_mode(
        &mut self,
        output: &mut Snapshot,
        mut boundary: OwnerBoundary,
        growing: bool,
        prepare: impl FnOnce(&mut Self) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
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
            self.tokens_used = 0;
            self.scopes_used = 0;
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
        prepare(self)?;
        let outstanding = self.outstanding();
        boundary.failure = self
            .retained
            .or(self.failure)
            .or(boundary.failure)
            .or_else(|| (outstanding != 0).then_some("Event feed acquired events unsettled"))
            .or_else(|| (!self.counts_valid).then_some("Event feed accounting exhausted"));
        output.required = self
            .entries
            .iter()
            .filter(|entry| entry.publication.is_none() && entry.fact())
            .count();
        if growing {
            output
                .grow(output.required)
                .map_err(|_| "Event feed snapshot allocation failed")?;
            let mut tokens = 0_usize;
            let mut scopes = 0_usize;
            for entry in &self.entries {
                if entry.publication.is_none()
                    && entry.fact()
                    && let Some(script) = entry.event.script
                {
                    tokens = tokens
                        .checked_add(script.token_len as usize)
                        .ok_or("event feed token snapshot demand overflow")?;
                    scopes = scopes
                        .checked_add(script.scope_len as usize)
                        .ok_or("event feed scope snapshot demand overflow")?;
                }
            }
            let token_growth = tokens.saturating_sub(output.script_tokens.len());
            let scope_growth = scopes.saturating_sub(output.script_scopes.len());
            allocation::reserve_exact(&mut output.script_tokens, token_growth)
                .map_err(|_| "event feed token snapshot allocation failed")?;
            allocation::reserve_exact(&mut output.script_scopes, scope_growth)
                .map_err(|_| "event feed scope snapshot allocation failed")?;
        }
        if output.required > output.events.capacity() {
            return Err("Event feed hourly storage needs growth");
        }
        if !growing {
            let (tokens, scopes) = self
                .entries
                .iter()
                .filter(|entry| entry.publication.is_none() && entry.fact())
                .filter_map(|entry| entry.event.script)
                .try_fold((0_usize, 0_usize), |(tokens, scopes), script| {
                    Ok::<_, &'static str>((
                        tokens
                            .checked_add(script.token_len as usize)
                            .ok_or("event feed token snapshot demand overflow")?,
                        scopes
                            .checked_add(script.scope_len as usize)
                            .ok_or("event feed scope snapshot demand overflow")?,
                    ))
                })?;
            if tokens > output.script_tokens.capacity() || scopes > output.script_scopes.capacity()
            {
                return Err("Event feed script hourly storage needs growth");
            }
        }
        if self.entries.iter().any(|entry| {
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
            self.entries.iter().any(|entry| {
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
        let publication_slot = self
            .publications
            .iter()
            .position(Option::is_none)
            .ok_or("Event feed publication receipts exceed pipeline capacity")?;
        self.advance()?;
        if boundary_refusal {
            // An older in-flight publication cannot acknowledge this loss.
            self.failure_revision = self.revision;
        }
        self.publications[publication_slot] = Some(self.revision);
        output.events.clear();
        output.script_tokens.clear();
        output.script_scopes.clear();
        for entry in &mut self.entries {
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
        output.boundary = boundary;
        output.counts = self.counts_valid.then_some(self.counts);
        output.outstanding = outstanding;
        output.publication = self.revision;
        if self.handoff.is_none() {
            self.handoff = Some((self.revision, boundary.coverage));
        }
        self.first_interval = boundary.coverage == EventFeedCoverage::Unavailable;
        Ok(())
    }
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
pub(crate) mod registry;
#[cfg(test)]
mod tests;
