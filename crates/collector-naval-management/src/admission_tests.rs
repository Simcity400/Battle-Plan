use super::*;
use engine_native::Memory;
use engine_native_naval::naval_management as native;
use replay_data::rows::*;
use replay_frame::frame::{
    DecodedFrame, FrameDecoder, FrameEncoder, FrameInput, Metrics, TABLE_COUNT, TableUpdate,
};
use std::collections::BTreeMap;

const STATE: usize = 0x10000;
const COUNTRY: usize = 0x20000;
const PRODUCTION: usize = 0x30000;
const NAVY: usize = 0x40000;
const MANAGER: usize = 0x50000;
const FLEET: usize = 0x60000;
const TASK: usize = 0x70000;
const SHIP: usize = 0x80000;
const LINE: usize = 0x90000;
const BASE: usize = 0xa0000;
const TARGET: usize = 0xb0000;
const VARIANT: usize = 0xc0000;
const AFTER_REPAIR: usize = 0xd0000;
const LISTS: usize = 0xe0000;
const FLEET_ID: u64 = 11 | (101 << 32);
const TASK_ID: u64 = 13 | (103 << 32);
const SHIP_ID: u64 = 37 | (107 << 32);
const LINE_ID: u64 =
    engine_native::cship_refit_production_line::REFERENCE_TYPE as u64 | (109 << 32);

#[derive(Default)]
struct Census {
    bytes: BTreeMap<usize, u8>,
}
impl Memory for Census {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (offset, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + offset)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}
impl Census {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (offset, &byte) in bytes.iter().enumerate() {
            self.bytes.insert(at + offset, byte);
        }
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn integer(&mut self, at: usize, value: i32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data as u64);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
    fn populated(phase: i32) -> Self {
        use engine_native::{ccountry, cgame_state};
        use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};
        let mut memory = Self::default();
        memory.array(STATE + cgame_state::COUNTRIES, LISTS, 8);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, LISTS + 0x100, 8);
        memory.word(LISTS + 7 * 8, COUNTRY as u64);
        memory.integer(LISTS + 0x100 + 7 * 4, 7);
        memory.word(COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        memory.integer(COUNTRY + ccountry::ID, 7);
        memory.word(COUNTRY + ccountry::PRODUCTION_STATUS, PRODUCTION as u64);
        memory.word(
            PRODUCTION,
            engine_native::cproduction_status::VTABLE_RVA.0 as u64,
        );
        memory.word(
            PRODUCTION + engine_native::cproduction_status::COUNTRY,
            COUNTRY as u64,
        );
        memory.integer(PRODUCTION + native::MAX_FACTORIES_FOR_REPAIR, 3 + phase);
        memory.word(STATE + cgame_state::STRATEGIC_NAVY_MANAGER, MANAGER as u64);
        memory.word(MANAGER, cstrategic_navy_manager::VTABLE_RVA.0 as u64);
        memory.array(MANAGER + cstrategic_navy_manager::NAVIES, LISTS + 0x200, 8);
        memory.word(LISTS + 0x200 + 7 * 8, NAVY as u64);
        memory.word(NAVY, cstrategic_navy::VTABLE_RVA.0 as u64);
        memory.word(
            NAVY + cstrategic_navy::SECONDARY,
            cstrategic_navy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        memory.integer(NAVY + cstrategic_navy::OWNER, 7);
        memory.integer(NAVY + native::MAX_REPAIR_DOCKYARDS, 5 + phase);
        memory.integer(NAVY + native::USED_REPAIR_DOCKYARDS, 2 + phase);
        memory.word(FLEET, engine_native::cfleet::VTABLE_RVA.0 as u64);
        memory.word(FLEET + engine_native::cfleet::HANDLE, FLEET_ID);
        memory.put(FLEET + native::AUTOMATED_HOMEBASE, &[(1 - phase) as u8]);
        memory.word(TASK, engine_native::ctask_force::VTABLE_RVA.0 as u64);
        memory.word(TASK + engine_native::cunit::HANDLE, TASK_ID);
        memory.integer(TASK + native::REPAIR_MODE, -2 - phase);
        memory.put(TASK + native::REPAIR_SPLIT, &[(1 - phase) as u8]);
        memory.put(TASK + native::AUTO_REINFORCEMENT, &[(1 - phase) as u8]);
        memory.word(
            TASK + native::MERGE_AFTER_REPAIR,
            29 | ((113 + phase as u64) << 32),
        );
        memory.integer(TASK + native::COMPOSITION_TOKEN, 47 + phase);
        memory.array(STATE + cgame_state::PROVINCES, LISTS + 0x300, 32);
        memory.word(LISTS + 0x300 + (17 + phase as usize) * 8, TARGET as u64);
        memory.word(TARGET, engine_native::cprovince::VTABLE_RVA.0 as u64);
        memory.integer(TARGET + engine_native::cprovince::ID, 17 + phase);
        memory.word(
            TASK + engine_native::ctask_force::REPAIR_TARGET,
            TARGET as u64,
        );
        for (offset, pointer, id) in [
            (native::REFIT_VARIANT, VARIANT, 19),
            (native::REFIT_AFTER_REPAIR, AFTER_REPAIR, 23),
        ] {
            memory.word(TASK + offset, pointer as u64);
            memory.word(
                pointer,
                engine_native::cequipment_variant::VTABLE_RVA.0 as u64,
            );
            memory.word(
                pointer + engine_native::cequipment_variant::TYPE,
                id | ((127 + phase as u64) << 32),
            );
        }
        memory.array(TASK + native::REPAIR_CHILDREN, LISTS + 0x500, 2);
        for ordinal in 0..2 {
            memory.word(
                LISTS + 0x500 + ordinal * 8,
                43 + ordinal as u64 + ((131 + phase as u64) << 32),
            );
        }
        for (kind, offset) in [native::REQUIREMENTS, native::FULFILLMENT]
            .into_iter()
            .enumerate()
        {
            let data = LISTS + 0x600 + kind * 0x100;
            memory.word(TASK + offset, native::ENTRIES_VTABLE_RVA.0 as u64);
            memory.array(TASK + offset + native::COMPOSITION_ENTRIES, data, 2);
            for ordinal in 0..2 {
                for (field, value) in [31 + phase, -3 - kind as i32, 59 + ordinal as i32, 5 + phase]
                    .into_iter()
                    .enumerate()
                {
                    memory.integer(
                        data + ordinal * native::COMPOSITION_ENTRY_BYTES + field * 4,
                        value,
                    );
                }
            }
        }
        memory.word(SHIP, engine_native::cship::VTABLE_RVA.0 as u64);
        memory.word(SHIP + engine_native::cship::HANDLE, SHIP_ID);
        memory.word(SHIP + native::SHIP_REFIT_LINE, LINE_ID);
        memory.word(
            engine_native::creference_registry::INVALID_REFERENCE_RVA.0,
            0,
        );
        memory.refit_registry();
        memory.put(
            LINE + engine_native::cproduction_line::PRODUCED,
            &(12345_i64 + i64::from(phase)).to_le_bytes(),
        );
        memory.word(BASE, engine_native_naval::cnaval_base::VTABLE_RVA.0 as u64);
        memory.integer(BASE + engine_native_naval::cnaval_base::PROVINCE_ID, 17);
        memory.integer(BASE + native::BASE_REPAIR_PRIORITY, 2 + phase);
        memory.integer(BASE + native::BASE_REPAIR_DOCKYARDS, 3 + phase);
        memory.array(BASE + native::BASE_REPAIR_QUEUE, LISTS + 0x800, 2);
        memory.word(LISTS + 0x800, SHIP_ID);
        memory.word(LISTS + 0x808, 61 | ((137 + phase as u64) << 32));
        memory
    }
    fn refit_registry(&mut self) {
        use engine_native::{creference_registry as registry, reference_bucket as bucket};
        const TABLE: usize = LISTS + 0x1000;
        const BUCKETS: usize = LISTS + 0x1100;
        self.word(
            registry::LOW_ROOTS_RVA.0 + LINE_ID as u32 as usize * 8,
            TABLE as u64,
        );
        self.word(TABLE + registry::ENTRIES, BUCKETS as u64);
        self.integer(TABLE + registry::COUNT, 1);
        self.integer(TABLE + registry::MASK, 7);
        self.put(TABLE + registry::OVERFLOW_DISTANCE, &[5]);
        self.put(BUCKETS, &[0; 14 * bucket::STRIDE]);
        self.put(BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        let hash = LINE_ID
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let cell = BUCKETS + index * bucket::STRIDE;
        self.put(cell + bucket::DISTANCE, &[1]);
        self.word(cell + bucket::TYPE, LINE_ID);
        self.word(cell + bucket::OBJECT, LINE as u64);
        self.word(
            LINE,
            engine_native::cship_refit_production_line::VTABLE_RVA.0 as u64,
        );
        self.word(
            LINE + engine_native::cproduction_line::REFERENCE_TYPE,
            LINE_ID,
        );
    }
    fn capture(&self) -> (forces::Captured, bases::Captured) {
        let mut forces = forces::Session::begin(self, 0, STATE);
        forces.visit(native::CensusVisit::Country(
            engine_native::ccountry::Country {
                pointer: COUNTRY,
                id: 7,
                canonical_slot: 7,
            },
        ));
        forces.visit(native::CensusVisit::Fleet {
            pointer: FLEET,
            handle: FLEET_ID,
        });
        forces.visit(native::CensusVisit::TaskForce {
            pointer: TASK,
            handle: TASK_ID,
        });
        forces.ship(
            SHIP,
            &engine_native::cship::Row {
                id: SHIP_ID,
                ..Default::default()
            },
        );
        let rows = engine_native::walk::Outcome::Rows { len: 1 };
        let forces = forces.finish(engine_native::cfleet::Capture {
            fleets: rows,
            task_forces: rows,
            ships: rows,
        });
        // SAFETY: tests serialize naval-management observer state with `changes::fixture::SERIAL`.
        let mut bases = unsafe { bases::Session::begin(self, 0) };
        bases.visit(
            BASE,
            &engine_native_naval::cnaval_base::Row {
                province: 17,
                ..Default::default()
            },
        );
        (forces, bases.finish(rows))
    }
}

fn expected(phase: i32) -> [TableRows; 9] {
    [
        TableRows::NavalRepairCountries(vec![NavalRepairCountryRow {
            country: 7,
            max_factories_for_repair: 3 + phase,
            max_allowed_dockyards_to_repair: 5 + phase,
            num_used_dockyards: 2 + phase,
        }]),
        TableRows::NavalRepairFleets(vec![NavalRepairFleetRow {
            fleet: FLEET_ID,
            automated_homebase: (1 - phase) as u8,
        }]),
        TableRows::NavalRepairTaskForces(vec![NavalRepairTaskForceRow {
            task_force: TASK_ID,
            repair_mode: -2 - phase,
            repair_split: (1 - phase) as u8,
            repair_target: (17 + phase) as u32,
            auto_reinforcement: (1 - phase) as u8,
            refit_variant: 19 | ((127 + phase as u64) << 32),
            refit_after_repair: 23 | ((127 + phase as u64) << 32),
            composition_token: (47 + phase) as u32,
            merge_after_repair: 29 | ((113 + phase as u64) << 32),
        }]),
        TableRows::NavalTaskForceComposition(
            (0..2)
                .flat_map(|kind| {
                    (0..2).map(move |ordinal| NavalTaskForceCompositionRow {
                        task_force: TASK_ID,
                        kind,
                        ordinal,
                        sub_unit: 31 + phase,
                        role: -3 - i32::from(kind),
                        definition: 59 + ordinal as i32,
                        amount: 5 + phase,
                    })
                })
                .collect(),
        ),
        TableRows::NavalRefitShips(vec![NavalRefitShipRow {
            ship: SHIP_ID,
            line: LINE_ID,
        }]),
        TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
            line: LINE_ID,
            progress_raw: 12345 + i64::from(phase),
        }]),
        TableRows::NavalRepairChildren(
            (0..2)
                .map(|ordinal| NavalRepairChildRow {
                    task_force: TASK_ID,
                    ordinal,
                    child: 43 + u64::from(ordinal) + ((131 + phase as u64) << 32),
                })
                .collect(),
        ),
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 17,
            priority: 2 + phase,
            allocated_dockyards: 3 + phase,
        }]),
        TableRows::NavalRepairQueue(vec![
            NavalRepairQueueRow {
                province: 17,
                ordinal: 0,
                ship: SHIP_ID,
            },
            NavalRepairQueueRow {
                province: 17,
                ordinal: 1,
                ship: 61 | ((137 + phase as u64) << 32),
            },
        ]),
    ]
}
fn assert_values(decoded: &DecodedFrame, phase: i32) {
    for (id, expected) in forces::TABLES
        .into_iter()
        .chain(bases::TABLES)
        .zip(expected(phase))
    {
        assert_eq!(
            &*decoded.tables[id.index()],
            &TableView::Complete(expected),
            "{id:?}, phase {phase}, hour {}",
            decoded.hour
        );
    }
}

fn input(hour: i32, forces: &forces::Captured, bases: &bases::Captured) -> FrameInput {
    let mut tables: [replay_frame::frame::TableInput; TABLE_COUNT] = std::array::from_fn(|_| {
        TableView::Unavailable {
            reason: "outside naval audit".into(),
        }
        .into()
    });
    forces.insert(forces::TABLES, &mut tables);
    bases.insert(bases::TABLES, &mut tables);
    FrameInput {
        branch: 1,
        hour,
        tables,
        metrics: Metrics::default(),
    }
}

#[test]
fn naval_management_populated_first_off_slot_and_post_gap_admission_preserves_every_column() {
    let _serial = changes::fixture::SERIAL.lock().unwrap();
    for hour in [1, 7, 23] {
        changes::fixture::prepare();
        let mut memory = Census::populated(0);
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let (forces, bases) = memory.capture();
        let first = encoder
            .encode_bytes(input(hour, &forces, &bases), true)
            .unwrap()
            .decode()
            .unwrap();
        assert_values(decoder.advance(&first).unwrap(), 0);
        for id in forces::TABLES.into_iter().chain(bases::TABLES) {
            assert!(
                matches!(&first.tables[id.index()], TableUpdate::Snapshot(_)),
                "{id:?}"
            );
        }
        // SAFETY: Unit test exclusively owns naval-management watch tables.
        unsafe { changes::boundary(false) };
        let (forces, bases) = memory.capture();
        let quiet = encoder
            .encode_bytes(input(hour + 1, &forces, &bases), false)
            .unwrap()
            .decode()
            .unwrap();
        assert_values(decoder.advance(&quiet).unwrap(), 0);

        memory = Census::populated(1);
        // SAFETY: Unit test exclusively owns naval-management watch tables.
        unsafe { changes::boundary(true) };
        let (forces, bases) = memory.capture();
        let recovered = encoder
            .encode_bytes(input(hour + 3, &forces, &bases), true)
            .unwrap()
            .decode()
            .unwrap();
        assert_values(decoder.advance(&recovered).unwrap(), 1);
        assert_values(FrameDecoder::new().advance(&recovered).unwrap(), 1);
        for id in forces::TABLES.into_iter().chain(bases::TABLES) {
            assert!(
                matches!(&recovered.tables[id.index()], TableUpdate::Snapshot(_)),
                "{id:?}"
            );
        }
    }
}

#[test]
fn naval_management_populated_observer_loss_refuses_then_readmits_every_column() {
    let _serial = changes::fixture::SERIAL.lock().unwrap();
    changes::fixture::prepare();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let (forces, bases) = Census::populated(0).capture();
    let first = encoder
        .encode_bytes(input(7, &forces, &bases), true)
        .unwrap()
        .decode()
        .unwrap();
    assert_values(decoder.advance(&first).unwrap(), 0);
    changes::fixture::unavailable();
    let (forces, bases) = Census::populated(1).capture();
    let refused = encoder
        .encode_bytes(input(8, &forces, &bases), false)
        .unwrap()
        .decode()
        .unwrap();
    let decoded = decoder.advance(&refused).unwrap();
    for id in forces::TABLES.into_iter().chain(bases::TABLES) {
        assert_eq!(
            &*decoded.tables[id.index()],
            &TableView::Unavailable {
                reason: changes::UNAVAILABLE.into()
            },
            "{id:?}"
        );
    }
    changes::fixture::prepare();
    let (forces, bases) = Census::populated(1).capture();
    let recovered = encoder
        .encode_bytes(input(9, &forces, &bases), true)
        .unwrap()
        .decode()
        .unwrap();
    assert_values(decoder.advance(&recovered).unwrap(), 1);
}

#[test]
fn naval_repair_bases_refused_by_a_full_table_register_at_the_next_hour() {
    const BASES: u32 = 1024;
    let _serial = changes::fixture::SERIAL.lock().unwrap();
    changes::fixture::undersized();
    let mut memory = Census::default();
    let pointer = |index: u32| BASE + index as usize * 0x100;
    for index in 0..BASES {
        memory.integer(pointer(index) + native::BASE_REPAIR_PRIORITY, 1);
        memory.integer(pointer(index) + native::BASE_REPAIR_DOCKYARDS, 2);
        memory.array(pointer(index) + native::BASE_REPAIR_QUEUE, LISTS, 0);
    }
    let capture = || {
        // SAFETY: test holds `changes::fixture::SERIAL` as the sole naval-base census caller.
        let mut session = unsafe { bases::Session::begin(&memory, 0) };
        for index in 0..BASES {
            session.visit(
                pointer(index),
                &engine_native_naval::cnaval_base::Row {
                    province: index + 1,
                    ..Default::default()
                },
            );
        }
        session.finish(engine_native::walk::Outcome::Rows { len: BASES as usize })
    };
    for table in capture().tables {
        assert_eq!(table.err(), Some("naval repair base registration refused"));
    }
    // SAFETY: Unit test exclusively owns naval-management watch tables.
    unsafe { changes::boundary(false) };
    let captured = capture();
    let TableRows::NavalRepairBases(rows) = captured.tables[0].as_ref().unwrap().as_ref() else {
        panic!("repair bases");
    };
    assert_eq!(rows.len(), BASES as usize);
    assert!(captured.tables[1].is_ok());
    changes::fixture::prepare();
}

struct Zeroed<'a>(&'a Census);
impl Memory for Zeroed<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (offset, byte) in destination.iter_mut().enumerate() {
            *byte = self.0.bytes.get(&(address + offset)).copied().unwrap_or(0);
        }
        true
    }
}

#[test]
fn naval_management_census_with_a_foreign_task_force_is_complete() {
    use engine_native::{ccountry, cfleet, cship, ctask_force, cunit};
    use engine_native_naval::cstrategic_navy;
    const FOREIGN: usize = 0xf0000;
    const FOREIGN_PRODUCTION: usize = 0xf1000;
    const FOREIGN_NAVY: usize = 0xf2000;
    const ROSTERS: usize = 0xf3000;
    let _serial = changes::fixture::SERIAL.lock().unwrap();
    changes::fixture::prepare();
    let mut census = Census::populated(0);
    census.word(LISTS + 5 * 8, FOREIGN as u64);
    census.integer(LISTS + 0x100 + 5 * 4, 5);
    census.word(FOREIGN, ccountry::VTABLE_RVA.0 as u64);
    census.integer(FOREIGN + ccountry::ID, 5);
    census.word(
        FOREIGN + ccountry::PRODUCTION_STATUS,
        FOREIGN_PRODUCTION as u64,
    );
    census.word(
        FOREIGN_PRODUCTION,
        engine_native::cproduction_status::VTABLE_RVA.0 as u64,
    );
    census.word(
        FOREIGN_PRODUCTION + engine_native::cproduction_status::COUNTRY,
        FOREIGN as u64,
    );
    census.word(LISTS + 0x200 + 5 * 8, FOREIGN_NAVY as u64);
    census.word(FOREIGN_NAVY, cstrategic_navy::VTABLE_RVA.0 as u64);
    census.word(
        FOREIGN_NAVY + cstrategic_navy::SECONDARY,
        cstrategic_navy::SECONDARY_VTABLE_RVA.0 as u64,
    );
    census.array(COUNTRY + ccountry::FLEETS, ROSTERS, 1);
    census.word(ROSTERS, FLEET as u64);
    census.word(FLEET + cfleet::OWNER, COUNTRY as u64);
    census.array(FLEET + cfleet::TASK_FORCES, ROSTERS + 0x10, 1);
    census.word(ROSTERS + 0x10, TASK as u64);
    census.word(
        TASK + ctask_force::SECONDARY_BASE,
        ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
    );
    census.word(TASK + ctask_force::FLEET, FLEET as u64);
    census.integer(TASK + cunit::ROSTER_OWNER, 5);
    census.integer(TASK + cunit::LOGICAL_OWNER, 5);
    census.array(TASK + ctask_force::SHIPS, ROSTERS + 0x20, 1);
    census.word(ROSTERS + 0x20, SHIP as u64);
    census.word(SHIP + cship::TASK_FORCE, TASK as u64);
    let memory = Zeroed(&census);
    let session = std::cell::RefCell::new(forces::Session::begin(&memory, 0, STATE));
    let (mut fleets, mut tasks, mut ships) = (Vec::new(), Vec::new(), Vec::new());
    let walked = cfleet::walk_timed_with_management(
        &memory,
        0,
        STATE,
        &mut fleets,
        &mut tasks,
        &mut ships,
        |_| 0,
        |pointer, row| session.borrow_mut().ship(pointer, row),
        &mut Vec::new(),
        |visit| session.borrow_mut().visit(visit),
    );
    let captured = session.into_inner().finish(walked.capture);
    for (id, table) in forces::TABLES.into_iter().zip(&captured.tables) {
        assert!(table.is_ok(), "{id:?}: {:?}", table.as_ref().err());
    }
    assert_eq!(
        (fleets[0].owner, tasks[0].roster_owner, tasks[0].logical_owner),
        (7, 5, 5)
    );
    assert_eq!(ships[0].task_force, TASK_ID);
    let TableRows::NavalRepairTaskForces(rows) = captured.tables[2].as_ref().unwrap().as_ref()
    else {
        panic!("task force settings");
    };
    assert_eq!(rows.iter().map(|row| row.task_force).collect::<Vec<_>>(), [TASK_ID]);
    let TableRows::NavalRepairCountries(rows) = captured.tables[0].as_ref().unwrap().as_ref()
    else {
        panic!("country settings");
    };
    assert_eq!(rows.iter().map(|row| row.country).collect::<Vec<_>>(), [5, 7]);
}
