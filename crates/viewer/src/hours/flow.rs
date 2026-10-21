//! Pure ownership/admission ledger for the event-driven hour coordinator.
//! The adapter owns frames, closures, slots and output bytes; this ledger never
//! waits or runs work. Tickets are allocated in original dispatch order.
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Ticket = u64;
const ALL_PARTS: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    Decode,
    Parts,
    Ready,
}

#[derive(Debug)]
pub(super) struct Flight<K> {
    pub epoch: u64,
    pub key: K,
    pub phase: Phase,
    /// Never cleared: one accepted part makes this target non-cancellable.
    accepted: u8,
    complete: u8,
    source_pending: bool,
    /// Only older live tickets: no forward/cyclic dependency is admitted.
    dependencies: BTreeSet<Ticket>,
    pub retired: bool,
}

/// A credit is retained through decode, parts, retirement and local output.
/// Only successful output admission or terminal retirement releases it.
#[derive(Debug)]
pub(super) struct Ledger<K> {
    limit: usize,
    flights: BTreeMap<Ticket, Flight<K>>,
}

impl<K: Eq> Ledger<K> {
    pub fn new(limit: usize) -> Self {
        assert!(limit != 0, "hour concurrency must be nonzero");
        Self {
            limit,
            flights: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.flights.len()
    }
    pub fn get(&self, ticket: Ticket) -> Option<&Flight<K>> {
        self.flights.get(&ticket)
    }
    pub fn contains(&self, epoch: u64, key: &K) -> bool {
        self.flights
            .values()
            .any(|flight| flight.epoch == epoch && &flight.key == key)
    }

    pub fn can_plan(&self) -> bool {
        self.len() < self.limit
            && !self.flights.values().any(|flight| flight.phase == Phase::Decode)
            && self.next_admission().is_none()
            // Do not hide an unresolved predecessor behind another plan.
            && self.flights.values().all(|flight|
                flight.phase != Phase::Parts || flight.accepted | flight.complete == ALL_PARTS)
    }

    pub fn reserve(
        &mut self,
        ticket: Ticket,
        epoch: u64,
        key: K,
        dependencies: BTreeSet<Ticket>,
    ) -> bool {
        if !self.can_plan()
            || self.contains(epoch, &key)
            || self.flights.contains_key(&ticket)
            || self
                .flights
                .last_key_value()
                .is_some_and(|(last, _)| ticket <= *last)
            || dependencies
                .iter()
                .any(|dependency| *dependency >= ticket || !self.flights.contains_key(dependency))
        {
            return false;
        }
        self.flights.insert(
            ticket,
            Flight {
                epoch,
                key,
                phase: Phase::Decode,
                accepted: 0,
                complete: 0,
                source_pending: false,
                dependencies,
                retired: false,
            },
        );
        true
    }

    /// Each native source read retains the target credit until its reply,
    /// including cancellation of a queued or currently executing read.
    pub fn source_started(&mut self, ticket: Ticket) -> bool {
        let Some(flight) = self.flights.get_mut(&ticket) else {
            return false;
        };
        if flight.phase != Phase::Decode || flight.source_pending {
            return false;
        }
        flight.source_pending = true;
        true
    }

    pub fn source_finished(&mut self, ticket: Ticket) -> bool {
        let Some(flight) = self.flights.get_mut(&ticket) else {
            return false;
        };
        if !flight.source_pending {
            return false;
        }
        flight.source_pending = false;
        true
    }

    pub fn obsolete_reads(&self) -> Vec<Ticket> {
        self.flights
            .iter()
            .filter_map(|(&ticket, flight)| {
                (flight.retired && flight.source_pending).then_some(ticket)
            })
            .collect()
    }

    /// Reused parts are already terminal and own no queued closure.
    pub fn decoded(&mut self, ticket: Ticket, reused: u8) -> bool {
        let Some(flight) = self.flights.get_mut(&ticket) else {
            return false;
        };
        if flight.phase != Phase::Decode || flight.source_pending {
            return false;
        }
        flight.complete = reused;
        flight.phase = if reused == ALL_PARTS {
            Phase::Ready
        } else {
            Phase::Parts
        };
        true
    }

    /// A source gap has a complete error result, with no partial build.
    pub fn gap(&mut self, ticket: Ticket) -> bool {
        self.decoded(ticket, ALL_PARTS)
    }

    /// FIFO across held jobs, including retired targets. A later dependent
    /// cannot occupy the pool while a required predecessor is still outside it.
    pub fn next_admission(&self) -> Option<(Ticket, u8)> {
        let (&ticket, flight) = self.flights.iter().find(|(_, flight)| {
            flight.phase == Phase::Parts && flight.accepted | flight.complete != ALL_PARTS
        })?;
        if flight.dependencies.iter().any(|dependency| {
            self.flights
                .get(dependency)
                .is_some_and(|before| before.accepted | before.complete != ALL_PARTS)
        }) {
            return None;
        }
        let missing = !(flight.accepted | flight.complete);
        Some((ticket, missing.trailing_zeros() as u8))
    }

    /// Called only after try_submit_status accepted the owned closure. Events
    /// posted by that closure cannot be reduced until this call returns.
    pub fn accepted(&mut self, ticket: Ticket, part: u8) -> bool {
        if self.next_admission() != Some((ticket, part)) {
            return false;
        }
        self.flights.get_mut(&ticket).expect("present").accepted |= 1 << part;
        true
    }

    /// Stale, duplicate or never-accepted completions cannot fill a replacement.
    pub fn completed(&mut self, ticket: Ticket, part: u8) -> bool {
        if part >= 8 {
            return false;
        }
        let Some(flight) = self.flights.get_mut(&ticket) else {
            return false;
        };
        let bit = 1 << part;
        if flight.phase != Phase::Parts || flight.accepted & bit == 0 || flight.complete & bit != 0
        {
            return false;
        }
        flight.complete |= bit;
        if flight.complete == ALL_PARTS {
            flight.phase = Phase::Ready
        }
        true
    }

    /// Equal readiness sets are enumerated by original dispatch ticket, not
    /// arrival order. An earlier unfinished target never blocks a ready one.
    pub fn ready(&self) -> Vec<Ticket> {
        self.flights
            .iter()
            .filter_map(|(&ticket, flight)| {
                (flight.phase == Phase::Ready && !flight.retired).then_some(ticket)
            })
            .collect()
    }

    pub fn terminal_retired(&self) -> Vec<Ticket> {
        self.flights
            .iter()
            .filter_map(|(&ticket, flight)| {
                (flight.phase == Phase::Ready && flight.retired).then_some(ticket)
            })
            .collect()
    }

    /// Retarget metadata only. Cancel an obsolete tail in reverse dispatch
    /// order, never an accepted job or a predecessor a retained target needs.
    /// The adapter drops only the returned payloads/held closures.
    pub fn retarget(&mut self, mut keep: impl FnMut(u64, &K) -> bool) -> Vec<Ticket> {
        for flight in self.flights.values_mut() {
            flight.retired = !keep(flight.epoch, &flight.key);
        }
        let candidates: Vec<_> = self.flights.keys().rev().copied().collect();
        let mut cancelled = Vec::new();
        for ticket in candidates {
            let flight = self.flights.get(&ticket).expect("present");
            if flight.retired
                && flight.accepted == 0
                && !flight.source_pending
                && !self
                    .flights
                    .values()
                    .any(|later| later.dependencies.contains(&ticket))
            {
                self.flights.remove(&ticket);
                cancelled.push(ticket);
            }
        }
        cancelled
    }

    /// A successful try_send or terminal retirement, never an attempted send.
    pub fn release(&mut self, ticket: Ticket) -> bool {
        if self
            .flights
            .get(&ticket)
            .is_none_or(|flight| flight.phase != Phase::Ready)
        {
            return false;
        }
        self.flights.remove(&ticket);
        true
    }
}

#[cfg(test)]
mod tests;
