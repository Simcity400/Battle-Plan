use super::*;
use engine_native::cgame_state;
use engine_native_memory::memory::SliceMemory;
use std::{cell::RefCell, ops::Range};

const BASE: usize = 0x140000000;
const GAME: usize = 0x1000;
const VARIANT: usize = 0x2000;
const DEFINITION: usize = 0x3000;
const TOKEN: u32 = 0x6543;

struct Fixture {
    bytes: Vec<u8>,
    next: usize,
}

impl Fixture {
    fn new(speed: i64) -> Self {
        let mut result = Self {
            bytes: vec![0; 0x20000],
            next: 0x4000,
        };
        result.vtable(DEFINITION, cequipment_type::VTABLE_RVA);
        result.put(DEFINITION + cequipment_type::TOKEN, TOKEN.to_le_bytes());
        result.put(
            VARIANT + cequipment_variant::EQUIPMENT_TOKEN,
            TOKEN.to_le_bytes(),
        );
        result.put(
            VARIANT + cequipment_variant::DEFINITION,
            (DEFINITION as u64).to_le_bytes(),
        );
        result.put(
            DEFINITION + TYPE_STATS + SPEED_STAT as usize * 8,
            speed.to_le_bytes(),
        );
        result.vtable(VARIANT + MIO_BONUS, MIO_VTABLE_RVA);
        result.vtable(
            VARIANT + cequipment_variant::UPGRADES,
            cequipment_upgrades_instance::VTABLE_RVA,
        );
        result.put(VARIANT + 0x2f0, (-999_i64).to_le_bytes());
        result.put(VARIANT + 0x488, u64::MAX.to_le_bytes());
        result
    }

    fn put(&mut self, address: usize, bytes: impl AsRef<[u8]>) {
        let bytes = bytes.as_ref();
        self.bytes[address..address + bytes.len()].copy_from_slice(bytes);
    }

    fn alloc(&mut self, len: usize) -> usize {
        let address = self.next;
        self.next += len.next_multiple_of(16);
        address
    }

    fn vtable(&mut self, address: usize, key: Rva) {
        self.put(address, ((BASE + key.0) as u64).to_le_bytes());
    }

    fn array(&mut self, header: usize, data: &[u8], stride: usize) -> usize {
        let count = (data.len() / stride) as u32;
        let address = if data.is_empty() {
            0
        } else {
            self.alloc(data.len())
        };
        self.put(header, (address as u64).to_le_bytes());
        self.put(header + 8, count.to_le_bytes());
        self.put(header + 12, count.to_le_bytes());
        if !data.is_empty() {
            self.put(address, data);
        }
        address
    }

    fn values(&mut self, header: usize, entries: &[(u32, i64)]) -> usize {
        let data: Vec<u8> = entries
            .iter()
            .flat_map(|&(key, value)| {
                let mut bytes = [0; 16];
                bytes[..4].copy_from_slice(&key.to_le_bytes());
                bytes[8..].copy_from_slice(&value.to_le_bytes());
                bytes
            })
            .collect();
        self.array(header, &data, 16)
    }

    fn bonus(&mut self, value: i64) -> usize {
        let bonus = self.alloc(0x80);
        self.vtable(bonus, SPECIFIC_VTABLE_RVA);
        self.vtable(bonus + 0x38, BONUS_VTABLE_RVA);
        self.values(bonus + 0x40, &[(SPEED_STAT, value), (SPEED_STAT, 900_000)]);
        bonus
    }

    fn direct(&mut self, pointers: &[usize]) -> usize {
        let data: Vec<u8> = pointers
            .iter()
            .flat_map(|&p| (p as u64).to_le_bytes())
            .collect();
        self.array(VARIANT + DIRECT_BONUSES, &data, 8)
    }

    fn named(
        &mut self,
        creator: u32,
        origin: i32,
        handles: &[i32],
        bonuses: &[(i32, i64)],
    ) -> (usize, usize) {
        let zero = self.alloc(0x1000);
        let one = self.alloc(0x1000);
        let two = self.alloc(0x1000);
        let roster: Vec<u8> = [zero, one, two]
            .into_iter()
            .flat_map(|p| (p as u64).to_le_bytes())
            .collect();
        self.array(GAME + cgame_state::COUNTRIES, &roster, 8);
        let links: Vec<u8> = [0_u32, 1, 2, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        self.array(GAME + cgame_state::COUNTRY_LINKS, &links, 4);
        for (id, country) in [(0_u32, zero), (1, one), (2, two)] {
            self.vtable(country, ccountry::VTABLE_RVA);
            self.put(country + ccountry::ID, id.to_le_bytes());
        }
        self.put(
            VARIANT + cequipment_variant::CREATOR_LINK,
            creator.to_le_bytes(),
        );
        self.put(
            VARIANT + cequipment_variant::ORIGIN_LINK,
            origin.to_le_bytes(),
        );
        let data: Vec<u8> = handles.iter().copied().flat_map(i32::to_le_bytes).collect();
        let handle_data = self.array(VARIANT + NAMED_HANDLES, &data, 4);
        let owner = if origin > 0 { origin as u32 } else { creator };
        let country = [zero, one, two, one][owner as usize];
        let production = self.alloc(0x200);
        self.vtable(production, cproduction_status::VTABLE_RVA);
        self.put(
            country + ccountry::PRODUCTION_STATUS,
            (production as u64).to_le_bytes(),
        );
        let data = vec![0; bonuses.len() * NAMED_STRIDE];
        let bonus_data = self.array(production + NAMED_BONUSES, &data, NAMED_STRIDE);
        for (ordinal, &(handle, value)) in bonuses.iter().enumerate() {
            let bonus = bonus_data + ordinal * NAMED_STRIDE;
            self.vtable(bonus, NAMED_VTABLE_RVA);
            self.vtable(bonus + 0x40, BONUS_VTABLE_RVA);
            self.put(bonus + NAMED_HANDLE, handle.to_le_bytes());
            self.values(bonus + 0x48, &[(SPEED_STAT, value)]);
        }
        (handle_data, bonus_data)
    }

    fn group(&mut self, source: usize, add: i64, average: Option<i64>, factor: i64) {
        self.vtable(source, GROUP_VTABLE_RVA);
        for (offset, value) in [(8, Some(add)), (0x28, average), (0x48, Some(factor))] {
            self.vtable(source + offset, STATS_VTABLE_RVA);
            self.values(
                source + offset + 8,
                &value.map(|v| vec![(SPEED_STAT, v)]).unwrap_or_default(),
            );
        }
    }

    fn upgrade(&mut self, level: u8, add: i64, average: Option<i64>, factor: i64) -> usize {
        let upgrade = self.alloc(0x200);
        self.vtable(upgrade, cequipment_upgrade::VTABLE_RVA);
        self.group(upgrade + 0x40, add, average, factor);
        let mut entry = [0; 16];
        entry[..8].copy_from_slice(&(upgrade as u64).to_le_bytes());
        entry[8] = level;
        self.array(VARIANT + cequipment_variant::UPGRADES + 8, &entry, 16);
        upgrade
    }

    fn module(&mut self, add: i64, average: Option<i64>, factor: i64) -> usize {
        let module = self.alloc(0x300);
        self.vtable(module, cequipment_module::VTABLE_RVA);
        self.group(module + 0x128, add, average, factor);
        let mut entry = [0; 16];
        entry[8..].copy_from_slice(&(module as u64).to_le_bytes());
        self.array(VARIANT + cequipment_variant::MODULES + 8, &entry, 16);
        module
    }

    fn memory(&self) -> Observed<'_> {
        Observed {
            memory: SliceMemory {
                base: 0,
                bytes: &self.bytes,
            },
            failures: vec![],
            reads: RefCell::new(vec![]),
        }
    }
}

struct Observed<'a> {
    memory: SliceMemory<'a>,
    failures: Vec<Range<usize>>,
    reads: RefCell<Vec<Range<usize>>>,
}

impl Memory for Observed<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let range = address..address.saturating_add(destination.len());
        self.reads.borrow_mut().push(range.clone());
        if self
            .failures
            .iter()
            .any(|fail| fail.start < range.end && range.start < fail.end)
        {
            return false;
        }
        self.memory.copy(address, destination)
    }
}

fn speed(memory: &impl Memory, pointers: engine_native_memory::walk::Buffer<'_, usize>) -> Result<i64, &'static str> {
    variant_speed(
        memory,
        BASE,
        GAME,
        VARIANT,
        &mut NamedBonuses {
            pointers,
            required: 0,
        },
    )
}

#[test]
fn railway_speed_rebuilds_current_graph_first_matches_and_zero_average_presence() {
    let mut fixture = Fixture::new(100_000);
    let direct = fixture.bonus(20_000);
    fixture.direct(&[direct]);
    fixture.named(
        3,
        2,
        &[11, -1, 999, 11, 22],
        &[(11, 10_000), (11, 900_000), (22, -5_000)],
    );
    fixture.values(
        VARIANT + MIO_BONUS + 8,
        &[
            (MAXIMUM_SPEED_TOKEN, 30_000),
            (MAXIMUM_SPEED_TOKEN, 800_000),
        ],
    );
    fixture.upgrade(2, 3_000, Some(8_000), 7_000);
    fixture.module(4_000, Some(0), -9_000);
    let memory = fixture.memory();
    assert_eq!(speed(&memory, engine_native_memory::walk::Buffer::Growing(&mut vec![0; 5])), Ok(200_600));
    for unused in [VARIANT + 0x2f0, VARIANT + 0x488] {
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|read| read.contains(&unused))
        );
    }
}

#[test]
fn railway_speed_empty_sources_skip_country_resolution_and_keep_negative_zero() {
    for value in [-123_457_i64, 0, 100_000] {
        let fixture = Fixture::new(value);
        let mut memory = fixture.memory();
        memory.failures.push(GAME..GAME + 0x400);
        assert_eq!(speed(&memory, engine_native_memory::walk::Buffer::Count), Ok(value));
    }
    let mut fixture = Fixture::new(100_000);
    fixture.named(0, 0, &[7], &[(7, 25_000)]);
    assert_eq!(speed(&fixture.memory(), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1])), Ok(125_000));
}

#[test]
fn railway_speed_resolves_creator_before_origin_and_rejects_torn_country_links() {
    let mut fixture = Fixture::new(100_000);
    fixture.named(3, 2, &[7], &[(7, 25_000)]);
    let links =
        usize::try_from(read::<u64>(&fixture.memory(), GAME + cgame_state::COUNTRY_LINKS).unwrap())
            .unwrap();
    fixture.put(links + 3 * 4, 0_u32.to_le_bytes());
    assert!(speed(&fixture.memory(), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1])).is_err());
    fixture.put(links + 3 * 4, 1_u32.to_le_bytes());
    let roster =
        usize::try_from(read::<u64>(&fixture.memory(), GAME + cgame_state::COUNTRIES).unwrap())
            .unwrap();
    let origin = usize::try_from(read::<u64>(&fixture.memory(), roster + 2 * 8).unwrap()).unwrap();
    fixture.put(origin + ccountry::ID, 1_u32.to_le_bytes());
    assert_eq!(
        speed(&fixture.memory(), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1])),
        Err("country slot and object resolve to different countries")
    );
}

#[test]
fn railway_speed_named_scratch_grows_to_the_raw_count_and_gathers_before_stats() {
    let mut fixture = Fixture::new(100_000);
    let (handles, bonus) = fixture.named(1, 0, &[8, -1, 999], &[(8, 20_000)]);
    let memory = fixture.memory();
    assert_eq!(named_bonus_demand(&memory, VARIANT), Ok(3));
    let mut pointers = vec![0; 2];
    let mut scratch = NamedBonuses {
        pointers: engine_native_memory::walk::Buffer::Growing(&mut pointers),
        required: 1,
    };
    assert_eq!(
        variant_speed(&memory, BASE, GAME, VARIANT, &mut scratch),
        Ok(120_000)
    );
    assert_eq!(scratch.required, 3);

    fixture.put(bonus, 0_u64.to_le_bytes());
    let mut memory = fixture.memory();
    memory.failures.push(handles + 4..handles + 8);
    assert_eq!(
        speed(&memory, engine_native_memory::walk::Buffer::Growing(&mut vec![0; 3])),
        Err("railway speed dependency unreadable")
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|read| read.contains(&bonus))
    );
}

#[test]
fn railway_speed_unreadable_and_unsupported_inputs_refuse_without_cache_fallback() {
    let mut fixture = Fixture::new(100_000);
    let bonus = fixture.bonus(20_000);
    let pointers = fixture.direct(&[bonus]);
    let upgrade = fixture.upgrade(1, 3_000, Some(8_000), 7_000);
    let module = fixture.module(4_000, None, -9_000);
    for address in [
        DEFINITION,
        DEFINITION + cequipment_type::TOKEN,
        DEFINITION + TYPE_STATS + SPEED_STAT as usize * 8,
        VARIANT + DIRECT_BONUSES,
        pointers,
        bonus,
        bonus + 0x40,
        VARIANT + MIO_BONUS,
        VARIANT + MIO_BONUS + 8,
        VARIANT + cequipment_variant::UPGRADES,
        upgrade,
        upgrade + 0x48,
        VARIANT + cequipment_variant::MODULES + 8,
        module,
        module + 0x130,
    ] {
        let mut memory = fixture.memory();
        memory.failures.push(address..address + 1);
        assert!(speed(&memory, engine_native_memory::walk::Buffer::Count).is_err(), "dependency {address:x}");
    }
    fixture.put(pointers, 0_u64.to_le_bytes());
    assert_eq!(
        speed(&fixture.memory(), engine_native_memory::walk::Buffer::Count),
        Err("railway speed dependency is null")
    );
    fixture.put(pointers, (bonus as u64).to_le_bytes());
    fixture.put(bonus, 0_u64.to_le_bytes());
    assert_eq!(
        speed(&fixture.memory(), engine_native_memory::walk::Buffer::Count),
        Err("railway speed dependency vtable mismatch")
    );
    fixture.vtable(bonus, SPECIFIC_VTABLE_RVA);
    fixture.put(
        DEFINITION + cequipment_type::TOKEN,
        (TOKEN + 1).to_le_bytes(),
    );
    assert_eq!(
        speed(&fixture.memory(), engine_native_memory::walk::Buffer::Count),
        Err("railway speed equipment definition token mismatch")
    );
    fixture.put(DEFINITION + cequipment_type::TOKEN, TOKEN.to_le_bytes());
    fixture.put(VARIANT + DIRECT_BONUSES + 8, 0_u32.to_le_bytes());
    assert_eq!(
        speed(&fixture.memory(), engine_native_memory::walk::Buffer::Count),
        Err("railway speed array unreadable or invalid")
    );
}

#[test]
fn railway_speed_group_scaling_wraps_and_reduction_keeps_native_sentinel() {
    let mut fixture = Fixture::new(0);
    fixture.upgrade(2, i64::MAX, None, 0);
    assert_eq!(speed(&fixture.memory(), engine_native_memory::walk::Buffer::Count), Ok(-2));
    for value in [i64::MIN, i64::MAX] {
        assert_eq!(
            Accumulator {
                average: value,
                average_count: 1,
                ..Accumulator::default()
            }
            .speed(),
            Ok(0xffff_ffff)
        );
    }
    assert_eq!(
        Accumulator {
            add: i64::MAX,
            ..Accumulator::default()
        }
        .speed(),
        Ok(-1)
    );
    assert_eq!(
        Accumulator {
            average: -900_001,
            average_count: 2,
            ..Accumulator::default()
        }
        .speed(),
        Ok(-450_000)
    );
    assert_eq!(
        Accumulator {
            add: 10,
            average: 99,
            average_count: -1,
            ..Accumulator::default()
        }
        .speed(),
        Ok(10)
    );
}

#[test]
fn growing_named_bonus_arena_uses_original_raw_header_demand() {
    let mut fixture = Fixture::new(100_000);
    fixture.bytes.resize(0x80000, 0);
    let mut pointers = Vec::new();
    for count in [5_000, 20_000] {
        let handles = vec![8; count];
        fixture.named(1, 0, &handles, &[(8, 1)]);
        let memory = fixture.memory();
        let mut scratch = NamedBonuses {
            pointers: engine_native_memory::walk::Buffer::Growing(&mut pointers),
            required: 0,
        };
        assert_eq!(
            variant_speed(&memory, BASE, GAME, VARIANT, &mut scratch),
            Ok(100_000 + count as i64)
        );
        assert_eq!(scratch.required, count);
        assert!(scratch.pointers.len() >= count);
    }
}
