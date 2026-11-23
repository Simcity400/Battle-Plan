use collector_memory::read_accounting;
use collector_primitives::clock::qpc;
use collector_primitives::table_capture::CaptureRows;
use collector_primitives::table_capture::grow_capacity;
use collector_primitives::table_capture::grow_vec;
use collector_production::production;
use engine_deployment::{
    army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status, cupgrade_delivery,
};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cequipment_variant::pool::{
    self as cequipment_variant_pool,
    DivisionSource,
};
use replay_data::rows::TableId;

const EMPTY: Outcome = Outcome::Unavailable {
    reason: "division details not captured",
};
#[derive(Debug)]
pub(crate) struct OwnedDetails {
    equipment: Vec<cequipment_variant_pool::DivisionRow>,
    groups: Vec<carmy_upgrades_requests::Row>,
    requests: Vec<army_upgrade_request_row::Row>,
    deliveries: Vec<cupgrade_delivery::Row>,
    produced: Vec<cupgrade_delivery::ProducedRow>,
    outcomes: [Outcome; 5],
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
            required: [0; 5],
        }
    }
}

impl OwnedDetails {
    #[cfg(any(test, feature = "test-support"))]
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
            equipment: engine_allocation::owned_vec(engine_allocation::default_vec::<
                cequipment_variant_pool::DivisionRow,
            >(required[0])),
            groups: engine_allocation::owned_vec(engine_allocation::default_vec::<
                carmy_upgrades_requests::Row,
            >(required[1])),
            requests: engine_allocation::owned_vec(engine_allocation::default_vec::<
                army_upgrade_request_row::Row,
            >(required[2])),
            deliveries: engine_allocation::owned_vec(engine_allocation::default_vec::<
                cupgrade_delivery::Row,
            >(required[3])),
            produced: engine_allocation::owned_vec(engine_allocation::default_vec::<
                cupgrade_delivery::ProducedRow,
            >(required[4])),
            outcomes: [EMPTY; 5],
            required,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.outcomes = [EMPTY; 5];
    }

    pub(crate) fn mark_skipped(&mut self) {
        self.outcomes = [Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        }; 5];
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
        let equipment_reads = read_accounting::table(TableId::DivisionEquipment);
        let started = qpc();
        self.outcomes[0] =
            cequipment_variant_pool::walk_divisions(memory, base, sources, &mut self.equipment);
        let equipment_ticks = qpc().saturating_sub(started).max(0) as u64;
        drop(equipment_reads);
        let _upgrade_reads = read_accounting::table(TableId::ArmyUpgradeGroups);
        let started = qpc();
        let result = carmy_upgrades_status::walk(
            memory,
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
        let upgrade_ticks = qpc().saturating_sub(started).max(0) as u64;
        [equipment_ticks, upgrade_ticks, 0, 0, 0]
    }

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
    use engine_native::carmy;
    use engine_native_memory::memory::SliceMemory;
    use std::cell::Cell;

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
                cequipment_variant_pool::VTABLE_RVA.0 as u64,
            ),
            (
                source.pointer + carmy::REQUESTS,
                (group - carmy_upgrades_requests::IN_ARMY_REQUESTS) as u64,
            ),
            (group, carmy_upgrades_requests::VTABLE_RVA.0 as u64),
            (
                group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
                carmy_upgrades_requests::PERSISTENT_VTABLE_RVA.0 as u64,
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
            fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
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
                cequipment_variant_pool::VTABLE_RVA.0 as u64,
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
            let equipment =
                cequipment_variant_pool::walk_divisions(&memory, 0, &sources, &mut Vec::new());
            let bounded = carmy_upgrades_status::walk(
                &memory,
                0,
                &sources,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            );
            let reads = memory.reads.take();
            let scalars = memory.scalars.replace(0);
            assert_eq!(scalars, 0);
            let mut owned = OwnedDetails::new(sources.len());
            let allocated = owned.allocated_bytes();
            owned.capture(&memory, 0, &sources, Outcome::Rows { len: 2 }, [0; 5]);
            assert_eq!(owned.allocated_bytes(), allocated);
            assert_eq!(memory.reads.take(), reads);
            assert_eq!(memory.scalars.replace(0), scalars);
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
            assert_eq!(expected, bounded.groups);
            if expected == (Outcome::Rows { len: 2 }) {
                assert_eq!(owned.groups[0], owned.groups[1]);
            } else {
                assert_eq!(owned.groups[1], carmy_upgrades_requests::Row::default());
            }
        }
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
        use engine_native::{ccountry, cequipment_variant, cgame_state};
        use engine_native_memory::cpdx_array;
        use engine_native_parts::equipment_entry;
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
        word(&mut bytes, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            variant + cequipment_variant::TYPE,
            0x8000_0000_0000_0046,
        );
        word(&mut bytes, delivery, cupgrade_delivery::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            delivery + cupgrade_delivery::PRODUCED_EQUIPMENT_POOL,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
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
        word(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
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
            engine_native::cproduction_status::VTABLE_RVA.0 as u64,
        );
        word(
            &mut bytes,
            status + engine_native::cproduction_status::COUNTRY,
            country as u64,
        );
        word(
            &mut bytes,
            status + engine_native::cproduction_status::STOCKPILE_POOL,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        array(&mut bytes, country + ccountry::DIVISIONS, army_slots, 1);
        word(&mut bytes, army_slots, sources[0].pointer as u64);
        word(&mut bytes, sources[0].pointer, carmy::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            sources[0].pointer + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        word(
            &mut bytes,
            sources[0].pointer + engine_native::cunit::HANDLE,
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
            replay_frame::frame::TableView::Complete(_)
        ));
        assert!(matches!(
            captured.tables()[9],
            replay_frame::frame::TableView::Complete(_)
        ));
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
        owned.grow(sources.len());
        let allocated = owned.allocated_bytes();
        owned.capture(&memory, 0, &sources, Outcome::Rows { len: 1 }, [1; 5]);
        assert_eq!(owned.allocated_bytes(), allocated);
        assert_eq!(owned.outcomes[1], Outcome::Rows { len: 1 });
        assert!(owned.groups.len() >= group_capacity);
    }
}
