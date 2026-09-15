//! Owned division detail rows. Acquisition consumes the shared authenticated
//! census once; the decode worker installs its results without replaying live
//! pointer walks or reconstructing a memory-span index.

use crate::{
    extended::production,
    sampling::{full_match, mismatches, qpc},
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status,
    cequipment_variant_pool::{self, DivisionSource},
    cupgrade_delivery,
    walk::Outcome,
};
use std::cell::Cell;

/// Counts successfully copied source bytes, including repeated reads. This is
/// acquisition traffic, not owned buffer size. Batches are forwarded intact;
/// failed fields may have copied a partial prefix that cannot be measured and
/// is deliberately excluded. No bytes or address lookup state are retained.
pub(crate) struct CountedMemory<'a, M> {
    memory: &'a M,
    bytes: Cell<u64>,
}

impl<'a, M: Memory> CountedMemory<'a, M> {
    pub(crate) fn new(memory: &'a M) -> Self {
        Self {
            memory,
            bytes: Cell::new(0),
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes.get()
    }

    fn add(&self, bytes: u64) {
        self.bytes.set(self.bytes.get().saturating_add(bytes));
    }
}

impl<M: Memory> Memory for CountedMemory<'_, M> {
    #[inline]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let copied = self.memory.copy(address, destination);
        if copied {
            self.add(destination.len() as u64);
        }
        copied
    }

    #[inline]
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.memory.copy_scattered(addresses, destinations);
        self.add(
            destinations
                .iter()
                .take(copied)
                .fold(0_u64, |sum, span| sum.saturating_add(span.len() as u64)),
        );
        copied
    }

    #[inline]
    fn copy_records(
        &self,
        sources: engine::memory::RecordSources<'_>,
        layout: &engine::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let copied = self.memory.copy_records(sources, layout, destination);
        self.add(layout.completed_bytes(copied) as u64);
        copied
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.memory.authenticated_countries(base, game_state)
    }
}

const EMPTY: Outcome = Outcome::Unavailable {
    reason: "division details not captured",
};
const CAPACITY: Outcome = Outcome::Unavailable {
    reason: "owned division detail row capacity insufficient",
};

/// Order is equipment, groups, requests, deliveries, produced. Every table
/// retains its own capacity and outcome even though upgrade acquisition shares
/// the army-owned parent graph. Only construction and `grow` allocate.
#[derive(Debug)]
pub(crate) struct OwnedDetails {
    equipment: Vec<cequipment_variant_pool::DivisionRow>,
    groups: Vec<carmy_upgrades_requests::Row>,
    requests: Vec<army_upgrade_request_row::Row>,
    deliveries: Vec<cupgrade_delivery::Row>,
    produced: Vec<cupgrade_delivery::ProducedRow>,
    outcomes: [Outcome; 5],
    acquired_bytes: [u64; 5],
    required: [usize; 5],
}

impl Default for OwnedDetails {
    fn default() -> Self {
        Self {
            equipment: Vec::new(),
            groups: Vec::new(),
            requests: Vec::new(),
            deliveries: Vec::new(),
            produced: Vec::new(),
            outcomes: [EMPTY; 5],
            acquired_bytes: [0; 5],
            required: [0; 5],
        }
    }
}

impl OwnedDetails {
    pub(crate) fn new() -> Self {
        let capacities = production::Capacities::default().rows;
        let required = [
            capacities[2],
            capacities[7],
            capacities[8],
            capacities[9],
            capacities[10],
        ];
        Self {
            equipment: vec![cequipment_variant_pool::DivisionRow::default(); required[0]],
            groups: vec![carmy_upgrades_requests::Row::default(); required[1]],
            requests: vec![army_upgrade_request_row::Row::default(); required[2]],
            deliveries: vec![cupgrade_delivery::Row::default(); required[3]],
            produced: vec![cupgrade_delivery::ProducedRow::default(); required[4]],
            outcomes: [EMPTY; 5],
            acquired_bytes: [0; 5],
            required,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.outcomes = [EMPTY; 5];
        self.acquired_bytes = [0; 5];
    }

    fn capacities(&self) -> [usize; 5] {
        [
            self.equipment.len(),
            self.groups.len(),
            self.requests.len(),
            self.deliveries.len(),
            self.produced.len(),
        ]
    }

    pub(crate) fn ready_for(&mut self, capacities: [usize; 5]) -> bool {
        for (required, capacity) in self.required.iter_mut().zip(capacities) {
            *required = (*required).max(capacity);
        }
        self.capacities()
            .into_iter()
            .zip(capacities)
            .all(|(available, required)| available >= required)
    }

    pub(crate) fn grow(&mut self) {
        let mut capacities = self.capacities();
        for (capacity, required) in capacities.iter_mut().zip(self.required) {
            grow_capacity(capacity, required);
        }
        grow_vec(&mut self.equipment, capacities[0]);
        grow_vec(&mut self.groups, capacities[1]);
        grow_vec(&mut self.requests, capacities[2]);
        grow_vec(&mut self.deliveries, capacities[3]);
        grow_vec(&mut self.produced, capacities[4]);
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.equipment.capacity() * size_of::<cequipment_variant_pool::DivisionRow>()
            + self.groups.capacity() * size_of::<carmy_upgrades_requests::Row>()
            + self.requests.capacity() * size_of::<army_upgrade_request_row::Row>()
            + self.deliveries.capacity() * size_of::<cupgrade_delivery::Row>()
            + self.produced.capacity() * size_of::<cupgrade_delivery::ProducedRow>()
    }

    #[cfg(test)]
    pub(crate) fn outcomes(&self) -> [Outcome; 5] {
        self.outcomes
    }

    pub(crate) fn acquired_bytes(&self) -> [u64; 5] {
        self.acquired_bytes
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sources: &[DivisionSource],
        source_outcome: Outcome,
        capacities: [usize; 5],
    ) -> [u64; 5] {
        self.clear();
        self.ready_for(capacities);
        let sources = match production::complete_source(sources, source_outcome) {
            Ok(sources) => sources,
            Err(reason) => {
                self.outcomes = [Outcome::Unavailable { reason }; 5];
                return [0; 5];
            }
        };
        let memory = CountedMemory::new(memory);
        let available = self.capacities();
        let started = qpc();
        self.outcomes[0] = if available[0] >= capacities[0] {
            cequipment_variant_pool::walk_divisions(
                &memory,
                base,
                sources,
                &mut self.equipment[..capacities[0]],
            )
        } else {
            CAPACITY
        };
        self.acquired_bytes[0] = memory.bytes();
        let equipment_ticks = qpc().saturating_sub(started).max(0) as u64;
        let started = qpc();
        let result = carmy_upgrades_status::walk(
            &memory,
            base,
            sources,
            &mut self.groups[..capacities[1].min(available[1])],
            &mut self.requests[..capacities[2].min(available[2])],
            &mut self.deliveries[..capacities[3].min(available[3])],
            &mut self.produced[..capacities[4].min(available[4])],
        );
        self.outcomes[1..].copy_from_slice(&[
            result.groups,
            result.requests,
            result.deliveries,
            result.produced,
        ]);
        for index in 1..5 {
            if available[index] < capacities[index] {
                self.outcomes[index] = CAPACITY;
            }
        }
        // The existing read group assigns its shared traversal cost to groups;
        // child zero slots therefore never claim that their acquisition is free.
        let upgrade_ticks = qpc().saturating_sub(started).max(0) as u64;
        self.acquired_bytes[1] = memory.bytes().saturating_sub(self.acquired_bytes[0]);
        [equipment_ticks, upgrade_ticks, 0, 0, 0]
    }

    pub(crate) fn equal(&self, other: &Self) -> [bool; 5] {
        [
            same(
                self.outcomes[0],
                &self.equipment,
                other.outcomes[0],
                &other.equipment,
            ),
            same(
                self.outcomes[1],
                &self.groups,
                other.outcomes[1],
                &other.groups,
            ),
            same(
                self.outcomes[2],
                &self.requests,
                other.outcomes[2],
                &other.requests,
            ),
            same(
                self.outcomes[3],
                &self.deliveries,
                other.outcomes[3],
                &other.deliveries,
            ),
            same(
                self.outcomes[4],
                &self.produced,
                other.outcomes[4],
                &other.produced,
            ),
        ]
    }

    pub(crate) fn differences(&self, other: &Self) -> [usize; 5] {
        [
            mismatches(
                self.outcomes[0],
                &self.equipment,
                other.outcomes[0],
                &other.equipment,
            ),
            mismatches(
                self.outcomes[1],
                &self.groups,
                other.outcomes[1],
                &other.groups,
            ),
            mismatches(
                self.outcomes[2],
                &self.requests,
                other.outcomes[2],
                &other.requests,
            ),
            mismatches(
                self.outcomes[3],
                &self.deliveries,
                other.outcomes[3],
                &other.deliveries,
            ),
            mismatches(
                self.outcomes[4],
                &self.produced,
                other.outcomes[4],
                &other.produced,
            ),
        ]
    }

    /// Transfer ownership without copying rows. Comparison flags set by the
    /// acquisition coordinator survive; the displaced buffers return to the
    /// same packet pool for the next callback.
    pub(crate) fn install(&mut self, captured: &mut production::Captured) {
        install(
            &mut self.equipment,
            self.outcomes[0],
            &mut captured.division_equipment,
        );
        install(&mut self.groups, self.outcomes[1], &mut captured.groups);
        install(&mut self.requests, self.outcomes[2], &mut captured.requests);
        install(
            &mut self.deliveries,
            self.outcomes[3],
            &mut captured.deliveries,
        );
        install(&mut self.produced, self.outcomes[4], &mut captured.produced);
        captured.divisions_raw_mode = false;
        self.clear();
    }
}

fn same<R: PartialEq>(first: Outcome, rows: &[R], second: Outcome, other: &[R]) -> bool {
    full_match(first, second, mismatches(first, rows, second, other))
}

fn install<R>(rows: &mut Vec<R>, outcome: Outcome, captured: &mut CaptureRows<R>) {
    let capacity = captured.rows.len();
    std::mem::swap(rows, &mut captured.rows);
    captured.rows.truncate(capacity);
    captured.outcome = outcome;
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{carmy, memory::SliceMemory};

    fn fixture() -> (Vec<u8>, [DivisionSource; 1]) {
        let mut bytes = vec![0_u8; 0x4000];
        let source = DivisionSource {
            pointer: 0x1000,
            division: 9000,
            template: 70,
            country: 3,
        };
        let group = 0x2000;
        for (address, value) in [
            (
                source.pointer + carmy::EQUIPMENT_POOL,
                cequipment_variant_pool::VTABLE_RVA as u64,
            ),
            (
                source.pointer + carmy::REQUESTS,
                (group - carmy_upgrades_requests::IN_ARMY_REQUESTS) as u64,
            ),
            (group, carmy_upgrades_requests::VTABLE_RVA as u64),
            (
                group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
                carmy_upgrades_requests::PERSISTENT_VTABLE_RVA as u64,
            ),
            (group + carmy_upgrades_requests::ARMY, source.pointer as u64),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        (bytes, [source])
    }

    #[test]
    fn bounded_group_capacity_does_not_truncate_empty_child_tables() {
        let (bytes, sources) = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut owned = OwnedDetails::new();
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [0; 5]);
        assert_eq!(
            owned.outcomes(),
            [
                Outcome::Rows { len: 0 },
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 0 },
            ]
        );
        assert_eq!(owned.equal(&owned), [true, false, true, true, true]);
    }

    #[test]
    fn incomplete_census_never_becomes_empty_success() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut owned = OwnedDetails::new();
        owned.capture(
            &memory,
            0,
            &[],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1,
            },
            [0; 5],
        );
        assert_eq!(
            owned.outcomes(),
            [Outcome::Unavailable {
                reason: "source census incomplete"
            }; 5]
        );
        assert_eq!(owned.equal(&owned), [false; 5]);
    }

    #[test]
    fn install_transfers_buffers_and_preserves_independent_checks() {
        let (bytes, sources) = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut owned = OwnedDetails::new();
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [1; 5]);
        let pointer = owned.groups.as_ptr();
        let mut captured = production::Captured::with_capacities(
            &production::Capacities::default(),
            &[0; production::N],
        );
        captured.groups.checked = true;
        captured.requests.mismatches = 7;
        captured.divisions_raw_mode = true;
        owned.install(&mut captured);
        assert_eq!(captured.groups.rows.as_ptr(), pointer);
        assert_eq!(captured.groups.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.groups.rows[0].army, sources[0].division);
        assert!(captured.groups.checked);
        assert_eq!(captured.requests.mismatches, 7);
        assert!(!captured.divisions_raw_mode);
        assert_eq!(owned.outcomes(), [EMPTY; 5]);
    }

    #[test]
    fn only_off_thread_growth_satisfies_larger_requested_capacity() {
        let mut owned = OwnedDetails::new();
        let mut capacities = owned.capacities();
        capacities[3] = capacities[3] * 3 + 1;
        let previous = owned.allocated_bytes();
        assert!(!owned.ready_for(capacities));
        assert_eq!(owned.allocated_bytes(), previous);
        owned.grow();
        assert!(owned.ready_for(capacities));
        assert!(owned.allocated_bytes() > previous);
    }

    #[test]
    fn callback_fallback_is_empty_and_install_preserves_frame_row_bound() {
        let fallback = OwnedDetails::default();
        assert_eq!(fallback.allocated_bytes(), 0);
        assert_eq!(fallback.capacities(), [0; 5]);
        let (bytes, sources) = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut owned = OwnedDetails::new();
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [1; 5]);
        let capacities = production::Capacities {
            rows: [1; production::N],
            ..production::Capacities::default()
        };
        let mut captured = production::Captured::with_capacities(&capacities, &[0; production::N]);
        owned.install(&mut captured);
        assert_eq!(captured.division_equipment.rows.len(), 1);
        assert_eq!(captured.groups.rows.len(), 1);
        assert_eq!(captured.requests.rows.len(), 1);
        assert_eq!(captured.deliveries.rows.len(), 1);
        assert_eq!(captured.produced.rows.len(), 1);
    }
}
