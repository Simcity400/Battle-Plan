//! Populated native fixtures enter through discovery, writer lifetimes and spool.
use super::*;
use collector_projection::projection::ProjectionScratch;
use engine_native::{ccountry, cgame_state};
use engine_production::{cpurchase_contract as contract, market_ledger as market};
use replay_data::rows::tables;
use replay_frame::frame::{DecodedFrame, FrameDecoder, FrameEncoder, FrameInput, Metrics};
use std::{cell::RefCell, collections::BTreeSet};

const GAME: usize = 0x100;
const MANAGER: usize = 0x8000;
const CONTRACT: usize = 0x10000;
const OTHER: usize = 0x11000;
const VARIANT: usize = 0x1e000;
const VARIANT_ID: u64 = (55_u64 << 32) | 70;
fn id(n: u32) -> u64 {
    (u64::from(n) << 32) | u64::from(contract::TYPE)
}
struct Native {
    bytes: Vec<u8>,
    forbidden: Vec<std::ops::Range<usize>>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Native {
    fn new() -> Self {
        let mut s = Self {
            bytes: vec![0; 0x20000],
            forbidden: Vec::new(),
            reads: RefCell::new(Vec::new()),
        };
        s.u64(GAME + cgame_state::MARKET_MANAGER, MANAGER as u64);
        s.u64(
            VARIANT,
            engine_native::cequipment_variant::VTABLE_RVA as u64,
        );
        s.u64(VARIANT + 8, VARIANT_ID);
        s.country(0, 0x1000, 0, 1);
        s.country(5000, 0x3000, 1, 1);
        s.contract(CONTRACT, 17);
        s.array(MANAGER, 0x9000, 1);
        s.u64(0x9000, CONTRACT as u64);
        s
    }
    fn u64(&mut self, at: usize, v: u64) {
        self.bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn tree(&mut self, at: usize, head: usize) {
        self.u64(at, head as u64);
        for offset in [0, 8, 16] {
            self.u64(head + offset, head as u64);
        }
        self.bytes[head + 25] = 1;
    }
    fn pool(&mut self, at: usize, entries: usize, amount: i64) {
        self.u64(
            at,
            engine_native::cequipment_variant_pool::VTABLE_RVA as u64,
        );
        self.array(
            at + engine_native::cequipment_variant_pool::ITEMS,
            entries,
            1,
        );
        self.u64(entries, VARIANT as u64);
        self.u64(entries + 8, amount as u64);
    }
    fn country(&mut self, id: u32, pointer: usize, slot: u32, instance: u32) -> ccountry::Country {
        let status = pointer + 0x1800;
        let stock = pointer + 0x1900;
        let resources = pointer + 0x1a00;
        self.u32(pointer + ccountry::ID, id);
        self.u32(pointer + ccountry::ID_INSTANCE, instance);
        self.u64(pointer + ccountry::EQUIPMENT_MARKET_STATUS, status as u64);
        self.u64(pointer + ccountry::RESOURCES, resources as u64);
        self.u64(
            status + engine_production::cequipment_market_status::MANAGER,
            MANAGER as u64,
        );
        self.u64(status + market::COUNTRY_BACKLINK, pointer as u64);
        self.u64(
            status + engine_production::cequipment_market_status::STOCKPILE,
            stock as u64,
        );
        self.pool(
            stock + market::STOCKPILE_EQUIPMENT,
            pointer + 0x1b00,
            700_000,
        );
        self.tree(status + market::COUNTRY_PRICE_LEVELS, pointer + 0x1c00);
        self.u64(
            status + market::COUNTRY_SUBSIDY_TOTAL,
            12345 + u64::from(instance),
        );
        self.bytes[status + market::AUTOMATION] = 1;
        ccountry::Country {
            pointer,
            id,
            canonical_slot: slot,
        }
    }
    fn contract(&mut self, at: usize, n: u32) {
        self.bytes[at..at + 0x400].fill(0);
        self.u64(at, contract::VTABLE_RVA as u64);
        self.u64(at + contract::ID_TYPE, id(n));
        self.bytes[at + contract::REGISTERED] = 1;
        self.u32(at + contract::SELLER, 5000);
        self.u32(at + contract::BUYER, 0);
        self.u32(at + contract::SPEED, 2);
        self.pool(at + market::DRAFT, at + 0x500, 100_000);
        self.pool(
            at + market::DRAFT + market::DRAFT_EQUIPMENT,
            at + 0x520,
            2_000_000,
        );
        self.pool(at + market::CONTRACT_ROUTE_POOL, at + 0x540, 300_000);
        self.tree(at + market::DRAFT + market::DRAFT_PRICE_LEVELS, at + 0x580);
        self.bytes[at + market::DRAFT + market::DEFINITION_COST_VALID] = 1;
        self.u64(at + market::DRAFT + market::DEFINITION_TOTAL_COST, 100_000);
        self.variables(at, n);
    }
    fn variables(&mut self, at: usize, seed: u32) {
        let store = at + market::CONTRACT_VARIABLES;
        let storage = at + 0x600;
        self.u64(store, engine_scripts::cvariables::VTABLE_RVA as u64);
        self.u32(store + engine_scripts::cvariables::SEED, seed);
        self.u64(store + engine_scripts::cvariables::STORAGE, storage as u64);
        self.u32(store + engine_scripts::cvariables::ENTRY_COUNT, 1);
        self.u32(store + engine_scripts::cvariables::BUCKET_MASK, 1);
        self.bytes[store + engine_scripts::cvariables::PROBE_TAIL] = 3;
        self.bytes[storage..storage + 0x100].fill(0);
        self.bytes[storage + engine_scripts::variable_hash_node::DISTANCE] = 1;
        let key = storage + engine_scripts::variable_hash_node::KEY;
        self.bytes[key..key + 4].copy_from_slice(b"paid");
        self.u64(key + engine_native::msvc_string::LENGTH, 4);
        self.u64(key + engine_native::msvc_string::CAPACITY, 15);
        self.u64(
            storage + engine_scripts::variable_hash_node::VALUE,
            u64::from(seed) * 100_000,
        );
        self.bytes[storage
            + 5 * engine_scripts::variable_hash_node::STRIDE
            + engine_scripts::variable_hash_node::DISTANCE] = 255;
    }
    fn clock(&mut self, hour: i32) {
        self.u32(GAME + cgame_state::CURRENT_HOUR, hour as u32);
    }
    fn start(&self) {
        for (country, slot, pointer) in [(0, 0, 0x1000), (5000, 1, 0x3000)] {
            held::discover_country(
                self,
                0,
                GAME,
                ccountry::Country {
                    id: country,
                    canonical_slot: slot,
                    pointer,
                },
                0,
            );
        }
        held::finish_seed(true);
        held::recover_contracts(self, 0, GAME);
    }
}
impl Memory for Native {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, out.len()));
        assert!(
            self.forbidden
                .iter()
                .all(|r| at + out.len() <= r.start || at >= r.end),
            "removed holder read at {at:#x}"
        );
        if at == cgame_state::SINGLETON_RVA && out.len() == 8 {
            out.copy_from_slice(&(GAME as u64).to_le_bytes());
            return true;
        }
        let Some(bytes) = self.bytes.get(at..at + out.len()) else {
            return false;
        };
        out.copy_from_slice(bytes);
        true
    }
}
fn boundary(hour: i32) {
    producer::boundary(
        Some(cgame_state::Clock {
            hour,
            ..Default::default()
        }),
        false,
    );
}
fn record(
    memory: &impl Memory,
    hour: i32,
    encoder: &mut FrameEncoder,
    decoder: &mut FrameDecoder,
) -> DecodedFrame {
    let mut pack = Captured::vacant();
    pack.capture_growing(memory, 0, GAME, hour, false);
    assert!(
        pack.outcomes()[..3]
            .iter()
            .all(|o| matches!(o, Outcome::Rows { .. })),
        "hour {hour}: {:?}",
        pack.outcomes()
    );
    let scratch = ProjectionScratch::default();
    let mut projection = Projection::new(encoder, 1, hour, false, &scratch);
    let inputs = pack.table_inputs(Some(&mut projection));
    let mut frame = FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| unavailable("unrelated table").into()),
        metrics: Metrics::default(),
    };
    for (table, input) in TABLES.into_iter().zip(inputs) {
        frame.tables[table.index()] = input;
    }
    let wire = encoder.encode_bytes(frame, false).unwrap();
    let frame = wire.decode().unwrap();
    replay::spool::encode_record(&replay::spool::Record::Frame(frame.clone())).unwrap();
    let decoded = decoder.apply(&frame).unwrap();
    for table in &TABLES[..3] {
        let TableView::Complete(rows) = &*decoded.tables[table.index()] else {
            panic!("{table:?} unavailable");
        };
        for row in rows.iter() {
            assert_eq!(row.not_recorded_fields(), 0);
            row.visit_nested_field_gaps(&mut |f| panic!("missing {f}"));
        }
    }
    decoded.clone()
}
struct NoReads;
impl Memory for NoReads {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("quiet acquisition must carry without native reads");
    }
}

#[test]
fn populated_native_startup_and_zero_country_replacement_survive_production_spool() {
    let session = producer::test_support::Session::default();
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    let frame = record(&memory, 100, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::CountryMarket>().unwrap();
    assert_eq!(
        rows.iter().map(|r| r.country).collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 5000])
    );
    assert!(
        rows.iter()
            .all(|r| r.inventory.items[0].amount_raw == 700_000 && r.auto_accept_market_access)
    );
    let contracts = frame.complete::<tables::MarketContractTerms>().unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].variables[0].name, b"paid");
    assert_eq!(contracts[0].equipment.items[0].amount_raw, 2_000_000);
    memory.clock(101);
    session.automation(0);
    memory.bytes[0x2800 + market::AUTOMATION] = 0;
    boundary(101);
    let frame = record(&memory, 101, &mut encoder, &mut decoder);
    assert!(
        !frame
            .complete::<tables::CountryMarket>()
            .unwrap()
            .iter()
            .find(|r| r.country == 0)
            .unwrap()
            .auto_accept_market_access
    );
    memory.clock(102);
    let replacement = memory.country(0, 0x5000, 0, 2);
    session.reset_country(&memory, 0);
    boundary(102);
    held::discover_country(&memory, 0, GAME, replacement, 0);
    let frame = record(&memory, 102, &mut encoder, &mut decoder);
    let row = frame
        .complete::<tables::CountryMarket>()
        .unwrap()
        .iter()
        .find(|r| r.country == 0)
        .unwrap();
    assert_eq!(row.subsidy_total_raw, 12347);
    assert!(row.auto_accept_market_access);
    boundary(103);
    assert!(!held::full_publication_required());
    record(&NoReads, 103, &mut encoder, &mut decoder);
}

#[test]
fn active_removal_precedes_retired_cleanup_variable_marks_and_new_contract_admission() {
    let session = producer::test_support::Session::default();
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    memory.clock(101);
    drop(producer::variable_write(
        CONTRACT + market::CONTRACT_VARIABLES,
    ));
    memory.u64(CONTRACT + contract::EQUIPMENT_PROGRESS, 50_000);
    session.remove_contract(&memory, CONTRACT);
    memory.u64(
        CONTRACT + contract::ID_TYPE,
        (u64::from(engine_production::market_mutations::RETIRED_REFERENCE_ID) << 32) | 17,
    );
    memory.bytes[CONTRACT + contract::REGISTERED] = 0;
    session.destroy_contract(&memory, CONTRACT).unwrap();
    memory
        .forbidden
        .push(CONTRACT + market::CONTRACT_VARIABLES..CONTRACT + 0x400);
    memory.contract(OTHER, 18);
    session.create_contract(&memory, OTHER);
    memory.u64(OTHER + contract::EQUIPMENT_PROGRESS, 50_000);
    session.contract_delivery(&memory, OTHER);
    // Shared reset/variable owners notify unrelated stores too; never read them.
    drop(producer::variable_write(0xdeadbeef));
    boundary(101);
    held::seed_contract(&memory, 0, OTHER, id(18), 5000);
    held::finish_contract_seed(true);
    let frame = record(&memory, 101, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::MarketContractTerms>().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id(18));
    assert_eq!(rows[0].variable_state, 18);
    let events = frame.complete::<tables::MarketEvents>().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|r| r.kind == producer::CONTRACT_REMOVED)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|r| r.kind == producer::CONTRACT_CREATED)
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|r| r.kind == producer::DELIVERY_UNAVAILABLE)
    );
    assert_eq!(
        events
            .iter()
            .filter(|r| r.kind == producer::CONTRACT_DELIVERED)
            .map(|r| r.amount_raw)
            .collect::<Vec<_>>(),
        [1_000_000, 1_000_000]
    );
    memory.clock(102);
    session.remove_contract(&memory, OTHER);
    memory.u64(
        OTHER + contract::ID_TYPE,
        (u64::from(engine_production::market_mutations::RETIRED_REFERENCE_ID) << 32) | 18,
    );
    memory.bytes[OTHER + contract::REGISTERED] = 0;
    session.destroy_contract(&memory, OTHER).unwrap();
    memory.forbidden.clear();
    memory.contract(CONTRACT, 19);
    session.create_contract(&memory, CONTRACT);
    drop(producer::variable_write(
        CONTRACT + market::CONTRACT_VARIABLES,
    ));
    boundary(102);
    held::seed_contract(&memory, 0, CONTRACT, id(19), 5000);
    held::finish_contract_seed(true);
    let frame = record(&memory, 102, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::MarketContractTerms>().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id(19));
    assert_eq!(rows[0].variables[0].amount_raw, 1_900_000);
    boundary(103);
    assert!(!held::full_publication_required());
    record(&NoReads, 103, &mut encoder, &mut decoder);
}

#[test]
fn unobserved_birth_and_unreadable_selected_variable_holder_still_refuse() {
    for missing_birth in [true, false] {
        let _session = producer::test_support::Session::default();
        let mut memory = Native::new();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&TABLES);
        memory.clock(100);
        boundary(100);
        memory.start();
        record(&memory, 100, &mut encoder, &mut decoder);
        memory.clock(101);
        if !missing_birth {
            drop(producer::variable_write(
                CONTRACT + market::CONTRACT_VARIABLES,
            ));
        }
        boundary(101);
        if missing_birth {
            held::seed_contract(&memory, 0, OTHER, id(18), 5000);
        }
        struct Failure<'a>(&'a Native);
        impl Memory for Failure<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at == CONTRACT + contract::ID_TYPE {
                    return false;
                }
                self.0.copy(at, out)
            }
        }
        let mut pack = Captured::vacant();
        pack.capture_growing(&Failure(&memory), 0, GAME, 101, false);
        let expected = if missing_birth {
            "market new contract has no observed birth"
        } else {
            "market variable holder identity changed"
        };
        assert_eq!(
            pack.outcomes()[1],
            Outcome::Unavailable { reason: expected }
        );
        assert!(matches!(pack.outcomes()[0], Outcome::Rows { .. }));
        assert!(matches!(pack.outcomes()[3], Outcome::Rows { .. }));
    }
}

#[test]
fn daily_discovery_then_observed_birth_and_removal_needs_no_surviving_definition() {
    let session = producer::test_support::Session::default();
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    memory.clock(101);
    memory.contract(OTHER, 20);
    session.create_contract(&memory, OTHER);
    session.remove_contract(&memory, OTHER);
    boundary(101);
    // Discovery belongs to the existing census before queued lifetimes drain.
    held::seed_contract(&memory, 0, OTHER, id(20), 5000);
    memory.forbidden.push(OTHER..OTHER + 0x1000);
    let frame = record(&memory, 101, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::MarketContractTerms>().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id(17));
    let events = frame.complete::<tables::MarketEvents>().unwrap();
    assert_eq!(
        events.iter().map(|r| r.kind).collect::<Vec<_>>(),
        [producer::CONTRACT_CREATED, producer::CONTRACT_REMOVED]
    );
    assert!(events.iter().all(|r| r.agreement == id(20)));
}
