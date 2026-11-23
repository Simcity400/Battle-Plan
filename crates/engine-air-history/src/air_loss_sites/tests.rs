use super::*;
use engine_native::cgame_state;
use engine_native_air::cstrategic_air_manager;
use engine_native_parts::cstrategic_region;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x0001_4000_0000;
const GAME: usize = 0x10000;
const COUNTRY: usize = 0x11000;
const MANAGER: usize = 0x12000;
const AIR: usize = 0x13000;
const HISTORY: usize = 0x14000;
const WINDOW: usize = 0x15000;
const RECORD: usize = 0x16000;
const VALUES: usize = 0x17000;
const REGION: usize = 0x18000;
const POOL: usize = 0x19000;

struct Fixture(Vec<u8>);

impl Fixture {
    fn bytes(&mut self, address: usize, bytes: &[u8]) {
        let first = address - GAME;
        self.0[first..first + bytes.len()].copy_from_slice(bytes);
    }

    fn u32(&mut self, address: usize, value: u32) {
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

    fn value(&mut self, field: usize, value: i64) {
        self.bytes(VALUES + field * 8, &value.to_le_bytes());
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: GAME,
            bytes: &self.0,
        }
    }

    fn new() -> Self {
        let mut f = Self(vec![0; 0xb000]);
        f.array(GAME + cgame_state::COUNTRIES, 0x10800, 3);
        f.pointer(0x10808, COUNTRY);
        f.pointer(0x10810, COUNTRY);
        f.array(GAME + cgame_state::COUNTRY_LINKS, 0x10820, 4);
        for index in 1..4 {
            f.u32(0x10820 + index * 4, 1);
        }
        f.pointer(COUNTRY, BASE + ccountry::VTABLE_RVA.0);
        f.u32(COUNTRY + ccountry::ID, 3);
        f.pointer(GAME + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER);
        f.pointer(MANAGER, BASE + cstrategic_air_manager::VTABLE_RVA.0);
        f.array(MANAGER + cstrategic_air_manager::COUNTRY_AIR, 0x12800, 3);
        f.pointer(0x12808, AIR);
        f.pointer(AIR, BASE + cstrategic_air::VTABLE_RVA.0);
        f.u32(AIR + cstrategic_air::COUNTRY, 2);
        f.array(AIR + cstrategic_air::HISTORY_TABLE, 0x13800, 24);
        f.pointer(0x13800 + 23 * 8, HISTORY);
        f.pointer(HISTORY, BASE + cloop_history::VTABLE_RVA.0);
        f.pointer(HISTORY + cloop_history::SELF_REFERENCE, HISTORY);
        f.u32(HISTORY + cloop_history::FIELD_COUNT, FIELD_COUNT as u32);
        f.pointer(HISTORY + cloop_history::DAILY_WINDOW, WINDOW);
        f.pointer(WINDOW, BASE + loop_history_window::VTABLE_RVA.0);
        f.array(WINDOW + loop_history_window::RECORDS, 0x15800, 30);
        f.u32(WINDOW + loop_history_window::CAPACITY, 30);
        f.u32(WINDOW + loop_history_window::CURRENT_SLOT, 29);
        f.bytes(WINDOW + loop_history_window::WRAPPED, &[1]);
        f.pointer(WINDOW + loop_history_window::RECORD_FIELDS, RECORD);
        f.pointer(WINDOW + loop_history_window::HISTORY, HISTORY);
        f.pointer(0x15800 + 29 * 8, RECORD);
        f.array(RECORD, VALUES, FIELD_COUNT as u32);
        f.pointer(RECORD + loop_history_record_fields::HISTORY, HISTORY);
        f.pointer(REGION, BASE + cstrategic_region::VTABLE_RVA.0);
        f.u32(REGION + cstrategic_region::ID, 23);
        f.array(AIR + cstrategic_air::POOLS, 0x138c0, 1);
        f.pointer(0x138c0, POOL);
        f.pointer(POOL, BASE + cair_wing_pool::VTABLE_RVA.0);
        f.array(POOL + 0x28, 0x19800, 2);
        f
    }

    fn before(&self, enemy: u8, cause: i32) -> Snapshot {
        before(
            &self.memory(),
            BASE,
            GAME,
            AIR,
            23,
            enemy,
            cause,
            if cause == 6 {
                40
            } else if enemy == 1 {
                71
            } else {
                30
            },
        )
        .unwrap()
    }
}

#[test]
fn air_loss_snapshot_reads_selected_scout_words_aliases_and_clamp_effects() {
    let mut f = Fixture::new();
    for index in 0..FIELD_COUNT {
        f.value(index, -(index as i64));
    }
    const NATIVE_MAX: i64 = 0x53e2_d620_fb40;
    f.value(71, NATIVE_MAX - 7);
    let snapshot = f.before(1, 1);
    assert_eq!(
        (snapshot.country, snapshot.region, snapshot.slot),
        (2, 23, 29)
    );
    assert_eq!(snapshot.fields[1], -72);
    f.value(71, NATIVE_MAX);
    f.value(72, 99_928);
    let change = after(&f.memory(), BASE, GAME, &snapshot).unwrap();
    assert_eq!((change.enemy, change.cause), (1, 1));
    assert_eq!(change.daily_slot, 29);
    assert_eq!(change.fields[0], 7);
    assert_eq!(change.fields[1], 100_000);
    assert_eq!(change.fields.iter().filter(|&&value| value != 0).count(), 2);
}

#[test]
fn air_loss_new_history_zero_is_known_only_at_the_writer_boundary() {
    let mut f = Fixture::new();
    f.pointer(0x13800 + 23 * 8, 0);
    let snapshot = f.before(0, 6);
    assert_eq!(snapshot.fields, [0; 2]);
    assert!(after(&f.memory(), BASE, GAME, &snapshot).is_err());
    f.pointer(0x13800 + 23 * 8, HISTORY);
    f.u32(WINDOW + loop_history_window::CURRENT_SLOT, 7);
    f.pointer(0x15800 + 7 * 8, RECORD);
    f.value(40, 200_000);
    f.value(46, 200_000);
    let change = after(&f.memory(), BASE, GAME, &snapshot).unwrap();
    assert_eq!(change.daily_slot, 7);
    assert_eq!((change.fields[0], change.fields[1]), (200_000, 200_000));
}

#[test]
fn daily_roster_keeps_true_false_aliases_and_refuses_invalid_slots() {
    let mut f = Fixture::new();
    f.array(GAME + cgame_state::DAILY_COUNTRY_ROSTER, 0x109e0, 1);
    f.u32(0x109e0, 1);
    let mut rows = Vec::new();
    roster(&f.memory(), BASE, GAME, &mut rows).unwrap();
    assert_eq!(
        rows,
        vec![RosterRow {
            country: 3,
            eligible: true
        }]
    );
    f.array(GAME + cgame_state::DAILY_COUNTRY_ROSTER, 0x109e0, 0);
    roster(&f.memory(), BASE, GAME, &mut rows).unwrap();
    assert_eq!(
        rows,
        vec![RosterRow {
            country: 3,
            eligible: false
        }]
    );
    f.array(GAME + cgame_state::DAILY_COUNTRY_ROSTER, 0x109e0, 1);
    f.u32(0x109e0, 2);
    f.pointer(0x10810, 0);
    assert_eq!(
        roster(&f.memory(), BASE, GAME, &mut rows),
        Err("country pointer is null")
    );
    f.array(GAME + cgame_state::DAILY_COUNTRY_ROSTER, 0x109e0, 1);
    f.u32(0x109e0, 4);
    assert!(roster(&f.memory(), BASE, GAME, &mut rows).is_err());
}

#[test]
fn opponent_reads_the_opposite_native_signed_tag_word() {
    let mut f = Fixture::new();
    f.u32(VALUES, 2);
    f.u32(VALUES + 4, 4999);
    let tags = [VALUES, VALUES + 4];
    assert_eq!(opponent(&f.memory(), tags, 0), Ok(Some(4999)));
    assert_eq!(opponent(&f.memory(), tags, 1), Ok(Some(2)));
    f.u32(VALUES + 4, u32::MAX);
    assert_eq!(opponent(&f.memory(), tags, 0), Ok(None));
}

#[test]
fn air_loss_current_cursor_layout_and_short_fields_are_guarded() {
    let mut f = Fixture::new();
    f.u32(WINDOW + loop_history_window::CURRENT_SLOT, 0);
    f.bytes(WINDOW + loop_history_window::WRAPPED, &[0]);
    f.pointer(0x15800, RECORD);
    assert_eq!(f.before(0, 3).slot, 0);
    let corruptions = [
        (
            HISTORY + cloop_history::FIELD_COUNT,
            88_u32.to_le_bytes().to_vec(),
        ),
        (
            WINDOW + loop_history_window::CURRENT_SLOT,
            30_u32.to_le_bytes().to_vec(),
        ),
        (WINDOW + loop_history_window::WRAPPED, vec![2]),
        (
            WINDOW + loop_history_window::HISTORY,
            0_u64.to_le_bytes().to_vec(),
        ),
        (
            RECORD + loop_history_record_fields::HISTORY,
            0_u64.to_le_bytes().to_vec(),
        ),
        (
            RECORD + loop_history_record_fields::COUNT,
            88_u32.to_le_bytes().to_vec(),
        ),
    ];
    for (address, bytes) in corruptions {
        let mut broken = Fixture::new();
        broken.bytes(address, &bytes);
        assert!(current(&broken.memory(), BASE, HISTORY, 30, 1).is_err());
    }
    let memory = SliceMemory {
        base: GAME,
        bytes: &f.0[..VALUES - GAME + 32 * 8 - 1],
    };
    assert!(matches!(
        current(&memory, BASE, HISTORY, 30, 1),
        Err("air loss selected field unreadable")
    ));
}

#[test]
fn air_loss_reload_cursor_record_and_signed_delta_changes_refuse_the_event() {
    let mut f = Fixture::new();
    let snapshot = f.before(0, 1);
    f.u32(WINDOW + loop_history_window::CURRENT_SLOT, 0);
    f.pointer(0x15800, RECORD);
    assert_eq!(
        after(&f.memory(), BASE, GAME, &snapshot),
        Err("air loss writer current record changed during call")
    );
    let mut f = Fixture::new();
    let snapshot = f.before(0, 1);
    f.pointer(WINDOW + loop_history_window::RECORD_FIELDS, VALUES);
    assert_eq!(
        after(&f.memory(), BASE, GAME, &snapshot),
        Err("air loss writer current record changed during call")
    );
    let mut f = Fixture::new();
    f.pointer(0x13800 + 23 * 8, 0);
    let snapshot = f.before(0, 1);
    f.pointer(0x13800 + 23 * 8, HISTORY);
    assert!(after(&f.memory(), BASE, GAME, &snapshot).is_ok());
    f.u32(AIR + cstrategic_air::COUNTRY, 0);
    assert!(after(&f.memory(), BASE, GAME, &snapshot).is_err());
    let mut f = Fixture::new();
    f.value(30, i64::MIN);
    let snapshot = f.before(0, 0);
    f.value(30, i64::MAX);
    assert_eq!(
        after(&f.memory(), BASE, GAME, &snapshot),
        Err("air loss writer delta overflow")
    );
}

#[test]
fn air_loss_current_record_reads_the_selected_words_in_one_copy() {
    let mut f = Fixture::new();
    f.value(30, 7);
    f.value(31, 9);
    let memory = engine_native_memory::memory::CountingMemory {
        inner: f.memory(),
        calls: std::cell::Cell::new(0),
        bytes: std::cell::Cell::new(0),
    };
    let current = current(&memory, BASE, HISTORY, 30, 1).unwrap();
    assert_eq!(
        (current.record, current.slot, current.fields),
        (RECORD, 29, [7, 9])
    );
    assert_eq!((memory.calls.get(), memory.bytes.get()), (15, 125));
}

#[test]
fn air_loss_after_an_existing_history_reads_only_the_witnesses_and_words() {
    let mut f = Fixture::new();
    let snapshot = f.before(0, 1);
    f.value(30, 100_000);
    f.value(31, 100_000);
    let memory = engine_native_memory::memory::CountingMemory {
        inner: f.memory(),
        calls: std::cell::Cell::new(0),
        bytes: std::cell::Cell::new(0),
    };
    let change = after(&memory, BASE, GAME, &snapshot).unwrap();
    assert_eq!((change.country, change.daily_slot), (snapshot.country, 29));
    assert_eq!(change.fields, [100_000; 2]);
    assert_eq!((memory.calls.get(), memory.bytes.get()), (2, 4 + 8 + 2 * 8));
}

#[test]
fn air_loss_wing_sizing_reads_live_headers_without_following_wings() {
    let mut f = Fixture::new();
    f.pointer(0x19800, usize::MAX);
    assert_eq!(wing_count(&f.memory(), BASE, GAME), Ok(2));
    f.u32(POOL + 0x28 + 12, 3);
    assert_eq!(
        wing_count(&f.memory(), BASE, GAME),
        Err("air loss writer array unreadable or invalid")
    );
    f.u32(POOL + 0x28 + 12, 2);
    f.pointer(POOL, 0);
    assert_eq!(
        wing_count(&f.memory(), BASE, GAME),
        Err("air loss writer object vtable unreadable or mismatched")
    );
}

#[test]
fn air_loss_family_selection_keeps_native_priority_unknown_and_content_flags() {
    let mut f = Fixture::new();
    const WING: usize = 0x19100;
    const DEFINITION: usize = 0x1a000;
    f.pointer(WING, BASE + engine_native_parts::cair_wing_layout::VTABLE_RVA.0);
    f.pointer(WING + engine_native_parts::cair_wing_layout::POOL, POOL);
    f.pointer(POOL + cair_wing_pool::DEFINITION, DEFINITION);
    f.pointer(DEFINITION, BASE + csub_unit_definition::VTABLE_RVA.0);
    for (flags, own, credited) in [
        (1_u64 << 32, 3, 44),
        (1 << 17, 10, 51),
        (1 << 14, 17, 58),
        (1 << 20, 24, 65),
        (1 << 18, 30, 71),
        (1 << 36, 40, 81),
        ((1 << 14) | (1 << 18), 17, 58),
        ((1 << 32) | (1 << 17), 3, 44),
        (0, 0, 0),
    ] {
        f.bytes(
            DEFINITION + csub_unit_definition::AIR_FLAGS,
            &flags.to_le_bytes(),
        );
        assert_eq!(selected_family(&f.memory(), BASE, WING, 0), Ok(own));
        assert_eq!(selected_family(&f.memory(), BASE, WING, 1), Ok(credited));
    }
    f.pointer(DEFINITION, BASE + csub_unit_definition::NULL_VTABLE_RVA.0);
    assert_eq!(selected_family(&f.memory(), BASE, WING, 0), Ok(0));
    f.pointer(DEFINITION, 0);
    assert_eq!(
        selected_family(&f.memory(), BASE, WING, 0),
        Err("air loss definition form unsupported")
    );
    f.pointer(DEFINITION, BASE + csub_unit_definition::VTABLE_RVA.0);
    let memory = SliceMemory {
        base: GAME,
        bytes: &f.0[..DEFINITION - GAME + csub_unit_definition::AIR_FLAGS + 7],
    };
    assert_eq!(
        selected_family(&memory, BASE, WING, 0),
        Err("air loss writer field unreadable or address overflow")
    );
    f.pointer(POOL + cair_wing_pool::DEFINITION, usize::MAX);
    assert_eq!(
        selected_family(&f.memory(), BASE, WING, 0),
        Err("air loss definition vtable unreadable")
    );
    f.pointer(WING + engine_native_parts::cair_wing_layout::POOL, 0);
    assert_eq!(
        selected_family(&f.memory(), BASE, WING, 0),
        Err("air loss writer object vtable unreadable or mismatched")
    );
    f.pointer(WING + engine_native_parts::cair_wing_layout::POOL, usize::MAX);
    assert_eq!(
        selected_family(&f.memory(), BASE, WING, 0),
        Err("air loss writer object vtable unreadable or mismatched")
    );
    assert_eq!(
        selected_family(&f.memory(), BASE, WING, 2),
        Err("air loss writer enemy flag is not boolean")
    );
}

struct ScaleFixture {
    bytes: Vec<u8>,
    address: usize,
    airs: Vec<usize>,
    wings: Vec<usize>,
    values: Vec<usize>,
}

impl ScaleFixture {
    fn new(countries: usize) -> Self {
        const BUNDLE: usize = 0x900;
        let countries_at = 0x4000;
        let links_at = countries_at + (countries + 1) * 8;
        let bundles_at = (links_at + (countries + 1) * 4 + 15) & !15;
        let manager_slots = bundles_at + countries * BUNDLE;
        let active_slots = manager_slots + (countries + 1) * 8;
        let bytes = vec![0; active_slots + countries * 4];
        let address = bytes.as_ptr() as usize;
        let mut f = Self {
            bytes,
            address,
            airs: Vec::with_capacity(countries),
            wings: Vec::with_capacity(countries),
            values: Vec::with_capacity(countries),
        };
        f.array(cgame_state::DAILY_COUNTRY_ROSTER, active_slots, countries);
        for ordinal in 0..countries {
            f.u32(active_slots + ordinal * 4, ordinal as u32 + 1);
        }
        f.array(cgame_state::COUNTRIES, countries_at, countries + 1);
        f.array(cgame_state::COUNTRY_LINKS, links_at, countries + 1);
        f.pointer(cgame_state::STRATEGIC_AIR_MANAGER, address + 0x1000);
        f.pointer(0x1000, BASE + cstrategic_air_manager::VTABLE_RVA.0);
        f.array(
            0x1000 + cstrategic_air_manager::COUNTRY_AIR,
            manager_slots,
            countries + 1,
        );
        f.pointer(0x2000, BASE + csub_unit_definition::VTABLE_RVA.0);
        f.bytes(
            0x2000 + csub_unit_definition::AIR_FLAGS,
            &(1_u64 << 18).to_le_bytes(),
        );
        f.pointer(0x2800, BASE + cstrategic_region::VTABLE_RVA.0);
        f.u32(0x2800 + cstrategic_region::ID, 1);
        for country in 1..=countries {
            let first = bundles_at + (country - 1) * BUNDLE;
            let air = first + 0x20;
            let history = first + 0x200;
            let window = first + 0x240;
            let record = first + 0x280;
            let values = first + 0x2a0;
            let pool = first + 0x680;
            let wing = first + 0x6e0;
            f.pointer(countries_at + country * 8, address + first);
            f.u32(links_at + country * 4, country as u32);
            f.pointer(first, BASE + ccountry::VTABLE_RVA.0);
            f.u32(first + ccountry::ID, country as u32);
            f.pointer(manager_slots + country * 8, address + air);
            f.pointer(air, BASE + cstrategic_air::VTABLE_RVA.0);
            f.u32(air + cstrategic_air::COUNTRY, country as u32);
            f.array(air + cstrategic_air::HISTORY_TABLE, first + 0x570, 2);
            f.pointer(first + 0x578, address + history);
            f.pointer(history, BASE + cloop_history::VTABLE_RVA.0);
            f.pointer(history + cloop_history::SELF_REFERENCE, address + history);
            f.u32(history + cloop_history::FIELD_COUNT, FIELD_COUNT as u32);
            f.pointer(history + cloop_history::DAILY_WINDOW, address + window);
            f.pointer(window, BASE + loop_history_window::VTABLE_RVA.0);
            f.array(window + loop_history_window::RECORDS, first + 0x580, 30);
            f.u32(window + loop_history_window::CAPACITY, 30);
            f.u32(window + loop_history_window::CURRENT_SLOT, 29);
            f.bytes(window + loop_history_window::WRAPPED, &[1]);
            f.pointer(
                window + loop_history_window::RECORD_FIELDS,
                address + record,
            );
            f.pointer(window + loop_history_window::HISTORY, address + history);
            f.pointer(first + 0x580 + 29 * 8, address + record);
            f.array(record, values, FIELD_COUNT);
            f.pointer(
                record + loop_history_record_fields::HISTORY,
                address + history,
            );
            f.array(air + cstrategic_air::POOLS, first + 0x6d0, 1);
            f.pointer(first + 0x6d0, address + pool);
            f.pointer(pool, BASE + cair_wing_pool::VTABLE_RVA.0);
            f.array(pool + cair_wing_pool::WINGS, first + 0x720, 1);
            f.pointer(first + 0x720, address + wing);
            f.pointer(pool + cair_wing_pool::DEFINITION, address + 0x2000);
            f.pointer(wing, BASE + engine_native_parts::cair_wing_layout::VTABLE_RVA.0);
            f.pointer(wing + engine_native_parts::cair_wing_layout::POOL, address + pool);
            f.airs.push(address + air);
            f.wings.push(address + wing);
            f.values.push(address + values);
        }
        f
    }

    fn bytes(&mut self, offset: usize, bytes: &[u8]) {
        self.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    fn pointer(&mut self, offset: usize, pointer: usize) {
        self.bytes(offset, &(pointer as u64).to_le_bytes());
    }

    fn u32(&mut self, offset: usize, value: u32) {
        self.bytes(offset, &value.to_le_bytes());
    }

    fn array(&mut self, offset: usize, data: usize, count: usize) {
        self.pointer(offset, self.address + data);
        self.u32(offset + 8, count as u32);
        self.u32(offset + 12, count as u32);
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: self.address,
            bytes: &self.bytes,
        }
    }
}

struct Faulted<'a> {
    inner: SliceMemory<'a>,
    fault: usize,
    once: bool,
    met: std::cell::Cell<bool>,
}

impl Memory for Faulted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let touches = (address..address + destination.len()).contains(&self.fault);
        if touches && !(self.once && self.met.replace(true)) {
            return false;
        }
        self.inner.copy(address, destination)
    }
}

impl ScaleFixture {
    const SINGLETON: usize = 0x3000;
    const REGION: usize = 0x2800;

    fn call(
        &self,
        memory: &impl Memory,
        ordinal: usize,
        enemy: u8,
        cause: i32,
    ) -> Option<Gathered> {
        let air = self.airs[ordinal];
        gather(
            memory,
            BASE,
            self.address + Self::SINGLETON,
            air,
            self.wings[ordinal],
            self.address + Self::REGION,
            enemy,
            cause,
            [air + cstrategic_air::COUNTRY; 2],
        )
    }

    fn serial(
        &self,
        memory: &impl Memory,
        ordinal: usize,
        enemy: u8,
        cause: i32,
    ) -> Result<Gathered, &'static str> {
        let air = self.airs[ordinal];
        let region = cstrategic_region::id(memory, BASE, self.address + Self::REGION)?.unwrap();
        let game = read::<u64>(memory, self.address + Self::SINGLETON).ok_or("game")? as usize;
        let hour = read::<i32>(memory, game + cgame_state::CURRENT_HOUR).ok_or("hour")?;
        let family = selected_family(memory, BASE, self.wings[ordinal], enemy)?;
        let snapshot = before(memory, BASE, game, air, region, enemy, cause, family)?;
        let opponent = opponent(memory, [air + cstrategic_air::COUNTRY; 2], enemy)?;
        Ok(Gathered {
            region,
            game,
            hour,
            family,
            snapshot,
            opponent,
        })
    }
}

#[test]
fn air_loss_gathered_call_equals_the_serial_reads() {
    let mut f = ScaleFixture::new(70);
    let game = f.address;
    f.pointer(ScaleFixture::SINGLETON, game);
    for ordinal in 0..f.airs.len() {
        let offset = f.values[ordinal] - f.address;
        for field in 0..FIELD_COUNT {
            let value =
                (ordinal * FIELD_COUNT + field) as i64 * if field % 2 == 0 { 1 } else { -1 };
            f.bytes(offset + field * 8, &value.to_le_bytes());
        }
    }
    for ordinal in 0..f.airs.len() {
        for (enemy, cause) in [(0, 1), (1, 6)] {
            let serial = f.serial(&f.memory(), ordinal, enemy, cause).unwrap();
            assert_eq!(serial.snapshot.country, ordinal as u32 + 1);
            assert_ne!(serial.snapshot.fields, [0; 2]);
            assert_eq!(f.call(&f.memory(), ordinal, enemy, cause), Some(serial));
        }
    }
    fn counting(f: &ScaleFixture) -> engine_native_memory::memory::CountingMemory<'_> {
        engine_native_memory::memory::CountingMemory {
            inner: f.memory(),
            calls: std::cell::Cell::new(0),
            bytes: std::cell::Cell::new(0),
        }
    }
    let memory = counting(&f);
    f.serial(&memory, 0, 0, 1).unwrap();
    let serial = memory.calls.get();
    let memory = counting(&f);
    engine_native::ccountry::Reader::new(&memory, BASE, game)
        .unwrap()
        .resolve(1)
        .unwrap();
    let resolution = memory.calls.get();
    let memory = counting(&f);
    f.call(&memory, 0, 0, 1).unwrap();
    assert_eq!(memory.calls.get(), 6 + resolution);
    assert!(serial >= memory.calls.get() + 20, "{serial}");
}

#[test]
fn air_loss_gathered_faults_fall_back_to_the_serial_reads() {
    let mut f = ScaleFixture::new(4);
    let game = f.address;
    f.pointer(ScaleFixture::SINGLETON, game);
    let healthy = f.serial(&f.memory(), 1, 0, 1).unwrap();
    let healthy_reads = {
        let reads = std::cell::RefCell::new(Vec::new());
        struct Traced<'a>(SliceMemory<'a>, &'a std::cell::RefCell<Vec<(usize, usize)>>);
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.1.borrow_mut().push((address, destination.len()));
                self.0.copy(address, destination)
            }
        }
        f.call(&Traced(f.memory(), &reads), 1, 0, 1).unwrap();
        reads.into_inner()
    };
    for &(address, _) in &healthy_reads {
        let faulted = |once| Faulted {
            inner: f.memory(),
            fault: address,
            once,
            met: std::cell::Cell::new(false),
        };
        let persistent = faulted(false);
        assert_eq!(f.call(&persistent, 1, 0, 1), None, "{address:#x}");
        assert!(f.serial(&persistent, 1, 0, 1).is_err(), "{address:#x}");
        let once = faulted(true);
        assert_eq!(f.call(&once, 1, 0, 1), None, "{address:#x}");
        assert_eq!(f.serial(&once, 1, 0, 1), Ok(healthy), "{address:#x}");
    }
    f.u32(ScaleFixture::REGION + cstrategic_region::ID, 7);
    assert_eq!(f.call(&f.memory(), 1, 0, 1), None);
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[test]
#[ignore = "release synthetic guarded-read cost; not gameplay wall time"]
fn air_loss_live_memory_five_thousand_country_cost() {
    use engine_native_memory::memory::{CountedMemory, LiveMemory, ReadCounts};

    let f = ScaleFixture::new(5_000);
    let memory = CountedMemory::new(unsafe { LiveMemory::new() });
    let prior_counts = ReadCounts::take_current();
    let census_started = std::time::Instant::now();
    assert_eq!(wing_count(&memory, BASE, f.address), Ok(5_000));
    let census_ms = census_started.elapsed().as_secs_f64() * 1_000.0;
    let census_counts = ReadCounts::take_current();
    let mut times = Vec::with_capacity(7);
    let mut event_counts = ReadCounts::ZERO;
    for _ in 0..7 {
        let started = std::time::Instant::now();
        for ordinal in 0..f.airs.len() {
            let snapshot = before(&memory, BASE, f.address, f.airs[ordinal], 1, 0, 1, 30).unwrap();
            assert_eq!(selected_family(&memory, BASE, f.wings[ordinal], 0), Ok(30));
            let change = after(&memory, BASE, f.address, &snapshot).unwrap();
            assert_eq!(change.country, ordinal as u32 + 1);
            assert_eq!(change.fields, [0; 2]);
        }
        times.push(started.elapsed().as_secs_f64() * 1_000.0);
        let counts = ReadCounts::take_current();
        if event_counts != ReadCounts::ZERO {
            assert_eq!(counts, event_counts);
        }
        event_counts = counts;
    }
    prior_counts.add_current();
    times.sort_by(f64::total_cmp);
    eprintln!(
        "air loss LiveMemory synthetic: 5000 countries, {} arena bytes; census {:.3} ms {:?}; 5000 before/family/after pairs median {:.3} ms ({:.3} us/pair) {:?}",
        f.bytes.len(),
        census_ms,
        census_counts,
        times[3],
        times[3] * 1_000.0 / 5_000.0,
        event_counts,
    );
}
