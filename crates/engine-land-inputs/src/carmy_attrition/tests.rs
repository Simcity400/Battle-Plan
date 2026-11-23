use super::*;
use engine_native_memory::memory::SliceMemory;
use engine_native_parts::modifier_definition;
use std::cell::RefCell;

struct Fixture {
    bytes: Vec<u8>,
    blocked: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.blocked.is_some_and(|blocked| {
            (address..address.saturating_add(output.len())).contains(&blocked)
        }) {
            return false;
        }
        if address == modifier_definition::TABLE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
            return true;
        }
        if (0x3000_0000..0x3004_0000).contains(&address) {
            output.fill(0);
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

impl Fixture {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x8000],
            blocked: None,
            reads: RefCell::default(),
        }
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn modifiers(&mut self, root: usize, data: usize, values: &[(u32, i64)]) {
        self.word(root, data as u64);
        self.word(root + 8, (values.len() as u64) * 0x1_0000_0001);
        for (index, &(key, value)) in values.iter().enumerate() {
            self.put(data + index * 16, &key.to_le_bytes());
            self.word(data + index * 16 + 8, value as u64);
        }
    }
}

#[test]
fn fresh_attrition_order_uses_equipment_floor_supply_and_final_country_multiplier() {
    let mut inputs = Inputs::skipped();
    inputs.province = Ok(Some(1));
    inputs.upkeep = Ok(100);
    inputs.terrain = Ok(Some(Terrain {
        attrition: Ok(500),
        equipment: Ok(-1_000),
    }));
    inputs.state = Ok(Some(3_000));
    inputs.supply = Ok(Supply {
        penalty: Ok(50_000),
        attrition_factor: Ok(10_000),
    });
    inputs.moving_state = Ok(Some(1_000));
    inputs.weather = Ok(Some(Weather {
        acclimatization: Ok(80_000),
        attrition: Ok(2_000),
        heat: Ok(Some(Thermal {
            base: 10_000,
            country_factor: Ok(-100_000),
            leader_factor: Ok(None),
        })),
        winter: Ok(Some(Thermal {
            base: 5_000,
            country_factor: Ok(50_000),
            leader_factor: Ok(Some(25_000)),
        })),
    }));
    inputs.training_attrition = Ok(Some(5_000));
    inputs.border = Ok(Some(Border {
        state: Ok(1),
        country: Ok(1),
        factor: Ok(20_000),
    }));
    inputs.country_attrition_modifier = Ok(50_000);
    inputs.sources.fill(Err("tree unreadable"));
    assert_eq!(inputs.evaluate(Some(25_000)), Ok(44_625));
    inputs.country_attrition_modifier = Ok(-200_000);
    assert_eq!(inputs.evaluate(Some(25_000)), Ok(0));
    inputs.terrain.as_mut().unwrap().as_mut().unwrap().equipment = Err("equipment unreadable");
    assert_eq!(inputs.evaluate(Some(25_000)), Err("equipment unreadable"));
}

#[test]
fn fresh_attrition_heat_factor_fault_keeps_weather_winter_and_source_identities() {
    let mut fixture = Fixture::new();
    fixture.modifiers(
        0x1000,
        0x2000,
        &[
            (ATTRITION, 500),
            (WINTER_ATTRITION, 10_000),
            (HEAT_ATTRITION, 20_000),
        ],
    );
    fixture.modifiers(
        0x3000,
        0x4000,
        &[
            (WINTER_ATTRITION_FACTOR, 20_000),
            (HEAT_ATTRITION_FACTOR, -20_000),
        ],
    );
    fixture.blocked = Some(0x4018);
    let seed = carmy::DivisionSeed {
        orders_group: Some(0),
        ..Default::default()
    };
    let mut sources = [Ok(None); SOURCE_COUNT];
    let heat = thermal(
        &fixture,
        0,
        &seed,
        0x1000,
        Ok(0x3000),
        Ok(80_000),
        HEAT_ATTRITION,
        HEAT_ATTRITION_FACTOR,
        HEAT_SOURCE,
        &mut sources,
        &mut Memo::default(),
    )
    .unwrap()
    .unwrap();
    let winter = thermal(
        &fixture,
        0,
        &seed,
        0x1000,
        Ok(0x3000),
        Ok(80_000),
        WINTER_ATTRITION,
        WINTER_ATTRITION_FACTOR,
        WINTER_SOURCE,
        &mut sources,
        &mut Memo::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(modifier(&fixture, 0, 0x1000, ATTRITION), Ok(500));
    assert_eq!(heat.base, 16_000);
    assert!(heat.country_factor.is_err());
    assert_eq!(heat.leader_factor, Ok(None));
    assert_eq!(winter.evaluate(), Ok(9_600));
    assert_eq!(sources[HEAT_SOURCE].unwrap().unwrap().accumulator, 0x1000);
    assert_eq!(
        sources[HEAT_COUNTRY_SOURCE].unwrap().unwrap().accumulator,
        0x3000
    );
    assert_eq!(sources[HEAT_LEADER_SOURCE], Ok(None));
    fixture.blocked = Some(0x2028);
    let mut unknown_sources = [Ok(None); SOURCE_COUNT];
    assert!(
        thermal(
            &fixture,
            0,
            &seed,
            0x1000,
            Ok(0x3000),
            Ok(80_000),
            HEAT_ATTRITION,
            HEAT_ATTRITION_FACTOR,
            HEAT_SOURCE,
            &mut unknown_sources,
            &mut Memo::default()
        )
        .is_err()
    );
    assert!(
        unknown_sources[HEAT_SOURCE..=HEAT_LEADER_SOURCE]
            .iter()
            .all(Result::is_err)
    );
}

#[test]
fn fresh_attrition_nonpositive_thermal_base_does_not_read_country_or_leader() {
    let mut fixture = Fixture::new();
    fixture.modifiers(0x1000, 0x2000, &[(HEAT_ATTRITION, -20_000)]);
    let seed = carmy::DivisionSeed::default();
    let mut sources = [Ok(None); SOURCE_COUNT];
    assert_eq!(
        thermal(
            &fixture,
            0,
            &seed,
            0x1000,
            Err("country unavailable"),
            Ok(80_000),
            HEAT_ATTRITION,
            HEAT_ATTRITION_FACTOR,
            HEAT_SOURCE,
            &mut sources,
            &mut Memo::default()
        ),
        Ok(None)
    );
    assert!(sources.iter().all(|source| *source == Ok(None)));
    assert!(
        !fixture
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == seed.pointer)
    );
}

#[test]
fn fresh_attrition_zero_supply_penalty_and_missing_label_do_not_blank_total() {
    let mut inputs = Inputs::skipped();
    inputs.province = Ok(Some(1));
    inputs.upkeep = Ok(700);
    inputs.supply = Ok(Supply {
        penalty: Ok(0),
        attrition_factor: Err("define unreadable"),
    });
    assert_eq!(inputs.evaluate(None), Ok(700));
    assert_eq!(template_ratio(&Fixture::new(), 0, 0), Ok(0));
}

#[test]
fn fresh_attrition_acclimatization_keeps_first_maximum_and_refuses_foreign_embedded_data() {
    let mut fixture = Fixture::new();
    fixture.word(0x2000, 0x3000);
    fixture.word(0x2008, 50_000);
    fixture.word(0x2010, 0x3100);
    fixture.word(0x2018, 50_000);
    fixture.word(0x3030, 80_000);
    let mut header = [0; 0x40];
    header[..8].copy_from_slice(&(engine_native_parts::acclimatization_data::VTABLE_RVA.0 as u64).to_le_bytes());
    header[8..16].copy_from_slice(&0x2000_u64.to_le_bytes());
    header[16..24].copy_from_slice(&0x2_0000_0002_u64.to_le_bytes());
    header[0x20..0x28].copy_from_slice(&0x3000_u64.to_le_bytes());
    header[0x38..0x40].copy_from_slice(&100_000_i64.to_le_bytes());
    let mut seed = carmy::DivisionSeed {
        acclimatization: Some(header),
        counter_attrition: Some(123),
        ..Default::default()
    };
    assert_eq!(fresh_acclimatization(&fixture, 0, &seed), Ok(60_000));
    header[0x20..0x28].copy_from_slice(&0x3100_u64.to_le_bytes());
    seed.acclimatization = Some(header);
    assert_eq!(fresh_acclimatization(&fixture, 0, &seed), Ok(ONE));
    header[..8].fill(0);
    seed.acclimatization = Some(header);
    assert_eq!(
        fresh_acclimatization(&fixture, 0, &seed),
        Err("attrition acclimatization identity mismatch")
    );
}

#[test]
fn attrition_memo_shares_keyed_reads_and_failures_between_divisions() {
    let mut fixture = Fixture::new();
    fixture.modifiers(
        0x1000,
        0x2000,
        &[
            (ATTRITION, 500),
            (WINTER_ATTRITION, 10_000),
            (HEAT_ATTRITION, 20_000),
        ],
    );
    fixture.modifiers(
        0x3000,
        0x4000,
        &[
            (WINTER_ATTRITION_FACTOR, 20_000),
            (HEAT_ATTRITION_FACTOR, -20_000),
        ],
    );
    fixture.blocked = Some(0x4018);
    let seed = carmy::DivisionSeed {
        orders_group: Some(0),
        ..Default::default()
    };
    let run = |memo: &mut Memo| {
        let mut sources = [Ok(None); SOURCE_COUNT];
        let heat = thermal(
            &fixture,
            0,
            &seed,
            0x1000,
            Ok(0x3000),
            Ok(80_000),
            HEAT_ATTRITION,
            HEAT_ATTRITION_FACTOR,
            HEAT_SOURCE,
            &mut sources,
            memo,
        );
        let winter = thermal(
            &fixture,
            0,
            &seed,
            0x1000,
            Ok(0x3000),
            Ok(80_000),
            WINTER_ATTRITION,
            WINTER_ATTRITION_FACTOR,
            WINTER_SOURCE,
            &mut sources,
            memo,
        );
        let attrition = memo.modifier(&fixture, 0, 0x1000, ATTRITION);
        (heat, winter, attrition, sources)
    };
    let reads = || fixture.reads.borrow().len();
    let direct = run(&mut Memo::default());
    assert!(direct.0.unwrap().unwrap().country_factor.is_err());
    let direct_reads = reads();
    assert_eq!(run(&mut Memo::default()), direct);
    assert_eq!(
        reads(),
        2 * direct_reads,
        "a disabled memo reads every time"
    );
    fixture.reads.borrow_mut().clear();
    let mut memo = Memo::new();
    assert_eq!(run(&mut memo), direct);
    assert_eq!(reads(), direct_reads);
    assert_eq!(run(&mut memo), direct);
    assert_eq!(
        reads(),
        direct_reads,
        "the second division read memoized operands"
    );
    assert_eq!(memo.counts(), (10, 5));
    memo.clear();
    assert_eq!(memo.counts(), (0, 0));
    assert_eq!(run(&mut memo), direct);
    assert_eq!(reads(), 2 * direct_reads, "a cleared memo reads again");
}
