use super::*;
use engine::{carmy, cequipment_variant as variant, cequipment_variant_pool as pool};
use std::{hint::black_box, time::Instant};

static SERIAL: Mutex<()> = Mutex::new(());
static FORWARDED: Mutex<[usize; 8]> = Mutex::new([0; 8]);

// Real allocated addresses exercise the production SEH-guarded reads. The
// fake objects carry the native layout and arbitrary, non-content identities.
struct World {
    storage: Vec<u64>,
    designs: usize,
}
impl World {
    fn new(designs: usize) -> Self {
        let mut world = Self {
            storage: vec![0; (0x600 + designs * 0x40) / 8],
            designs,
        };
        world.owner(1);
        let p = world.army() + carmy::EQUIPMENT_POOL;
        world.word(carmy::EQUIPMENT_POOL, pool::VTABLE_RVA as u64);
        world.word(
            carmy::EQUIPMENT_POOL + pool::ITEMS,
            (world.army() + 0x600) as u64,
        );
        world.word(
            carmy::EQUIPMENT_POOL + pool::CAPACITY,
            (designs as u64) << 32 | designs as u64,
        );
        for d in 0..designs {
            let v = world.design(d);
            world.word(0x600 + d * 16, v as u64);
            world.word(0x600 + designs * 16 + d * 0x30, variant::VTABLE_RVA as u64);
            world.word(
                0x600 + designs * 16 + d * 0x30 + 8,
                ((d as u64 + 0xf000) << 32) | variant::REFERENCE_TYPE as u64,
            );
            world.amount(d, 900000);
        }
        assert_eq!(p, world.army() + carmy::EQUIPMENT_POOL);
        world
    }
    fn army(&self) -> usize {
        self.storage.as_ptr() as usize
    }
    fn design(&self, d: usize) -> usize {
        self.army() + 0x600 + self.designs * 16 + d * 0x30
    }
    fn word(&mut self, at: usize, value: u64) {
        self.storage[at / 8] = value;
    }
    fn rebase(&mut self, base: usize) {
        self.word(carmy::EQUIPMENT_POOL, (base + pool::VTABLE_RVA) as u64);
        for d in 0..self.designs {
            self.word(
                0x600 + self.designs * 16 + d * 0x30,
                (base + variant::VTABLE_RVA) as u64,
            );
        }
    }
    fn owner(&mut self, country: u32) {
        self.word(engine::cunit::ROSTER_OWNER, country as u64);
    }
    fn amount(&mut self, d: usize, amount: i64) {
        self.word(0x600 + d * 16 + 8, amount as u64);
    }
}

unsafe extern "system" fn subtract(pool: usize, design: usize, amount: usize) {
    let items = unsafe { *((pool + pool::ITEMS) as *const usize) };
    let count = unsafe { *((pool + pool::COUNT) as *const i32) };
    for i in 0..count as usize {
        let row = (items + i * 16) as *mut u64;
        if unsafe { *row } == design as u64 {
            // 140FF7A30 clamps signed quantities at these fixed5 limits; it
            // does not clamp an exhausted pool to zero.
            let limit = 0x53e2d620fb40i128;
            unsafe {
                *row.add(1) = ((*row.add(1) as i64 as i128 - amount as i64 as i128)
                    .clamp(-limit, limit) as i64) as u64;
            }
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn forwarding(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    *FORWARDED.lock().unwrap() = [a, b, c, d, e, f, g, h];
    assert_eq!(active(), 1);
    assert!(crate::runtime::observers_active());
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn land_removal(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    unsafe {
        forwarding(a, b, c, d, e, f, g, h);
        subtract(a + carmy::EQUIPMENT_POOL, b, c);
    }
}

// A known real return PC proves the production entry shim's ninth argument.
// This framed test caller also preserves all four stack arguments and carries
// unwind metadata, using the same eight-slot Win64 ABI as the native writer.
core::arch::global_asm!(
    ".text", ".globl destruction_land_test_caller", ".seh_proc destruction_land_test_caller",
    "destruction_land_test_caller:", "sub rsp, 0x48", ".seh_stackalloc 0x48", ".seh_endprologue",
    "mov rax, [rsp + 0x70]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x78]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0x80]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0x88]", "mov [rsp + 0x38], rax",
    "call {entry}", ".globl destruction_land_test_return", "destruction_land_test_return:",
    "add rsp, 0x48", "ret", ".seh_endproc", entry = sym destruction_land_entry);
unsafe extern "system" {
    fn destruction_land_test_caller();
    fn destruction_land_test_return();
}

fn setup(capacity: usize, designs: usize) {
    *STORE.lock().unwrap() = Store {
        entries: Vec::with_capacity(capacity),
        before: vec![Default::default(); designs],
        after: vec![Default::default(); designs],
        raw: vec![0; designs * engine::equipment_entry::STRIDE],
        ..Store::new()
    };
    *CLOCK.lock().unwrap() = None;
    BASE.store(0, Ordering::Relaxed);
    CAPACITY.store(capacity, Ordering::Release);
    DESIGNS.store(designs, Ordering::Release);
    HOURLY_PEAK_EVENTS.store(0, Ordering::Relaxed);
    HOURLY_PEAK_CAPACITY.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    POISONED.store(false, Ordering::SeqCst);
    ORIGINAL[1].store(subtract as *const () as usize, Ordering::Release);
}
fn clock(hour: i32, session: i64) -> Option<Clock> {
    Some(Clock {
        hour,
        play_session_started_at: session,
        game_state: 0,
        play_time: 0,
        seed: 0,
        campaign_id: Default::default(),
        speed: 0,
        tick_busy: 0,
    })
}

#[test]
fn actual_events_every_country_fail_closed_and_detach() {
    let _serial = SERIAL.lock().unwrap();
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let mut world = World::new(3);
    setup(5000, 3);
    boundary(clock(1, 1));
    assert!(
        drain(&mut [Entry::default(); 1])
            .unwrap_err()
            .contains("partial")
    );
    boundary(clock(2, 1));
    let p = world.army() + carmy::EQUIPMENT_POOL;
    let v = world.design(1);
    world.amount(1, 1000000);
    unsafe {
        attrition(p, v, 900000);
    }
    let mut out = vec![Entry::default(); 5000];
    assert_eq!(drain(&mut out).unwrap(), 1);
    assert_eq!(out[0].amount_raw, 900000);
    assert_eq!(
        out[0].variant,
        (0xf001u64 << 32) | variant::REFERENCE_TYPE as u64
    );
    unsafe {
        attrition(p, v, 0);
    }
    assert_eq!(drain(&mut out).unwrap(), 0); // no actual removal
    for countries in [1500, 5000] {
        for country in 0..countries {
            world.owner(country);
            world.amount(1, 900000);
            unsafe {
                attrition(p, v, 100000);
            }
        }
        let len = drain(&mut out).unwrap();
        assert_eq!(len, countries as usize);
        for (country, row) in out[..len].iter().enumerate() {
            assert_eq!(row.country, country as u32);
        }
    }
    assert_eq!(status()["hourly_peak_events"].as_u64(), Some(5000));
    assert_eq!(status()["hourly_peak_capacity"].as_u64(), Some(5000));
    // Full-pool path preserves multiple exact designs, including rounding.
    world.amount(0, 450001);
    world.amount(1, 300000);
    world.amount(2, 100000);
    {
        let mut s = STORE.lock().unwrap();
        observe_pool(
            &mut s,
            0,
            world.army(),
            native::Cause::LandCombat,
            &memory,
            0,
            || {
                world.amount(0, 370000);
                world.amount(1, 300000);
                world.amount(2, 0);
            },
        )
        .unwrap();
    }
    assert_eq!(drain(&mut out).unwrap(), 2);
    assert_eq!((out[0].amount_raw, out[1].amount_raw), (80001, 100000));
    // A short output loses completeness permanently on this branch.
    world.amount(1, 900000);
    unsafe {
        attrition(p, v, 100000);
    }
    assert!(drain(&mut []).is_err());
    boundary(clock(3, 1));
    assert!(drain(&mut out).is_err());
    boundary(clock(3, 2)); // actual load signal starts a new partial interval
    assert!(drain(&mut out).unwrap_err().contains("partial"));
    boundary(clock(4, 2));
    assert_eq!(drain(&mut out).unwrap(), 0);
    STORE.lock().unwrap().entries = Vec::new();
    world.amount(1, 900000);
    unsafe {
        attrition(p, v, 100000);
    }
    assert!(drain(&mut out).unwrap_err().contains("overflow"));
    boundary(clock(5, 2));
    assert!(drain(&mut out).is_err());
    // A writer overlapping the drain also breaks completeness.
    boundary(clock(1, 3));
    boundary(clock(2, 3));
    let held = STORE.lock().unwrap();
    unsafe {
        attrition(p, v, 100000);
    }
    drop(held);
    assert!(drain(&mut out).is_err());
    assert_eq!(active(), 0);
    boundary(clock(1, 4));
    boundary(clock(2, 4));
    STORE.lock().unwrap().entries = Vec::with_capacity(2);
    world.amount(1, 50);
    unsafe {
        attrition(p, v, 100);
    }
    assert!(drain(&mut out).unwrap_err().contains("negative"));
    for site in 0..4 {
        let held = Call::enter(site);
        assert!(crate::runtime::observers_active());
        drop(held);
        assert_eq!(active(), 0);
    }
    // Actual assembly shim round-trip preserves all eight full ABI slots.
    ENABLED.store(false, Ordering::SeqCst);
    ORIGINAL[0].store(forwarding as *const () as usize, Ordering::Release);
    let shim: ArmyFn = unsafe { std::mem::transmute(destruction_land_entry as *const ()) };
    let slots = std::array::from_fn::<_, 8, _>(|i| usize::MAX - i);
    unsafe {
        shim(
            slots[0], slots[1], slots[2], slots[3], slots[4], slots[5], slots[6], slots[7],
        );
    }
    assert_eq!(*FORWARDED.lock().unwrap(), slots);
    assert_eq!(active(), 0);
}

#[test]
fn failed_prepare_observations_skip_growth_without_losing_the_branch() {
    let _serial = SERIAL.lock().unwrap();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let entry = Entry {
        country: 4999,
        cause: native::Cause::Attrition,
        variant: 0xf00000000046,
        amount_raw: 100000,
    };
    STORE.lock().unwrap().append(entry).unwrap();
    let failures = PREPARE_FAILURES.load(Ordering::Relaxed);
    let lost = LOST.load(Ordering::Relaxed);
    let initial_capacity = capacity();
    let initial_designs = DESIGNS.load(Ordering::Acquire);

    // The design census is unreadable: retain the existing buffers and event.
    prepare(
        &engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        },
        0,
        5000,
    );
    // A readable empty census still cannot represent this requested bound.
    prepare(
        &engine::memory::SliceMemory {
            base: engine::cgame_state::SINGLETON_RVA,
            bytes: &[0; 8],
        },
        0,
        usize::MAX,
    );
    assert_eq!(PREPARE_FAILURES.load(Ordering::Relaxed), failures + 2);
    assert_eq!(status()["prepare_failures"].as_u64(), Some(failures + 2));
    assert_eq!(LOST.load(Ordering::Relaxed), lost);
    assert_eq!(capacity(), initial_capacity);
    assert_eq!(DESIGNS.load(Ordering::Acquire), initial_designs);
    assert!(!POISONED.load(Ordering::SeqCst));
    assert_eq!(HOURLY_PEAK_EVENTS.load(Ordering::Relaxed), 1);
    assert_eq!(
        HOURLY_PEAK_CAPACITY.load(Ordering::Relaxed),
        initial_capacity
    );
    let mut out = [Entry::default(); 1];
    assert_eq!(drain(&mut out).unwrap(), 1);
    assert_eq!(out[0], entry);
    ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn enabled_land_shim_passes_caller_pc_and_records_stable_causes() {
    use replay::{
        frame::TableView,
        rows::{EquipmentDestructionCause, TableRows},
    };

    let _serial = SERIAL.lock().unwrap();
    let caller = destruction_land_test_return as *const () as usize;
    let invoke: ArmyFn = unsafe { std::mem::transmute(destruction_land_test_caller as *const ()) };
    for (return_rva, cause, recorded_cause) in [
        (
            native::LAND_COMBAT_RETURN,
            native::Cause::LandCombat,
            EquipmentDestructionCause::LandCombat,
        ),
        (
            native::NAVAL_TRANSPORT_RETURN,
            native::Cause::NavalTransport,
            EquipmentDestructionCause::NavalTransport,
        ),
        (
            native::IMMEDIATE_RETURN,
            native::Cause::Immediate,
            EquipmentDestructionCause::Immediate,
        ),
        (
            native::LAND_COMBAT_RETURN + 1,
            native::Cause::Unknown,
            EquipmentDestructionCause::Unknown,
        ),
    ] {
        let mut world = World::new(1);
        setup(8, 1);
        let base = caller.checked_sub(return_rva).unwrap();
        BASE.store(base, Ordering::Relaxed);
        world.rebase(base);
        ORIGINAL[0].store(land_removal as *const () as usize, Ordering::Release);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let unknown = UNKNOWN_CALLERS.load(Ordering::Relaxed);
        let slots = [
            world.army(),
            world.design(0),
            100000,
            usize::MAX - 3,
            usize::MAX - 4,
            usize::MAX - 5,
            usize::MAX - 6,
            usize::MAX - 7,
        ];
        assert!(ENABLED.load(Ordering::SeqCst));
        unsafe {
            invoke(
                slots[0], slots[1], slots[2], slots[3], slots[4], slots[5], slots[6], slots[7],
            );
        }
        assert_eq!(*FORWARDED.lock().unwrap(), slots);
        assert_eq!(active(), 0);
        assert!(!POISONED.load(Ordering::SeqCst));
        assert_eq!(
            UNKNOWN_CALLERS.load(Ordering::Relaxed),
            unknown + u64::from(cause == native::Cause::Unknown)
        );
        let mut captured = crate::extended::equipment_destruction::Captured::with_capacities(&1);
        captured.capture(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
        );
        let [TableView::Complete(TableRows::EquipmentDestruction(rows))] = captured.tables() else {
            panic!("enabled shim event must publish a complete destruction table");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cause, recorded_cause);
        assert_eq!(rows[0].country, 1);
        assert_eq!(rows[0].amount_raw, 100000);
        assert_eq!(
            rows[0].variant,
            (0xf000u64 << 32) | variant::REFERENCE_TYPE as u64
        );
        assert_eq!(status()["hourly_peak_events"].as_u64(), Some(1));
        assert_eq!(status()["hourly_peak_capacity"].as_u64(), Some(8));
    }
    ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
#[ignore = "release benchmark; run with --release --ignored --nocapture --test-threads=1"]
fn destruction_scale_bench() {
    let _serial = SERIAL.lock().unwrap();
    if cfg!(debug_assertions) {
        panic!("benchmark requires release");
    }
    for countries in [1500usize, 5000] {
        for designs in [16usize, 64] {
            let mut worlds: Vec<_> = (0..countries)
                .map(|country| {
                    let mut world = World::new(designs);
                    world.owner(country as u32);
                    world
                })
                .collect();
            let events = countries * 2;
            let capacity = log_capacity(countries, designs).unwrap();
            setup(capacity, designs);
            boundary(clock(1, 1));
            boundary(clock(2, 1));
            let mut out = vec![Entry::default(); capacity];
            let mut event_ns = Vec::new();
            let mut drain_ns = Vec::new();
            for _ in 0..21 {
                let start = Instant::now();
                for e in 0..events {
                    let world = &mut worlds[e % countries];
                    let p = world.army() + carmy::EQUIPMENT_POOL;
                    world.amount(e % designs, 900000);
                    unsafe {
                        attrition(black_box(p), black_box(world.design(e % designs)), 100000);
                    }
                }
                event_ns.push(start.elapsed().as_nanos() as u64 / events as u64);
                let start = Instant::now();
                let len = drain(black_box(&mut out)).unwrap();
                drain_ns.push(start.elapsed().as_nanos() as u64);
                assert_eq!(len, events);
                black_box(&out[..len]);
            }
            event_ns.sort_unstable();
            drain_ns.sort_unstable();
            println!(
                "destruction_bench countries={countries} pool_designs={designs} events_per_hour={events} capacity={} event_median_ns={} event_p95_ns={} drain_median_ns={} drain_p95_ns={} total_median_ms={:.3} producer_bytes={} drain_bytes={} fixture_bytes={}",
                capacity,
                event_ns[10],
                event_ns[19],
                drain_ns[10],
                drain_ns[19],
                (event_ns[10] * events as u64 + drain_ns[10]) as f64 / 1e6,
                capacity * std::mem::size_of::<Entry>()
                    + 2 * designs * std::mem::size_of::<equipment_entry::Row>()
                    + designs * equipment_entry::STRIDE,
                out.len() * std::mem::size_of::<Entry>(),
                worlds.iter().map(|w| w.storage.len() * 8).sum::<usize>()
            );
        }
    }
    ENABLED.store(false, Ordering::SeqCst);
}
