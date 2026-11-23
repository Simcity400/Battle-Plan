use super::*;
use crate::{ccountry, cgame_state};
use engine_native_parts::{modifier_accumulator, modifier_definition as md};
use engine_native_memory::modifier_cache::Value;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

fn maximum_organisation(
    memory: &impl Memory,
    base: usize,
    ship: usize,
) -> Result<i64, &'static str> {
    read_maximum_organisation_memo(memory, base, ship, &mut OrganisationMemo::default())
}

const IMAGE: usize = 0x140000000;
const SHIP: usize = 0x1000;
const TASK: usize = 0x2000;
const GAME: usize = 0x3000;
const COUNTRY: usize = 0x5000;
#[derive(Clone)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    calls: Cell<usize>,
    copied: Cell<usize>,
}
impl Fixture {
    fn put(&mut self, p: usize, v: &[u8]) {
        for (i, &b) in v.iter().enumerate() {
            self.bytes.insert(p + i, b);
        }
    }
    fn word(&mut self, p: usize, v: u64) {
        self.put(p, &v.to_le_bytes());
    }
    fn dword(&mut self, p: usize, v: u32) {
        self.put(p, &v.to_le_bytes());
    }
    fn array(&mut self, p: usize, data: usize, count: u32) {
        self.word(p, data as u64);
        self.dword(p + 8, count);
        self.dword(p + 12, count);
    }
    fn modifier(&mut self, p: usize, data: usize, values: &[(u32, i64)]) {
        self.array(p, data, values.len() as u32);
        for (i, &(key, value)) in values.iter().enumerate() {
            self.dword(data + i * 16, key);
            self.word(data + i * 16 + 8, value as u64);
        }
    }
}
impl Memory for Fixture {
    fn copy(&self, p: usize, out: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.copied.set(self.copied.get() + out.len());
        for (i, b) in out.iter_mut().enumerate() {
            let Some(v) = self.bytes.get(&(p + i)) else {
                return false;
            };
            *b = *v;
        }
        true
    }
}
struct Snapshotted<'a, M> {
    source: &'a M,
    arena: RefCell<modifier_accumulator::SnapshotArena>,
    lookups: Cell<usize>,
}
impl<M: Memory> Memory for Snapshotted<'_, M> {
    fn copy(&self, p: usize, out: &mut [u8]) -> bool {
        self.source.copy(p, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.source.copy_scattered(addresses, destinations)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        self.lookups.set(self.lookups.get() + 1);
        let mut arena = self.arena.borrow_mut();
        Some(
            arena
                .capture_at(self.source, address, key)
                .and_then(|observation| arena.value(observation)),
        )
    }
}
fn fixture() -> Fixture {
    let mut f = Fixture {
        bytes: BTreeMap::new(),
        calls: Cell::new(0),
        copied: Cell::new(0),
    };
    f.put(0x1000, &vec![0; 0x20000]);
    f.word(SHIP, (IMAGE + VTABLE_RVA.0) as u64);
    f.word(SHIP + HANDLE, (42 << 32) | 51);
    f.word(SHIP + TASK_FORCE, TASK as u64);
    f.word(TASK, (IMAGE + ctask_force::VTABLE_RVA.0) as u64);
    f.dword(TASK + cunit::ROSTER_OWNER, 1);
    f.word(IMAGE + cgame_state::SINGLETON_RVA.0, GAME as u64);
    f.array(GAME + cgame_state::COUNTRIES, 0x4000, 2);
    f.word(0x4008, COUNTRY as u64);
    f.array(GAME + cgame_state::COUNTRY_LINKS, 0x4100, 2);
    f.dword(0x4104, 1);
    f.word(COUNTRY, (IMAGE + ccountry::VTABLE_RVA.0) as u64);
    f.dword(COUNTRY + ccountry::ID, 1);
    f.word(SHIP + 0x8f8, 0x7000);
    f.word(TASK + 0x640, 0x8000);
    f.word(0x8000, 0x8000);
    f.put(0x8019, &[1]);
    f.word(IMAGE + md::TABLE_RVA.0, 0x9000);
    f.word(SHIP + BASE_ORGANISATION, 10_000_000);
    f.modifier(
        COUNTRY + 0x5b8,
        0x6000,
        &[(0x46, 100000), (0x47, 10000), (0x27e, 20000)],
    );
    f
}

fn support_fixture() -> Fixture {
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    let mut f = fixture();
    let handle = (2_u64 << 32) | 7;
    f.array(TASK + 0x728, 0x12000, 1);
    f.word(0x12000, 0x12100);
    f.word(0x12140, 0x12200);
    f.word(0x12208, handle);
    f.word(IMAGE + registry::LOW_ROOTS_RVA.0 + 7 * 8, 0x14000);
    f.word(0x14000 + registry::ENTRIES, 0x15000);
    f.dword(0x14000 + registry::COUNT, 1);
    f.dword(0x14000 + registry::MASK, 7);
    f.put(0x14000 + registry::OVERFLOW_DISTANCE, &[5]);
    f.put(0x15000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let first = ((hash >> 32) as u32 ^ hash as u32) & 7;
    let cell = 0x15000 + first as usize * bucket::STRIDE;
    f.put(cell + bucket::DISTANCE, &[1]);
    f.word(cell + 8, handle);
    f.word(cell + 16, 0x17000);
    f.word(0x17000, (IMAGE + crate::ccharacter::VTABLE_RVA.0) as u64);
    f.word(0x17000 + crate::ccharacter::UNIT_LEADER, 0x18000);
    f.word(0x18000, (IMAGE + engine_native_parts::cnavy_leader::VTABLE_RVA.0) as u64);
    f.dword(0x18000 + crate::cunit_leader::CATEGORY, 2);
    for (index, offset) in [0xf70, 0xf80, 0xfa0, 0xf90].into_iter().enumerate() {
        let definition = 0x1a000 + index * 0x200;
        f.word(0x18000 + offset, definition as u64);
        f.modifier(
            definition + 0xe0,
            definition + 0x100,
            &[(0x46, 3), (0x47, 4), (0x1bb, 6), (0x27e, 5)],
        );
    }
    f
}

const PROVINCE: usize = 0x22000;
const PROVINCE_DEFINITION: usize = 0x23000;
const TERRAIN_SOURCE: usize = 0x24000;
const TERRAIN: usize = 0x25000;
const TERRAIN_ROWS: usize = 0x26000;
const TERRAIN_BUCKET: usize = 0x27000;
const WEATHER: usize = 0x28000;
const FLEET: usize = 0x2c000;
const LEADER: usize = 0x2e000;
const CLASS_ROWS: usize = 0x30000;

fn shared_sources_fixture() -> Fixture {
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    let mut f = fixture();
    f.put(PROVINCE, &vec![0; 0x20000]);
    f.word(TASK + cunit::LOCATION, PROVINCE as u64);
    f.word(PROVINCE, (IMAGE + crate::cprovince::VTABLE_RVA.0) as u64);
    f.word(PROVINCE + 0xc8, PROVINCE_DEFINITION as u64);
    f.word(PROVINCE_DEFINITION + 0x30, TERRAIN_SOURCE as u64);
    f.word(TERRAIN_SOURCE + 0x130, TERRAIN as u64);
    f.word(TERRAIN, (IMAGE + engine_native_parts::cterrain_type::VTABLE_RVA.0) as u64);
    f.put(TERRAIN + 0x10, &[1]);
    f.dword(SHIP + CLASS_DATABASE_ID, 9);
    f.word(TERRAIN + 0x1b8, TERRAIN_BUCKET as u64);
    f.put(TERRAIN + 0x1c8, &[1]);
    f.put(TERRAIN_BUCKET, &[1]);
    f.dword(TERRAIN_BUCKET + 4, 9);
    f.array(TERRAIN + 0x198, TERRAIN_ROWS, 1);
    f.modifier(
        TERRAIN_ROWS + 0x38,
        0x34000,
        &[(0x46, 101), (0x47, 103), (0x1bb, 109), (0x27e, 107)],
    );
    f.word(GAME + cgame_state::WEATHER_MANAGER, WEATHER as u64);
    f.word(
        WEATHER,
        (IMAGE + crate::cweather_manager::VTABLE_RVA.0) as u64,
    );
    f.array(WEATHER + 0x40, 0x29000, 1);
    f.array(WEATHER + 0x370, 0x2a000, 1);
    f.word(SHIP + 0x270, 100000);

    let handle = (2_u64 << 32) | 7;
    f.word(TASK + ctask_force::FLEET, FLEET as u64);
    f.word(FLEET, (IMAGE + crate::cfleet::VTABLE_RVA.0) as u64);
    f.word(FLEET + 0xb0, handle);
    f.word(IMAGE + registry::LOW_ROOTS_RVA.0 + 7 * 8, 0x14000);
    f.word(0x14000 + registry::ENTRIES, 0x15000);
    f.dword(0x14000 + registry::COUNT, 1);
    f.dword(0x14000 + registry::MASK, 7);
    f.put(0x14000 + registry::OVERFLOW_DISTANCE, &[5]);
    f.put(0x15000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let first = ((hash >> 32) as u32 ^ hash as u32) & 7;
    let cell = 0x15000 + first as usize * bucket::STRIDE;
    f.put(cell + bucket::DISTANCE, &[1]);
    f.word(cell + 8, handle);
    f.word(cell + 16, LEADER as u64);
    f.word(LEADER, (IMAGE + engine_native_parts::cnavy_leader::VTABLE_RVA.0) as u64);
    f.modifier(
        LEADER + 0x418,
        0x33000,
        &[(0x46, 11), (0x47, 13), (0x1bb, 19), (0x27e, 17)],
    );
    f.dword(SHIP + 0x60c, 2);
    f.array(LEADER + 0x148, CLASS_ROWS, 2);
    f.dword(CLASS_ROWS + 8, 1);
    f.dword(CLASS_ROWS + 0xd0 + 8, 2);
    f.modifier(
        CLASS_ROWS + 0xd0 + 0x20,
        0x35000,
        &[(0x46, 47), (0x47, 53), (0x1bb, 61), (0x27e, 59)],
    );
    f
}

fn weather_fixture() -> Fixture {
    let mut f = shared_sources_fixture();
    f.modifier(
        COUNTRY + 0x5b8,
        0x6000,
        &[
            (0x46, 100000),
            (0x47, 10000),
            (0x248, -25000),
            (0x27e, 20000),
        ],
    );
    for (address, entries, values) in [
        (0x29000 + 0x58, 0x37000, [50, 60, 70, 80]),
        (0x2a000 + 0x280, 0x37100, [11, 13, 17, 19]),
        (TASK + 0x668, 0x37200, [23, 29, 31, 37]),
    ] {
        f.modifier(
            address,
            entries,
            &[
                (0x46, values[0]),
                (0x47, values[1]),
                (0x1bb, values[3]),
                (0x27e, values[2]),
            ],
        );
    }
    f.array(SHIP + 0x918, 0x36000, 1);
    f.word(0x36000, 0x7400);
    f.dword(0x36008, 1);
    f.modifier(0x7400 + 0x98, 0x36100, &[(0x46, 100000)]);
    f
}

fn current_task_terms(memory: &impl Memory, memo: &mut OrganisationMemo) -> TaskTerms {
    let task = TaskForceFields {
        pointer: TASK,
        vtable: (IMAGE + ctask_force::VTABLE_RVA.0) as u64,
        handle: 0,
        roster_owner: 1,
        location: read::<u64>(memory, TASK + cunit::LOCATION).unwrap() as usize,
        fleet: read::<u64>(memory, TASK + ctask_force::FLEET).unwrap() as usize,
        mine_exposure: 0,
        modifiers: TASK + 0x668,
        influences_count: Some(0),
    };
    read_task_terms(memory, IMAGE, &task, memo)
        .unwrap()
        .unwrap()
}

#[test]
fn organisation_certified_operands_match_direct_after_task_terms_change() {
    use engine_native_memory::read_census::ReadCensus;
    for factor in [100000_i64, 25000, -1, i64::MIN, i64::MAX] {
        let mut f = weather_fixture();
        f.word(SHIP + 0x270, factor as u64);
        f.modifier(SHIP + 0x750, 0x6100, &[(0x46, 33333), (0x1bb, -50000)]);
        f.modifier(0x7000 + 0x1b0, 0x6200, &[(0x46, -33333)]);
        let mut memo = OrganisationMemo::default();
        let before = read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
        let held = memo.organisation_operands(SHIP).unwrap();
        assert!(memo.organisation_operands(SHIP + 8).is_none());

        const MOVED: usize = 0x40000;
        f.word(TASK + cunit::LOCATION, MOVED as u64);
        f.word(MOVED, (IMAGE + crate::cprovince::VTABLE_RVA.0) as u64);
        f.word(MOVED + 0xc8, PROVINCE_DEFINITION as u64);
        f.dword(MOVED + crate::cprovince::ID, 0);
        f.modifier(TASK + 0x668, 0x37200, &[(0x46, 700000), (0x1bb, 25000)]);
        let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
        if factor == 100000 {
            assert_ne!(before, expected);
        }
        let mut memo = OrganisationMemo::default();
        let terms = current_task_terms(&f, &mut memo);
        let census = ReadCensus::new(&f);
        assert_eq!(
            read_maximum_organisation_from_terms(&census, IMAGE, SHIP, &held, &terms, &mut memo),
            Ok(Some(expected)),
        );
        let carried_calls = census.calls();
        assert_eq!(memo.organisation_operands(SHIP), Some(held));

        for address in [
            SHIP,
            SHIP + HANDLE,
            SHIP + CLASS_DATABASE_ID,
            SHIP + 0x270,
            SHIP + BASE_ORGANISATION,
            SHIP + 0x60c,
            SHIP + TASK_FORCE,
            SHIP + 0x750,
            SHIP + 0x8f8,
            SHIP + 0x918,
            0x7000 + 0x1b0,
            0x6100,
            0x6200,
        ] {
            f.bytes.remove(&address);
        }
        let mut memo = OrganisationMemo::default();
        let terms = current_task_terms(&f, &mut memo);
        assert_eq!(
            read_maximum_organisation_from_terms(&f, IMAGE, SHIP, &held, &terms, &mut memo),
            Ok(Some(expected)),
        );
        let fresh = weather_fixture();
        let census = ReadCensus::new(&fresh);
        assert!(maximum_organisation(&census, IMAGE, SHIP).is_ok());
        assert!(carried_calls < census.calls());
    }
}

#[test]
fn organisation_certified_operands_reread_critical_and_class_sources() {
    let mut f = weather_fixture();
    let mut memo = OrganisationMemo::default();
    let before = read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    let held = memo.organisation_operands(SHIP).unwrap();
    f.dword(0x36008, 3);
    f.modifier(0x7400 + 0x98, 0x36100, &[(0x46, 33333), (0x1bb, -50000)]);
    f.modifier(TERRAIN_ROWS + 0x38, 0x34000, &[(0x46, 44444)]);
    f.modifier(CLASS_ROWS + 0xd0 + 0x20, 0x35000, &[(0x46, 55555)]);
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_ne!(before, expected);
    let mut memo = OrganisationMemo::default();
    let terms = current_task_terms(&f, &mut memo);
    assert_eq!(
        read_maximum_organisation_from_terms(&f, IMAGE, SHIP, &held, &terms, &mut memo),
        Ok(Some(expected)),
    );
    for address in [
        0x36008,
        0x36000,
        0x7400 + 0x98,
        0x36100,
        TERRAIN_BUCKET + 4,
        CLASS_ROWS + 0xd0 + 0x20,
    ] {
        let mut faulted = f.clone();
        faulted.bytes.remove(&address);
        let expected = maximum_organisation(&faulted, IMAGE, SHIP).unwrap_err();
        let mut memo = OrganisationMemo::default();
        let terms = current_task_terms(&faulted, &mut memo);
        assert_eq!(
            read_maximum_organisation_from_terms(&faulted, IMAGE, SHIP, &held, &terms, &mut memo),
            Err(expected),
            "uncertified source {address:#x}",
        );
        assert!(memo.organisation_operands(SHIP).is_none());
    }
}

#[test]
fn organisation_certified_operands_keep_the_second_critical_observation() {
    struct SecondCountFault<'a> {
        source: &'a Fixture,
        counts: Cell<usize>,
    }
    impl Memory for SecondCountFault<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address == 0x36008 {
                self.counts.set(self.counts.get() + 1);
                if self.counts.get() == 2 {
                    return false;
                }
            }
            self.source.copy(address, out)
        }
    }
    let f = weather_fixture();
    let mut memo = OrganisationMemo::default();
    read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    let held = memo.organisation_operands(SHIP).unwrap();
    let direct = SecondCountFault {
        source: &f,
        counts: Cell::new(0),
    };
    assert_eq!(
        maximum_organisation(&direct, IMAGE, SHIP),
        Err("naval field unreadable")
    );
    assert_eq!(direct.counts.get(), 2);
    let carried = SecondCountFault {
        source: &f,
        counts: Cell::new(0),
    };
    let mut memo = OrganisationMemo::default();
    let terms = current_task_terms(&carried, &mut memo);
    assert_eq!(
        read_maximum_organisation_from_terms(&carried, IMAGE, SHIP, &held, &terms, &mut memo),
        Err("naval field unreadable"),
    );
    assert_eq!(carried.counts.get(), 2);
}

#[test]
fn organisation_certified_operands_decline_newly_reached_ship_selectors() {
    let mut f = weather_fixture();
    f.word(TASK + cunit::LOCATION, 0);
    f.word(TASK + ctask_force::FLEET, 0);
    let mut memo = OrganisationMemo::default();
    read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    let held = memo.organisation_operands(SHIP).unwrap();
    assert!(held.class.is_none() && held.weather_factor.is_none() && held.subunit.is_none());
    for (location, fleet) in [(PROVINCE, 0), (0, FLEET), (PROVINCE, FLEET)] {
        f.word(TASK + cunit::LOCATION, location as u64);
        f.word(TASK + ctask_force::FLEET, fleet as u64);
        let mut memo = OrganisationMemo::default();
        let terms = current_task_terms(&f, &mut memo);
        let calls = f.calls.get();
        assert_eq!(
            read_maximum_organisation_from_terms(&f, IMAGE, SHIP, &held, &terms, &mut memo),
            Ok(None),
        );
        assert_eq!(f.calls.get(), calls, "fallback precedes native reads");
    }
    let mut memo = OrganisationMemo::default();
    read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    f.bytes.remove(&(SHIP + BASE_ORGANISATION));
    assert!(read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).is_err());
    assert!(memo.organisation_operands(SHIP).is_none());
}

#[test]
fn organisation_certified_operands_apply_current_own_only_feature_eligibility() {
    use engine_native_memory::read_census::ReadCensus;
    let mut f = fixture();
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1bb, -50000)]);
    f.array(SHIP + 0x918, 0x6300, 1);
    f.word(0x6300, 0x7400);
    f.dword(0x6308, 1);
    f.modifier(0x7400 + 0x98, 0x6400, &[(0x46, 100000)]);
    f.dword(0x9000 + 0x1bb * md::STRIDE + md::FEATURE_SELECTOR, 4);
    f.word(
        IMAGE + md::ELIGIBILITY_CALLBACK_RVA.0,
        (IMAGE + md::ELIGIBILITY_FLAGS_GETTER_RVA.0) as u64,
    );
    f.dword(IMAGE + md::TRADE_FEATURE_FLAGS_RVA.0, 0);
    let mut memo = OrganisationMemo::default();
    let before = read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    let held = memo.organisation_operands(SHIP).unwrap();
    let old_terms = current_task_terms(&f, &mut OrganisationMemo::default());
    f.dword(IMAGE + md::TRADE_FEATURE_FLAGS_RVA.0, 4);
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_ne!(before, expected);
    let mut memo = OrganisationMemo::default();
    let terms = current_task_terms(&f, &mut memo);
    assert_eq!(
        terms, old_terms,
        "the changed key is absent from task terms"
    );
    let census = ReadCensus::new(&f);
    assert_eq!(
        organisation_modifier_guard_matches(&census, IMAGE, held.modifier_guard(), &mut memo),
        Ok(false)
    );
    let calls = census.calls();
    assert!(calls > 0);
    assert_eq!(
        organisation_modifier_guard_matches(&census, IMAGE, held.modifier_guard(), &mut memo),
        Ok(false)
    );
    assert_eq!(census.calls(), calls, "a guard is checked once per capture");
    assert_eq!(
        read_maximum_organisation_from_terms(&f, IMAGE, SHIP, &held, &terms, &mut memo),
        Ok(Some(expected)),
    );
    let current = memo.organisation_operands(SHIP).unwrap();
    assert_ne!(current.modifier_guard(), held.modifier_guard());
    assert_eq!(
        organisation_modifier_guard_matches(&f, IMAGE, current.modifier_guard(), &mut memo),
        Ok(true)
    );

    let mut faulted = f.clone();
    faulted.bytes.remove(&(IMAGE + md::TRADE_FEATURE_FLAGS_RVA.0));
    let mut memo = OrganisationMemo::default();
    let terms = current_task_terms(&faulted, &mut memo);
    assert_eq!(
        organisation_modifier_guard_matches(&faulted, IMAGE, held.modifier_guard(), &mut memo),
        Err("modifier feature flags unreadable")
    );
    assert_eq!(
        read_maximum_organisation_from_terms(&faulted, IMAGE, SHIP, &held, &terms, &mut memo),
        maximum_organisation(&faulted, IMAGE, SHIP).map(Some),
    );
    let mut memo = OrganisationMemo::default();
    let census = ReadCensus::new(&faulted);
    assert_eq!(
        organisation_modifier_guard_matches(&census, IMAGE, 0, &mut memo),
        Ok(true)
    );
    assert_eq!(census.calls(), 0, "absent own keys need no metadata");
}

#[test]
fn organisation_weather_packet_preserves_every_ship_factor_and_critical_operand() {
    let mut f = weather_fixture();
    let mut memo = OrganisationMemo::default();
    for factor in [100000_i64, 25000, 0, -1, -200000, i64::MIN, i64::MAX] {
        f.word(SHIP + 0x270, factor as u64);
        let scale = factor.wrapping_sub(100000).wrapping_add(75000);
        let mut total = [100159_i64, 10169, 20183, 189];
        for ((total, regional), local) in total
            .iter_mut()
            .zip([50_i64, 60, 70, 80])
            .zip([34, 42, 48, 56])
        {
            *total = total
                .wrapping_add(regional.wrapping_mul(scale) / 100000)
                .wrapping_add(local);
        }
        total[0] =
            total[0].wrapping_add(100000_i64.wrapping_mul(total[3].wrapping_add(100000)) / 100000);
        let expected = organisation_maximum(10_000_000, [total[0], total[1], total[2]]);
        assert_eq!(
            read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
            Ok(expected),
            "factor {factor}"
        );
        assert_eq!(
            maximum_organisation(&f, IMAGE, SHIP),
            Ok(expected),
            "fresh factor {factor}"
        );
    }
    f.word(TASK + cunit::LOCATION, 0);
    f.bytes.remove(&WEATHER);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        maximum_organisation(&f, IMAGE, SHIP)
    );
    f.word(TASK + cunit::LOCATION, PROVINCE as u64);
    f.word(
        WEATHER,
        (IMAGE + crate::cweather_manager::VTABLE_RVA.0) as u64,
    );
    f.word(SHIP + 0x270, 100000);
    f.modifier(0x29000 + 0x58, 0x37000, &[(0x47, 100000)]);
    memo = OrganisationMemo::default();
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        maximum_organisation(&f, IMAGE, SHIP)
    );
    let weather = memo.weather.as_ref().unwrap();
    assert_eq!(weather.region, [0, 100000, 0, 0]);
    assert_eq!(
        weather.sources,
        [GAME, TASK, COUNTRY, PROVINCE, PROVINCE_DEFINITION]
    );
}

#[test]
fn organisation_weather_packet_removes_graph_reads_and_thirteen_existing_snapshot_lookups() {
    let f = weather_fixture();
    let census = engine_native_memory::read_census::ReadCensus::new(&f);
    let snapshot = Snapshotted {
        source: &census,
        arena: RefCell::new(modifier_accumulator::SnapshotArena::new(
            modifier_accumulator::SnapshotCapacity {
                nodes: 16,
                entries: 64,
            },
        )),
        lookups: Cell::new(0),
    };
    let mut memo = OrganisationMemo::default();
    let expected = read_maximum_organisation_memo(&snapshot, IMAGE, SHIP, &mut memo).unwrap();
    census.reset();
    snapshot.lookups.set(0);
    assert_eq!(
        read_maximum_organisation_memo(&snapshot, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    let warm = (
        census.calls(),
        census.spans(),
        census.bytes(),
        snapshot.lookups.get(),
    );
    memo.weather = None;
    census.reset();
    snapshot.lookups.set(0);
    assert_eq!(
        read_maximum_organisation_memo(&snapshot, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    assert_eq!(
        (
            census.calls() - warm.0,
            census.spans() - warm.1,
            census.bytes() - warm.2,
            snapshot.lookups.get() - warm.3
        ),
        (6, 6, 56, 13)
    );
}

#[test]
fn organisation_weather_packet_keeps_zero_scale_reads_and_never_stores_failed_graphs() {
    let base = weather_fixture();
    for factor in [25000_i64, 0, -200000] {
        for (failed, reason) in [
            (
                GAME + cgame_state::WEATHER_MANAGER,
                "naval field unreadable",
            ),
            (WEATHER, "naval field unreadable"),
            (PROVINCE_DEFINITION + 0x58, "naval field unreadable"),
            (WEATHER + 0x40, "ship organisation weather regions invalid"),
            (SHIP + 0x270, "naval field unreadable"),
            (
                0x29000 + 0x58,
                "modifier accumulator header unreadable or invalid",
            ),
            (0x37000 + 3 * 16, "modifier accumulator entry unreadable"),
            (
                WEATHER + 0x370,
                "ship organisation weather provinces invalid",
            ),
            (PROVINCE + crate::cprovince::ID, "naval field unreadable"),
            (
                0x2a000 + 0x280,
                "modifier accumulator header unreadable or invalid",
            ),
            (
                TASK + 0x668,
                "modifier accumulator header unreadable or invalid",
            ),
            (0x37200 + 3 * 16, "modifier accumulator entry unreadable"),
        ] {
            let mut f = base.clone();
            f.word(SHIP + 0x270, factor as u64);
            f.bytes.remove(&failed);
            let mut memo = OrganisationMemo::default();
            assert_eq!(
                read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
                Err(reason),
                "factor {factor}, source {failed:#x}"
            );
            assert!(
                memo.weather.is_none(),
                "factor {factor}, source {failed:#x}"
            );
        }
    }
    for (earlier, later, reason) in [
        (SHIP + 0x270, 0x29000 + 0x58, "naval field unreadable"),
        (
            0x29000 + 0x58,
            PROVINCE + crate::cprovince::ID,
            "modifier accumulator header unreadable or invalid",
        ),
        (
            WEATHER + 0x370,
            TASK + 0x668,
            "ship organisation weather provinces invalid",
        ),
    ] {
        let mut f = base.clone();
        f.bytes.remove(&earlier);
        f.bytes.remove(&later);
        let mut memo = OrganisationMemo::default();
        assert_eq!(
            read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
            Err(reason)
        );
        assert!(memo.weather.is_none());
    }
}

#[test]
fn organisation_weather_packet_rejects_changed_location_and_definition_sources() {
    for change_definition in [true, false] {
        let mut f = weather_fixture();
        let mut memo = OrganisationMemo::default();
        assert!(read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).is_ok());
        const OTHER: usize = 0x40000;
        if change_definition {
            f.word(PROVINCE + 0xc8, OTHER as u64);
            f.word(OTHER + 0x30, TERRAIN_SOURCE as u64);
            f.dword(OTHER + 0x58, 1);
            memo.location = None;
        } else {
            f.word(TASK + cunit::LOCATION, OTHER as u64);
            f.word(OTHER, (IMAGE + crate::cprovince::VTABLE_RVA.0) as u64);
            f.word(OTHER + 0xc8, PROVINCE_DEFINITION as u64);
            f.dword(OTHER + crate::cprovince::ID, 1);
        }
        let expected = if change_definition {
            "ship organisation weather region missing"
        } else {
            "ship organisation weather province missing"
        };
        assert_eq!(
            read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
            Err(expected)
        );
        assert_eq!(
            memo.weather.as_ref().unwrap().sources,
            [GAME, TASK, COUNTRY, PROVINCE, PROVINCE_DEFINITION]
        );
    }
}

#[test]
fn organisation_shared_selectors_remove_complete_scans_without_replacing_ship_terms() {
    use engine_native_memory::read_census::ReadCensus;
    let mut f = shared_sources_fixture();
    let mut memo = OrganisationMemo::default();
    let expected = organisation_maximum(10_000_000, [100159, 10169, 20183]);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    let census = ReadCensus::new(&f);
    assert_eq!(
        read_maximum_organisation_memo(&census, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    let warm = (census.calls(), census.spans(), census.bytes());
    memo.terrain = Recent::default();
    memo.terrain_class = Recent::default();
    memo.fleet = None;
    memo.leader_class = Recent::default();
    census.reset();
    assert_eq!(
        read_maximum_organisation_memo(&census, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    assert_eq!(
        (
            census.calls() - warm.0,
            census.spans() - warm.1,
            census.bytes() - warm.2
        ),
        (25, 34, 405),
    );

    let snapshot = Snapshotted {
        source: &census,
        lookups: Cell::new(0),
        arena: RefCell::new(modifier_accumulator::SnapshotArena::new(
            modifier_accumulator::SnapshotCapacity {
                nodes: 16,
                entries: 64,
            },
        )),
    };
    memo = OrganisationMemo::default();
    assert_eq!(
        read_maximum_organisation_memo(&snapshot, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    census.reset();
    assert_eq!(
        read_maximum_organisation_memo(&snapshot, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    let warm = (census.calls(), census.spans(), census.bytes());
    memo.terrain = Recent::default();
    memo.terrain_class = Recent::default();
    memo.fleet = None;
    memo.leader_class = Recent::default();
    census.reset();
    assert_eq!(
        read_maximum_organisation_memo(&snapshot, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    assert_eq!(
        (
            census.calls() - warm.0,
            census.spans() - warm.1,
            census.bytes() - warm.2
        ),
        (19, 19, 165),
    );

    f.dword(SHIP + 0x60c, 1);
    f.dword(SHIP + CLASS_DATABASE_ID, 8);
    f.word(SHIP + BASE_ORGANISATION, 4_000_003);
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    assert_eq!(memo.terrain_class.get(&(TERRAIN, 8)), Some([0; 4]));
    assert_eq!(memo.leader_class.get(&(LEADER, 1)), Some([0; 4]));

    f.word(TASK + ctask_force::FLEET, 0);
    f.word(TASK + cunit::LOCATION, 0);
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    assert_eq!(memo.fleet, Some((0, None, [0; 4])));

    f.word(TASK + ctask_force::FLEET, FLEET as u64);
    f.word(TASK + cunit::LOCATION, PROVINCE as u64);
    f.modifier(LEADER + 0x418, 0x33000, &[(0x46, -1111)]);
    memo = OrganisationMemo::default();
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(expected)
    );
    assert_eq!(memo.fleet, Some((FLEET, Some(LEADER), [-1111, 0, 0, 0])));
}

#[test]
fn organisation_memo_keeps_recent_selectors_across_alternating_ships() {
    use super::gathered::FaultAt;
    const OTHER_SHIP: usize = 0xa000;
    let mut f = shared_sources_fixture();
    for offset in 0..SIZE {
        let byte = f.bytes[&(SHIP + offset)];
        f.bytes.insert(OTHER_SHIP + offset, byte);
    }
    f.word(OTHER_SHIP + HANDLE, (43 << 32) | 51);
    f.dword(OTHER_SHIP + CLASS_DATABASE_ID, 8);
    f.dword(OTHER_SHIP + 0x60c, 1);
    let trace = FaultAt::new(&f, None);
    let mut memo = OrganisationMemo::default();
    let first = read_maximum_organisation_memo(&trace, IMAGE, SHIP, &mut memo);
    assert!(first.is_ok());
    assert_eq!(
        read_maximum_organisation_memo(&trace, IMAGE, OTHER_SHIP, &mut memo),
        maximum_organisation(&f, IMAGE, OTHER_SHIP)
    );
    trace.reads.borrow_mut().clear();
    assert_eq!(
        read_maximum_organisation_memo(&trace, IMAGE, SHIP, &mut memo),
        first
    );
    for (name, source) in [
        ("pride ship", COUNTRY + 0x250),
        ("terrain class table", TERRAIN + 0x1b8),
        ("commander class array", LEADER + 0x148),
    ] {
        assert!(
            !trace
                .reads
                .borrow()
                .iter()
                .any(|&(address, len)| (address..address + len).contains(&source)),
            "{name} read again"
        );
    }
}

#[test]
fn organisation_shared_sources_never_cache_failed_terrain_fleet_or_class_observations() {
    for failed in [
        TERRAIN_SOURCE + 0x130,
        TERRAIN_BUCKET + 8,
        FLEET,
        LEADER + 0x418,
        CLASS_ROWS + 0xd0 + 8,
    ] {
        let mut f = shared_sources_fixture();
        let original = f.bytes.remove(&failed).unwrap();
        let mut memo = OrganisationMemo::default();
        let expected = maximum_organisation(&f, IMAGE, SHIP);
        assert!(expected.is_err());
        assert_eq!(
            read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
            expected
        );
        match failed {
            p if p == TERRAIN_SOURCE + 0x130 => assert!(memo.terrain.is_empty()),
            p if p == TERRAIN_BUCKET + 8 => assert!(memo.terrain_class.is_empty()),
            FLEET => assert!(memo.fleet.is_none()),
            p if p == LEADER + 0x418 => assert!(memo.fleet.is_none()),
            _ => assert!(memo.leader_class.is_empty()),
        }
        f.bytes.insert(failed, original);
        assert_eq!(
            read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
            maximum_organisation(&f, IMAGE, SHIP),
        );
    }
    let mut f = shared_sources_fixture();
    f.dword(SHIP + 0x60c, 2);
    f.dword(CLASS_ROWS + 8, 2);
    let mut memo = OrganisationMemo::default();
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Err("naval commander class order invalid"),
    );
    assert!(memo.leader_class.is_empty());
}

#[test]
fn organisation_memo_shares_country_and_support_but_reads_current_ship_terms() {
    let mut f = support_fixture();
    let mut memo = OrganisationMemo::default();
    let first = read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    assert_eq!(
        first,
        organisation_maximum(10_000_000, [100012, 10016, 20020])
    );
    assert_eq!(memo.country, Some((COUNTRY, [100000, 10000, 20000, 0])));
    assert_eq!(memo.support, Some((TASK, [12, 16, 20, 24])));
    let cold = f.calls.replace(0);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(first)
    );
    assert!(f.calls.get() < cold);

    f.word(SHIP + BASE_ORGANISATION, 5_000_001);
    f.modifier(SHIP + 0x750, 0x6100, &[(0x46, 33333), (0x1bb, -50000)]);
    f.modifier(0x7000 + 0x1b0, 0x6200, &[(0x47, 777)]);
    f.array(SHIP + 0x918, 0x6300, 1);
    f.word(0x6300, 0x7400);
    f.dword(0x6308, 3);
    f.modifier(0x7400 + 0x98, 0x6400, &[(0x46, 33333)]);
    let current = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_ne!(current, first);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(current)
    );

    const OTHER_TASK: usize = 0x22000;
    f.put(OTHER_TASK, &[0; 0x900]);
    f.word(OTHER_TASK, (IMAGE + ctask_force::VTABLE_RVA.0) as u64);
    f.dword(OTHER_TASK + cunit::ROSTER_OWNER, 1);
    f.word(OTHER_TASK + 0x640, 0x8000);
    f.word(SHIP + TASK_FORCE, OTHER_TASK as u64);
    let other = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_ne!(other, current);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(other)
    );
    assert_eq!(memo.support, Some((OTHER_TASK, [0; 4])));

    const OTHER_COUNTRY: usize = 0x24000;
    f.put(OTHER_COUNTRY, &[0; 0xc00]);
    f.array(GAME + cgame_state::COUNTRIES, 0x4000, 3);
    f.array(GAME + cgame_state::COUNTRY_LINKS, 0x4100, 3);
    f.word(0x4010, OTHER_COUNTRY as u64);
    f.dword(0x4108, 2);
    f.word(OTHER_COUNTRY, (IMAGE + ccountry::VTABLE_RVA.0) as u64);
    f.dword(OTHER_COUNTRY + ccountry::ID, 2);
    f.dword(OTHER_TASK + cunit::ROSTER_OWNER, 2);
    let other = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(other)
    );
    assert_eq!(memo.country, Some((OTHER_COUNTRY, [0; 4])));
}

#[test]
fn organisation_memo_reacquires_mutable_terms_and_never_retains_failed_support() {
    let mut f = support_fixture();
    let mut memo = OrganisationMemo::default();
    let first = read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo).unwrap();
    f.modifier(COUNTRY + 0x5b8, 0x6000, &[(0x46, -33333), (0x47, 777)]);
    f.dword(METADATA + 0x46 * md::STRIDE + md::FLAGS, md::CLAMP_MAXIMUM);
    f.modifier(0x1a000 + 0xe0, 0x1a000 + 0x100, &[(0x46, 55555)]);
    memo = OrganisationMemo::default();
    let current = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    assert_ne!(current, first);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(current)
    );

    memo = OrganisationMemo::default();
    let failed = 0x1a600 + 0xe0;
    let byte = f.bytes.remove(&failed).unwrap();
    let expected = Err("modifier accumulator header unreadable or invalid");
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        expected
    );
    assert!(memo.support.is_none());
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        expected
    );
    assert!(memo.support.is_none());
    f.bytes.insert(failed, byte);
    assert_eq!(
        read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo),
        Ok(current)
    );
    assert!(memo.support.is_some());
}

const NATIVE_KEYS: [u32; 4] = [0x46, 0x47, 0x27e, 0x1bb];
const ACCUMULATOR: usize = COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
const ENTRIES: usize = 0x6000;
const METADATA: usize = 0x9000;

fn scalar_values(memory: &impl Memory) -> Result<[i64; 4], &'static str> {
    let mut values = [0; 4];
    for (value, key) in values.iter_mut().zip(NATIVE_KEYS) {
        *value =
            modifier_accumulator::lookup_effective_at(memory, IMAGE, ACCUMULATOR, key)?.effective;
    }
    Ok(values)
}

struct Hooked<'a> {
    source: &'a Fixture,
    values: BTreeMap<u32, Result<Value, &'static str>>,
    hooks: RefCell<Vec<u32>>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl<'a> Hooked<'a> {
    fn new(source: &'a Fixture) -> Self {
        Self {
            source,
            values: BTreeMap::new(),
            hooks: RefCell::new(Vec::new()),
            reads: RefCell::new(Vec::new()),
        }
    }
}
impl Memory for Hooked<'_> {
    fn copy(&self, p: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((p, out.len()));
        self.source.copy(p, out)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        assert_eq!(address, ACCUMULATOR);
        self.hooks.borrow_mut().push(key);
        self.values.get(&key).copied()
    }
}

#[test]
fn organisation_batch_keeps_native_order_and_removes_three_full_censuses() {
    let mut f = fixture();
    let entries = [
        (1, 99),
        (0x46, -13),
        (0x47, 15),
        (0x1bb, -33333),
        (0x27e, 17),
        (0x300, i64::MAX),
    ];
    f.modifier(ACCUMULATOR, ENTRIES, &entries);
    let scalar = scalar_values(&f);
    let scalar_reads = f.calls.replace(0);
    let scalar_bytes = f.copied.replace(0);
    assert_eq!(scalar, Ok([-13, 15, 17, -33333]));
    assert_eq!(
        organisation_modifier_values(
            &f,
            IMAGE,
            ACCUMULATOR,
            &engine_native_parts::modifier_definition::DefinitionMemo::default()
        ),
        scalar
    );
    let saved_reads = 3 * (entries.len() + 1);
    assert_eq!(scalar_reads - f.calls.get(), saved_reads);
    assert_eq!(scalar_bytes - f.copied.get(), saved_reads * 16);
}

#[test]
fn organisation_batch_preserves_every_mixed_hook_pattern_and_metadata_order() {
    let mut f = fixture();
    f.modifier(
        ACCUMULATOR,
        ENTRIES,
        &[(0x46, 7), (0x47, 11), (0x1bb, 13), (0x27e, 17)],
    );
    for mask in 0..16 {
        let mut memory = Hooked::new(&f);
        let mut expected = [7, 11, 17, 13];
        for (index, key) in NATIVE_KEYS.into_iter().enumerate() {
            if mask & (1 << index) != 0 {
                expected[index] = 100 + index as i64;
                memory.values.insert(
                    key,
                    Ok(Value {
                        raw: expected[index],
                        present: true,
                    }),
                );
            }
        }
        assert_eq!(
            organisation_modifier_values(
                &memory,
                IMAGE,
                ACCUMULATOR,
                &engine_native_parts::modifier_definition::DefinitionMemo::default()
            ),
            Ok(expected),
            "hook mask {mask}"
        );
        assert_eq!(*memory.hooks.borrow(), NATIVE_KEYS);
        let reads = memory.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|&&(p, n)| p == ACCUMULATOR && n == 16)
                .count(),
            if mask == 15 { 0 } else { 1 },
            "one full census only when a hook declines"
        );
        let selectors: Vec<_> = reads
            .iter()
            .filter_map(|&(p, n)| {
                NATIVE_KEYS.into_iter().find(|&key| {
                    n == 4 && p == METADATA + key as usize * md::STRIDE + md::FEATURE_SELECTOR
                })
            })
            .collect();
        assert_eq!(selectors, NATIVE_KEYS, "hook mask {mask}");
    }
}

#[test]
fn organisation_batch_absence_skips_metadata_but_present_zero_does_not() {
    let mut f = fixture();
    f.bytes.remove(&(IMAGE + md::TABLE_RVA.0));
    for entries in [&[][..], &[(1, i64::MIN), (0x300, i64::MAX)][..]] {
        f.modifier(ACCUMULATOR, ENTRIES, entries);
        assert_eq!(
            organisation_modifier_values(
                &f,
                IMAGE,
                ACCUMULATOR,
                &engine_native_parts::modifier_definition::DefinitionMemo::default()
            ),
            Ok([0; 4])
        );
        assert_eq!(scalar_values(&f), Ok([0; 4]));
    }
    f.modifier(ACCUMULATOR, ENTRIES, &[(0x46, 0)]);
    assert_eq!(
        organisation_modifier_values(
            &f,
            IMAGE,
            ACCUMULATOR,
            &engine_native_parts::modifier_definition::DefinitionMemo::default()
        ),
        Err("modifier metadata table unreadable or null")
    );
    let mut memory = Hooked::new(&f);
    memory.values.insert(
        0x46,
        Ok(Value {
            raw: i64::MAX,
            present: false,
        }),
    );
    assert_eq!(
        organisation_modifier_values(
            &memory,
            IMAGE,
            ACCUMULATOR,
            &engine_native_parts::modifier_definition::DefinitionMemo::default()
        ),
        Ok([0; 4])
    );
}

#[test]
fn organisation_batch_hook_errors_stop_in_order_even_after_a_direct_census() {
    let f = fixture();
    for (index, key) in NATIVE_KEYS.into_iter().enumerate() {
        let mut memory = Hooked::new(&f);
        memory.values.insert(key, Err("snapshot fault"));
        assert_eq!(
            organisation_modifier_values(
                &memory,
                IMAGE,
                ACCUMULATOR,
                &engine_native_parts::modifier_definition::DefinitionMemo::default()
            ),
            Err("snapshot fault")
        );
        assert_eq!(&*memory.hooks.borrow(), &NATIVE_KEYS[..=index]);
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|&&(p, n)| p == ACCUMULATOR && n == 16)
                .count(),
            if index == 0 { 0 } else { 1 }
        );
    }
}

#[test]
fn organisation_batch_metadata_failure_precedes_later_hook_failure() {
    for failed_index in [0, 2] {
        let mut f = fixture();
        f.bytes.remove(
            &(METADATA + NATIVE_KEYS[failed_index] as usize * md::STRIDE + md::FEATURE_SELECTOR),
        );
        for use_hooks in [false, true] {
            let mut memory = Hooked::new(&f);
            if use_hooks {
                for &key in &NATIVE_KEYS[..=failed_index] {
                    memory.values.insert(
                        key,
                        Ok(Value {
                            raw: 0,
                            present: true,
                        }),
                    );
                }
            }
            memory
                .values
                .insert(NATIVE_KEYS[failed_index + 1], Err("later snapshot fault"));
            assert_eq!(
                organisation_modifier_values(
                    &memory,
                    IMAGE,
                    ACCUMULATOR,
                    &engine_native_parts::modifier_definition::DefinitionMemo::default()
                ),
                Err("modifier selector unreadable")
            );
            assert_eq!(&*memory.hooks.borrow(), &NATIVE_KEYS[..=failed_index]);
        }
    }
}

#[test]
fn organisation_batch_rejects_bad_headers_suffixes_and_entry_faults_before_metadata() {
    for fault in 0..7 {
        let mut f = fixture();
        f.modifier(
            ACCUMULATOR,
            ENTRIES,
            &[(0x46, 7), (0x47, 11), (0x1bb, 13), (0x27e, 17), (0x300, 19)],
        );
        f.bytes.remove(&(IMAGE + md::TABLE_RVA.0));
        let expected = match fault {
            0 => {
                f.bytes.remove(&(ACCUMULATOR + 15));
                "modifier accumulator header unreadable or invalid"
            }
            1 => {
                f.dword(ACCUMULATOR + 8, 4);
                "modifier accumulator header unreadable or invalid"
            }
            2 => {
                f.word(ACCUMULATOR, 0);
                "modifier accumulator header unreadable or invalid"
            }
            3 | 4 => {
                f.dword(ENTRIES + 4 * 16, if fault == 3 { 0 } else { 0x27e });
                "modifier accumulator keys not positive and increasing"
            }
            5 => {
                f.bytes.remove(&(ENTRIES + 4 * 16 + 15));
                "modifier accumulator entry unreadable"
            }
            _ => {
                f.dword(ENTRIES, 0);
                f.bytes.remove(&(ENTRIES + 4 * 16 + 15));
                "modifier accumulator keys not positive and increasing"
            }
        };
        assert_eq!(
            organisation_modifier_values(
                &f,
                IMAGE,
                ACCUMULATOR,
                &engine_native_parts::modifier_definition::DefinitionMemo::default()
            ),
            Err(expected)
        );
        assert_eq!(scalar_values(&f), Err(expected));
    }
}

#[test]
fn organisation_batch_preserves_metadata_rules_and_full_graph_fixed5_results() {
    struct Scalar<'a>(&'a Fixture);
    impl Memory for Scalar<'_> {
        fn copy(&self, p: usize, out: &mut [u8]) -> bool {
            self.0.copy(p, out)
        }
        fn modifier_accumulator_value(
            &self,
            address: usize,
            key: u32,
        ) -> Option<Result<Value, &'static str>> {
            Some(modifier_accumulator::lookup_at(self.0, address, key))
        }
    }
    let mut f = fixture();
    assert_eq!(maximum_organisation(&f, IMAGE, SHIP), Ok(13_130_000));
    assert_eq!(
        maximum_organisation(&Scalar(&f), IMAGE, SHIP),
        Ok(13_130_000)
    );
    f.modifier(
        ACCUMULATOR,
        ENTRIES,
        &[(0x46, 200001), (0x47, -1), (0x1bb, -50000), (0x27e, 20000)],
    );
    f.dword(METADATA + 0x46 * md::STRIDE + md::FLAGS, md::CLAMP_MAXIMUM);
    f.dword(METADATA + 0x47 * md::STRIDE + md::FEATURE_SELECTOR, 4);
    f.word(
        IMAGE + md::ELIGIBILITY_CALLBACK_RVA.0,
        (IMAGE + md::ELIGIBILITY_FLAGS_GETTER_RVA.0) as u64,
    );
    for enabled in [false, true] {
        f.dword(
            IMAGE + md::TRADE_FEATURE_FLAGS_RVA.0,
            if enabled { 4 } else { 0 },
        );
        assert_eq!(
            organisation_modifier_values(
                &f,
                IMAGE,
                ACCUMULATOR,
                &engine_native_parts::modifier_definition::DefinitionMemo::default()
            ),
            Ok([100000, if enabled { -1 } else { 0 }, 20000, -50000])
        );
        assert_eq!(
            organisation_modifier_values(
                &f,
                IMAGE,
                ACCUMULATOR,
                &engine_native_parts::modifier_definition::DefinitionMemo::default()
            ),
            scalar_values(&f)
        );
    }
    f.array(SHIP + 0x918, 0x6300, 2);
    for (i, p) in [0x7400, 0x7600].into_iter().enumerate() {
        f.word(0x6300 + i * 16, p as u64);
        f.dword(0x6308 + i * 16, if i == 0 { 1 } else { (-2_i32) as u32 });
        f.modifier(p + 0x98, 0x6400 + i * 32, &[(0x46, 1)]);
    }
    for (base, additive, scale) in [
        (10_000_001_i64, 33333_i64, -50000_i64),
        (-200000, -33333, 0),
        (i64::MAX, i64::MAX, 33333),
        (i64::MIN, i64::MIN, -100001),
    ] {
        f.word(SHIP + BASE_ORGANISATION, base as u64);
        f.modifier(SHIP + 0x750, 0x6100, &[(0x46, additive), (0x1bb, scale)]);
        let batch =
            maximum_organisation(&f, IMAGE, SHIP).expect("complete batched organisation graph");
        let scalar = maximum_organisation(&Scalar(&f), IMAGE, SHIP)
            .expect("complete scalar organisation graph");
        assert_eq!(
            batch, scalar,
            "base={base}, additive={additive}, critical scale={scale}"
        );
    }
}

#[test]
fn derives_country_ship_experience_and_per_entry_critical_terms_with_fixed5_clamp() {
    let mut f = fixture();
    assert_eq!(maximum_organisation(&f, IMAGE, SHIP), Ok(13_130_000));
    f.modifier(SHIP + 0x750, 0x6100, &[(0x46, 2), (0x1bb, -50000)]);
    f.modifier(0x7000 + 0x1b0, 0x6200, &[(0x46, 3)]);
    f.array(SHIP + 0x918, 0x6300, 2);
    for (i, p) in [0x7400, 0x7600].into_iter().enumerate() {
        f.word(0x6300 + i * 16, p as u64);
        f.dword(0x6308 + i * 16, 1);
        f.modifier(p + 0x98, 0x6400 + i * 32, &[(0x46, 1)]);
    }
    assert_eq!(maximum_organisation(&f, IMAGE, SHIP), Ok(13_130_006));
    f.word(SHIP + BASE_ORGANISATION, (-200000i64) as u64);
    assert_eq!(maximum_organisation(&f, IMAGE, SHIP), Ok(100));
    f.dword(SHIP + 0x924, 4097);
    assert!(maximum_organisation(&f, IMAGE, SHIP).is_err());
}

#[test]
fn organisation_snapshot_reuses_raw_arrays_but_evaluates_current_metadata() {
    use modifier_accumulator::{SnapshotArena, SnapshotCapacity};
    struct Source<'a> {
        fixture: &'a Fixture,
        clamp: Cell<bool>,
    }
    impl Memory for Source<'_> {
        fn copy(&self, p: usize, out: &mut [u8]) -> bool {
            if !self.fixture.copy(p, out) {
                return false;
            }
            if p == METADATA + 0x46 * md::STRIDE + md::FLAGS && self.clamp.get() {
                out.copy_from_slice(&md::CLAMP_MAXIMUM.to_le_bytes());
            }
            true
        }
    }
    let mut f = fixture();
    f.modifier(
        ACCUMULATOR,
        ENTRIES,
        &[(0x46, 200001), (0x47, 10000), (0x27e, 20000)],
    );
    let source = Source {
        fixture: &f,
        clamp: Cell::new(false),
    };
    let memory = Snapshotted {
        source: &source,
        lookups: Cell::new(0),
        arena: RefCell::new(SnapshotArena::new(SnapshotCapacity {
            nodes: 4,
            entries: 8,
        })),
    };
    assert_eq!(maximum_organisation(&memory, IMAGE, SHIP), Ok(13_260_001));
    let first_reads = f.calls.replace(0);
    assert_eq!(maximum_organisation(&memory, IMAGE, SHIP), Ok(13_260_001));
    assert!(
        f.calls.get() < first_reads,
        "a repeated derivation reuses every raw accumulator"
    );
    source.clamp.set(true);
    assert_eq!(maximum_organisation(&memory, IMAGE, SHIP), Ok(13_130_000));
    assert_eq!(maximum_organisation(&source, IMAGE, SHIP), Ok(13_130_000));
}
#[test]
fn rejects_unreadable_sources_bad_identity_and_doctrine_cycles() {
    for p in [SHIP, TASK, COUNTRY, 0x7000 + 0x1b0, TASK + 0x640] {
        let mut f = fixture();
        f.bytes.remove(&p);
        assert!(maximum_organisation(&f, IMAGE, SHIP).is_err());
    }
    let mut f = fixture();
    f.word(TASK + 0x648, 1);
    f.word(0x8000, 0x8100);
    f.word(0x8100, 0x8100);
    f.word(0x8110, 0x8100);
    f.word(0x8130, 0x8200);
    assert_eq!(
        maximum_organisation(&f, IMAGE, SHIP),
        Err("ship organisation doctrine traversal bound exceeded")
    );
}

#[test]
fn organisation_arrays_use_original_counts_above_former_limits() {
    let mut f = fixture();
    f.array(SHIP + 0x918, 0x22000, 4_097);
    f.modifier(0x7400 + 0x98, 0, &[]);
    for index in 0..4_097 {
        f.word(0x22000 + index * 16, 0x7400);
        f.dword(0x22008 + index * 16, 1);
    }
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    f.array(TASK + 0x728, 0x40000, 4_097);
    f.word(0x12140, 0);
    for index in 0..4_097 {
        f.word(0x40000 + index * 8, 0x12100);
    }
    assert_eq!(maximum_organisation(&f, IMAGE, SHIP), Ok(expected));
    let mut weather = weather_fixture();
    weather.array(WEATHER + 0x40, 0x29000, 65_537);
    weather.array(WEATHER + 0x370, 0x2a000, 65_537);
    assert!(maximum_organisation(&weather, IMAGE, SHIP).is_ok());
}

#[test]
fn organisation_doctrine_uses_native_map_size_and_deep_successors() {
    let mut f = fixture();
    const COUNT: usize = 4_097;
    const NODES: usize = 0x50000;
    const HEAD: usize = 0x8000;
    f.word(TASK + 0x648, COUNT as u64);
    f.word(HEAD, NODES as u64);
    for index in 0..COUNT {
        let at = NODES + index * 0x40;
        f.word(at, HEAD as u64);
        f.word(at + 8, if index == 0 { HEAD } else { at - 0x40 } as u64);
        f.word(
            at + 0x10,
            if index + 1 == COUNT { HEAD } else { at + 0x40 } as u64,
        );
        f.put(at + 0x19, &[0]);
        f.dword(at + 0x20, index as u32);
        f.word(at + 0x30, 0x7800);
    }
    f.modifier(0x7810, 0, &[]);
    assert!(maximum_organisation(&f, IMAGE, SHIP).is_ok());
    f.word(TASK + 0x648, COUNT as u64 - 1);
    assert_eq!(
        maximum_organisation(&f, IMAGE, SHIP),
        Err("ship organisation doctrine size mismatch")
    );
}

#[test]
fn gathered_organisation_reads_equal_the_direct_reader_at_every_fault() {
    use super::gathered::FaultAt;
    use engine_native_memory::read_census::ReadCensus;
    fn through_fields(memory: &impl Memory) -> Result<i64, &'static str> {
        let mut fields = ShipFields::new(IMAGE);
        fields.visit(memory, SHIP);
        maximum_organisation(&fields.memory(memory), IMAGE, SHIP)
    }
    for (name, f) in [
        ("country and doctrine", fixture()),
        ("support", support_fixture()),
        ("province and fleet", shared_sources_fixture()),
        ("weather and criticals", weather_fixture()),
    ] {
        let direct = FaultAt::new(&f, None);
        let expected = maximum_organisation(&direct, IMAGE, SHIP);
        assert!(expected.is_ok(), "{name}");
        let gathered = FaultAt::new(&f, None);
        assert_eq!(through_fields(&gathered), expected, "{name}");
        let mut edges = direct.edges();
        edges.extend(gathered.edges());
        edges.sort_unstable();
        edges.dedup();
        for fault in edges {
            let direct = FaultAt::new(&f, Some(fault));
            let gathered = FaultAt::new(&f, Some(fault));
            assert_eq!(
                through_fields(&gathered),
                maximum_organisation(&direct, IMAGE, SHIP),
                "{name}: fault at {fault:#x}"
            );
            assert!(!gathered.retried(), "{name}: fault at {fault:#x}");
        }
        let direct = ReadCensus::new(&f);
        assert_eq!(maximum_organisation(&direct, IMAGE, SHIP), expected);
        let gathered = ReadCensus::new(&f);
        assert_eq!(through_fields(&gathered), expected);
        assert!(
            gathered.calls() < direct.calls(),
            "{name}: {} guarded calls, {} directly",
            gathered.calls(),
            direct.calls()
        );
    }
}

#[test]
fn organisation_memo_shares_an_influence_across_task_forces() {
    use super::gathered::FaultAt;
    const OTHER_TASK: usize = 0x22000;
    let mut f = support_fixture();
    f.put(OTHER_TASK, &[0; 0x900]);
    f.word(OTHER_TASK, (IMAGE + ctask_force::VTABLE_RVA.0) as u64);
    f.dword(OTHER_TASK + cunit::ROSTER_OWNER, 1);
    f.word(OTHER_TASK + 0x640, 0x8000);
    f.array(OTHER_TASK + 0x728, 0x12000, 1);
    let mut memo = OrganisationMemo::default();
    let first = read_maximum_organisation_memo(&f, IMAGE, SHIP, &mut memo);
    assert!(first.is_ok());
    f.word(SHIP + TASK_FORCE, OTHER_TASK as u64);
    let trace = FaultAt::new(&f, None);
    assert_eq!(
        read_maximum_organisation_memo(&trace, IMAGE, SHIP, &mut memo),
        maximum_organisation(&f, IMAGE, SHIP)
    );
    assert_eq!(memo.support, Some((OTHER_TASK, [12, 16, 20, 24])));
    assert!(
        !trace
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| address == 0x17000 || address == 0x18000),
        "the commander was read again"
    );
}

#[test]
fn held_location_and_fleet_terms_read_no_province_fleet_or_leader() {
    use super::gathered::FaultAt;
    let f = weather_fixture();
    let mut memo = OrganisationMemo::default();
    let terms = current_task_terms(&f, &mut memo);
    let (definition, terrain) = memo.location_source(PROVINCE).unwrap();
    let (leader, fleet_terms) = memo.fleet_source(FLEET).unwrap();
    assert_eq!(leader, Some(LEADER));
    let trace = FaultAt::new(&f, None);
    let mut next = OrganisationMemo::default();
    next.hold_location(PROVINCE, definition, terrain);
    next.hold_fleet(FLEET, leader, fleet_terms);
    assert_eq!(current_task_terms(&trace, &mut next), terms);
    for (name, source) in [
        ("province", PROVINCE),
        ("province definition", PROVINCE + 0xc8),
        ("terrain graph", PROVINCE_DEFINITION + 0x30),
        ("fleet", FLEET),
        ("fleet leader binding", FLEET + 0xb0),
        ("leader", LEADER),
        ("leader tree", LEADER + 0x418),
    ] {
        assert!(
            !trace
                .reads
                .borrow()
                .iter()
                .any(|&(address, len)| (address..address + len.max(1)).contains(&source)),
            "{name} read again"
        );
    }
}
