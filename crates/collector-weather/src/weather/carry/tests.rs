use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::{Cell, RefCell};

const STATE: usize = 0;
const MANAGER: usize = 0x1000;
const REGIONS: usize = 0x2000;

struct World {
    base: usize,
    bytes: Vec<u8>,
    provinces: u32,
    regions: u32,
    accumulators: usize,
    province_data: usize,
    custom: usize,
}

impl World {
    fn new(provinces: u32, regions: u32, budgets: (i32, i32)) -> Self {
        Self::build(provinces, regions, budgets, false)
    }

    fn build(provinces: u32, regions: u32, budgets: (i32, i32), live: bool) -> Self {
        let accumulators = REGIONS + regions as usize * sweather_per_region::STRIDE;
        let province_data = (accumulators
            + regions as usize * COMBAT_KEYS.len() * engine_native_parts::modifier_entry::STRIDE
            + 0xff)
            & !0xff;
        let snow = province_data + provinces as usize * sweather_per_province::STRIDE;
        let mud = snow + provinces as usize;
        let clocks = (mud + provinces as usize + 0xff) & !0xff;
        let custom = (clocks + provinces as usize * sweather_per_province::CLOCK_STRIDE + 0xf) & !0xf;
        let end = custom + regions as usize * 4;
        let bytes = vec![0; end];
        let mut world = Self {
            base: if live {
                bytes.as_ptr() as usize
            } else {
                0x10_0000
            },
            bytes,
            provinces,
            regions,
            accumulators,
            province_data,
            custom,
        };
        world.put(
            STATE + engine_native::cgame_state::WEATHER_MANAGER,
            &((world.base + MANAGER) as u64).to_le_bytes(),
        );
        world.put(
            MANAGER,
            &(cweather_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        for (offset, data, count) in [
            (cweather_manager::REGIONS, REGIONS, regions),
            (cweather_manager::PROVINCES, province_data, provinces),
            (cweather_manager::DISPLAY_SNOW, snow, provinces),
            (cweather_manager::DISPLAY_MUD, mud, provinces),
            (cweather_manager::PROVINCE_CLOCKS, clocks, provinces),
        ] {
            world.put(
                MANAGER + offset,
                &((world.base + data) as u64).to_le_bytes(),
            );
            world.put(MANAGER + offset + 8, &count.to_le_bytes());
            world.put(MANAGER + offset + 12, &count.to_le_bytes());
        }
        world.budgets(budgets);
        for slot in 0..regions {
            let record = world.region(slot);
            world.put(
                record,
                &(sweather_per_region::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            world.put(
                record + sweather_per_region::ID,
                &(slot as i32 + 1).to_le_bytes(),
            );
            let entries = accumulators
                + slot as usize * COMBAT_KEYS.len() * engine_native_parts::modifier_entry::STRIDE;
            world.put(
                record + sweather_per_region::MODIFIERS,
                &((world.base + entries) as u64).to_le_bytes(),
            );
            world.put(
                record + sweather_per_region::MODIFIERS + 8,
                &(COMBAT_KEYS.len() as u32).to_le_bytes(),
            );
            world.put(
                record + sweather_per_region::MODIFIERS + 12,
                &(COMBAT_KEYS.len() as u32).to_le_bytes(),
            );
            for (index, key) in COMBAT_KEYS.into_iter().enumerate() {
                let entry = entries + index * engine_native_parts::modifier_entry::STRIDE;
                world.put(entry, &key.to_le_bytes());
                world.put(entry + 8, &(i64::from(slot) * 1000).to_le_bytes());
            }
            let count = world.custom_count(slot);
            let header = record + sweather_per_region::CUSTOM_SEA;
            world.put(
                header,
                &((world.base + world.custom_values(slot)) as u64).to_le_bytes(),
            );
            world.put(header + 8, &2_u32.to_le_bytes());
            world.put(header + 12, &count.to_le_bytes());
            let values = world.custom_values(slot);
            world.put(values, &[1, 0, 0, 0]);
        }
        for slot in 0..provinces {
            let record = world.province(slot);
            world.put(
                record,
                &(sweather_per_province::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            world.put(record + sweather_per_province::ID, &slot.to_le_bytes());
            world.put(
                record + sweather_per_province::TEMPERATURE,
                &(i64::from(slot) * 100_000).to_le_bytes(),
            );
        }
        world
    }

    fn put(&mut self, address: usize, value: &[u8]) {
        self.bytes[address..address + value.len()].copy_from_slice(value);
    }

    fn word(&self, address: usize) -> i32 {
        i32::from_le_bytes(self.bytes[address..address + 4].try_into().unwrap())
    }

    fn i64(&self, address: usize) -> i64 {
        i64::from_le_bytes(self.bytes[address..address + 8].try_into().unwrap())
    }

    fn region(&self, slot: u32) -> usize {
        REGIONS + slot as usize * sweather_per_region::STRIDE
    }

    fn province(&self, slot: u32) -> usize {
        self.province_data + slot as usize * sweather_per_province::STRIDE
    }

    fn custom_count(&self, slot: u32) -> u32 {
        if slot % 3 == 2 { 0 } else { 2 }
    }

    fn custom_values(&self, slot: u32) -> usize {
        self.custom + slot as usize * 4
    }

    fn combat_value(&self, slot: u32) -> usize {
        self.accumulators
            + slot as usize * COMBAT_KEYS.len() * engine_native_parts::modifier_entry::STRIDE
            + 8
    }

    fn budgets(&mut self, (provinces, regions): (i32, i32)) {
        self.put(
            MANAGER + cweather_manager::PROVINCES_PER_UPDATE,
            &provinces.to_le_bytes(),
        );
        self.put(
            MANAGER + cweather_manager::REGIONS_PER_UPDATE,
            &regions.to_le_bytes(),
        );
    }

    fn cursors(&mut self, (provinces, regions): (i32, i32)) {
        self.put(
            MANAGER + cweather_manager::UPDATE_CURSOR,
            &provinces.to_le_bytes(),
        );
        self.put(
            MANAGER + cweather_manager::REGION_CURSOR,
            &regions.to_le_bytes(),
        );
    }

    fn province_entry(&mut self, slot: u32, key: u32, value: i64) -> usize {
        let entry = (self.bytes.len() + 0xf) & !0xf;
        self.bytes
            .resize(entry + engine_native_parts::modifier_entry::STRIDE, 0);
        let header = self.province(slot) + sweather_per_province::MODIFIERS;
        self.put(header, &((self.base + entry) as u64).to_le_bytes());
        self.put(header + 8, &1_u32.to_le_bytes());
        self.put(header + 12, &1_u32.to_le_bytes());
        self.put(entry, &key.to_le_bytes());
        self.put(entry + 8, &value.to_le_bytes());
        entry + 8
    }

    fn province_statics(&mut self, slot: u32, entries: &[(&str, i32)]) -> usize {
        let name = engine_native_parts::cstatic_modifier::NAME;
        let definition_bytes = name + engine_native_memory::msvc_string::STRIDE;
        let list = (self.bytes.len() + 0xf) & !0xf;
        let stride = sweather_per_province::STATIC_MODIFIER_STRIDE;
        let first = list + entries.len() * stride;
        self.bytes
            .resize(first + entries.len() * definition_bytes, 0);
        for (index, &(text, days)) in entries.iter().enumerate() {
            assert!(text.len() < 16);
            let definition = first + index * definition_bytes;
            self.put(
                definition,
                &(engine_native_parts::cstatic_modifier::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.put(definition + name, text.as_bytes());
            self.put(
                definition + name + engine_native_memory::msvc_string::LENGTH,
                &(text.len() as u64).to_le_bytes(),
            );
            self.put(
                definition + name + engine_native_memory::msvc_string::CAPACITY,
                &engine_native_memory::msvc_string::INLINE_CAPACITY.to_le_bytes(),
            );
            let entry = list + index * stride;
            self.put(entry, &((self.base + definition) as u64).to_le_bytes());
            self.put(
                entry + sweather_per_province::STATIC_MODIFIER_DAYS,
                &days.to_le_bytes(),
            );
        }
        let header = self.province(slot) + sweather_per_province::STATIC_MODIFIERS;
        self.put(header, &((self.base + list) as u64).to_le_bytes());
        self.put(header + 8, &(entries.len() as u32).to_le_bytes());
        self.put(header + 12, &(entries.len() as u32).to_le_bytes());
        list
    }

    fn write_province(&mut self, slot: u32, temperature: i64) {
        self.put(
            self.province(slot) + sweather_per_province::TEMPERATURE,
            &temperature.to_le_bytes(),
        );
    }

    fn pass(&mut self, tick: i64) {
        for (cursor, count, budget, regions) in [
            (
                cweather_manager::UPDATE_CURSOR,
                self.provinces,
                cweather_manager::PROVINCES_PER_UPDATE,
                false,
            ),
            (
                cweather_manager::REGION_CURSOR,
                self.regions,
                cweather_manager::REGIONS_PER_UPDATE,
                true,
            ),
        ] {
            let (first, visits) = window(
                self.word(MANAGER + cursor),
                count as i32,
                self.word(MANAGER + budget),
            )
            .unwrap();
            for visit in 0..visits {
                let slot = (first + visit) % count;
                if regions {
                    let flag = self.region(slot) + sweather_per_region::SNOW;
                    self.bytes[flag] ^= 1;
                    let value = self.combat_value(slot);
                    self.put(value, &(self.i64(value) + tick).to_le_bytes());
                    if self.custom_count(slot) != 0 {
                        let value = self.custom_values(slot) + 2;
                        self.bytes[value] ^= 1;
                    }
                } else {
                    let temperature = self.province(slot) + sweather_per_province::TEMPERATURE;
                    self.put(temperature, &(self.i64(temperature) + tick).to_le_bytes());
                }
            }
            let next = (first + visits) % count;
            self.put(MANAGER + cursor, &(next as i32).to_le_bytes());
        }
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: self.base,
            bytes: &self.bytes,
        }
    }
}

struct Census<'a> {
    inner: SliceMemory<'a>,
    touched: RefCell<Vec<usize>>,
}

impl<'a> Census<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            inner: world.memory(),
            touched: RefCell::new(Vec::new()),
        }
    }

    fn slots(&self, data: usize, stride: usize, count: u32) -> Vec<u32> {
        self.slots_where(data, stride, count, |_| true)
    }

    fn slots_where(
        &self,
        data: usize,
        stride: usize,
        count: u32,
        keep: impl Fn(usize) -> bool,
    ) -> Vec<u32> {
        let mut slots: Vec<u32> = self
            .touched
            .borrow()
            .iter()
            .filter(|&&address| address >= data && address < data + count as usize * stride)
            .filter(|&&address| keep((address - data) % stride))
            .map(|&address| ((address - data) / stride) as u32)
            .collect();
        slots.sort_unstable();
        slots.dedup();
        slots
    }
}

impl Memory for Census<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.touched.borrow_mut().push(address);
        self.inner.copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.touched.borrow_mut().extend(addresses);
        self.inner.copy_scattered(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let records: Vec<usize> = match sources {
            RecordSources::Strided {
                base,
                stride,
                count,
            } => (0..count).map(|index| base + index * stride).collect(),
            RecordSources::Pointers(pointers) => pointers.to_vec(),
        };
        self.touched.borrow_mut().extend(records.into_iter().flat_map(|record| {
            layout
                .fields()
                .iter()
                .map(move |field| record + field.source_offset)
        }));
        self.inner.copy_records(sources, layout, destination)
    }
}

fn carry() -> Box<WeatherCarry> {
    Box::default()
}

#[derive(Debug, PartialEq)]
struct Capture {
    regions: Vec<u8>,
    provinces: Vec<u8>,
    combat: Vec<Option<RegionModifiers>>,
    province_modifiers: ProvinceModifiers,
    province_statics: Option<ProvinceStatics>,
    custom_sea: super::super::entries::CustomSea,
    log: Vec<u8>,
}

fn run(
    memory: &impl Memory,
    state: usize,
    carry: Option<Box<WeatherCarry>>,
) -> (Capture, WeatherJob) {
    let mut job = WeatherJob::new();
    job.carry = carry;
    let report = copy_weather_refresh(
        memory,
        WeatherCopyContext {
            base: 0,
            game_state: state,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert!(!job.staging_failed());
    let regions = job.regions.array.unwrap().count as usize * sweather_per_region::RECORD_BYTES;
    let provinces =
        job.provinces.array.unwrap().count as usize * sweather_per_province::RECORD_BYTES;
    let captured = Capture {
        regions: job.regions.bytes[..regions].to_vec(),
        provinces: job.provinces.bytes[..provinces].to_vec(),
        combat: job.combat.clone(),
        province_modifiers: job.province_modifiers.clone(),
        province_statics: job.province_statics.as_deref().cloned(),
        custom_sea: job.custom_sea.clone(),
        log: job.log.bytes.to_vec(),
    };
    (captured, job)
}

fn hour<'a>(
    world: &'a World,
    mut carry: Box<WeatherCarry>,
    hour: i32,
    force_full: bool,
    writes: Option<u64>,
) -> (Box<WeatherCarry>, Census<'a>) {
    carry.begin(hour, force_full, writes);
    let census = Census::new(world);
    let (carried, mut job) = run(&census, world.base + STATE, Some(carry));
    let (full, _) = run(&world.memory(), world.base + STATE, None);
    assert_eq!(carried, full, "hour {hour}");
    (job.carry.take().unwrap(), census)
}

fn read(world: &World, census: &Census<'_>) -> (Vec<u32>, Vec<u32>) {
    (
        census.slots(
            world.base + REGIONS,
            sweather_per_region::STRIDE,
            world.regions,
        ),
        census.slots_where(
            world.base + world.province_data,
            sweather_per_province::STRIDE,
            world.provinces,
            |offset| offset != sweather_per_province::STATIC_MODIFIERS,
        ),
    )
}

fn headers(world: &World, census: &Census<'_>) -> Vec<u32> {
    census.slots_where(
        world.base + world.province_data,
        sweather_per_province::STRIDE,
        world.provinces,
        |offset| offset == sweather_per_province::STATIC_MODIFIERS,
    )
}

fn custom_read(world: &World, census: &Census<'_>) -> Vec<u32> {
    census.slots(world.base + world.custom, 4, world.regions)
}

fn with_custom(world: &World, slots: Vec<u32>) -> Vec<u32> {
    slots
        .into_iter()
        .filter(|&slot| world.custom_count(slot) != 0)
        .collect()
}

fn expected(window: (u32, u32), count: u32) -> Vec<u32> {
    (0..count).filter(|&slot| in_window(slot, window, count)).collect()
}

fn every(count: u32) -> Vec<u32> {
    (0..count).collect()
}

fn next_windows(world: &World) -> ((u32, u32), (u32, u32)) {
    let word = |offset| world.word(MANAGER + offset);
    (
        window(
            word(cweather_manager::REGION_CURSOR),
            world.regions as i32,
            word(cweather_manager::REGIONS_PER_UPDATE),
        )
        .unwrap(),
        window(
            word(cweather_manager::UPDATE_CURSOR),
            world.provinces as i32,
            word(cweather_manager::PROVINCES_PER_UPDATE),
        )
        .unwrap(),
    )
}

fn held(provinces: u32, regions: u32, budgets: (i32, i32)) -> (World, Box<WeatherCarry>) {
    let world = World::new(provinces, regions, budgets);
    let (carry, census) = hour(&world, carry(), 1, false, Some(0));
    assert_eq!(read(&world, &census), (every(regions), every(provinces)));
    assert!(carry.held.is_some());
    (world, carry)
}

#[test]
fn window_ports_the_update_passes_loop() {
    assert_eq!(window(5, 10, 3), Some((5, 3)));
    assert_eq!(window(8, 10, 3), Some((8, 3)));
    assert_eq!(advanced(8, 3, 10), 1);
    assert_eq!(window(2, 10, 0), Some((2, 1)));
    assert_eq!(window(2, 10, -7), Some((2, 1)));
    assert_eq!(window(4, 10, 584), Some((4, 10)));
    assert_eq!(advanced(4, 10, 10), 4);
    assert_eq!(window(10, 10, 3), Some((10, 0)));
    assert_eq!(window(-1, 10, 3), None);
    assert_eq!(window(0, 0, 3), None);
}

#[test]
fn window_hours_read_only_the_visited_slots() {
    for (provinces, regions, budgets) in [
        (60, 14, (7, 3)),
        (60, 14, (1, 1)),
        (60, 14, (0, -2)),
        (60, 14, (59, 13)),
        (60, 14, (60, 14)),
        (60, 14, (584, 7)),
        (61, 15, (13, 4)),
    ] {
        let (mut world, mut carry) = held(provinces, regions, budgets);
        world.cursors((57, 12));
        let (next, census) = hour(&world, carry, 2, false, Some(0));
        assert_eq!(read(&world, &census), (every(regions), every(provinces)));
        carry = next;
        for at in 3..60 {
            let (region_window, province_window) = next_windows(&world);
            world.pass(i64::from(at));
            let (next, census) = hour(&world, carry, at, false, Some(0));
            carry = next;
            assert_eq!(carry.followed, 3, "{budgets:?} hour {at}");
            assert_eq!(
                read(&world, &census),
                (
                    expected(region_window, regions),
                    expected(province_window, provinces)
                ),
                "{budgets:?} hour {at}"
            );
            assert_eq!(
                custom_read(&world, &census),
                with_custom(&world, expected(region_window, regions)),
                "{budgets:?} hour {at}"
            );
        }
    }
}

#[test]
fn a_counted_write_forces_a_full_read() {
    let (mut world, carry) = held(40, 9, (5, 2));
    world.pass(1);
    world.write_province(30, -123_456);
    let (carry, census) = hour(&world, carry, 2, false, Some(2));
    assert_eq!(read(&world, &census), (every(9), every(40)));
    assert_eq!(carry.followed, 0);
    world.pass(2);
    let (carry, _) = hour(&world, carry, 3, false, Some(2));
    assert_eq!(carry.followed, 3);
    world.pass(3);
    let (carry, census) = hour(&world, carry, 4, false, None);
    assert_eq!(read(&world, &census), (every(9), every(40)));
    assert!(carry.held.is_none());
}

#[test]
fn a_cursor_jump_falls_back() {
    let (mut world, carry) = held(40, 9, (5, 2));
    world.pass(1);
    world.pass(2);
    let (carry, census) = hour(&world, carry, 2, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
    let (carry, census) = hour(&world, carry, 3, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
    world.pass(3);
    let (carry, _) = hour(&world, carry, 4, false, Some(0));
    assert_eq!(carry.followed, 3);
    world.pass(4);
    world.cursors((world.word(MANAGER + cweather_manager::UPDATE_CURSOR), 0));
    let (_, census) = hour(&world, carry, 5, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
}

type Case = dyn Fn(&mut World, &mut i32, &mut bool);

#[test]
fn every_other_hour_falls_back() {
    let (mut world, mut carry) = held(40, 9, (5, 2));
    let mut at = 1;
    let cases: [&Case; 5] = [
        &|_, at, _| *at += 1,
        &|_, _, force| *force = true,
        &|world, _, _| world.put(MANAGER + cweather_manager::INITIAL_MODE, &[1]),
        &|world, _, _| world.put(MANAGER + cweather_manager::SUSPEND_FLAGS + 1, &[1]),
        &|world, _, _| world.budgets((6, 2)),
    ];
    for case in cases {
        world.pass(at.into());
        at += 1;
        let mut force = false;
        case(&mut world, &mut at, &mut force);
        let (next, census) = hour(&world, carry, at, force, Some(0));
        assert_eq!(read(&world, &census), (every(9), every(40)), "hour {at}");
        world.put(MANAGER + cweather_manager::INITIAL_MODE, &[0, 0, 0]);
        world.pass(at.into());
        at += 1;
        let (next, _) = hour(&world, next, at, false, Some(0));
        assert_eq!(next.followed, 3, "hour {at}");
        carry = next;
    }
    carry.forget();
    world.pass(99);
    let (_, census) = hour(&world, carry, at + 1, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
}

#[test]
fn an_unfollowed_province_hour_reads_every_province_modifier_entry() {
    let provinces = 80;
    let mut world = World::new(provinces, 9, (5, 2));
    let key = engine_executable::modifier_keys::ARMY_SPEED_FACTOR;
    let entries: Vec<usize> = (0..provinces)
        .map(|slot| world.province_entry(slot, key, i64::from(slot)))
        .collect();
    let (carry, _) = hour(&world, carry(), 1, false, Some(0));
    assert_eq!(carry.province_modifiers.len(), provinces as usize);
    let (_, window) = next_windows(&world);
    world.pass(1);
    let entry = (0..provinces).find(|&slot| !in_window(slot, window, provinces)).unwrap();
    world.put(entries[entry as usize], &(-7_i64).to_le_bytes());
    let (carry, census) = hour(&world, carry, 2, false, Some(1));
    assert!(!carry.provinces_followed);
    assert_eq!(read(&world, &census).1, every(provinces));
}

#[test]
fn static_modifier_lists_are_read_only_after_a_counted_write_or_the_daily_update() {
    let provinces = 80;
    let mut world = World::new(provinces, 9, (5, 2));
    let stride = sweather_per_province::STATIC_MODIFIER_STRIDE;
    let days = sweather_per_province::STATIC_MODIFIER_DAYS;
    let mut lists: Vec<Option<(usize, usize)>> = (0..provinces)
        .map(|slot| {
            (slot % 4 != 3).then(|| {
                let entries = [("the_great_wall", -1), ("flooded", 3 + slot as i32)];
                (world.province_statics(slot, &entries), entries.len())
            })
        })
        .collect();
    lists[0] = Some((world.province_statics(0, &[("the_great_wall", 0)]), 1));
    let base = world.base;
    let listed = |lists: &[Option<(usize, usize)>], census: &Census<'_>| {
        lists
            .iter()
            .flatten()
            .filter(|&&(list, _)| census.touched.borrow().contains(&(base + list)))
            .count()
    };
    let with_lists = lists.iter().flatten().count();
    let statics = |carry: &WeatherCarry, slot: usize| -> Vec<(Vec<u8>, i32)> {
        super::super::entries::statics(
            carry.province_statics.as_ref().unwrap().get(slot).unwrap(),
        )
        .map(|(name, days)| (name.to_vec(), days))
        .collect()
    };
    let (carry, census) = hour(&world, carry(), 20, false, Some(0));
    assert_eq!(headers(&world, &census), every(provinces));
    assert_eq!(listed(&lists, &census), with_lists);
    world.pass(1);
    let held = carry.province_statics.clone().unwrap();
    let (carry, census) = hour(&world, carry, 21, false, Some(0));
    assert_eq!(carry.followed, 3);
    assert!(headers(&world, &census).is_empty());
    assert_eq!(listed(&lists, &census), 0);
    assert!(SharedBox::ptr_eq(&held, carry.province_statics.as_ref().unwrap()));
    drop(held);
    world.pass(2);
    lists[3] = Some((world.province_statics(3, &[("dam_broken", 5)]), 1));
    let (carry, census) = hour(&world, carry, 22, false, Some(2));
    assert_eq!(headers(&world, &census), every(provinces));
    assert_eq!(statics(&carry, 3), [(b"dam_broken".to_vec(), 5)]);
    world.pass(3);
    let (carry, census) = hour(&world, carry, 23, false, Some(2));
    assert!(headers(&world, &census).is_empty());
    for &(list, entries) in lists.iter().flatten() {
        for entry in 0..entries {
            let at = list + entry * stride + days;
            if world.word(at) > 0 {
                world.put(at, &(world.word(at) - 1).to_le_bytes());
            }
        }
    }
    world.pass(4);
    let (carry, census) = hour(&world, carry, 24, false, Some(2));
    assert_eq!(carry.followed, 3);
    assert_eq!(headers(&world, &census), every(provinces));
    assert_eq!(
        statics(&carry, 1),
        [(b"the_great_wall".to_vec(), -1), (b"flooded".to_vec(), 3)]
    );
    assert_eq!(statics(&carry, 0), [(b"the_great_wall".to_vec(), 0)]);
    assert_eq!(statics(&carry, 3), [(b"dam_broken".to_vec(), 4)]);
    world.pass(5);
    lists[1] = Some((world.province_statics(1, &[("the_great_wall", -1)]), 1));
    let (carry, census) = hour(&world, carry, 25, false, Some(4));
    assert_eq!(headers(&world, &census), every(provinces));
    assert_eq!(statics(&carry, 1), [(b"the_great_wall".to_vec(), -1)]);
    world.pass(6);
    let (carry, census) = hour(&world, carry, 26, false, Some(4));
    assert!(headers(&world, &census).is_empty());
    world.pass(7);
    lists[2] = Some((world.province_statics(2, &[("flooded", 9)]), 1));
    lists[7] = Some((world.province_statics(7, &[("flooded", 1)]), 1));
    let (carry, census) = hour(&world, carry, 27, false, Some(6));
    assert_eq!(headers(&world, &census), every(provinces));
    assert_eq!(statics(&carry, 7), [(b"flooded".to_vec(), 1)]);
    world.pass(8);
    let (_, census) = hour(&world, carry, 28, false, Some(6));
    assert!(headers(&world, &census).is_empty());
    assert_eq!(listed(&lists, &census), 0);
}

struct FailsOnce<'a> {
    inner: SliceMemory<'a>,
    fault: usize,
    attempts: Cell<usize>,
}

impl Memory for FailsOnce<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address <= self.fault && self.fault < address.saturating_add(destination.len()) {
            self.attempts.set(self.attempts.get() + 1);
            if self.attempts.get() == 1 {
                return false;
            }
        }
        self.inner.copy(address, destination)
    }
}

#[test]
fn a_failed_window_record_is_not_read_again() {
    for regions in [true, false] {
        let (mut world, mut carry) = held(40, 9, (5, 2));
        let (region_window, province_window) = next_windows(&world);
        world.pass(1);
        let fault = world.base
            + if regions {
                world.region(region_window.0) + sweather_per_region::SNOW
            } else {
                world.province(province_window.0) + sweather_per_province::TEMPERATURE
            };
        let memory = FailsOnce {
            inner: world.memory(),
            fault,
            attempts: Cell::new(0),
        };
        carry.begin(2, false, Some(0));
        let (_, mut job) = run(&memory, world.base + STATE, Some(carry));
        assert_eq!(memory.attempts.get(), 1, "regions {regions}");
        let faulted = if regions {
            job.regions.faulted(sweather_per_region::FIELDS.len())
        } else {
            job.provinces.faulted(sweather_per_province::FIELDS.len())
        };
        assert!(faulted, "regions {regions}");
        assert!(job.log.fault[usize::from(!regions)]);
        carry = job.carry.take().unwrap();
        assert!(carry.held.is_none());
        world.pass(2);
        let (_, census) = hour(&world, carry, 3, false, Some(0));
        assert_eq!(read(&world, &census), (every(9), every(40)));
    }
}

#[test]
fn a_failed_window_combat_lookup_is_not_read_again() {
    let (mut world, carry) = held(40, 9, (5, 2));
    let (region_window, _) = next_windows(&world);
    world.pass(1);
    let failed = region_window.0 as usize;
    let memory = FailsOnce {
        inner: world.memory(),
        fault: world.base + world.region(region_window.0) + sweather_per_region::MODIFIERS,
        attempts: Cell::new(0),
    };
    let mut carry = carry;
    carry.begin(2, false, Some(0));
    let (captured, mut job) = run(&memory, world.base + STATE, Some(carry));
    assert_eq!(memory.attempts.get(), 2);
    let (full, _) = run(&world.memory(), world.base + STATE, None);
    for slot in 0..9 {
        let expected = if slot == failed {
            None
        } else {
            full.combat[slot]
        };
        assert_eq!(captured.combat[slot], expected, "region {slot}");
    }
    let carry = job.carry.take().unwrap();
    assert!(carry.held.is_none() && carry.combat.is_empty());
    world.pass(2);
    let (_, census) = hour(&world, carry, 3, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
}

#[test]
fn a_failed_window_custom_sea_read_is_refused_and_read_in_full_next_hour() {
    let (mut world, carry) = held(40, 9, (5, 2));
    assert_eq!(carry.custom_sea.len(), 9);
    let (region_window, _) = next_windows(&world);
    world.pass(1);
    let failed = (0..region_window.1)
        .map(|visit| (region_window.0 + visit) % 9)
        .find(|&slot| world.custom_count(slot) != 0)
        .unwrap();
    let memory = FailsOnce {
        inner: world.memory(),
        fault: world.base + world.custom_values(failed),
        attempts: Cell::new(0),
    };
    let refusals = CUSTOM_SEA_REFUSALS.load(Relaxed);
    let mut carry = carry;
    carry.begin(2, false, Some(0));
    let (captured, mut job) = run(&memory, world.base + STATE, Some(carry));
    assert_eq!(memory.attempts.get(), 1);
    assert!(CUSTOM_SEA_REFUSALS.load(Relaxed) > refusals);
    let (full, _) = run(&world.memory(), world.base + STATE, None);
    for slot in 0..9 {
        let expected = if slot == failed as usize {
            None
        } else {
            full.custom_sea.get(slot)
        };
        assert_eq!(captured.custom_sea.get(slot), expected, "region {slot}");
    }
    assert_eq!(full.custom_sea.get(2), Some(&[][..]));
    assert_eq!(full.custom_sea.get(0).map(<[i16]>::len), Some(2));
    let carry = job.carry.take().unwrap();
    assert!(carry.held.is_some() && carry.custom_sea.is_empty());
    world.pass(2);
    let (carry, census) = hour(&world, carry, 3, false, Some(0));
    assert_eq!(carry.followed, 3);
    assert_eq!(custom_read(&world, &census), with_custom(&world, every(9)));
}

