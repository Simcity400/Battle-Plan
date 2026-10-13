//! Owned native observations and pointer-free hourly publications with exact receipts.

use engine::{Memory, allocation, event_feed_sites as native};
use replay::{
    event_feed_rows::{
        EventFeedCause, EventFeedCountry, EventFeedCounts, EventFeedCoverage, EventFeedEvent,
        EventFeedHistoryRow, EventFeedSource,
    },
    rows::Recorded,
};
use std::{io, mem::size_of};

// Spool recycles one captured frame before completing its prepared write.
pub(crate) const PUBLICATION_CAPACITY: usize = crate::sampling::BUFFERS + 1;

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
}

/// Pointer-free hourly copy. Its allocation and later encoding are off-thread.
pub(crate) struct Snapshot {
    pub(crate) boundary: OwnerBoundary,
    counts: Option<EventFeedCounts>,
    outstanding: u64,
    events: Vec<EventFeedEvent>,
    publication: u64,
    pub(crate) required: usize,
}
impl Snapshot {
    pub(crate) fn new(capacity: usize) -> Self {
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
            publication: 0,
            required: 0,
        }
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.events.capacity() * size_of::<EventFeedEvent>()
    }
    pub(crate) fn grow(&mut self, capacity: usize) -> io::Result<()> {
        let additional = capacity.saturating_sub(self.events.len());
        allocation::reserve_exact(&mut self.events, additional)
    }
    pub(crate) fn refuse(&mut self, generation: u64, hour: i32, reason: &'static str) {
        self.events.clear();
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
    pub(crate) fn publication(&self) -> u64 {
        self.publication
    }
    pub(crate) fn prepare_pack(&self) -> io::Result<Prepared> {
        let failure = self
            .boundary
            .failure
            .map(|reason| allocation::copy_vec(reason.as_bytes()))
            .transpose()?;
        let row = EventFeedHistoryRow {
            generation: self.boundary.generation,
            read_hour: self.boundary.read_hour,
            coverage: self.boundary.coverage,
            counts: self.counts,
            outstanding: self.outstanding,
            failure,
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
pub(crate) struct DetachLosses {
    pub(crate) unrecorded_tail_events: u64,
    pub(crate) unrecorded_tail_revisions: u64,
    pub(crate) failed_batches: u64,
}
impl Prepared {
    pub fn row(&self) -> &EventFeedHistoryRow {
        &self.row
    }
    pub(crate) fn publication(&self) -> u64 {
        self.revision
    }
}

pub struct Store {
    entries: Vec<Entry>,
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
            entries: Vec::new(),
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
        })
    }

    /// Grow off-thread from live counts or observed pressure.
    pub fn prepare_capacity(&mut self, additional: usize) -> io::Result<()> {
        allocation::reserve_exact(&mut self.entries, additional)?;
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.entries),
            ..Default::default()
        });
        Ok(())
    }
    pub(crate) fn restore_counts(&mut self, counts: EventFeedCounts) {
        self.counts = counts;
        if counts.acquired != counts.settled || counts.failed > counts.settled {
            self.counts_valid = false;
            self.retained = Some("retained Event feed acquisitions have no settlement receipt");
        }
    }
    pub(crate) fn capacity(&self) -> usize {
        self.entries.capacity()
    }
    pub(crate) fn capacity_demand(&self) -> usize {
        self.entries.len().saturating_add(1)
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
        self.failure.get_or_insert(reason);
        self.failure_revision = self.revision;
    }

    /// Count an operation whose inputs or storage could not be retained.
    pub fn reject(&mut self, reason: &'static str) -> &'static str {
        let _ = self.advance();
        self.failure.get_or_insert(reason);
        self.failure_revision = self.revision;
        if let Some(rejected) = self.counts.rejected.checked_add(1) {
            self.counts.rejected = rejected;
        } else {
            self.counts_valid = false;
            self.retained
                .get_or_insert("Event feed rejected count exhausted");
        }
        reason
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
        });
        self.next_sequence = next;
        self.counts.acquired = acquired;
        self.counts.settled = settled;
        Ok(ticket)
    }

    /// An event a provenance hook proved at its own native boundary
    /// ([`crate::war_provenance`]): settled when acquired.
    pub(crate) fn known(
        &mut self,
        generation: u64,
        hour: i32,
        fact: Fact,
    ) -> Result<(), &'static str> {
        self.acquire(
            generation,
            |sequence| EventFeedEvent {
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
    pub(crate) fn world_returned(&mut self, ticket: Ticket) -> Result<(), Retained> {
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
    pub(crate) fn install_capacity(&mut self, spare: &mut Self) {
        if self.outstanding() == 0 && spare.entries.capacity() > self.entries.capacity() {
            spare.entries.clear();
            spare.entries.append(&mut self.entries);
            std::mem::swap(&mut self.entries, &mut spare.entries);
            std::mem::swap(&mut self.charge, &mut spare.charge);
        }
    }

    /// Only the hourly consumer touches live history. Output acknowledges an
    /// immutable publication number, so newer callbacks cannot stale its receipt.
    pub(crate) fn acknowledged(&mut self, publication: u64) {
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
    pub(crate) fn discarded(&mut self, publication: u64) {
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
    pub(crate) fn unacknowledged(&self) -> usize {
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
    pub(crate) fn detach_losses(&self) -> Result<DetachLosses, &'static str> {
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
    #[cfg(test)]
    pub(crate) fn snapshot(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
    ) -> Result<(), &'static str> {
        self.snapshot_mode(output, boundary, false)
    }
    pub(crate) fn snapshot_growing(
        &mut self,
        output: &mut Snapshot,
        boundary: OwnerBoundary,
    ) -> Result<(), &'static str> {
        self.snapshot_mode(output, boundary, true)
    }
    fn snapshot_mode(
        &mut self,
        output: &mut Snapshot,
        mut boundary: OwnerBoundary,
        growing: bool,
    ) -> Result<(), &'static str> {
        // No outstanding ticket can still refer to an index that retain moves.
        if self.outstanding() == 0 && self.retained.is_none() {
            self.entries.retain(|entry| entry.state != State::Published);
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
            self.generation = Some(boundary.generation);
            self.first_interval = true;
            if discarded != 0 {
                self.failure
                    .get_or_insert("Event feed unsampled prior-world events discarded");
            }
            self.failure_revision = self.revision;
            self.handoff = None;
        }
        self.observing = self.retained.is_none();
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
        }
        if output.required > output.events.capacity() {
            return Err("Event feed hourly storage needs growth");
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
        let publication_slot = self
            .publications
            .iter()
            .position(Option::is_none)
            .ok_or("Event feed publication receipts exceed pipeline capacity")?;
        self.advance()?;
        self.publications[publication_slot] = Some(self.revision);
        output.events.clear();
        for entry in &mut self.entries {
            if entry.publication.is_none() && entry.fact() {
                output.events.push(entry.event);
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
pub(crate) struct Fact {
    pub(crate) source: EventFeedSource,
    pub(crate) actor: native::CountryIdentity,
    pub(crate) target: native::CountryIdentity,
    pub(crate) cause: EventFeedCause,
}

#[cfg(test)]
mod tests;
