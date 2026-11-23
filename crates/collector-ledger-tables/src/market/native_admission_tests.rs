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
const COUNTRY_STRIDE: usize = 0x3000;
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
            engine_native::cequipment_variant::VTABLE_RVA.0 as u64,
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
            engine_native::cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        self.array(
            at + engine_native::cequipment_variant_pool::ITEMS,
            entries,
            1,
        );
        self.u64(entries, VARIANT as u64);
        self.u64(entries + 8, amount as u64);
    }
    fn country(
        &mut self,
        id: u32,
        pointer: usize,
        slot: u32,
        surrender_target: u32,
    ) -> ccountry::Country {
        let status = pointer + 0x1800;
        let stock = pointer + 0x1900;
        let resources = pointer + 0x1a00;
        self.u64(pointer, ccountry::VTABLE_RVA.0 as u64);
        self.u32(pointer + ccountry::ID, id);
        self.u32(pointer + ccountry::SURRENDER_TARGET, surrender_target);
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
            12345 + u64::from(surrender_target),
        );
        self.bytes[status + market::AUTOMATION] = 1;
        ccountry::Country {
            pointer,
            id,
            canonical_slot: slot,
        }
    }
    fn country_price(&mut self, pointer: usize, level: u32) {
        let head = pointer + 0x1c00;
        let node = head + 0x40;
        self.u64(pointer + 0x1800 + market::COUNTRY_PRICE_LEVELS + 8, 1);
        for offset in [0, 8, 16] {
            self.u64(head + offset, node as u64);
            self.u64(node + offset, head as u64);
        }
        self.u64(node + market::PRICE_VARIANT, VARIANT_ID);
        self.u32(node + market::PRICE_LEVEL, level);
    }
    fn contract(&mut self, at: usize, n: u32) {
        self.bytes[at..at + 0x400].fill(0);
        self.u64(at, contract::VTABLE_RVA.0 as u64);
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
        self.u64(store, engine_scripts::cvariables::VTABLE_RVA.0 as u64);
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
        if at == cgame_state::SINGLETON_RVA.0 && out.len() == 8 {
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
    unsafe { producer::boundary(
        Some(cgame_state::Clock {
            hour,
            ..Default::default()
        }),
        false,
    ) };
}
fn record(
    memory: &impl Memory,
    hour: i32,
    encoder: &mut FrameEncoder,
    decoder: &mut FrameDecoder,
) -> DecodedFrame {
    let mut pack = Captured::vacant();
    unsafe { pack.capture(memory, 0, GAME, hour, false) };
    assert!(
        pack.outcomes()[..3]
            .iter()
            .all(|o| matches!(o, Outcome::Rows { .. })),
        "hour {hour}: {:?}",
        pack.outcomes()
    );
    let decoded = encode_capture(pack, hour, encoder, decoder);
    for table in &TABLES[..3] {
        let TableView::Complete(rows) = &*decoded.tables[table.index()] else {
            panic!("{table:?} unavailable");
        };
        for row in rows.iter() {
            assert_eq!(row.not_recorded_fields(), 0);
            row.visit_nested_field_gaps(&mut |f| panic!("missing {f}"));
        }
    }
    decoded
}
fn encode_capture(
    pack: Captured,
    hour: i32,
    encoder: &mut FrameEncoder,
    decoder: &mut FrameDecoder,
) -> DecodedFrame {
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
    replay_capture_spool::spool::encode_record(&replay_capture_spool::spool::Record::Frame(frame.clone())).unwrap();
    let decoded = decoder.apply(&frame).unwrap();
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
    let session = unsafe { producer::test_support::Session::new() };
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
            .all(|r| r.inventory.items[0].amount_raw == 700_000 && r.auto_send_market_access)
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
            .auto_send_market_access
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
    assert!(row.auto_send_market_access);
    boundary(103);
    assert!(!held::full_publication_required());
    record(&NoReads, 103, &mut encoder, &mut decoder);
}

#[test]
fn active_removal_precedes_retired_cleanup_variable_marks_and_new_contract_admission() {
    let session = unsafe { producer::test_support::Session::new() };
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
        let _session = unsafe { producer::test_support::Session::new() };
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
        unsafe { pack.capture(&Failure(&memory), 0, GAME, 101, false) };
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
    let session = unsafe { producer::test_support::Session::new() };
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

#[test]
fn an_unobserved_variable_change_rereads_every_contract_and_keeps_its_terms() {
    let _session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    let frame = record(&memory, 100, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::MarketContractTerms>().unwrap();
    assert_eq!(rows[0].variable_state, 17);
    for (hour, seed) in [(101, 23_u32), (103, 29)] {
        memory.clock(hour);
        producer::variables_uncertified();
        memory.variables(CONTRACT, seed);
        boundary(hour);
        let frame = record(&memory, hour, &mut encoder, &mut decoder);
        let rows = frame.complete::<tables::MarketContractTerms>().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].variable_state, u64::from(seed));
        assert_eq!(rows[0].variables[0].amount_raw, i64::from(seed) * 100_000);
        memory.clock(hour + 1);
        boundary(hour + 1);
        assert!(!held::full_publication_required());
        record(&NoReads, hour + 1, &mut encoder, &mut decoder);
    }
}

#[test]
fn surrender_target_changes_do_not_change_inventory_or_marked_country_identity() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    let baseline = record(&memory, 100, &mut encoder, &mut decoder);
    memory.forbidden.extend([0x100c..0x1010, 0x300c..0x3010]);
    for (index, target) in [77, 0, 5000, u32::MAX, 19, 0, 31, 0]
        .into_iter()
        .enumerate()
    {
        let hour = 101 + index as i32;
        memory.clock(hour);
        memory.u32(0x300c, target);
        if index % 4 == 0 {
            session.inventory(&memory, 5000);
        } else {
            session.country_domain(5000, index % 4 - 1);
        }
        boundary(hour);
        memory.start();
        let frame = record(&memory, hour, &mut encoder, &mut decoder);
        assert_eq!(
            frame.complete::<tables::CountryMarket>().unwrap(),
            baseline.complete::<tables::CountryMarket>().unwrap()
        );
    }
    boundary(109);
    record(&NoReads, 109, &mut encoder, &mut decoder);
}

#[test]
fn authenticated_census_replacement_rebinds_holdings_without_a_reset_occurrence() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    memory.clock(101);
    let replacement = memory.country(5000, 0x5000, 1, 2);
    memory.u64(0x5000 + 0x1b00 + 8, 9_900_000);
    session.inventory(&memory, 5000);
    session.automation(5000);
    memory.u64(CONTRACT + contract::EQUIPMENT_PROGRESS, 50_000);
    session.contract_delivery(&memory, CONTRACT);
    boundary(101);
    held::discover_country(&memory, 0, GAME, replacement, 0);
    memory.forbidden.push(0x3000..0x4d00);
    let frame = record(&memory, 101, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::CountryMarket>().unwrap();
    assert_eq!(rows.len(), 2);
    let row = rows.iter().find(|r| r.country == 5000).unwrap();
    assert_eq!(row.inventory.items[0].amount_raw, 9_900_000);
    assert_eq!(row.subsidy_total_raw, 12347);
    assert_eq!(
        frame
            .complete::<tables::MarketContractTerms>()
            .unwrap()
            .len(),
        1
    );
    let events = frame.complete::<tables::MarketEvents>().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].kind, events[0].amount_raw),
        (producer::CONTRACT_DELIVERED, 1_000_000)
    );
    boundary(102);
    record(&NoReads, 102, &mut encoder, &mut decoder);
}

struct Missing<'a> {
    native: &'a Native,
    range: std::ops::Range<usize>,
}
impl Memory for Missing<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if at < self.range.end && at + out.len() > self.range.start {
            return false;
        }
        self.native.copy(at, out)
    }
}

#[test]
fn each_country_read_refusal_recovers_first_exact_hour_and_keeps_contract_totals() {
    for domain in 0..4 {
        let session = unsafe { producer::test_support::Session::new() };
        let mut memory = Native::new();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&TABLES);
        memory.clock(100);
        boundary(100);
        memory.start();
        record(&memory, 100, &mut encoder, &mut decoder);
        for hour in 101..=103 {
            memory.clock(hour);
            memory.u64(0x3000 + 0x1b00 + 8, hour as u64 * 100_000);
            memory.u64(0x4800 + market::COUNTRY_SUBSIDY_TOTAL, hour as u64);
            memory.u64(
                CONTRACT + contract::EQUIPMENT_PROGRESS,
                (hour - 100) as u64 * 25_000,
            );
            session.contract_delivery(&memory, CONTRACT);
            if hour == 101 {
                if domain == 0 {
                    session.inventory(&memory, 5000);
                } else {
                    session.country_domain(5000, domain - 1);
                }
            }
            boundary(hour);
            memory.start();
            let mut pack = Captured::vacant();
            let fault = Missing {
                native: &memory,
                range: 0x3000..0x3008,
            };
            if hour < 103 {
                unsafe { pack.capture(&fault, 0, GAME, hour, false) };
            } else {
                unsafe { pack.capture(&memory, 0, GAME, hour, false) };
            }
            assert_eq!(
                matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
                hour < 103
            );
            assert!(matches!(pack.outcomes()[1], Outcome::Rows { .. }));
            assert!(matches!(pack.outcomes()[3], Outcome::Rows { .. }));
            let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
            if hour < 103 {
                assert!(matches!(
                    &*frame.tables[TableId::CountryMarket.index()],
                    TableView::Unavailable { .. }
                ));
            } else {
                let rows = frame.complete::<tables::CountryMarket>().unwrap();
                assert_eq!(rows.len(), 2);
                let row = rows.iter().find(|r| r.country == 5000).unwrap();
                assert_eq!(row.inventory.items[0].amount_raw, 10_300_000);
                assert_eq!(row.subsidy_total_raw, 103);
            }
            let events = frame.complete::<tables::MarketEvents>().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(
                (events[0].kind, events[0].amount_raw),
                (producer::CONTRACT_DELIVERED, 500_000)
            );
        }
        boundary(104);
        assert!(!held::full_publication_required());
        record(&NoReads, 104, &mut encoder, &mut decoder);
    }
}

#[test]
fn unreadable_reset_holder_is_retained_for_first_exact_rebind() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    memory.clock(101);
    memory.country(5000, 0x3000, 1, 2);
    session.reset_country(&memory, 5000);
    boundary(101);
    let mut pack = Captured::vacant();
    unsafe { pack.capture(
        &Missing {
            native: &memory,
            range: 0x4800..0x4808,
        },
        0,
        GAME,
        101,
        false,
    ) };
    assert!(matches!(pack.outcomes()[0], Outcome::Unavailable { .. }));
    encode_capture(pack, 101, &mut encoder, &mut decoder);
    memory.clock(102);
    boundary(102);
    memory.start();
    let frame = record(&memory, 102, &mut encoder, &mut decoder);
    let row = frame
        .complete::<tables::CountryMarket>()
        .unwrap()
        .iter()
        .find(|r| r.country == 5000)
        .unwrap();
    assert_eq!(row.subsidy_total_raw, 12347);
    assert_eq!(row.inventory.items[0].amount_raw, 700_000);
    boundary(103);
    record(&NoReads, 103, &mut encoder, &mut decoder);
}

#[test]
fn country_identity_validation_stays_strict_and_recovers_after_each_bad_holder() {
    for field in [
        0x3000,
        0x3000 + ccountry::ID,
        0x4800 + market::COUNTRY_BACKLINK,
        0x4800 + engine_production::cequipment_market_status::MANAGER,
    ] {
        let session = unsafe { producer::test_support::Session::new() };
        let mut memory = Native::new();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&TABLES);
        memory.clock(100);
        boundary(100);
        memory.start();
        record(&memory, 100, &mut encoder, &mut decoder);
        let previous = memory.bytes[field..field + 8].to_vec();
        memory.u64(field, 123);
        memory.clock(101);
        session.inventory(&memory, 5000);
        boundary(101);
        let mut pack = Captured::vacant();
        unsafe { pack.capture(&memory, 0, GAME, 101, false) };
        assert!(matches!(pack.outcomes()[0], Outcome::Unavailable { .. }));
        encode_capture(pack, 101, &mut encoder, &mut decoder);
        memory.bytes[field..field + 8].copy_from_slice(&previous);
        memory.clock(102);
        boundary(102);
        memory.start();
        let frame = record(&memory, 102, &mut encoder, &mut decoder);
        assert_eq!(frame.complete::<tables::CountryMarket>().unwrap().len(), 2);
    }
}

#[test]
fn country_recovery_rebinds_a_fresh_market_manager_from_the_existing_census() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    const NEXT_MANAGER: usize = 0xa000;
    memory.u64(GAME + cgame_state::MARKET_MANAGER, NEXT_MANAGER as u64);
    for status in [0x2800, 0x4800] {
        memory.u64(
            status + engine_production::cequipment_market_status::MANAGER,
            NEXT_MANAGER as u64,
        );
    }
    memory.clock(101);
    session.automation(5000);
    boundary(101);
    let mut pack = Captured::vacant();
    unsafe { pack.capture(&memory, 0, GAME, 101, false) };
    assert!(matches!(pack.outcomes()[0], Outcome::Unavailable { .. }));
    encode_capture(pack, 101, &mut encoder, &mut decoder);
    for hour in 102..=104 {
        memory.clock(hour);
        boundary(hour);
        memory.reads.borrow_mut().clear();
        memory.start();
        let mut pack = Captured::vacant();
        if hour < 104 {
            unsafe { pack.capture(
                &Missing {
                    native: &memory,
                    range: 0x4900 + market::STOCKPILE_EQUIPMENT
                        ..0x4900 + market::STOCKPILE_EQUIPMENT + 8,
                },
                0,
                GAME,
                hour,
                false,
            ) };
        } else {
            unsafe { pack.capture(&memory, 0, GAME, hour, false) };
        }
        assert_eq!(
            matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
            hour < 104
        );
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| { at == 0x1000 + ccountry::EQUIPMENT_MARKET_STATUS })
                .count(),
            usize::from(hour == 102)
        );
        let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
        if hour == 104 {
            assert_eq!(frame.complete::<tables::CountryMarket>().unwrap().len(), 2);
        }
        assert_eq!(
            frame
                .complete::<tables::MarketContractTerms>()
                .unwrap()
                .len(),
            1
        );
    }
    memory.clock(105);
    session.automation(0);
    boundary(105);
    record(&memory, 105, &mut encoder, &mut decoder);
    boundary(106);
    record(&NoReads, 106, &mut encoder, &mut decoder);
}

#[test]
fn country_market_writer_rebinding_has_no_country_count_or_id_cap() {
    let session = unsafe { producer::test_support::Session::new() };
    // SAFETY: `session` serializes market-change globals with no live hooks.
    unsafe { producer::presize(1500, 1500) };
    let mut memory = Native::new();
    memory.bytes.resize(0x20000 + 1498 * COUNTRY_STRIDE, 0);
    let holders: Vec<_> = (0..1498)
        .map(|index| {
            memory.country(
                10_000 + index,
                0x20000 + index as usize * COUNTRY_STRIDE,
                index + 2,
                0,
            )
        })
        .collect();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    for country in &holders {
        held::discover_country(&memory, 0, GAME, *country, 0);
    }
    held::finish_seed(true);
    record(&memory, 100, &mut encoder, &mut decoder);
    memory.clock(101);
    for country in &holders {
        memory.u32(country.pointer + ccountry::SURRENDER_TARGET, 5000);
        session.inventory(&memory, country.id);
        session.automation(country.id);
    }
    boundary(101);
    let frame = record(&memory, 101, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::CountryMarket>().unwrap();
    assert_eq!(rows.len(), 1500);
    assert!(
        rows.iter()
            .all(|row| row.inventory.items[0].amount_raw == 700_000)
    );
}

#[test]
fn failed_initial_country_seed_recovers_without_publishing_a_default_row() {
    let _session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    let fault = Missing {
        native: &memory,
        range: 0x4800..0x4808,
    };
    for (id, canonical_slot, pointer) in [(0, 0, 0x1000), (5000, 1, 0x3000)] {
        held::discover_country(
            &fault,
            0,
            GAME,
            ccountry::Country {
                id,
                canonical_slot,
                pointer,
            },
            0,
        );
    }
    held::finish_seed(true);
    held::recover_contracts(&memory, 0, GAME);
    let mut pack = Captured::vacant();
    unsafe { pack.capture(&fault, 0, GAME, 100, false) };
    assert!(matches!(pack.outcomes()[0], Outcome::Unavailable { .. }));
    encode_capture(pack, 100, &mut encoder, &mut decoder);
    memory.clock(101);
    boundary(101);
    memory.start();
    let frame = record(&memory, 101, &mut encoder, &mut decoder);
    let rows = frame.complete::<tables::CountryMarket>().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row.inventory.items[0].amount_raw == 700_000 && row.subsidy_total_raw == 12346
    }));
    boundary(102);
    record(&NoReads, 102, &mut encoder, &mut decoder);
}

#[test]
fn persistent_country_failure_retries_only_its_holder_and_preserves_healthy_writer_marks_at_scale()
{
    let session = unsafe { producer::test_support::Session::new() };
    // SAFETY: `session` serializes market-change globals with no live hooks.
    unsafe { producer::presize(1500, 1500) };
    let mut memory = Native::new();
    memory.bytes.resize(0x20000 + 1498 * COUNTRY_STRIDE, 0);
    let holders: Vec<_> = (0..1498)
        .map(|index| {
            memory.country(
                10_000 + index,
                0x20000 + index as usize * COUNTRY_STRIDE,
                index + 2,
                0,
            )
        })
        .collect();
    memory.country_price(holders[2].pointer, 1);
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    for country in &holders {
        held::discover_country(&memory, 0, GAME, *country, 0);
    }
    held::finish_seed(true);
    let baseline = record(&memory, 100, &mut encoder, &mut decoder);
    let baseline = baseline.complete::<tables::CountryMarket>().unwrap();
    assert_eq!(baseline.len(), 1500);
    let mut expected = baseline.to_vec();
    memory
        .forbidden
        .push(0x4800 + market::COUNTRY_PRICE_LEVELS..0x4800 + market::AUTOMATION + 1);
    for range in [
        0..8,
        market::COUNTRY_PRICE_LEVELS..market::COUNTRY_SUBSIDY_TOTAL,
        market::AUTOMATION..market::AUTOMATION + 1,
    ] {
        memory.forbidden.push(
            holders[0].pointer + 0x1800 + range.start..holders[0].pointer + 0x1800 + range.end,
        );
    }
    for (country, range) in [
        (holders[1], 0..8),
        (holders[1], market::COUNTRY_PRICE_LEVELS..market::AUTOMATION),
        (holders[2], 0..8),
        (
            holders[2],
            market::COUNTRY_SUBSIDY_TOTAL..market::AUTOMATION + 1,
        ),
    ] {
        memory
            .forbidden
            .push(country.pointer + 0x1800 + range.start..country.pointer + 0x1800 + range.end);
    }
    for hour in 101..=105 {
        memory.clock(hour);
        memory.u64(0x1000 + 0x1b00 + 8, hour as u64 * 100_000);
        memory.u64(0x3000 + 0x1b00 + 8, hour as u64 * 200_000);
        memory.u64(
            holders[0].pointer + 0x1800 + market::COUNTRY_SUBSIDY_TOTAL,
            hour as u64,
        );
        memory.bytes[holders[1].pointer + 0x1800 + market::AUTOMATION] = (hour % 2) as u8;
        memory.country_price(holders[2].pointer, (hour % 3) as u32);
        if hour == 101 {
            session.inventory(&memory, 0);
        }
        session.inventory(&memory, 5000);
        session.country_domain(holders[0].id, producer::SUBSIDIES);
        session.automation(holders[1].id);
        session.country_domain(holders[2].id, producer::PRICES);
        memory.u64(
            CONTRACT + contract::EQUIPMENT_PROGRESS,
            (hour - 100) as u64 * 25_000,
        );
        session.contract_delivery(&memory, CONTRACT);
        boundary(hour);
        memory.reads.borrow_mut().clear();
        memory.start();
        for country in &holders {
            held::discover_country(&memory, 0, GAME, *country, 0);
        }
        held::finish_seed(true);
        let mut pack = Captured::vacant();
        if hour < 105 {
            unsafe { pack.capture(
                &Missing {
                    native: &memory,
                    range: 0x2900 + market::STOCKPILE_EQUIPMENT
                        ..0x2900 + market::STOCKPILE_EQUIPMENT + 8,
                },
                0,
                GAME,
                hour,
                false,
            ) };
        } else {
            unsafe { pack.capture(&memory, 0, GAME, hour, false) };
        }
        assert_eq!(
            matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
            hour < 105,
        );
        {
            let changes = pack.changes.lock().unwrap();
            let changes = changes.as_ref().unwrap();
            assert!(!changes.full);
            if hour < 105 {
                assert_eq!(changes.countries.len(), 4);
            } else {
                assert!(changes.countries_full);
                assert_eq!(changes.countries.len(), 1500);
            }
        }
        let roots: Vec<_> = memory
            .reads
            .borrow()
            .iter()
            .filter_map(|&(at, len)| {
                (len == 8
                    && [
                        0x1000,
                        0x3000,
                        holders[0].pointer,
                        holders[1].pointer,
                        holders[2].pointer,
                    ]
                    .into_iter()
                    .any(|pointer| at == pointer + ccountry::EQUIPMENT_MARKET_STATUS))
                .then_some(at)
            })
            .collect();
        assert_eq!(roots.len(), 5, "hour {hour}: {roots:?}");
        for pointer in [
            0x1000,
            0x3000,
            holders[0].pointer,
            holders[1].pointer,
            holders[2].pointer,
        ] {
            assert_eq!(
                roots
                    .iter()
                    .filter(|&&at| at == pointer + ccountry::EQUIPMENT_MARKET_STATUS)
                    .count(),
                1
            );
        }
        for country in &holders[3..] {
            assert!(
                memory.reads.borrow().iter().all(|&(at, len)| {
                    at + len <= country.pointer || at >= country.pointer + COUNTRY_STRIDE
                }),
                "quiet country {} reread at hour {hour}",
                country.id
            );
        }
        for row in &mut expected {
            if row.country == 0 || row.country == 5000 {
                row.inventory.items[0].amount_raw =
                    i64::from(hour) * if row.country == 0 { 100_000 } else { 200_000 };
            } else if row.country == holders[0].id {
                row.subsidy_total_raw = i64::from(hour);
            } else if row.country == holders[1].id {
                row.auto_send_market_access = hour % 2 != 0;
            } else if row.country == holders[2].id {
                row.prices[0].level = hour % 3;
            }
        }
        {
            let changes = pack.changes.lock().unwrap();
            let changes = changes.as_ref().unwrap();
            for owner in [5000, holders[0].id, holders[1].id, holders[2].id] {
                assert_eq!(
                    changes.countries.iter().find(|row| row.country == owner),
                    expected.iter().find(|row| row.country == owner),
                    "healthy writer mark at hour {hour}",
                );
            }
        }
        let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
        if hour < 105 {
            assert!(matches!(
                &*frame.tables[TableId::CountryMarket.index()],
                TableView::Unavailable { .. }
            ));
        } else {
            assert_eq!(frame.complete::<tables::CountryMarket>().unwrap(), expected);
        }
        let events = frame.complete::<tables::MarketEvents>().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            (events[0].kind, events[0].amount_raw),
            (producer::CONTRACT_DELIVERED, 500_000),
        );
    }
    boundary(106);
    record(&NoReads, 106, &mut encoder, &mut decoder);
}

#[test]
fn country_witness_recovery_recertifies_once_and_keeps_recovered_holders() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    for hour in 101..=104 {
        memory.clock(hour);
        memory.u64(0x1000 + 0x1b00 + 8, hour as u64 * 100_000);
        if hour <= 102 {
            memory.u64(0x3000 + 0x1b00 + 8, hour as u64 * 200_000);
            memory.u64(0x4800 + market::COUNTRY_SUBSIDY_TOTAL, hour as u64);
        }
        if hour == 101 {
            session.inventory(&memory, 0);
        }
        if hour == 103 {
            memory.forbidden.push(0x3000..0x5000);
        }
        let running = (hour == 101).then(|| session.country_domain_running(producer::PRICES));
        boundary(hour);
        memory.start();
        memory.reads.borrow_mut().clear();
        let mut pack = Captured::vacant();
        if hour < 104 {
            unsafe { pack.capture(
                &Missing {
                    native: &memory,
                    range: 0x2900 + market::STOCKPILE_EQUIPMENT
                        ..0x2900 + market::STOCKPILE_EQUIPMENT + 8,
                },
                0,
                GAME,
                hour,
                false,
            ) };
        } else {
            unsafe { pack.capture(&memory, 0, GAME, hour, false) };
        }
        drop(running);
        assert_eq!(
            matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
            hour < 104
        );
        if hour >= 103 {
            assert_eq!(
                memory
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| { at == 0x1000 + ccountry::EQUIPMENT_MARKET_STATUS })
                    .count(),
                1
            );
        }
        let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
        if hour == 104 {
            let rows = frame.complete::<tables::CountryMarket>().unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].inventory.items[0].amount_raw, 10_400_000);
            assert_eq!(rows[1].inventory.items[0].amount_raw, 20_400_000);
            assert_eq!(rows[1].subsidy_total_raw, 102);
        }
    }
    boundary(105);
    record(&NoReads, 105, &mut encoder, &mut decoder);
}

#[test]
fn refused_country_registration_renews_only_once_without_reacquiring_recovered_holders() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    let owner = session.fill_country_domain(producer::PRICES);
    memory.bytes.resize(0x20000 + COUNTRY_STRIDE, 0);
    let country = memory.country(owner, 0x20000, 2, 0);
    for hour in 101..=104 {
        memory.clock(hour);
        if hour == 101 {
            session.inventory(&memory, 0);
        }
        boundary(hour);
        memory.reads.borrow_mut().clear();
        memory.start();
        held::discover_country(&memory, 0, GAME, country, 0);
        held::finish_seed(true);
        let mut pack = Captured::vacant();
        if hour < 104 {
            unsafe { pack.capture(
                &Missing {
                    native: &memory,
                    range: 0x2900 + market::STOCKPILE_EQUIPMENT
                        ..0x2900 + market::STOCKPILE_EQUIPMENT + 8,
                },
                0,
                GAME,
                hour,
                false,
            ) };
        } else {
            unsafe { pack.capture(&memory, 0, GAME, hour, false) };
        }
        assert_eq!(
            matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
            hour < 104
        );
        for pointer in [0x3000, country.pointer] {
            if hour >= 102 {
                assert_eq!(
                    memory
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(at, _)| { at == pointer + ccountry::EQUIPMENT_MARKET_STATUS })
                        .count(),
                    usize::from(hour == 102)
                );
            }
        }
        let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
        if hour == 104 {
            let rows = frame.complete::<tables::CountryMarket>().unwrap();
            assert_eq!(rows.len(), 3);
            assert!(
                rows.iter()
                    .all(|row| row.inventory.items[0].amount_raw == 700_000)
            );
        }
    }
    boundary(105);
    record(&NoReads, 105, &mut encoder, &mut decoder);
}

#[test]
fn exhausted_country_registration_retries_a_deferred_renewal_at_the_first_quiet_hour() {
    let session = unsafe { producer::test_support::Session::new() };
    let mut memory = Native::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    memory.clock(100);
    boundary(100);
    memory.start();
    record(&memory, 100, &mut encoder, &mut decoder);
    let owner = session.fill_country_domain(producer::PRICES);
    memory.bytes.resize(0x20000 + COUNTRY_STRIDE, 0);
    let country = memory.country(owner, 0x20000, 2, 0);
    for hour in 101..=103 {
        memory.clock(hour);
        memory.forbidden.clear();
        if hour == 102 {
            memory.country_price(0x1000, 2);
            memory.u64(0x2800 + market::COUNTRY_SUBSIDY_TOTAL, 102);
            session.country_domain(0, producer::PRICES);
            session.country_domain(0, producer::SUBSIDIES);
            memory.forbidden.extend([0x2900..0x2a00, 0x4900..0x4a00]);
        }
        if hour >= 102 {
            memory.u64(
                CONTRACT + contract::EQUIPMENT_PROGRESS,
                (hour - 101) as u64 * 25_000,
            );
            session.contract_delivery(&memory, CONTRACT);
        }
        let running = (hour == 102).then(|| session.country_domain_running(producer::PRICES));
        boundary(hour);
        drop(running);
        memory.start();
        held::discover_country(&memory, 0, GAME, country, 0);
        held::finish_seed(true);
        let mut pack = Captured::vacant();
        unsafe { pack.capture(&memory, 0, GAME, hour, false) };
        assert_eq!(
            matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
            hour < 103,
            "hour {hour}: {:?}",
            pack.outcomes()
        );
        if hour == 102 {
            let changes = pack.changes.lock().unwrap();
            let changes = changes.as_ref().unwrap();
            let healthy = changes
                .countries
                .iter()
                .find(|row| row.country == 0)
                .unwrap();
            assert_eq!(healthy.prices[0].level, 2);
            assert_eq!(healthy.subsidy_total_raw, 102);
            assert!(!changes.countries.iter().any(|row| row.country == owner));
        }
        let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
        if hour >= 102 {
            let events = frame.complete::<tables::MarketEvents>().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(
                (events[0].kind, events[0].amount_raw),
                (producer::CONTRACT_DELIVERED, 500_000)
            );
        }
        if hour == 103 {
            let rows = frame.complete::<tables::CountryMarket>().unwrap();
            assert_eq!(rows.len(), 3);
            assert!(rows.iter().any(|row| row.country == owner));
            assert!(
                rows.iter()
                    .all(|row| row.inventory.items[0].amount_raw == 700_000)
            );
            let healthy = rows.iter().find(|row| row.country == 0).unwrap();
            assert_eq!(healthy.prices[0].level, 2);
            assert_eq!(healthy.subsidy_total_raw, 102);
        }
    }
    boundary(104);
    record(&NoReads, 104, &mut encoder, &mut decoder);
}

#[test]
fn offered_country_growth_survives_a_draining_predecessor_and_recovers_after_collection() {
    for offer_hour in [101, 103] {
        let session = unsafe { producer::test_support::Session::new() };
        let mut memory = Native::new();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&TABLES);
        boundary(99);
        let slots = session.country_domain_capacity(producer::PRICES);
        // SAFETY: `session` serializes market-change globals with no live hooks.
        unsafe { producer::presize(16, slots / 2) };
        memory.clock(100);
        unsafe { producer::boundary(
            Some(cgame_state::Clock {
                hour: 100,
                ..Default::default()
            }),
            true,
        ) };
        session.defer_country_domain_retirement(producer::PRICES);
        memory.start();
        record(&memory, 100, &mut encoder, &mut decoder);
        let slots = session.country_domain_capacity(producer::PRICES);
        let bucket = (0..slots)
            .find(|&bucket| {
                [0_u32, 5000].into_iter().all(|owner| {
                    let slot =
                        collector_primitives::table_hash::slot((u64::from(owner) + 1) << 3, slots);
                    (slot + slots - bucket) % slots >= 16
                })
            })
            .unwrap();
        let mut owners = Vec::new();
        for (target, count) in [(bucket * 2, 16), (bucket * 2 + 1, 1)] {
            let first = owners.last().map_or(10_000, |&owner| owner + 1);
            owners.extend(
                (first..=u32::MAX)
                    .filter(|&owner| {
                        collector_primitives::table_hash::slot(
                            (u64::from(owner) + 1) << 3,
                            slots * 2,
                        ) == target
                    })
                    .take(count),
            );
        }
        memory.bytes.resize(0x20000 + owners.len() * COUNTRY_STRIDE, 0);
        let countries: Vec<_> = owners
            .into_iter()
            .enumerate()
            .map(|(index, owner)| {
                memory.country(owner, 0x20000 + index * COUNTRY_STRIDE, index as u32 + 2, 0)
            })
            .collect();
        let recovered_hour = offer_hour.max(102) + 1;
        for hour in 101..=recovered_hour {
            for domain in 0..4 {
                if (domain == producer::PRICES && hour == offer_hour)
                    || (domain != producer::PRICES && hour == 101)
                {
                    session.offer_country_domain_growth(domain, slots / 2);
                    if domain != producer::PRICES {
                        session.collect_country_domain(domain);
                    }
                }
            }
            memory.clock(hour);
            if hour == recovered_hour {
                session.collect_country_domain(producer::PRICES);
            }
            let running = (hour == 101).then(|| session.country_domain_running(producer::PRICES));
            boundary(hour);
            drop(running);
            memory.start();
            for &country in &countries {
                held::discover_country(&memory, 0, GAME, country, 0);
            }
            held::finish_seed(true);
            let mut pack = Captured::vacant();
            unsafe { pack.capture(&memory, 0, GAME, hour, false) };
            assert_eq!(
                matches!(pack.outcomes()[0], Outcome::Unavailable { .. }),
                hour < recovered_hour,
                "hour {hour}: {:?}",
                pack.outcomes()
            );
            assert!(matches!(pack.outcomes()[1], Outcome::Rows { .. }));
            assert!(matches!(pack.outcomes()[3], Outcome::Rows { .. }));
            assert_eq!(
                session.country_domain_capacity(producer::PRICES),
                slots * if hour == recovered_hour { 2 } else { 1 }
            );
            let frame = encode_capture(pack, hour, &mut encoder, &mut decoder);
            if hour == recovered_hour {
                let rows = frame.complete::<tables::CountryMarket>().unwrap();
                assert_eq!(rows.len(), countries.len() + 2);
                assert!(rows.iter().all(|row| {
                    row.inventory.items[0].amount_raw == 700_000 && row.auto_send_market_access
                }));
                for country in &countries {
                    assert!(rows.iter().any(|row| row.country == country.id));
                }
            }
        }
        boundary(recovered_hour + 1);
        record(&NoReads, recovered_hour + 1, &mut encoder, &mut decoder);
    }
}
