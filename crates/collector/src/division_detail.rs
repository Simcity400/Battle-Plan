//! Owned division detail rows. Acquisition consumes the shared authenticated
//! census once; the decode worker installs its results without replaying live
//! pointer walks or reconstructing a memory-span index.

use crate::{
    extended::production,
    read_accounting,
    sampling::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status,
    cequipment_variant_pool::{self, DivisionSource},
    cupgrade_delivery,
    walk::Outcome,
};
use replay::rows::TableId;
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
    fn prefetch(&self, address: usize, len: usize) {
        // Hints acquire no bytes; retain the source adapter's hint handling
        // without adding acquisition traffic or consuming a guarded read.
        self.memory.prefetch(address, len);
    }

    #[inline]
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.memory.read_scalar::<T>(address)?;
        self.add(std::mem::size_of::<T>() as u64);
        Some(value)
    }

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

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<engine::ccountry::Country, &'static str>> {
        self.memory.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::ReservedCountryObservation> {
        self.memory
            .authenticated_reserved_country_observation(base, game_state)
    }
}

const EMPTY: Outcome = Outcome::Unavailable {
    reason: "division details not captured",
};
/// Order is equipment, groups, requests, deliveries, produced. Every table
/// retains its own capacity and outcome even though upgrade acquisition shares
/// the army-owned parent graph. Groups retain one row per accepted census
/// source; acquisition is worker-only and grows from each original live header.
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
    #[cfg(test)]
    pub(crate) fn new(source_capacity: usize) -> Self {
        let capacities = production::Capacities::default().rows;
        let required = [
            capacities[2],
            capacities[7].max(source_capacity),
            capacities[8],
            capacities[9],
            capacities[10],
        ];
        Self {
            equipment: engine::allocation::owned_vec(engine::allocation::default_vec::<
                cequipment_variant_pool::DivisionRow,
            >(required[0])),
            groups: engine::allocation::owned_vec(engine::allocation::default_vec::<
                carmy_upgrades_requests::Row,
            >(required[1])),
            requests: engine::allocation::owned_vec(engine::allocation::default_vec::<
                army_upgrade_request_row::Row,
            >(required[2])),
            deliveries: engine::allocation::owned_vec(engine::allocation::default_vec::<
                cupgrade_delivery::Row,
            >(required[3])),
            produced: engine::allocation::owned_vec(engine::allocation::default_vec::<
                cupgrade_delivery::ProducedRow,
            >(required[4])),
            outcomes: [EMPTY; 5],
            acquired_bytes: [0; 5],
            required,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.outcomes = [EMPTY; 5];
        self.acquired_bytes = [0; 5];
    }

    pub(crate) fn mark_skipped(&mut self) {
        self.outcomes = [Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        }; 5];
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

    pub(crate) fn grow(&mut self, source_capacity: usize) {
        self.required[1] = self.required[1].max(source_capacity);
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
        self.grow(sources.len());
        let memory = CountedMemory::new(memory);
        let equipment_reads = read_accounting::table(TableId::DivisionEquipment);
        let started = qpc();
        self.outcomes[0] = cequipment_variant_pool::walk_divisions_growing(
            &memory,
            base,
            sources,
            &mut self.equipment,
        );
        self.acquired_bytes[0] = memory.bytes();
        let equipment_ticks = qpc().saturating_sub(started).max(0) as u64;
        drop(equipment_reads);
        let _upgrade_reads = read_accounting::table(TableId::ArmyUpgradeGroups);
        let started = qpc();
        let result = carmy_upgrades_status::walk_growing(
            &memory,
            base,
            sources,
            &mut self.groups,
            &mut self.requests,
            &mut self.deliveries,
            &mut self.produced,
        );
        self.outcomes[1..].copy_from_slice(&[
            result.groups,
            result.requests,
            result.deliveries,
            result.produced,
        ]);
        // The existing read group assigns its shared traversal cost to groups;
        // child zero slots therefore never claim that their acquisition is free.
        let upgrade_ticks = qpc().saturating_sub(started).max(0) as u64;
        self.acquired_bytes[1] = memory.bytes().saturating_sub(self.acquired_bytes[0]);
        [equipment_ticks, upgrade_ticks, 0, 0, 0]
    }

    /// Transfer ownership without copying rows. The displaced buffers return
    /// to the same packet pool for the next callback. Prepare the displaced
    /// group buffer here so both owners retain the census capacity bound.
    pub(crate) fn install(&mut self, captured: &mut production::Captured) {
        install(
            &mut self.equipment,
            self.outcomes[0],
            &mut captured.division_equipment,
        );
        grow_vec(&mut captured.groups.rows, self.groups.len());
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

fn install<R>(rows: &mut Vec<R>, outcome: Outcome, captured: &mut CaptureRows<R>) {
    let capacity = captured.rows.len().max(outcome.len());
    std::mem::swap(rows, &mut captured.rows);
    captured.rows.truncate(capacity);
    captured.outcome = outcome;
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{carmy, memory::SliceMemory};

    #[test]
    fn counted_memory_forwards_hints_without_reading_or_charging_bytes() {
        #[derive(Default)]
        struct Hints {
            hints: std::cell::RefCell<Vec<(usize, usize)>>,
            copies: Cell<usize>,
        }
        impl Memory for Hints {
            fn copy(&self, _: usize, destination: &mut [u8]) -> bool {
                self.copies.set(self.copies.get() + 1);
                destination.fill(7);
                true
            }

            fn prefetch(&self, address: usize, len: usize) {
                self.hints.borrow_mut().push((address, len));
            }
        }
        let memory = Hints::default();
        let counted = CountedMemory::new(&memory);
        counted.prefetch(0x1000, 0xaa);
        counted.prefetch(usize::MAX, 8);
        assert_eq!(*memory.hints.borrow(), [(0x1000, 0xaa), (usize::MAX, 8)]);
        assert_eq!(memory.copies.get(), 0);
        assert_eq!(counted.bytes(), 0);
        assert!(counted.copy(0x1000, &mut [0; 8]));
        assert_eq!(memory.copies.get(), 1);
        assert_eq!(counted.bytes(), 8);
    }

    #[test]
    fn recycled_details_size_from_live_rows() {
        for population in [1, 1500, 5000] {
            let (bytes, fixture_sources) = fixture();
            let sources: Vec<_> = (0..population)
                .map(|index| DivisionSource {
                    country: index as u32 + 1,
                    division: index as u64 + 1,
                    ..fixture_sources[0]
                })
                .collect();
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut owned = OwnedDetails::default();
            let mut capacities = production::Capacities {
                rows: [3; production::N],
                ..Default::default()
            };
            let mut frames: Vec<_> = (0..6)
                .map(|_| production::Captured::with_capacities(&capacities))
                .collect();
            let mut previous = 0;
            for hour in 0..128 {
                let captured = &mut frames[hour % 6];
                let hints = [
                    captured.division_equipment.rows.len(),
                    captured.groups.rows.len(),
                    captured.requests.rows.len(),
                    captured.deliveries.rows.len(),
                    captured.produced.rows.len(),
                ];
                owned.capture(
                    &memory,
                    0,
                    &sources,
                    Outcome::Rows { len: population },
                    hints,
                );
                assert_eq!(owned.outcomes()[1], Outcome::Rows { len: population });
                owned.install(captured);
                for frame in &mut frames {
                    frame.grow(&mut capacities);
                }
                let allocated = frames
                    .iter()
                    .map(production::Captured::allocated_bytes)
                    .sum::<usize>()
                    + owned.allocated_bytes();
                if hour >= frames.len() {
                    assert_eq!(
                        allocated, previous,
                        "population {population}, hour {hour}: {capacities:?}"
                    );
                }
                previous = allocated;
            }
        }
    }

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
    fn group_retention_preserves_aliases_and_original_reads_through_late_failure() {
        struct Trace<'a> {
            memory: SliceMemory<'a>,
            reads: std::cell::RefCell<Vec<(usize, usize)>>,
            scalars: Cell<usize>,
        }
        impl Memory for Trace<'_> {
            fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
                self.scalars.set(self.scalars.get() + 1);
                self.reads
                    .borrow_mut()
                    .push((address, std::mem::size_of::<T>()));
                self.memory.read_scalar::<T>(address)
            }

            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, destination.len()));
                self.memory.copy(address, destination)
            }
        }
        let (mut bytes, [source]) = fixture();
        let invalid = DivisionSource {
            pointer: 0x1100,
            ..source
        };
        for (address, value) in [
            (
                invalid.pointer + carmy::EQUIPMENT_POOL,
                cequipment_variant_pool::VTABLE_RVA as u64,
            ),
            (
                invalid.pointer + carmy::REQUESTS,
                (0x2800 - carmy_upgrades_requests::IN_ARMY_REQUESTS) as u64,
            ),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        let memory = Trace {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            reads: Default::default(),
            scalars: Cell::new(0),
        };
        for (sources, expected) in [
            ([source, source], Outcome::Rows { len: 2 }),
            (
                [source, invalid],
                Outcome::Unavailable {
                    reason: "army upgrade group vtable mismatch",
                },
            ),
        ] {
            let equipment = cequipment_variant_pool::walk_divisions(&memory, 0, &sources, &mut []);
            let bounded = carmy_upgrades_status::walk(
                &memory,
                0,
                &sources,
                &mut [],
                &mut [],
                &mut [],
                &mut [],
            );
            let reads = memory.reads.take();
            let scalars = memory.scalars.replace(0);
            // This fixture uses eight-byte scalars and bulk reads. Their
            // original copy path stays outside the four-byte optimization.
            assert_eq!(scalars, 0);
            let mut owned = OwnedDetails::new(sources.len());
            let allocated = owned.allocated_bytes();
            owned.capture(&memory, 0, &sources, Outcome::Rows { len: 2 }, [0; 5]);
            assert_eq!(owned.allocated_bytes(), allocated);
            assert_eq!(memory.reads.take(), reads);
            assert_eq!(memory.scalars.replace(0), scalars);
            assert_eq!(
                owned.acquired_bytes().iter().sum::<u64>(),
                reads.iter().map(|&(_, len)| len as u64).sum::<u64>()
            );
            assert_eq!(
                owned.outcomes(),
                [
                    equipment,
                    expected,
                    bounded.requests,
                    bounded.deliveries,
                    bounded.produced
                ]
            );
            assert_eq!(owned.groups[0].army, source.division);
            if expected == (Outcome::Rows { len: 2 }) {
                assert_eq!(owned.groups[0], owned.groups[1]);
                assert_eq!(
                    bounded.groups,
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 2
                    }
                );
            } else {
                assert_eq!(expected, bounded.groups);
                assert_eq!(owned.groups[1], carmy_upgrades_requests::Row::default());
            }
        }
        let counted = CountedMemory::new(&memory);
        assert!(counted.read_scalar::<u32>(source.pointer).is_some());
        assert_eq!(counted.bytes(), 4);
        assert_eq!(counted.read_scalar::<u64>(bytes.len() - 4), None);
        assert_eq!(counted.bytes(), 4);
        assert_eq!(memory.scalars.get(), 2);
    }

    #[test]
    fn incomplete_census_never_becomes_empty_success() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut owned = OwnedDetails::new(0);
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
    }

    #[test]
    fn live_pool_and_delivery_headers_grow_before_first_capture_and_installation() {
        use engine::{ccountry, cequipment_variant, cgame_state, cpdx_array, equipment_entry};
        let (mut bytes, sources) = fixture();
        let equipment_rows = 40_000_usize;
        let delivery_rows = 5_000_usize;
        let entries = 0x4000;
        let pointers = entries + equipment_rows * equipment_entry::STRIDE;
        let delivery = pointers + delivery_rows * 8;
        let variant = delivery + 0x100;
        bytes.resize(variant + 0x100, 0);
        let word = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let array = |bytes: &mut [u8], at: usize, data: usize, count: usize| {
            word(bytes, at, data as u64);
            bytes[at + 8..at + 12].copy_from_slice(&(count as u32).to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&(count as u32).to_le_bytes());
        };
        array(
            &mut bytes,
            sources[0].pointer + carmy::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
            entries,
            equipment_rows,
        );
        array(
            &mut bytes,
            0x2000 + carmy_upgrades_requests::DELIVERIES_DATA,
            pointers,
            delivery_rows,
        );
        word(&mut bytes, variant, cequipment_variant::VTABLE_RVA as u64);
        word(
            &mut bytes,
            variant + cequipment_variant::TYPE,
            0x8000_0000_0000_0046,
        );
        word(&mut bytes, delivery, cupgrade_delivery::VTABLE_RVA as u64);
        word(
            &mut bytes,
            delivery + cupgrade_delivery::PRODUCED_EQUIPMENT_POOL,
            cequipment_variant_pool::VTABLE_RVA as u64,
        );
        for index in 0..equipment_rows {
            word(
                &mut bytes,
                entries + index * equipment_entry::STRIDE,
                variant as u64,
            );
            word(
                &mut bytes,
                entries + index * equipment_entry::STRIDE + 8,
                index as u64,
            );
        }
        for index in 0..delivery_rows {
            word(&mut bytes, pointers + index * 8, delivery as u64);
        }
        let state = variant + 0x200;
        let country = state + 0x2000;
        let status = country + 0x1000;
        let country_slots = state + 0x1800;
        let links = state + 0x1900;
        let army_slots = state + 0x1a00;
        bytes.resize(status + 0x1000, 0);
        array(&mut bytes, state + cgame_state::COUNTRIES, country_slots, 2);
        array(&mut bytes, state + cgame_state::COUNTRY_LINKS, links, 2);
        word(&mut bytes, country_slots + 8, country as u64);
        bytes[links + 4..links + 8].copy_from_slice(&1_u32.to_le_bytes());
        word(&mut bytes, country, ccountry::VTABLE_RVA as u64);
        bytes[country + ccountry::ID..country + ccountry::ID + 4]
            .copy_from_slice(&1_u32.to_le_bytes());
        word(
            &mut bytes,
            country + ccountry::PRODUCTION_STATUS,
            status as u64,
        );
        word(
            &mut bytes,
            status,
            engine::cproduction_status::VTABLE_RVA as u64,
        );
        word(
            &mut bytes,
            status + engine::cproduction_status::COUNTRY,
            country as u64,
        );
        word(
            &mut bytes,
            status + engine::cproduction_status::STOCKPILE_POOL,
            cequipment_variant_pool::VTABLE_RVA as u64,
        );
        array(&mut bytes, country + ccountry::DIVISIONS, army_slots, 1);
        word(&mut bytes, army_slots, sources[0].pointer as u64);
        word(&mut bytes, sources[0].pointer, carmy::VTABLE_RVA as u64);
        word(
            &mut bytes,
            sources[0].pointer + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA as u64,
        );
        word(
            &mut bytes,
            sources[0].pointer + engine::cunit::HANDLE,
            sources[0].division,
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let limits = production::Capacities::default();
        let mut sized = limits;
        sized.presize(&memory, 0, state).unwrap();
        assert!(sized.rows[2] >= equipment_rows && sized.rows[9] >= delivery_rows);
        let mut owned = OwnedDetails::new(1);
        owned.capture(
            &memory,
            0,
            &sources,
            Outcome::Rows { len: 1 },
            [
                limits.rows[2],
                limits.rows[7],
                limits.rows[8],
                limits.rows[9],
                limits.rows[10],
            ],
        );
        assert_eq!(
            owned.outcomes()[0],
            Outcome::Rows {
                len: equipment_rows
            }
        );
        assert_eq!(owned.outcomes()[3], Outcome::Rows { len: delivery_rows });
        let mut captured = production::Captured::with_capacities(&limits);
        owned.install(&mut captured);
        assert_eq!(
            captured.division_equipment.outcome,
            Outcome::Rows {
                len: equipment_rows
            }
        );
        assert_eq!(
            captured.deliveries.outcome,
            Outcome::Rows { len: delivery_rows }
        );
        assert_eq!(
            captured.division_equipment.rows[equipment_rows - 1].amount_raw,
            (equipment_rows - 1) as i64
        );
        assert_eq!(
            captured.deliveries.rows[delivery_rows - 1].ordinal,
            (delivery_rows - 1) as u32
        );
        assert!(matches!(
            captured.tables()[2],
            replay::frame::TableView::Complete(_)
        ));
        assert!(matches!(
            captured.tables()[9],
            replay::frame::TableView::Complete(_)
        ));
        // The actual native pool header, not its allocation capacity, supplied
        // the exact initial demand above.
        assert_eq!(
            cpdx_array::Array::read(
                &memory,
                sources[0].pointer + carmy::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
                equipment_entry::STRIDE
            )
            .unwrap()
            .count as usize,
            equipment_rows
        );
    }

    #[test]
    fn install_transfers_primary_buffers_and_preserves_exact_rows() {
        let (bytes, sources) = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut owned = OwnedDetails::new(sources.len());
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [1; 5]);
        let pointer = owned.groups.as_ptr();
        let mut captured =
            production::Captured::with_capacities(&production::Capacities::default());
        captured.divisions_raw_mode = true;
        owned.install(&mut captured);
        assert_eq!(captured.groups.rows.as_ptr(), pointer);
        assert_eq!(captured.groups.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.groups.rows[0].army, sources[0].division);
        assert!(!captured.divisions_raw_mode);
        assert_eq!(owned.outcomes(), [EMPTY; 5]);
    }

    #[test]
    fn only_off_thread_growth_satisfies_larger_requested_capacity() {
        let mut owned = OwnedDetails::new(1);
        let mut capacities = owned.capacities();
        capacities[3] = capacities[3] * 3 + 1;
        let previous = owned.allocated_bytes();
        assert!(!owned.ready_for(capacities));
        assert_eq!(owned.allocated_bytes(), previous);
        let source_capacity = owned.groups.len() * 2 + 1;
        owned.grow(source_capacity);
        assert!(owned.ready_for(capacities));
        assert!(owned.groups.len() >= source_capacity);
        assert!(owned.allocated_bytes() > previous);
    }

    #[test]
    fn callback_fallback_is_empty_and_install_preserves_group_bound_for_reuse() {
        let fallback = OwnedDetails::default();
        assert_eq!(fallback.allocated_bytes(), 0);
        assert_eq!(fallback.capacities(), [0; 5]);
        let (bytes, sources) = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut owned = OwnedDetails::new(sources.len());
        let group_capacity = owned.groups.len();
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [1; 5]);
        let capacities = production::Capacities {
            rows: [1; production::N],
            ..production::Capacities::default()
        };
        let mut captured = production::Captured::with_capacities(&capacities);
        captured.groups.rows = Vec::new();
        owned.install(&mut captured);
        assert_eq!(captured.division_equipment.rows.len(), 1);
        assert_eq!(captured.groups.rows.len(), group_capacity);
        assert_eq!(captured.groups.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.groups.rows[0].army, sources[0].division);
        assert_eq!(captured.requests.rows.len(), 1);
        assert_eq!(captured.deliveries.rows.len(), 1);
        assert_eq!(captured.produced.rows.len(), 1);
        let mut capacities = capacities;
        captured.grow(&mut capacities);
        assert_eq!(capacities.rows[7], captured.groups.rows.len());
        assert!(owned.ready_for([1; 5]));
        // The displaced packet has smaller logical rows; restore its retained
        // prepared floors off-thread before measuring allocation-free reuse.
        owned.grow(sources.len());
        let allocated = owned.allocated_bytes();
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [1; 5]);
        assert_eq!(owned.allocated_bytes(), allocated);
        assert_eq!(owned.outcomes[1], Outcome::Rows { len: 1 });
        assert!(owned.groups.len() >= group_capacity);
    }
}
