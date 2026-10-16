use super::*;
use engine::memory::SliceMemory;
use std::cell::{Cell, RefCell};

// Offsets from the world's base.
const STATE: usize = 0;
const MANAGER: usize = 0x1000;
const REGIONS: usize = 0x2000;

/// A live weather manager with `provinces` and `regions` records, each
/// region with its own combat modifier entries, at their native offsets.
struct World {
    base: usize,
    bytes: Vec<u8>,
    provinces: u32,
    regions: u32,
    accumulators: usize,
    province_data: usize,
}

impl World {
    fn new(provinces: u32, regions: u32, budgets: (i32, i32)) -> Self {
        Self::build(provinces, regions, budgets, false)
    }

    /// At its own address, for [`engine::memory::LiveMemory`]; otherwise at
    /// a fixed base for [`SliceMemory`].
    fn build(provinces: u32, regions: u32, budgets: (i32, i32), live: bool) -> Self {
        let accumulators = REGIONS + regions as usize * sweather_per_region::STRIDE;
        let province_data =
            (accumulators + regions as usize * 2 * engine::modifier_entry::STRIDE + 0xff) & !0xff;
        let snow = province_data + provinces as usize * sweather_per_province::STRIDE;
        let mud = snow + provinces as usize;
        let clocks = (mud + provinces as usize + 0xff) & !0xff;
        let end = clocks + provinces as usize * sweather_per_province::CLOCK_STRIDE;
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
        };
        world.put(
            STATE + engine::cgame_state::WEATHER_MANAGER,
            &((world.base + MANAGER) as u64).to_le_bytes(),
        );
        world.put(
            MANAGER,
            &(cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
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
                &(sweather_per_region::VTABLE_RVA as u64).to_le_bytes(),
            );
            world.put(
                record + sweather_per_region::ID,
                &(slot as i32 + 1).to_le_bytes(),
            );
            let entries = accumulators + slot as usize * 2 * engine::modifier_entry::STRIDE;
            world.put(
                record + sweather_per_region::MODIFIERS,
                &((world.base + entries) as u64).to_le_bytes(),
            );
            world.put(
                record + sweather_per_region::MODIFIERS + 8,
                &2_u32.to_le_bytes(),
            );
            world.put(
                record + sweather_per_region::MODIFIERS + 12,
                &2_u32.to_le_bytes(),
            );
            for (index, key) in COMBAT_KEYS.into_iter().enumerate() {
                let entry = entries + index * engine::modifier_entry::STRIDE;
                world.put(entry, &key.to_le_bytes());
                world.put(entry + 8, &(i64::from(slot) * 1000).to_le_bytes());
            }
        }
        for slot in 0..provinces {
            let record = world.province(slot);
            world.put(
                record,
                &(sweather_per_province::VTABLE_RVA as u64).to_le_bytes(),
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

    fn combat_value(&self, slot: u32) -> usize {
        self.accumulators + slot as usize * 2 * engine::modifier_entry::STRIDE + 8
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

    /// Gives province `slot` one weather modifier entry `key` = `value`, stored after
    /// the world's arrays; returns the value's address.
    fn province_entry(&mut self, slot: u32, key: u32, value: i64) -> usize {
        let entry = (self.bytes.len() + 0xf) & !0xf;
        self.bytes.resize(entry + engine::modifier_entry::STRIDE, 0);
        let header = self.province(slot) + sweather_per_province::MODIFIERS;
        self.put(header, &((self.base + entry) as u64).to_le_bytes());
        self.put(header + 8, &1_u32.to_le_bytes());
        self.put(header + 12, &1_u32.to_le_bytes());
        self.put(entry, &key.to_le_bytes());
        self.put(entry + 8, &value.to_le_bytes());
        entry + 8
    }

    /// A province write (as a console command or an unhooked writer would).
    fn write_province(&mut self, slot: u32, temperature: i64) {
        self.put(
            self.province(slot) + sweather_per_province::TEMPERATURE,
            &temperature.to_le_bytes(),
        );
    }

    /// One ordinary pass of weather window pass: every visited
    /// province and region record changes, and both cursors advance and wrap.
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

/// Memory that notes every address a read starts at.
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

    /// The slots of the `count` records of `stride` at `data` any read
    /// touched.
    fn slots(&self, data: usize, stride: usize, count: u32) -> Vec<u32> {
        let mut slots: Vec<u32> = self
            .touched
            .borrow()
            .iter()
            .filter(|&&address| address >= data && address < data + count as usize * stride)
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
        match sources {
            RecordSources::Strided {
                base,
                stride,
                count,
            } => self
                .touched
                .borrow_mut()
                .extend((0..count).map(|index| base + index * stride)),
            RecordSources::Pointers(pointers) => self.touched.borrow_mut().extend(pointers),
        }
        self.inner.copy_records(sources, layout, destination)
    }
}

thread_local! {
    /// The write count [`WeatherCarry::observe`] reports in these tests.
    static WRITES: Cell<Option<u64>> = const { Cell::new(Some(0)) };
}

fn observed() -> Option<u64> {
    WRITES.with(Cell::get)
}

/// A carry that compares only the hour's slice and reads the test's count.
fn carry() -> Box<WeatherCarry> {
    WRITES.with(|writes| writes.set(Some(0)));
    Box::new(WeatherCarry {
        observe: observed,
        every: false,
        ..Default::default()
    })
}

/// What one hour's job captured.
#[derive(Debug, PartialEq)]
struct Capture {
    regions: Vec<u8>,
    provinces: Vec<u8>,
    combat: Vec<Option<RegionModifiers>>,
    province_modifiers: Vec<Option<ProvinceModifiers>>,
    log: Vec<u8>,
}

fn run(
    memory: &impl Memory,
    state: usize,
    carry: Option<Box<WeatherCarry>>,
) -> (Capture, WeatherJob) {
    let mut job = WeatherJob::new();
    job.carry = carry;
    let report = copy_weather_refresh_growing(
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
        log: job.log.bytes.to_vec(),
    };
    (captured, job)
}

/// One hour through `carry`: what it captured (asserted equal to a full read
/// of the same memory), the carry back, and the census of its reads.
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

/// The slots one hour read of each array: (regions, provinces).
fn read(world: &World, census: &Census<'_>) -> (Vec<u32>, Vec<u32>) {
    (
        census.slots(
            world.base + REGIONS,
            sweather_per_region::STRIDE,
            world.regions,
        ),
        census.slots(
            world.base + world.province_data,
            sweather_per_province::STRIDE,
            world.provinces,
        ),
    )
}

/// The window's slots plus the hour's comparison slice.
fn expected(window: (u32, u32), count: u32, hour: i32) -> Vec<u32> {
    let phase = hour.rem_euclid(crate::sampled_verify::ROTATION as i32) as u32;
    (0..count)
        .filter(|&slot| {
            in_window(slot, window, count) || slot % crate::sampled_verify::ROTATION as u32 == phase
        })
        .collect()
}

fn every(count: u32) -> Vec<u32> {
    (0..count).collect()
}

/// The windows the next pass visits.
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

/// A world read once in full, its carry holding hour 1.
fn held(provinces: u32, regions: u32, budgets: (i32, i32)) -> (World, Box<WeatherCarry>) {
    let world = World::new(provinces, regions, budgets);
    let (carry, census) = hour(&world, carry(), 1, false, Some(0));
    assert_eq!(read(&world, &census), (every(regions), every(provinces)));
    assert!(carry.held.is_some());
    (world, carry)
}

#[test]
fn window_ports_the_update_passes_loop() {
    // Ordinary: the budget from the cursor.
    assert_eq!(window(5, 10, 3), Some((5, 3)));
    // Wrapping at the count.
    assert_eq!(window(8, 10, 3), Some((8, 3)));
    assert_eq!(advanced(8, 3, 10), 1);
    // A budget of zero or below still visits once (do/while).
    assert_eq!(window(2, 10, 0), Some((2, 1)));
    assert_eq!(window(2, 10, -7), Some((2, 1)));
    // A budget past the count stops when the cursor returns to its start.
    assert_eq!(window(4, 10, 584), Some((4, 10)));
    assert_eq!(advanced(4, 10, 10), 4);
    // A cursor at or past the count visits nothing.
    assert_eq!(window(10, 10, 3), Some((10, 0)));
    // A negative cursor or an empty array is refused.
    assert_eq!(window(-1, 10, 3), None);
    assert_eq!(window(0, 0, 3), None);
}

/// The old read took every record every hour; a held hour now reads only
/// the slots the pass visited (and the hour's comparison slice), sized from
/// the live budgets a mod sets, and captures exactly what a full read does.
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
        // Moving a cursor while held is a jump: the next hour reads all.
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
                    expected(region_window, regions, at),
                    expected(province_window, provinces, at)
                ),
                "{budgets:?} hour {at}"
            );
        }
    }
}

/// A reseed, reset, load or console command moves the write count: the hour
/// reads every record, including the ones it wrote outside the window.
#[test]
fn a_counted_write_forces_a_full_read() {
    let (mut world, carry) = held(40, 9, (5, 2));
    world.pass(1);
    // The console snow/mud command or a reseed writes outside the window.
    world.write_province(30, -123_456);
    let (carry, census) = hour(&world, carry, 2, false, Some(2));
    assert_eq!(read(&world, &census), (every(9), every(40)));
    assert_eq!(carry.followed, 0);
    // Held again under the new count: the next pass is a window hour.
    world.pass(2);
    let (carry, _) = hour(&world, carry, 3, false, Some(2));
    assert_eq!(carry.followed, 3);
    // An unobserved count (hooks off) never follows the window.
    world.pass(3);
    let (carry, census) = hour(&world, carry, 4, false, None);
    assert_eq!(read(&world, &census), (every(9), every(40)));
    assert!(carry.held.is_none());
}

/// A cursor that moved by anything but one ordinary pass reads all.
#[test]
fn a_cursor_jump_falls_back() {
    let (mut world, carry) = held(40, 9, (5, 2));
    // Two passes in one hour (the console command that runs hourly state).
    world.pass(1);
    world.pass(2);
    let (carry, census) = hour(&world, carry, 2, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
    // No pass (the suspension gates held it).
    let (carry, census) = hour(&world, carry, 3, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
    // A region cursor that moved alone.
    world.pass(3);
    let (carry, _) = hour(&world, carry, 4, false, Some(0));
    assert_eq!(carry.followed, 3);
    world.pass(4);
    world.cursors((world.word(MANAGER + cweather_manager::UPDATE_CURSOR), 0));
    let (_, census) = hour(&world, carry, 5, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
}

/// One fallback case: what it does to the world, the hour and the forced
/// flag.
type Case = dyn Fn(&mut World, &mut i32, &mut bool);

/// A gap, a forced full hour, the initial-mode or suspension bytes, or a
/// budget a content reload changed each read every record.
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
        // Held again: the next ordinary pass follows the window.
        world.pass(at.into());
        at += 1;
        let (next, _) = hour(&world, next, at, false, Some(0));
        assert_eq!(next.followed, 3, "hour {at}");
        carry = next;
    }
    // A job that did not complete forgets what it held.
    carry.forget();
    world.pass(99);
    let (_, census) = hour(&world, carry, at + 1, false, Some(0));
    assert_eq!(read(&world, &census), (every(9), every(40)));
}

/// A write no hook saw differs from the carried record: the hour reads
/// every record and the carry stays poisoned; one the count explains does
/// not poison.
#[test]
fn a_difference_reads_all_and_only_an_unexplained_one_poisons() {
    let (mut world, mut carry) = held(40, 9, (5, 2));
    carry.every = true;
    // Explained: the count moved while the hour read.
    world.pass(1);
    world.write_province(30, 77);
    WRITES.with(|writes| writes.set(Some(2)));
    let (carry, _) = hour(&world, carry, 2, false, Some(0));
    assert!(!carry.poisoned);
    assert_eq!(carry.followed, 2);
    // Held under the moved count: a full hour, then the window.
    WRITES.with(|writes| writes.set(Some(4)));
    world.pass(2);
    let (carry, _) = hour(&world, carry, 3, false, Some(4));
    assert_eq!(carry.followed, 0);
    world.pass(3);
    let (carry, _) = hour(&world, carry, 4, false, Some(4));
    assert_eq!(carry.followed, 3);
    // Unexplained: a region's combat entry changed outside the window.
    let (region_window, _) = next_windows(&world);
    world.pass(4);
    let outside = (0..9)
        .find(|&slot| !in_window(slot, region_window, 9))
        .unwrap();
    let value = world.combat_value(outside);
    world.put(value, &(-5_i64).to_le_bytes());
    let (carry, _) = hour(&world, carry, 5, false, Some(4));
    assert!(carry.poisoned);
    assert_eq!(carry.followed, 2);
    world.pass(5);
    let (carry, _) = hour(&world, carry, 6, false, Some(4));
    assert!(carry.poisoned);
    assert_eq!(carry.followed, 0);
}

/// A province record difference reads every province that hour, and every province's
/// weather modifier entries with it: an entry changed outside the window and outside the
/// hour's comparison slice is read afresh, not carried. A later entry difference the
/// slice finds poisons the carry like a record difference.
#[test]
fn a_record_difference_reads_every_province_modifier_entry() {
    let rotation = crate::sampled_verify::ROTATION as u32;
    let mut world = World::new(80, 9, (5, 2));
    let key = PROVINCE_MODIFIER_KEYS[0];
    let entries: Vec<usize> = (0..80)
        .map(|slot| world.province_entry(slot, key, i64::from(slot)))
        .collect();
    let (carry, _) = hour(&world, carry(), 1, false, Some(0));
    assert_eq!(carry.province_modifiers.len(), 80);
    // Hour 2 compares the slots of its phase; one record there and one entry elsewhere
    // change outside the window.
    let (_, window) = next_windows(&world);
    world.pass(1);
    let phase = 2 % rotation;
    let outside = |slot: &u32| !in_window(*slot, window, 80);
    let record = (0..80)
        .filter(outside)
        .find(|slot| slot % rotation == phase)
        .unwrap();
    let entry = (0..80)
        .filter(outside)
        .find(|slot| slot % rotation != phase && *slot != record)
        .unwrap();
    world.write_province(record, -1);
    world.put(entries[entry as usize], &(-7_i64).to_le_bytes());
    // `hour` asserts the carried capture equals a full read of the same memory.
    let (carry, census) = hour(&world, carry, 2, false, Some(0));
    assert!(carry.poisoned);
    assert!(!carry.provinces_followed);
    assert_eq!(read(&world, &census).1, every(80));
}

/// A weather hour's guarded read at the vanilla census (13,414 provinces,
/// 305 region slots, `provinces_per_update` 584 and `regions_per_update` 7
/// from vanilla `common/weather.txt`): every record, the windows with the
/// hour's comparison slice, and the windows with every carried record
/// compared (`weather_verify`). Cold: the world is evicted from cache before
/// each hour, as a game hour finds it.
/// `cargo test --release -p collector weather_window_bench -- --ignored --nocapture`
#[test]
#[ignore = "synthetic guarded weather window timing"]
fn weather_window_bench() {
    let mut world = World::build(13_414, 305, (584, 7), true);
    let live = unsafe { engine::memory::LiveMemory::new() };
    let state = world.base + STATE;
    let mut evict = vec![0_u8; 256 << 20];
    let mut jobs: [WeatherJob; 3] = std::array::from_fn(|_| WeatherJob::new());
    for (index, job) in jobs.iter_mut().enumerate().skip(1) {
        job.carry = Some(Box::new(WeatherCarry {
            observe: observed,
            every: index == 2,
            ..Default::default()
        }));
    }
    let mut samples: [Vec<f64>; 3] = Default::default();
    let mut read = [0_u64; 3];
    for at in 1..=40 {
        world.pass(i64::from(at));
        for (index, job) in jobs.iter_mut().enumerate() {
            if let Some(carry) = job.carry.as_deref_mut() {
                carry.begin(at, false, Some(0));
            }
            evict.fill(at as u8);
            std::hint::black_box(&evict);
            let started = std::time::Instant::now();
            let report = copy_weather_refresh_growing(
                &live,
                WeatherCopyContext {
                    base: 0,
                    game_state: state,
                },
                job,
            );
            let elapsed = started.elapsed().as_secs_f64() * 1e3;
            assert!(report.raw_ok);
            if at > 2 {
                samples[index].push(elapsed);
                read[index] += report.raw_bytes.iter().sum::<u64>();
                if let Some(carry) = job.carry.as_deref() {
                    assert_eq!(carry.followed, 3, "hour {at}");
                }
            }
        }
    }
    for (index, (name, mut sample)) in [
        "every record",
        "windows + hourly slice",
        "windows + every carried record (weather_verify)",
    ]
    .into_iter()
    .zip(samples)
    .enumerate()
    {
        sample.sort_by(f64::total_cmp);
        let len = sample.len();
        println!(
            "WEATHER_WINDOW {name}: {:.3} ms median ({:.3}..{:.3}), {} bytes/hour",
            sample[len / 2],
            sample[0],
            sample[len - 1],
            read[index] / len as u64,
        );
    }
}
