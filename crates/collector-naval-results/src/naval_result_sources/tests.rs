use super::*;
#[cfg(test)]
use engine_native::{cgame_state, cnaval_combat, creference_registry};
use engine_naval_history::cnaval_combat_results;

static SERIAL: Mutex<()> = Mutex::new(());
static FORWARDED: AtomicUsize = AtomicUsize::new(0);
static PROBE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

pub fn with_active_observer(check: impl FnOnce()) {
    let call = Call::enter(0);
    check();
    call.returned(0);
}

pub struct Observation {
    _serial: std::sync::MutexGuard<'static, ()>,
}
impl Observation {
    pub fn new() -> Self {
        let serial = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self { _serial: serial }
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
struct World {
    globals: Vec<u64>,
    combat: Vec<u64>,
    result: Vec<u64>,
}
#[cfg(test)]
impl World {
    fn new() -> Self {
        let invalid_at =
            creference_registry::INVALID_REFERENCE_RVA.0 - cgame_state::SINGLETON_RVA.0;
        let globals_end = (invalid_at + size_of::<u64>())
            .max(0x100 + cgame_state::CURRENT_HOUR + size_of::<i32>());
        let mut world = Self {
            globals: vec![0; globals_end.div_ceil(size_of::<u64>())],
            combat: vec![0; 0x200 / 8],
            result: vec![0; 0x200 / 8],
        };
        let base = world.base();
        world.globals[0] = world.state() as u64;
        world.globals[invalid_at / 8] = u64::MAX;
        world.combat[0] = (base + cnaval_combat::VTABLE_RVA.0) as u64;
        world.result[0] = (base + cnaval_combat_results::VTABLE_RVA.0) as u64;
        world.combat[cnaval_combat::HANDLE / 8] = 0xf001_0000_0000_0046;
        world.combat[0x20 / 8] = 1;
        world.result[cnaval_combat_results::HANDLE / 8] = 0xf002_0000_0000_0046;
        world.result[0x10 / 8] = 1;
        world.hour(11);
        world
    }
    fn base(&self) -> usize {
        self.globals.as_ptr() as usize - cgame_state::SINGLETON_RVA.0
    }
    fn state(&self) -> usize {
        self.globals.as_ptr() as usize + 0x100
    }
    fn combat(&self) -> usize {
        self.combat.as_ptr() as usize
    }
    fn result(&self) -> usize {
        self.result.as_ptr() as usize
    }
    fn hour(&mut self, hour: i32) {
        self.globals[(0x100 + cgame_state::CURRENT_HOUR) / 8] = hour as u32 as u64;
    }
    fn transient(&mut self) {
        self.combat[cnaval_combat::HANDLE / 8] = u64::MAX;
        self.combat[0x20 / 8] = 0;
        let flags = (self.combat() + cnaval_combat::RESULT_FLAG_A) as *mut u8;
        unsafe {
            *flags = 1;
        }
    }
}
unsafe extern "system-unwind" fn original_constructor(pointer: usize) -> usize {
    assert!(active() > 0);
    assert!(active() != 0);
    FORWARDED.fetch_add(1, Ordering::Relaxed);
    pointer
}
#[cfg(test)]
static GATE: std::sync::LazyLock<std::sync::Barrier> =
    std::sync::LazyLock::new(|| std::sync::Barrier::new(2));
#[cfg(test)]
unsafe extern "system-unwind" fn gated_result(result: usize, combat: usize) -> usize {
    GATE.wait();
    GATE.wait();
    unsafe { original_result(result, combat) }
}
unsafe extern "system-unwind" fn original_result(result: usize, _combat: usize) -> usize {
    assert!(active() > 0);
    assert!(active() != 0);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let hour = native::creation_hour(&memory, BASE.load(Ordering::Relaxed)).unwrap();
    unsafe {
        *((result + cnaval_combat_results::RESULT_HOUR) as *mut i32) = hour;
    }
    FORWARDED.fetch_add(1, Ordering::Relaxed);
    0xabc
}
pub fn setup(capacity: usize, base: usize) {
    drop(LOG.take());
    *store().lock().unwrap() = Store {
        entries: Vec::with_capacity(capacity),
        births: HashMap::with_capacity(capacity),
        ..Store::new()
    };
    *CLOCK.lock().unwrap() = None;
    BASE.store(base, Ordering::Relaxed);
    EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
    BIRTH_ATTEMPTS.store(0, Ordering::Relaxed);
    EVENT_DEMAND.store(0, Ordering::Relaxed);
    BIRTH_DEMAND.store(0, Ordering::Relaxed);
    HOURLY_PEAK.store(0, Ordering::Relaxed);
    HOURLY_PEAK_CAPACITY.store(0, Ordering::Relaxed);
    GAME_THREAD.store(0, Ordering::Relaxed);
    for thread in &CALL_THREADS {
        thread.store(0, Ordering::Relaxed);
    }
    THREAD_MISMATCHES.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    for original in &ORIGINAL[..2] {
        original.store(original_result as *const () as usize, Ordering::Release);
    }
    ORIGINAL[2].store(
        original_constructor as *const () as usize,
        Ordering::Release,
    );
}
#[cfg(test)]
fn folded() -> std::sync::MutexGuard<'static, Store> {
    let mut current = store().lock().unwrap();
    current.fold();
    current
}
#[cfg(test)]
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
fn release_keeps_storage_until_every_site_and_callback_is_quiet() {
    let _observation = Observation::new();
    setup(2, 0);
    store().lock().unwrap().entries.push(Entry {
        id: 7,
        ..Entry::default()
    });
    lost_drain();
    assert!(release().is_err());
    ENABLED.store(false, Ordering::SeqCst);
    ACTIVE[1].store(1, Ordering::SeqCst);
    assert!(release().is_err());
    ACTIVE[1].store(0, Ordering::SeqCst);
    SITE_ENABLED[2].store(true, Ordering::SeqCst);
    assert!(release().is_err());
    assert_eq!(store().lock().unwrap().entries[0].id, 7);
    assert!(!LOG.is_empty());
    SITE_ENABLED[2].store(false, Ordering::SeqCst);
    release().unwrap();
    assert!(LOG.is_empty());
    assert_eq!(store().lock().unwrap().entries.capacity(), 0);
    assert_eq!(store().lock().unwrap().births.capacity(), 0);
}

#[test]
fn reattached_source_ids_continue_retained_history_and_survive_branch_boundaries() {
    let _observation = Observation::new();
    let mut world = World::new();
    setup(2, world.base());
    ENABLED.store(false, Ordering::SeqCst);
    restore_sequence(40).unwrap();
    restore_sequence(7).unwrap();
    assert_eq!(store().lock().unwrap().next_id, 41);
    assert_eq!(
        restore_sequence(u64::MAX),
        Err("naval result source event sequence exhausted")
    );
    ENABLED.store(true, Ordering::SeqCst);
    assert!(restore_sequence(99).is_err());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    unsafe {
        fill(world.result(), world.combat());
    }
    let mut output = vec![Entry::default(); 2];
    assert_eq!(drain(&mut output).unwrap(), 1);
    assert_eq!(output[0].id, 41);
    boundary(clock(12, 2));
    boundary(clock(13, 2));
    world.hour(13);
    unsafe {
        fill(world.result(), world.combat());
    }
    assert_eq!(drain(&mut output).unwrap(), 1);
    assert_eq!(output[0].id, 42);
    let retained = output[0].id;
    setup(2, world.base());
    ENABLED.store(false, Ordering::SeqCst);
    restore_sequence(retained).unwrap();
    ENABLED.store(true, Ordering::SeqCst);
    boundary(clock(14, 2));
    boundary(clock(15, 2));
    world.hour(15);
    unsafe {
        fill(world.result(), world.combat());
    }
    assert_eq!(drain(&mut output).unwrap(), 1);
    assert_eq!(output[0].id, retained + 1);
}

#[test]
fn actual_new_merge_transient_and_skipped_sources() {
    let _observation = Observation::new();
    let mut world = World::new();
    setup(40, world.base());
    boundary(clock(10, 1));
    let mut output = vec![Entry::default(); 40];
    assert_eq!(
        drain(&mut output).unwrap_err(),
        "naval result sources first/branch interval is partial"
    );
    boundary(clock(11, 1));
    unsafe {
        assert_eq!(
            constructor(world.combat(), world.base() + 0xc0b22d),
            world.combat()
        );
        assert_eq!(fill(world.result(), world.combat()), 0xabc);
    }
    world.transient();
    world.result[cnaval_combat_results::HANDLE / 8] += 1 << 32;
    unsafe {
        constructor(world.combat(), world.base() + 0xc0bf4c);
        fill(world.result(), world.combat());
    }
    unsafe {
        constructor(world.combat(), world.base() + 0xc0bf4c);
        merge(world.result(), world.combat());
    }
    assert_eq!(drain(&mut output).unwrap(), 3);
    assert_eq!(output[0].id, 1);
    assert_eq!(output[0].combat_created_hour, Some(11));
    assert!(!output[0].merged);
    assert_ne!(output[0].result, output[1].result);
    assert_eq!(output[1].result, output[2].result);
    assert_ne!(output[0].combat, output[1].combat);
    assert!(!output[1].merged && output[1].transient);
    assert!(output[2].merged && output[2].transient);
    assert_eq!(output[2].combat, u64::MAX);
    assert_eq!(output[2].combat_created_hour, Some(11));
    unsafe {
        constructor(world.combat(), world.base() + 0xc0bf4c);
    }
    assert_eq!(drain(&mut output).unwrap(), 0);
    boundary(clock(12, 1));
    world.hour(12);
    world.combat[0x20 / 8] = 1;
    for index in 0..40u64 {
        world.combat[cnaval_combat::HANDLE / 8] = ((index + 1) << 32) | 0xf123;
        unsafe {
            fill(world.result(), world.combat());
        }
    }
    assert_eq!(drain(&mut output).unwrap(), 40);
    for (index, event) in output.iter().enumerate() {
        assert_eq!(event.id, index as u64 + 4);
        assert_eq!(event.combat, ((index as u64 + 1) << 32) | 0xf123);
        assert_eq!(event.game_hour, 12);
    }
    assert_eq!(active(), 0);
}

#[test]
fn worker_drain_grows_empty_and_recycled_storage_for_every_owned_source() {
    let _observation = Observation::new();
    let mut output = Vec::new();
    for count in [5000, 10003] {
        setup(count, 0);
        boundary(clock(10, 1));
        assert!(drain(&mut output).is_err());
        boundary(clock(11, 1));
        {
            let mut store = store().lock().unwrap();
            collector_test_support::with_allocations_denied(|| {
                for index in 0..count {
                    store.entries.push(Entry {
                        id: index as u64 + 1,
                        game_hour: 10,
                        result: ((index as u64 + 1) << 32) | 0x46,
                        combat: u64::MAX,
                        transient: true,
                        ..Default::default()
                    });
                }
            });
        }
        std::thread::scope(|threads| {
            threads
                .spawn(|| {
                    assert_eq!(drain(&mut output), Ok(count));
                    boundary(clock(12, 1));
                    assert_eq!(CLOCK.lock().unwrap().unwrap().hour, 12);
                    lost_drain();
                    assert_eq!(
                        drain(&mut output),
                        Err("naval result sources lost observation; interval unavailable")
                    );
                })
                .join()
                .unwrap();
        });
        assert_eq!(output[count - 1].id, count as u64);
        assert_eq!(output[count - 1].result, (count as u64) << 32 | 0x46);
    }
}

#[test]
fn a_held_drain_never_blocks_or_loses_a_callback_and_detach_publishes_nothing() {
    let _observation = Observation::new();
    let world = World::new();
    setup(1, world.base());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    let before = FORWARDED.load(Ordering::Relaxed);
    {
        let _held = store().lock().unwrap();
        unsafe {
            fill(world.result(), world.combat());
        }
    }
    assert_eq!(FORWARDED.load(Ordering::Relaxed), before + 1);
    assert_eq!(drain(&mut vec![Entry::default(); 1]), Ok(1));
    boundary(clock(12, 2));
    boundary(clock(13, 2));
    lost_drain();
    assert!(drain(&mut vec![Entry::default(); 1]).is_err());
    boundary(clock(14, 3));
    boundary(clock(15, 3));
    unsafe {
        fill(world.result(), world.combat());
        fill(world.result(), world.combat());
    }
    assert_eq!(drain(&mut vec![Entry::default(); 1]), Ok(2));
    assert!(EVENT_DEMAND.load(Ordering::Relaxed) >= 2);
    let call = Call::enter(0);
    assert!(active() != 0);
    call.returned(0);
    assert_eq!(active(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    let before = FORWARDED.load(Ordering::Relaxed);
    unsafe {
        fill(world.result(), world.combat());
    }
    assert_eq!(FORWARDED.load(Ordering::Relaxed), before + 1);
    assert_eq!(active(), 0);
    assert!(drain(&mut vec![Entry::default(); 1]).is_err());
}

#[test]
fn two_threads_in_the_family_at_once_keep_every_source_in_the_order_they_finish() {
    let _observation = Observation::new();
    let world = World::new();
    setup(2, world.base());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    ORIGINAL[0].store(gated_result as *const () as usize, Ordering::Release);
    let (result, combat) = (world.result(), world.combat());
    let writer = std::thread::spawn(move || unsafe { fill(result, combat) });
    GATE.wait();
    let merged = unsafe { merge(result, combat) };
    GATE.wait();
    let filled = writer.join().unwrap();
    let mut output = Vec::new();
    let drained = drain(&mut output);
    assert_eq!((filled, merged), (0xabc, 0xabc));
    assert_eq!(drained, Ok(2));
    let row = |id, merged| Entry {
        id,
        game_hour: 11,
        result: world.result[cnaval_combat_results::HANDLE / 8],
        combat: world.combat[cnaval_combat::HANDLE / 8],
        merged,
        transient: false,
        combat_created_hour: None,
    };
    assert_eq!(output, [row(1, true), row(2, false)]);
    assert_eq!(active(), 0);
}

#[test]
fn a_callback_across_the_hour_boundary_is_kept_in_the_interval_it_completes() {
    let _observation = Observation::new();
    let mut world = World::new();
    setup(2, world.base());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    unsafe { fill(world.result(), world.combat()) };
    ORIGINAL[0].store(gated_result as *const () as usize, Ordering::Release);
    let (result, combat) = (world.result(), world.combat());
    let writer = std::thread::spawn(move || unsafe { fill(result, combat) });
    GATE.wait();
    boundary(clock(12, 1));
    let mut first = Vec::new();
    let first_drain = drain(&mut first);
    world.hour(12);
    GATE.wait();
    writer.join().unwrap();
    boundary(clock(13, 1));
    let mut second = Vec::new();
    let second_drain = drain(&mut second);
    assert_eq!(first_drain, Ok(1));
    assert_eq!((first[0].id, first[0].game_hour), (1, 11));
    assert_eq!(second_drain, Ok(1));
    assert_eq!(
        (second[0].id, second[0].game_hour, second[0].merged),
        (2, 12, false)
    );
    assert_eq!(active(), 0);
}

#[cfg(test)]
unsafe extern "system-unwind" fn unlocked_constructor(pointer: usize) -> usize {
    assert!(store().try_lock().is_ok());
    unsafe { original_constructor(pointer) }
}

#[test]
fn loaded_constructor_bypasses_storage_and_unknown_runs_before_refusal() {
    let _observation = Observation::new();
    let world = World::new();
    setup(1, world.base());
    {
        let _held = store().lock().unwrap();
        assert_eq!(
            unsafe { constructor(world.combat(), world.base() + 0xc0dc19) },
            world.combat()
        );
    }
    assert!(LOG.is_empty());
    ORIGINAL[2].store(
        unlocked_constructor as *const () as usize,
        Ordering::Release,
    );
    let unknown = UNKNOWN_CONSTRUCTORS.load(Ordering::Relaxed);
    assert_eq!(
        unsafe { constructor(world.combat(), world.base()) },
        world.combat()
    );
    assert_eq!(UNKNOWN_CONSTRUCTORS.load(Ordering::Relaxed), unknown + 1);
    assert_eq!(
        folded().refusal,
        Some("naval result sources unknown constructor caller")
    );
    assert_eq!(active(), 0);
}

#[test]
fn lost_interval_recovers_partial_then_complete_without_resetting_ids() {
    let _observation = Observation::new();
    let mut world = World::new();
    setup(1, world.base());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    unsafe { fill(world.result(), world.combat()) };
    let mut output = vec![Entry::default(); 1];
    assert_eq!(drain(&mut output).unwrap(), 1);
    assert_eq!(output[0].id, 1);
    lost_drain();
    assert_eq!(
        drain(&mut output).unwrap_err(),
        "naval result sources lost observation; interval unavailable"
    );
    boundary(clock(12, 1));
    assert_eq!(
        drain(&mut output).unwrap_err(),
        "naval result sources recovery interval is partial"
    );
    boundary(clock(13, 1));
    world.hour(13);
    unsafe { fill(world.result(), world.combat()) };
    assert_eq!(drain(&mut output).unwrap(), 1);
    assert_eq!(output[0].id, 2);
    unsafe { fill(world.result(), world.combat()) };
    collector_test_support::with_allocations_denied(|| unsafe {
        fill(world.result(), world.combat())
    });
    boundary(clock(14, 1));
    assert_eq!(drain(&mut output).unwrap_err(), STORAGE_FAILED);
    boundary(clock(15, 1));
    assert_eq!(
        drain(&mut output).unwrap_err(),
        "naval result sources recovery interval is partial"
    );
    boundary(clock(16, 1));
    assert_eq!(drain(&mut output).unwrap(), 0);
    assert_eq!(THREAD_MISMATCHES.load(Ordering::Relaxed), 0);
}

#[test]
fn foreign_capture_and_decode_calls_cannot_mutate_observation_test_globals() {
    const TEST: &str = "naval_result_sources::tests::foreign_capture_and_decode_calls_cannot_mutate_observation_test_globals";
    const CHILD: &str = "BATTLEPLAN_NAVAL_OBSERVER_ISOLATION_TEST";
    if std::env::var_os(CHILD).is_some() {
        lost_drain();
        boundary(clock(20, 2));
        assert_eq!(
            drain(&mut vec![Entry::default(); 1]).unwrap_err(),
            "naval result sources hooks unavailable (see status unhooked)"
        );
        return;
    }
    let _observation = Observation::new();
    setup(1, 0);
    store().lock().unwrap().entries.push(Entry::default());
    let lost = LOST.load(Ordering::Relaxed);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "1")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(LOST.load(Ordering::Relaxed), lost);
    assert!(LOG.is_empty());
    assert!(CLOCK.lock().unwrap().is_none());
    assert_eq!(store().lock().unwrap().entries.len(), 1);
    lost_drain();
    assert_eq!(LOST.load(Ordering::Relaxed), lost + 1);
    assert!(folded().poisoned);
}

#[test]
fn callback_thread_counters_exclude_loaded_and_unobserved_calls() {
    let _observation = Observation::new();
    setup(1, 0);
    boundary(clock(10, 1));
    let expected = GAME_THREAD.load(Ordering::Relaxed);
    let unknown = UNKNOWN_CONSTRUCTORS.load(Ordering::Relaxed);
    ENABLED.store(false, Ordering::SeqCst);
    std::thread::spawn(|| unsafe {
        constructor(0x1234, 0xc0dc19);
        constructor(0x1234, 0);
    })
    .join()
    .unwrap();
    assert_eq!(THREAD_MISMATCHES.load(Ordering::Relaxed), 0);
    assert_eq!(CALL_THREADS[2].load(Ordering::Relaxed), 0);
    assert_eq!(UNKNOWN_CONSTRUCTORS.load(Ordering::Relaxed), unknown + 1);
    ENABLED.store(true, Ordering::SeqCst);
    std::thread::spawn(|| unsafe {
        constructor(0x1234, 0xc0dc19);
        assert_eq!(THREAD_MISMATCHES.load(Ordering::Relaxed), 0);
        constructor(0x1234, 0);
    })
    .join()
    .unwrap();
    assert_ne!(CALL_THREADS[2].load(Ordering::Relaxed), expected);
    assert_eq!(THREAD_MISMATCHES.load(Ordering::Relaxed), 1);
    assert_eq!(UNKNOWN_CONSTRUCTORS.load(Ordering::Relaxed), unknown + 2);
    assert_eq!(active(), 0);
}

#[test]
fn an_event_burst_is_kept_whole_and_raises_the_frame_demand() {
    let _observation = Observation::new();
    let mut world = World::new();
    setup(2, world.base());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    for _ in 0..4 {
        world.combat[cnaval_combat::HANDLE / 8] += 1 << 32;
        world.result[cnaval_combat_results::HANDLE / 8] += 1 << 32;
        unsafe { fill(world.result(), world.combat()) };
    }
    assert_eq!(EVENT_DEMAND.load(Ordering::Relaxed), 4);
    let mut output = vec![Entry::default(); 4];
    assert_eq!(drain(&mut output), Ok(4));
    assert_eq!(output.iter().map(|row| row.id).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(demand(), 4);
}

#[test]
fn a_birth_burst_is_kept_whole_and_raises_the_frame_demand() {
    let _observation = Observation::new();
    let world = World::new();
    setup(1, world.base());
    boundary(clock(10, 1));
    boundary(clock(11, 1));
    for pointer in 1..=4 {
        unsafe { constructor(pointer, world.base() + 0xc0b22d) };
    }
    assert_eq!(folded().births.len(), 4);
    assert_eq!(BIRTH_DEMAND.load(Ordering::Relaxed), 4);
    assert_eq!(drain(&mut Vec::new()), Ok(0));
}

#[test]
fn null_population_is_skipped_and_failed_reads_keep_the_demand() {
    struct Missing;
    impl Memory for Missing {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let _observation = Observation::new();
    let mut world = World::new();
    setup(3, world.base());
    world.globals[0] = 0;
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let population_failures = POPULATION_READ_FAILURES.load(Ordering::Relaxed);
    prepare(&memory, world.base());
    assert_eq!(
        POPULATION_READ_FAILURES.load(Ordering::Relaxed),
        population_failures
    );
    prepare(&Missing, world.base());
    assert_eq!(
        POPULATION_READ_FAILURES.load(Ordering::Relaxed),
        population_failures + 1
    );
    assert_eq!(demand(), 0);
    assert!(LOG.is_empty());
}

core::arch::global_asm!(
    ".text",
    ".globl naval_constructor_test_caller",
    ".seh_proc naval_constructor_test_caller",
    "naval_constructor_test_caller:",
    "sub rsp, 0x28",
    ".seh_stackalloc 0x28",
    ".seh_endprologue",
    "call {entry}",
    ".globl naval_constructor_test_return",
    "naval_constructor_test_return:",
    "add rsp, 0x28",
    "ret",
    ".seh_endproc",
    entry = sym constructor_entry
);
#[cfg(test)]
unsafe extern "system-unwind" {
    fn naval_constructor_test_caller(pointer: usize) -> usize;
    fn naval_constructor_test_return();
}

unsafe extern "system-unwind" fn unwind_constructor_original(pointer: usize) -> usize {
    let original: unsafe extern "system-unwind" fn(usize) -> usize =
        unsafe { std::mem::transmute(PROBE_ORIGINAL.load(Ordering::Relaxed)) };
    unsafe { original(pointer) }
}
unsafe extern "system-unwind" fn unwind_result_original(result: usize, combat: usize) -> usize {
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(PROBE_ORIGINAL.load(Ordering::Relaxed)) };
    unsafe { original(result, combat) }
}
core::arch::global_asm!(
    ".text",
    ".globl naval_unwind_call_caller",
    ".seh_proc naval_unwind_call_caller",
    "naval_unwind_call_caller:",
    "sub rsp, 0x28",
    ".seh_stackalloc 0x28",
    ".seh_endprologue",
    ".globl naval_unwind_call_site",
    "naval_unwind_call_site:",
    "call {result}",
    ".rept 11",
    "nop",
    ".endr",
    "add rsp, 0x28",
    "ret",
    ".seh_endproc",
    ".globl naval_unwind_constructor",
    ".seh_proc naval_unwind_constructor",
    "naval_unwind_constructor:",
    "mov [rsp + 8], rbx",
    "sub rsp, 0x28",
    ".seh_stackalloc 0x28",
    ".seh_endprologue",
    ".rept 11",
    "nop",
    ".endr",
    "call {constructor}",
    "add rsp, 0x28",
    "ret",
    ".seh_endproc",
    result = sym unwind_result_original,
    constructor = sym unwind_constructor_original
);
#[cfg(test)]
unsafe extern "system-unwind" {
    fn naval_unwind_call_caller(result: usize, combat: usize) -> usize;
    fn naval_unwind_call_site();
    fn naval_unwind_constructor(pointer: usize) -> usize;
}

core::arch::global_asm!(
    ".text",
    ".globl naval_ctor_relocation_target",
    ".seh_proc naval_ctor_relocation_target",
    "naval_ctor_relocation_target:",
    "mov [rsp + 8], rbx",
    ".seh_endprologue",
    ".rept 11",
    "nop",
    ".endr",
    "mov rax, rcx",
    "ret",
    ".seh_endproc"
);
#[cfg(test)]
unsafe extern "system-unwind" {
    fn naval_ctor_relocation_target(pointer: usize) -> usize;
}
#[test]
fn constructor_relocation_and_sentinel_restore_exactly() {
    let _family = collector_hooks::hook::FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _observation = Observation::new();
    let site = naval_ctor_relocation_target as *const () as usize;
    let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    setup(1, 0);
    ENABLED.store(false, Ordering::SeqCst);
    let mut hook =
        unsafe { Hook::new(site, constructor_entry as *const () as usize, sentinel) }.unwrap();
    ORIGINAL[2].store(hook.trampoline(), Ordering::Release);
    assert_eq!(unsafe { naval_ctor_relocation_target(0x1234) }, 0x1234);
    assert!(hook.set_enabled(true).unwrap());
    let calls = CALLS[2].load(Ordering::Relaxed);
    assert_eq!(unsafe { naval_ctor_relocation_target(0x5678) }, 0x5678);
    assert_eq!(CALLS[2].load(Ordering::Relaxed), calls + 1);
    assert_eq!(active(), 0);
    assert!(hook.set_enabled(false).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
}

#[test]
#[ignore = "requires BATTLEPLAN_NAVAL_UNWIND_PROBE pointing to the synthetic C++ probe DLL"]
fn native_foreign_unwind_probe() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::FreeLibrary,
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
    };
    let _observation = Observation::new();
    let path = std::env::var_os("BATTLEPLAN_NAVAL_UNWIND_PROBE").or_else(|| std::env::var_os("HOI4_NAVAL_UNWIND_PROBE")).expect("probe DLL path");
    let path: Vec<_> = path.encode_wide().chain(Some(0)).collect();
    let module = unsafe { LoadLibraryW(path.as_ptr()) };
    assert!(!module.is_null());
    let throw_constructor =
        unsafe { GetProcAddress(module, c"ThrowConstructor".as_ptr().cast()) }.unwrap();
    let throw_result = unsafe { GetProcAddress(module, c"ThrowResult".as_ptr().cast()) }.unwrap();
    type Constructor = unsafe extern "system-unwind" fn(usize) -> usize;
    type ResultFn = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let catch_constructor: unsafe extern "system" fn(Constructor, usize) -> u32 = unsafe {
        std::mem::transmute(GetProcAddress(module, c"CatchConstructor".as_ptr().cast()).unwrap())
    };
    let catch_result: unsafe extern "system" fn(ResultFn, usize, usize) -> u32 = unsafe {
        std::mem::transmute(GetProcAddress(module, c"CatchResult".as_ptr().cast()).unwrap())
    };
    for (site, original) in ORIGINAL.iter().enumerate() {
        setup(1, 0);
        let before = UNWINDS.load(Ordering::Relaxed);
        let target = if site == 2 {
            naval_unwind_constructor as *const () as usize
        } else {
            naval_unwind_call_site as *const () as usize
        };
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(target as *const u8, 16) }
            .try_into()
            .unwrap();
        let mut hook = if site == 2 {
            PROBE_ORIGINAL.store(throw_constructor as *const () as usize, Ordering::Relaxed);
            let hook =
                unsafe { Hook::new(target, constructor_entry as *const () as usize, sentinel) }
                    .unwrap();
            original.store(hook.trampoline(), Ordering::Release);
            hook
        } else {
            PROBE_ORIGINAL.store(throw_result as *const () as usize, Ordering::Relaxed);
            original.store(
                unwind_result_original as *const () as usize,
                Ordering::Release,
            );
            let callback: ResultFn = if site == 0 { fill } else { merge };
            unsafe {
                Hook::new_call_span(
                    target,
                    callback as *const () as usize,
                    sentinel,
                    if site == 0 { 16 } else { 13 },
                )
            }
            .unwrap()
        };
        assert!(hook.set_enabled(true).unwrap());
        let caught = if site == 2 {
            unsafe { catch_constructor(naval_unwind_constructor, 0x1234) }
        } else {
            unsafe { catch_result(naval_unwind_call_caller, 0x1234, 0x5678) }
        };
        assert_eq!(caught, 0x4e41564c);
        assert_eq!(UNWINDS.load(Ordering::Relaxed), before + 1);
        assert_eq!(active(), 0);
        assert!(store().try_lock().is_ok());
        assert_eq!(
            drain(&mut vec![Entry::default(); 1]).unwrap_err(),
            "naval result sources native writer unwound"
        );
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(target as *const u8, 16) },
            sentinel
        );
    }
    unsafe {
        FreeLibrary(module);
    }
}
#[test]
fn constructor_shim_return_pc_and_callback_unwind_metadata() {
    let _observation = Observation::new();
    let base = naval_constructor_test_return as *const () as usize - 0xc0dc19;
    setup(1, base);
    assert_eq!(unsafe { naval_constructor_test_caller(0x1234) }, 0x1234);
    assert!(LOG.is_empty());
    assert!(folded().births.is_empty());
    assert_eq!(active(), 0);
    use windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry;
    for function in [
        constructor as *const () as usize,
        result_call as *const () as usize,
        naval_constructor_test_caller as *const () as usize,
    ] {
        let mut image_base = 0;
        assert!(
            !unsafe {
                RtlLookupFunctionEntry(function as u64, &mut image_base, std::ptr::null_mut())
            }
            .is_null()
        );
    }
}

#[cfg(all(test, feature = "tap-foreign-probe"))]
mod foreign_unwind;
