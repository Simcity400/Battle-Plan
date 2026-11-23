use super::*;
use crate::loop_history_window as window;
use std::collections::BTreeMap;

const BASE: usize = 0x0001_4000_0000;
const GAME: usize = 0x10000;
const COUNTRIES: usize = 0x11000;
const LINKS: usize = 0x11100;
const COUNTS: usize = 0x12000;
const FLAGS: usize = 0x12100;
const DATABASE: usize = 0x90000;
const LIST: usize = 0x91000;
const TYPES: usize = 0x100_0000;
const TYPE_STRIDE: usize = 0x600;
const HUMAN: (usize, u32, usize) = (0x20000, 4, 0x20_0000);
const AI: (usize, u32, usize) = (0x30000, 2, 0x30_0000);
const MANUAL: (usize, u32, usize) = (0x40000, 3, 0x40_0000);
const SLOT: u32 = 7;

#[derive(Default)]
struct Fixture(BTreeMap<usize, u8>);

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (offset, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.0.get(&(address + offset)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

impl Fixture {
    fn bytes(&mut self, address: usize, bytes: &[u8]) {
        self.0
            .extend(bytes.iter().enumerate().map(|(i, &b)| (address + i, b)));
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn i64(&mut self, address: usize, value: i64) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn pointer(&mut self, address: usize, value: usize) {
        self.bytes(address, &(value as u64).to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.pointer(address, data);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn forget(&mut self, address: usize, len: usize) {
        for address in address..address + len {
            self.0.remove(&address);
        }
    }

    fn new(archetypes: u32) -> Self {
        let mut f = Self::default();
        f.array(GAME + cgame_state::COUNTRIES, COUNTRIES, 4);
        f.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 5);
        for (slot, (country, id, _)) in [HUMAN, AI, MANUAL].into_iter().enumerate() {
            f.pointer(COUNTRIES + (slot + 1) * 8, country);
            f.pointer(country, BASE + ccountry::VTABLE_RVA.0);
            f.u32(country + ccountry::ID, id);
        }
        f.pointer(COUNTRIES, 0);
        for (index, slot) in [0_u32, 1, 2, 3, 1].into_iter().enumerate() {
            f.u32(LINKS + index * 4, slot);
        }
        f.array(GAME + cgame_state::HUMAN_STATUS, COUNTS, 4);
        f.bytes(COUNTS, &[0, 1, 0, 0]);
        f.array(GAME + cgame_state::AUTONOMOUS_STATUS, FLAGS, 4);
        f.bytes(FLAGS, &[1, 1, 1, 0]);
        f.pointer(BASE + database::SINGLETON_RVA.0, DATABASE);
        f.pointer(DATABASE, BASE + database::VTABLE_RVA.0);
        f.array(DATABASE + database::ARCHETYPES, LIST, archetypes);
        for index in 0..archetypes {
            let archetype = TYPES + index as usize * TYPE_STRIDE;
            f.pointer(LIST + index as usize * 8, archetype);
            f.pointer(archetype, BASE + cequipment_type::VTABLE_RVA.0);
            f.u32(archetype + cequipment_type::TOKEN, token(index));
            f.u32(archetype + cequipment_type::STOCKPILE_INDEX, index);
            f.bytes(archetype + cequipment_type::IS_ARCHETYPE, &[1]);
        }
        for (country, _, region) in [HUMAN, MANUAL] {
            f.logistics(country, region, archetypes);
        }
        f
    }

    fn logistics(&mut self, country: usize, region: usize, archetypes: u32) {
        let status = region;
        let series = region + 0x100;
        self.pointer(country + ccountry::LOGISTICS_STATUS, status);
        self.pointer(status, BASE + VTABLE_RVA.0);
        self.array(status + SERIES, series, SERIES_COUNT);
        self.pointer(status + SERIES + 16, 0xa110c);
        self.pointer(status + COUNTRY, country);
        for index in 0..SERIES_COUNT as usize {
            self.pointer(series + index * 8, 0xdead_0000 + index);
        }
        for (index, series_index, roll_up) in [
            (0, STOCK_SERIES, STOCK_ROLL_UP),
            (1, BALANCE_SERIES, BALANCE_ROLL_UP),
        ] {
            let history = region + 0x1000 + index * 0x1000;
            let daily = history + 0x100;
            let records = history + 0x200;
            let record = history + 0x400;
            let values = history + 0x800;
            self.pointer(series + series_index as usize * 8, history);
            self.pointer(history, BASE + cloop_history::VTABLE_RVA.0);
            self.pointer(history + cloop_history::QUEUE, 0x9_0e0e);
            self.pointer(history + cloop_history::DAILY_WINDOW + 8, 0x12);
            self.pointer(history + cloop_history::WINDOW, 0x7);
            self.pointer(history + cloop_history::DAILY_WINDOW, daily);
            self.pointer(history + cloop_history::SELF_REFERENCE, history);
            self.bytes(history + cloop_history::ROLL_UP, &roll_up.to_le_bytes());
            self.u32(history + cloop_history::FIELD_COUNT, archetypes);
            self.pointer(daily, BASE + window::VTABLE_RVA.0);
            self.array(daily + window::RECORDS, records, 30);
            self.pointer(daily + window::RECORDS + 16, 0xa110c);
            self.u32(daily + window::CAPACITY, 30);
            self.u32(daily + window::CURRENT_SLOT, SLOT);
            self.bytes(daily + window::WRAPPED, &[1, 0, 0, 0, 0, 0, 0, 0]);
            self.pointer(daily + window::RECORD_FIELDS, record);
            self.pointer(daily + window::HISTORY, history);
            for slot in 0..30 {
                self.pointer(records + slot * 8, record + 0x40 + slot);
            }
            self.pointer(records + SLOT as usize * 8, record);
            self.array(record, values, archetypes);
            self.pointer(record + 16, 0xa110c);
            self.pointer(record + record_fields::HISTORY, history);
            for archetype in 0..archetypes {
                self.i64(
                    values + archetype as usize * 8,
                    value(country, series_index, archetype),
                );
            }
        }
    }

    fn history(region: usize, series: u32) -> usize {
        region + 0x1000 + (series - STOCK_SERIES) as usize * 0x1000
    }
}

fn token(index: u32) -> u32 {
    0x5000 + index * 3
}

fn value(country: usize, series: u32, archetype: u32) -> i64 {
    let magnitude = (country as i64) << 20 | i64::from(series) << 12 | i64::from(archetype);
    match (series, archetype) {
        (BALANCE_SERIES, 0) => i64::MIN,
        (STOCK_SERIES, 0) => i64::MAX,
        (BALANCE_SERIES, archetype) if archetype % 2 == 1 => -magnitude,
        _ => magnitude,
    }
}

#[test]
fn worker_sizes_the_roster_without_reading_native_excluded_countries() {
    const COUNTRY_POINTERS: usize = 0x500_0000;
    const COUNTRY_LINKS: usize = 0x501_0000;
    const HUMAN_COUNTS: usize = 0x502_0000;
    const AUTONOMY_FLAGS: usize = 0x503_0000;
    let mut fixture = Fixture::new(3);
    fixture.array(GAME + cgame_state::COUNTRIES, COUNTRY_POINTERS, 9);
    fixture.array(GAME + cgame_state::COUNTRY_LINKS, COUNTRY_LINKS, 9);
    fixture.array(GAME + cgame_state::HUMAN_STATUS, HUMAN_COUNTS, 9);
    fixture.array(GAME + cgame_state::AUTONOMOUS_STATUS, AUTONOMY_FLAGS, 9);
    fixture.bytes(HUMAN_COUNTS, &[0; 9]);
    fixture.bytes(AUTONOMY_FLAGS, &[1; 9]);
    fixture.bytes(HUMAN_COUNTS + 1, &[1]);
    fixture.bytes(AUTONOMY_FLAGS + 3, &[0]);
    fixture.pointer(COUNTRY_POINTERS, 0);
    fixture.u32(COUNTRY_LINKS, 0);
    for slot in 1..=8 {
        let pointer = match slot {
            1 => HUMAN.0,
            2 => AI.0,
            3 => MANUAL.0,
            _ => 0x600_0000 + slot as usize * 0x2000,
        };
        fixture.pointer(COUNTRY_POINTERS + slot as usize * 8, pointer);
        fixture.u32(COUNTRY_LINKS + slot as usize * 4, slot);
        fixture.pointer(pointer, BASE + ccountry::VTABLE_RVA.0);
        fixture.u32(pointer + ccountry::ID, slot);
    }
    assert_eq!(row_demand(&fixture, BASE, GAME), Ok(24));
    let mut output = Vec::new();
    assert_eq!(
        walk(&fixture, BASE, GAME, &mut output),
        Outcome::Rows { len: 6 }
    );
    assert_eq!((output[2].country, output[2].archetype), (1, 2));
    assert_eq!((output[5].country, output[5].archetype), (3, 2));
}

#[test]
fn worker_grows_from_mod_archetype_headers_on_first_and_later_samples() {
    let mut output = Vec::new();
    for count in [5_000_u32, 8_191] {
        let mut fixture = Fixture::new(0);
        fixture.array(DATABASE + database::ARCHETYPES, LIST, count);
        for index in 0..count {
            let pointer = TYPES + index as usize * TYPE_STRIDE;
            fixture.pointer(LIST + index as usize * 8, pointer);
            fixture.pointer(pointer, BASE + cequipment_type::VTABLE_RVA.0);
            fixture.u32(pointer + cequipment_type::TOKEN, token(index));
            fixture.u32(pointer + cequipment_type::STOCKPILE_INDEX, index);
            fixture.bytes(pointer + cequipment_type::IS_ARCHETYPE, &[1]);
        }
        for (owner_index, (country, _, region)) in [HUMAN, MANUAL].into_iter().enumerate() {
            for series in [STOCK_SERIES, BALANCE_SERIES] {
                let history = Fixture::history(region, series);
                let values = 0x300_0000
                    + owner_index * 0x10_0000
                    + (series - STOCK_SERIES) as usize * 0x8_0000;
                fixture.u32(history + cloop_history::FIELD_COUNT, count);
                fixture.array(history + 0x400, values, count);
                for index in 0..count {
                    fixture.i64(values + index as usize * 8, value(country, series, index));
                }
            }
        }
        assert_eq!(
            walk(&fixture, BASE, GAME, &mut output),
            Outcome::Rows {
                len: 2 * count as usize
            }
        );
        assert_eq!(output[count as usize - 1].archetype, count - 1);
        assert_eq!(output[2 * count as usize - 1].country, MANUAL.1);
        assert_eq!(
            output[2 * count as usize - 1].stock_raw,
            value(MANUAL.0, STOCK_SERIES, count - 1)
        );
    }
}

fn expected(country: (usize, u32, usize), archetype: u32) -> Row {
    Row {
        country: country.1,
        archetype,
        token: token(archetype),
        balance_raw: value(country.0, BALANCE_SERIES, archetype),
        stock_raw: value(country.0, STOCK_SERIES, archetype),
        balance_history: Fixture::history(country.2, BALANCE_SERIES),
        balance_values: Fixture::history(country.2, BALANCE_SERIES) + 0x800,
        available_raw: None,
    }
}

type Corruption = Box<dyn Fn(&mut Fixture)>;

fn capture(f: &Fixture, capacity: usize) -> (Outcome, Vec<Row>) {
    let mut rows = vec![Row::default(); capacity];
    let outcome = walk(f, BASE, GAME, &mut rows);
    (outcome, rows)
}

fn unavailable(f: &Fixture) -> &'static str {
    match capture(f, 64).0 {
        Outcome::Unavailable { reason } => reason,
        outcome => panic!("expected a refusal, got {outcome:?}"),
    }
}

#[test]
fn reads_the_current_daily_record_of_countries_the_game_computes_it_for() {
    let mut f = Fixture::new(3);
    assert!(!f.0.contains_key(&(AI.0 + ccountry::LOGISTICS_STATUS)));
    let (outcome, rows) = capture(&f, 8);
    assert_eq!(outcome, Outcome::Rows { len: 6 });
    let wanted: Vec<Row> = [HUMAN, MANUAL]
        .into_iter()
        .flat_map(|country| (0..3).map(move |archetype| expected(country, archetype)))
        .collect();
    assert_eq!(rows[..6], wanted[..]);
    f.bytes(COUNTS, &[0, 0, 0, 0]);
    f.bytes(FLAGS, &[1, 1, 1, 1]);
    f.forget(BASE + database::SINGLETON_RVA.0, 8);
    assert_eq!(capture(&f, 8).0, Outcome::Rows { len: 0 });
}

#[test]
fn names_and_reads_indices_across_batches() {
    let f = Fixture::new(70);
    let (outcome, rows) = capture(&f, 140);
    assert_eq!(outcome, Outcome::Rows { len: 140 });
    for (index, row) in rows.iter().enumerate() {
        let country = if index < 70 { HUMAN } else { MANUAL };
        assert_eq!(*row, expected(country, index as u32 % 70), "{index}");
    }
}

#[test]
fn the_count_probe_counts_every_country_unread_without_reading_its_values() {
    let mut f = Fixture::new(3);
    let manual_values = Fixture::history(MANUAL.2, BALANCE_SERIES) + 0x800;
    f.forget(manual_values, 3 * 8);
    assert_eq!(
        walk_buffer(&f, BASE, GAME, engine_native_memory::walk::Buffer::Count),
        Outcome::Truncated {
            len: 0,
            unread_rows: 6
        }
    );
}

#[test]
fn refuses_forms_the_constructor_and_database_cannot_produce() {
    let stock = Fixture::history(HUMAN.2, STOCK_SERIES);
    let balance = Fixture::history(HUMAN.2, BALANCE_SERIES);
    let cases: [(&'static str, Corruption); 16] = [
        (
            "logistics status vtable mismatch",
            Box::new(|f| f.pointer(HUMAN.2, 0x1234)),
        ),
        (
            "logistics status country mismatch",
            Box::new(|f| f.pointer(HUMAN.2 + COUNTRY, MANUAL.0)),
        ),
        (
            "logistics series array is not the constructed 19 series",
            Box::new(|f| f.u32(HUMAN.2 + SERIES + 12, 18)),
        ),
        (
            "logistics series roll-up mode differs from its constructed series",
            Box::new(move |f| f.bytes(stock + cloop_history::ROLL_UP, &1_i32.to_le_bytes())),
        ),
        (
            "logistics series self reference mismatch",
            Box::new(move |f| f.pointer(balance + cloop_history::SELF_REFERENCE, stock)),
        ),
        (
            "logistics daily window is not the constructed 30-record window",
            Box::new(move |f| {
                f.u32(stock + 0x100 + window::CAPACITY, 31);
                f.u32(stock + 0x100 + window::RECORDS + 12, 31);
                f.u32(stock + 0x100 + window::RECORDS + 8, 31);
            }),
        ),
        (
            "logistics daily window is not the constructed 30-record window",
            Box::new(move |f| f.u32(stock + 0x100 + window::CURRENT_SLOT, 30)),
        ),
        (
            "logistics stock and balance daily slots disagree",
            Box::new(move |f| f.u32(stock + 0x100 + window::CURRENT_SLOT, 6)),
        ),
        (
            "logistics current record disagrees with its daily slot",
            Box::new(move |f| f.pointer(balance + 0x200 + SLOT as usize * 8, 0x77)),
        ),
        (
            "logistics record owner mismatch",
            Box::new(move |f| f.pointer(balance + 0x400 + record_fields::HISTORY, stock)),
        ),
        (
            "logistics record size differs from its series field count",
            Box::new(move |f| f.u32(stock + 0x400 + record_fields::COUNT, 2)),
        ),
        (
            "logistics record size differs from the equipment archetype count",
            Box::new(|f| f.u32(DATABASE + database::ARCHETYPES + 12, 2)),
        ),
        (
            "equipment database vtable unreadable or mismatched",
            Box::new(|f| f.pointer(DATABASE, 0x1234)),
        ),
        (
            "equipment archetype index differs from its list position",
            Box::new(|f| f.u32(TYPES + TYPE_STRIDE + cequipment_type::STOCKPILE_INDEX, 2)),
        ),
        (
            "equipment archetype list holds a type that is not an archetype",
            Box::new(|f| {
                f.bytes(
                    TYPES + 2 * TYPE_STRIDE + cequipment_type::IS_ARCHETYPE,
                    &[0],
                )
            }),
        ),
        (
            "equipment archetype vtable mismatch",
            Box::new(|f| f.pointer(TYPES, 0x1234)),
        ),
    ];
    for (reason, corrupt) in cases {
        let mut f = Fixture::new(3);
        corrupt(&mut f);
        assert_eq!(unavailable(&f), reason);
    }
}

#[test]
fn a_failed_read_is_unavailable_never_a_short_or_zero_row() {
    let values = Fixture::history(MANUAL.2, STOCK_SERIES) + 0x800;
    let cases: [(usize, usize, &'static str); 5] = [
        (
            HUMAN.0 + ccountry::LOGISTICS_STATUS,
            8,
            "logistics status pointer unreadable or null",
        ),
        (values + 2 * 8 + 7, 1, "logistics record values unreadable"),
        (
            TYPES + TYPE_STRIDE + cequipment_type::TOKEN,
            1,
            "equipment archetype unreadable",
        ),
        (
            Fixture::history(HUMAN.2, BALANCE_SERIES) + 0x400,
            1,
            "logistics current record unreadable",
        ),
        (
            BASE + database::SINGLETON_RVA.0,
            8,
            "equipment database pointer unreadable or null",
        ),
    ];
    for (address, len, reason) in cases {
        let mut f = Fixture::new(3);
        f.forget(address, len);
        assert_eq!(unavailable(&f), reason);
    }
}

const GATHER_FAILED: &str = "human-controller counts or autonomy flags unreadable";

#[test]
fn ai_control_fails_at_its_first_operand_failure_without_reading_slots_singly() {
    let reference = capture(&Fixture::new(3), 8);
    assert_eq!(reference.0, Outcome::Rows { len: 6 });
    let mut f = Fixture::new(3);
    f.array(GAME + cgame_state::HUMAN_STATUS, COUNTS, 5);
    f.array(GAME + cgame_state::AUTONOMOUS_STATUS, FLAGS, 5);
    assert_eq!(unavailable(&f), GATHER_FAILED);
    let mut f = Fixture::new(3);
    f.forget(COUNTS + 3, 1);
    assert_eq!(unavailable(&f), GATHER_FAILED);
    let mut f = Fixture::new(3);
    f.array(GAME + cgame_state::AUTONOMOUS_STATUS, FLAGS, 3);
    assert_eq!(unavailable(&f), "autonomous flag unreadable");
    let mut f = Fixture::new(3);
    f.bytes(COUNTS + 1, &[0xff]);
    assert_eq!(unavailable(&f), "negative human-controller count");
}

struct FailOnce<'a> {
    memory: &'a Fixture,
    address: usize,
    reads: std::cell::Cell<u32>,
}

impl Memory for FailOnce<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if (address..address + destination.len()).contains(&self.address) {
            self.reads.set(self.reads.get() + 1);
            if self.reads.get() == 1 {
                return false;
            }
        }
        self.memory.copy(address, destination)
    }
}

#[test]
fn a_control_read_that_fails_once_stays_failed_for_the_sample() {
    let mut f = Fixture::new(3);
    f.array(GAME + cgame_state::HUMAN_STATUS, COUNTS, 2);
    f.array(GAME + cgame_state::AUTONOMOUS_STATUS, FLAGS, 2);
    f.array(GAME + cgame_state::PLAYER_RECORDS, 0x13000, 0);
    let (outcome, _) = capture(&f, 8);
    assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
    let memory = FailOnce {
        memory: &f,
        address: COUNTS + 1,
        reads: std::cell::Cell::new(0),
    };
    let mut rows = vec![Row::default(); 8];
    assert_eq!(
        walk(&memory, BASE, GAME, &mut rows),
        Outcome::Unavailable {
            reason: GATHER_FAILED
        }
    );
    assert_eq!(memory.reads.get(), 1);
    let memory = FailOnce {
        memory: &f,
        address: COUNTS + 1,
        reads: std::cell::Cell::new(0),
    };
    let countries = ccountry::Reader::new(&memory, BASE, GAME).unwrap();
    let mut control = cgame_state::AiControl::read(&memory, GAME).unwrap();
    for _ in 0..2 {
        assert_eq!(
            control.autonomous(&memory, BASE, &countries, HUMAN.1, 1),
            Err(GATHER_FAILED)
        );
    }
    assert_eq!(memory.reads.get(), 1);
    assert_eq!(
        control.autonomous(&memory, BASE, &countries, AI.1, 2),
        Ok(true)
    );
}

#[test]
fn a_slot_past_the_controller_counts_follows_the_live_players() {
    let mut f = Fixture::new(3);
    f.array(GAME + cgame_state::HUMAN_STATUS, COUNTS, 2);
    f.array(GAME + cgame_state::PLAYER_RECORDS, 0x13000, 1);
    f.pointer(0x13000, BASE + engine_native_parts::chuman_layout::VTABLE_RVA.0);
    f.u32(0x13000 + engine_native_parts::chuman_layout::COUNTRY_TAG, MANUAL.1);
    let (outcome, rows) = capture(&f, 8);
    assert_eq!(outcome, Outcome::Rows { len: 6 });
    assert_eq!(rows[3], expected(MANUAL, 0));
    f.u32(0x13000 + engine_native_parts::chuman_layout::COUNTRY_TAG, AI.1);
    f.logistics(AI.0, AI.2, 3);
    let (outcome, rows) = capture(&f, 8);
    assert_eq!(outcome, Outcome::Rows { len: 6 });
    assert_eq!(rows[3], expected(AI, 0));
}

#[test]
fn actual_balance_write_reads_only_the_native_available_slot_and_exact_store() {
    let mut f = Fixture::new(3);
    let history = Fixture::history(HUMAN.2, BALANCE_SERIES);
    let record = history + 0x400;
    let stack = 0xa000_0000;
    for (_, offset) in BALANCE_CALLS {
        f.i64(stack + offset, 7_000_001);
        let observed = balance_write(
            &f,
            record,
            value(HUMAN.0, BALANCE_SERIES, 1),
            1,
            stack,
            offset,
        )
        .unwrap();
        assert_eq!(observed.history, history);
        assert_eq!(observed.values, history + 0x800);
        assert_eq!(observed.available_raw, 7_000_001);
        assert!(balance_write(&f, record, 0, 1, stack, offset).is_err());
        f.forget(stack + offset, 8);
        assert!(
            balance_write(
                &f,
                record,
                value(HUMAN.0, BALANCE_SERIES, 1),
                1,
                stack,
                offset
            )
            .is_err()
        );
    }
    assert!(balance_identity(&f, record, u32::MAX).is_err());
    assert!(balance_identity(&f, record, 3).is_err());
    f.forget(record + record_fields::HISTORY, 8);
    assert!(balance_identity(&f, record, 0).is_err());
}

struct Scaled {
    fixture: Fixture,
    count: usize,
}
const S_COUNTRIES: usize = 0x3000_0000;
const S_LINKS: usize = 0x4000_0000;
const S_COUNTS: usize = 0x5000_0000;
const S_FLAGS: usize = 0x6000_0000;
const S_OBJECTS: usize = 0x10_0000_0000;
const S_REGIONS: usize = 0x20_0000_0000;
impl Scaled {
    fn new(count: usize) -> Self {
        let mut fixture = Fixture::new(3);
        for (offset, pointer) in [
            (cgame_state::COUNTRIES, S_COUNTRIES),
            (cgame_state::COUNTRY_LINKS, S_LINKS),
            (cgame_state::HUMAN_STATUS, S_COUNTS),
            (cgame_state::AUTONOMOUS_STATUS, S_FLAGS),
        ] {
            fixture.array(GAME + offset, pointer, count as u32 + 1);
        }
        Self { fixture, count }
    }
    fn byte(&self, address: usize) -> Option<u8> {
        let size = self.count + 1;
        if (S_COUNTRIES..S_COUNTRIES + size * 8).contains(&address) {
            let offset = address - S_COUNTRIES;
            let slot = offset / 8;
            let pointer = if slot == 0 {
                0
            } else {
                S_OBJECTS + (slot - 1) * 0x2000
            };
            return Some((pointer as u64).to_le_bytes()[offset % 8]);
        }
        if (S_LINKS..S_LINKS + size * 4).contains(&address) {
            let offset = address - S_LINKS;
            return Some((offset as u32 / 4).to_le_bytes()[offset % 4]);
        }
        for start in [S_COUNTS, S_FLAGS] {
            if (start..start + size).contains(&address) {
                return Some(u8::from(address != start));
            }
        }
        if (S_OBJECTS..S_OBJECTS + self.count * 0x2000).contains(&address) {
            let offset = (address - S_OBJECTS) % 0x2000;
            let slot = (address - S_OBJECTS) / 0x2000;
            if (ccountry::ID..ccountry::ID + 4).contains(&offset) {
                return Some((slot as u32 + 1).to_le_bytes()[offset - ccountry::ID]);
            }
            if (ccountry::LOGISTICS_STATUS..ccountry::LOGISTICS_STATUS + 8).contains(&offset) {
                return Some(
                    ((S_REGIONS + slot * 0x4000) as u64).to_le_bytes()
                        [offset - ccountry::LOGISTICS_STATUS],
                );
            }
            return self.fixture.0.get(&(HUMAN.0 + offset)).copied();
        }
        if (S_REGIONS..S_REGIONS + self.count * 0x4000).contains(&address) {
            let offset = (address - S_REGIONS) % 0x4000;
            let slot = (address - S_REGIONS) / 0x4000;
            let original = HUMAN.2 + offset;
            let aligned = original & !7;
            let mut bytes = [0; 8];
            let readable = (0..8).all(|index| match self.fixture.0.get(&(aligned + index)) {
                Some(value) => {
                    bytes[index] = *value;
                    true
                }
                None => false,
            });
            if readable {
                let pointer = u64::from_le_bytes(bytes) as usize;
                let replacement = if pointer == HUMAN.0 {
                    Some(S_OBJECTS + slot * 0x2000)
                } else if (HUMAN.2..HUMAN.2 + 0x4000).contains(&pointer) {
                    Some(S_REGIONS + slot * 0x4000 + pointer - HUMAN.2)
                } else {
                    None
                };
                if let Some(pointer) = replacement {
                    return Some((pointer as u64).to_le_bytes()[original % 8]);
                }
            }
            return self.fixture.0.get(&original).copied();
        }
        self.fixture.0.get(&address).copied()
    }
}
impl Memory for Scaled {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (offset, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.byte(address + offset) else {
                return false;
            };
            *byte = value;
        }
        true
    }
}

#[test]
fn native_acquisition_sizes_owners_and_rows_from_the_live_roster() {
    let fixture = Scaled::new(4);
    assert_eq!(owner_capacity(&fixture, BASE, GAME), Ok(5));
    let mut owners = vec![BalanceOwner::default(); 4];
    assert_eq!(
        walk_balance_owners(&fixture, BASE, GAME, &mut owners),
        Outcome::Rows { len: 4 }
    );
    assert_eq!(owners.last().unwrap().country, 4);
    assert_eq!(
        walk_balance_owners(&fixture, BASE, GAME, &mut owners[..3]),
        Outcome::Truncated {
            len: 3,
            unread_rows: 1
        }
    );
    let mut rows = vec![Row::default(); 12];
    assert_eq!(
        walk(&fixture, BASE, GAME, &mut rows),
        Outcome::Rows { len: 12 }
    );
    assert_eq!(rows[11].country, 4);
    assert_eq!(rows[11].token, token(2));
    assert_eq!(rows[11].balance_history, owners[3].history);
    let empty = Fixture::new(0);
    assert_eq!(walk(&empty, BASE, GAME, &mut Vec::new()), Outcome::Rows { len: 0 });
}
