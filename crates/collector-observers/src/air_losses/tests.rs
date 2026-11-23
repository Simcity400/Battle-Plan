use super::*;
use engine_air_history::{cloop_history, loop_history_record_fields, loop_history_window};
use engine_native::{
    cair_wing, cair_wing_pool, ccountry, cgame_state, cstrategic_air, cstrategic_region,
    csub_unit_definition,
};
use std::sync::Mutex;
#[cfg(feature = "tap-foreign-probe")]
mod foreign_unwind;
static SERIAL: Mutex<()> = Mutex::new(());
thread_local! {
    static OWNER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(super) fn owns_observation() -> bool {
    OWNER.get()
}
struct ObservationOwner {
    _serial: std::sync::MutexGuard<'static, ()>,
}
impl ObservationOwner {
    fn new() -> Self {
        let serial = SERIAL.lock().unwrap();
        OWNER.set(true);
        Self { _serial: serial }
    }
}
impl Drop for ObservationOwner {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::SeqCst);
        OWNER.set(false);
    }
}

static FORWARDED: Mutex<[usize; 7]> = Mutex::new([0; 7]);
type Writer = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize) -> usize;
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn forwarding(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
) -> usize {
    *FORWARDED.lock().unwrap() = [a, b, c, d, e, f, g];
    assert_eq!(active(), 1);
    0xfeed_0123_abcd_4567
}
#[test]
fn assembly_forwards_seven_slots_and_native_return() {
    let _owner = ObservationOwner::new();
    LEDGER.reset();
    ENABLED.store(false, Ordering::SeqCst);
    ORIGINAL.store(forwarding as *const () as usize, Ordering::Release);
    let entry: Writer = unsafe { std::mem::transmute(air_loss_entry as *const ()) };
    let family = crate::hook_timing::Family::AirLosses;
    let timed = crate::hook_timing::counts(family);
    assert_eq!(
        unsafe { entry(11, 22, 33, 44, 55, 66, 77) },
        0xfeed_0123_abcd_4567
    );
    let after = crate::hook_timing::counts(family);
    assert!(after.calls > timed.calls && after.returns > timed.returns);
    assert_eq!(*FORWARDED.lock().unwrap(), [11, 22, 33, 44, 55, 66, 77]);
    assert_eq!(active(), 0);
}
static PASSED: Mutex<[usize; 7]> = Mutex::new([0; 7]);
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn passed_through(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
) -> usize {
    *PASSED.lock().unwrap() = [a, b, c, d, e, f, g];
    assert_eq!(active(), 0);
    0x0bad_cafe_f00d_1234
}
#[test]
fn the_dispatch_passes_the_writers_no_write_calls_straight_through() {
    let _owner = ObservationOwner::new();
    LEDGER.reset();
    ENABLED.store(false, Ordering::SeqCst);
    let dispatch: Writer = unsafe { std::mem::transmute(air_loss_dispatch as *const ()) };
    let family = crate::hook_timing::Family::AirLosses;
    ORIGINAL.store(passed_through as *const () as usize, Ordering::Release);
    for arguments in [
        [11, 22, 33, 0, 0, 66, 77],
        [11, 22, 33, 1, 55, 66, 3],
        [11, 22, 33, 0x101, 55, 66, 3],
        [11, 22, 33, 1, 55, 66, 0x1_0000_0003],
    ] {
        let fast = FAST_ZERO_WRITES.load(Ordering::Relaxed);
        let timed = crate::hook_timing::counts(family);
        let calls = LEDGER.calls.load(Ordering::Relaxed);
        let [a, b, c, d, e, f, g] = arguments;
        assert_eq!(
            unsafe { dispatch(a, b, c, d, e, f, g) },
            0x0bad_cafe_f00d_1234
        );
        assert_eq!(*PASSED.lock().unwrap(), arguments);
        assert_eq!(
            FAST_ZERO_WRITES.load(Ordering::Relaxed),
            fast + 1,
            "{arguments:?}"
        );
        let after = crate::hook_timing::counts(family);
        assert_eq!((after.calls, after.returns), (timed.calls, timed.returns));
        assert_eq!(LEDGER.calls.load(Ordering::Relaxed), calls);
    }
    ORIGINAL.store(forwarding as *const () as usize, Ordering::Release);
    for arguments in [[11, 22, 33, 0, 55, 66, 3], [11, 22, 33, 1, 55, 66, 2]] {
        let fast = FAST_ZERO_WRITES.load(Ordering::Relaxed);
        let timed = crate::hook_timing::counts(family);
        let [a, b, c, d, e, f, g] = arguments;
        assert_eq!(
            unsafe { dispatch(a, b, c, d, e, f, g) },
            0xfeed_0123_abcd_4567
        );
        assert_eq!(*FORWARDED.lock().unwrap(), arguments);
        assert_eq!(FAST_ZERO_WRITES.load(Ordering::Relaxed), fast);
        let after = crate::hook_timing::counts(family);
        assert!(after.calls > timed.calls && after.returns > timed.returns);
    }
    assert_eq!(active(), 0);
}
core::arch::global_asm!(
    ".text", ".globl air_loss_tags_test", ".seh_proc air_loss_tags_test",
    "air_loss_tags_test:", "push rsi", ".seh_pushreg rsi", "push rdi", ".seh_pushreg rdi",
    "sub rsp, 0x48", ".seh_stackalloc 0x48", ".seh_endprologue",
    "mov rsi, rcx", "mov rdi, rdx", "xor r8d, r8d", "xor r9d, r9d",
    "mov qword ptr [rsp + 0x20], 0", "mov qword ptr [rsp + 0x28], 0",
    "mov qword ptr [rsp + 0x30], 0", "call {entry}",
    "add rsp, 0x48", "pop rdi", "pop rsi", "ret", ".seh_endproc",
    entry = sym air_loss_entry,
);
unsafe extern "system" {
    fn air_loss_tags_test(own: usize, credited: usize) -> usize;
}
#[test]
fn assembly_copies_both_nonvolatile_tag_pointers_before_rust() {
    let _owner = ObservationOwner::new();
    LEDGER.reset();
    ENABLED.store(false, Ordering::SeqCst);
    ORIGINAL.store(forwarding as *const () as usize, Ordering::Release);
    unsafe {
        air_loss_tags_test(0x1111, 0x2222);
    }
    assert_eq!(
        TAPPED_TAGS
            .each_ref()
            .map(|tag| tag.load(Ordering::Relaxed)),
        [0x1111, 0x2222]
    );
}
#[test]
fn compact_delta_checks_clamps_double_writes_and_other_selected_changes() {
    let change = native::Change {
        country: 4999,
        region: 701,
        enemy: 0,
        cause: 1,
        fields: [100000; 2],
        daily_slot: 29,
    };
    assert_eq!(validate_delta(&change, 30, 1), Ok(100000));
    assert!(
        validate_delta(
            &native::Change {
                fields: [7, 100000],
                ..change
            },
            30,
            1
        )
        .is_err()
    );
    assert_eq!(
        validate_delta(
            &native::Change {
                cause: 0,
                fields: [200000; 2],
                ..change
            },
            30,
            1
        ),
        Ok(100000)
    );
    assert_eq!(
        validate_delta(
            &native::Change {
                fields: [-100000; 2],
                ..change
            },
            30,
            -1
        ),
        Ok(-100000)
    );
    assert!(
        validate_delta(
            &native::Change {
                cause: i32::MAX,
                ..change
            },
            30,
            1
        )
        .is_err()
    );
}
struct World {
    storage: Vec<u64>,
    countries: usize,
}
impl World {
    const GAME: usize = 0x1000;
    const AIR: usize = 0x3000;
    const HISTORY: usize = 0x4000;
    const WINDOW: usize = 0x5000;
    const RECORD: usize = 0x6000;
    const VALUES: usize = 0x7000;
    const REGION: usize = 0x8000;
    const WING: usize = 0x9000;
    const POOL: usize = 0xa000;
    const DEFINITION: usize = 0xb000;
    const LINKS: usize = 0xc000;
    fn pointer(&self, offset: usize) -> usize {
        self.storage.as_ptr() as usize + offset
    }
    fn base(&self) -> usize {
        self.pointer(0) - cgame_state::SINGLETON_RVA.0
    }
    fn u32(&mut self, offset: usize, value: u32) {
        let word = &mut self.storage[offset / 8];
        let shift = (offset % 8) * 8;
        *word = (*word & !(0xffff_ffff << shift)) | ((value as u64) << shift);
    }
    fn word(&mut self, offset: usize, value: u64) {
        self.storage[offset / 8] = value;
    }
    fn array(&mut self, offset: usize, data: usize, count: usize) {
        self.word(offset, self.pointer(data) as u64);
        self.u32(offset + 8, count as u32);
        self.u32(offset + 12, count as u32);
    }
    fn owner(&mut self, country: u32) {
        self.u32(Self::AIR + cstrategic_air::COUNTRY, country);
    }
    fn country_table(&self) -> usize {
        (Self::LINKS + (self.countries + 1) * 4).div_ceil(8) * 8
    }
    fn country_object(&self, country: usize) -> usize {
        self.country_table() + (self.countries + 1) * 8 + country * 16
    }
    fn new(countries: usize) -> Self {
        let mut world = Self {
            storage: vec![0; (Self::LINKS + (countries + 1) * 32).div_ceil(8)],
            countries,
        };
        world.word(0, world.pointer(Self::GAME) as u64);
        world.u32(Self::GAME + cgame_state::CURRENT_HOUR, 48);
        world.array(
            Self::GAME + cgame_state::COUNTRIES,
            world.country_table(),
            countries + 1,
        );
        world.array(
            Self::GAME + cgame_state::COUNTRY_LINKS,
            Self::LINKS,
            countries + 1,
        );
        for country in 1..=countries {
            world.u32(Self::LINKS + country * 4, country as u32);
            let object = world.country_object(country);
            world.word(
                world.country_table() + country * 8,
                world.pointer(object) as u64,
            );
            world.word(object, (world.base() + ccountry::VTABLE_RVA.0) as u64);
            world.u32(object + ccountry::ID, country as u32);
        }
        world.word(
            Self::AIR,
            (world.base() + cstrategic_air::VTABLE_RVA.0) as u64,
        );
        world.array(Self::AIR + cstrategic_air::HISTORY_TABLE, 0x3800, 24);
        world.word(0x3800 + 23 * 8, world.pointer(Self::HISTORY) as u64);
        world.word(
            Self::HISTORY,
            (world.base() + cloop_history::VTABLE_RVA.0) as u64,
        );
        world.word(
            Self::HISTORY + cloop_history::SELF_REFERENCE,
            world.pointer(Self::HISTORY) as u64,
        );
        world.u32(
            Self::HISTORY + cloop_history::FIELD_COUNT,
            native::FIELD_COUNT as u32,
        );
        world.word(
            Self::HISTORY + cloop_history::DAILY_WINDOW,
            world.pointer(Self::WINDOW) as u64,
        );
        world.word(
            Self::WINDOW,
            (world.base() + loop_history_window::VTABLE_RVA.0) as u64,
        );
        world.array(Self::WINDOW + loop_history_window::RECORDS, 0x5800, 30);
        world.u32(Self::WINDOW + loop_history_window::CAPACITY, 30);
        world.u32(Self::WINDOW + loop_history_window::CURRENT_SLOT, 29);
        world.u32(Self::WINDOW + loop_history_window::WRAPPED, 1);
        world.word(
            Self::WINDOW + loop_history_window::RECORD_FIELDS,
            world.pointer(Self::RECORD) as u64,
        );
        world.word(
            Self::WINDOW + loop_history_window::HISTORY,
            world.pointer(Self::HISTORY) as u64,
        );
        world.word(0x5800 + 29 * 8, world.pointer(Self::RECORD) as u64);
        world.array(Self::RECORD, Self::VALUES, native::FIELD_COUNT);
        world.word(
            Self::RECORD + loop_history_record_fields::HISTORY,
            world.pointer(Self::HISTORY) as u64,
        );
        world.u32(Self::REGION + 0x58, 23);
        world.word(
            Self::REGION,
            (world.base() + cstrategic_region::VTABLE_RVA.0) as u64,
        );
        world.word(Self::WING, (world.base() + cair_wing::VTABLE_RVA.0) as u64);
        world.word(
            Self::WING + cair_wing::POOL,
            world.pointer(Self::POOL) as u64,
        );
        world.word(Self::WING + 0x100, world.pointer(Self::VALUES) as u64);
        world.word(
            Self::POOL,
            (world.base() + cair_wing_pool::VTABLE_RVA.0) as u64,
        );
        world.word(
            Self::POOL + cair_wing_pool::DEFINITION,
            world.pointer(Self::DEFINITION) as u64,
        );
        world.word(
            Self::DEFINITION,
            (world.base() + csub_unit_definition::VTABLE_RVA.0) as u64,
        );
        world.word(Self::DEFINITION + csub_unit_definition::AIR_FLAGS, 1 << 18);
        world.owner(1);
        world
    }
    fn invoke(&self, enemy: usize, cause: usize) {
        let tags = [
            self.pointer(Self::AIR + cstrategic_air::COUNTRY),
            self.pointer(Self::GAME + 0x800),
        ];
        let mut context = CallContext {
            tags,
            observation: None,
        };
        unsafe {
            pre(
                &mut context,
                self.pointer(Self::AIR),
                self.pointer(Self::WING),
                1,
                enemy,
                self.pointer(Self::REGION),
                0,
                cause,
            );
            native_write(
                self.pointer(Self::AIR),
                self.pointer(Self::WING),
                1,
                enemy,
                self.pointer(Self::REGION),
                0,
                cause,
            );
            post(&mut context);
        }
    }
}
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn native_write(
    _receiver: usize,
    wing: usize,
    count: usize,
    enemy: usize,
    _region: usize,
    _selection: usize,
    cause: usize,
) -> usize {
    if enemy == 1 && cause == 3 {
        return 0;
    }
    let values = unsafe { *((wing + 0x100) as *const usize) };
    let field = if enemy == 0 { 30 } else { 71 };
    let delta = count as i64 * 100000;
    for index in [field, field + cause] {
        let pointer = (values + index * 8) as *mut i64;
        unsafe {
            *pointer = (*pointer + delta).min(0x53e2_d620_fb40);
        }
    }
    0
}
fn clock(hour: i32) -> Option<Clock> {
    Some(Clock {
        hour,
        play_session_started_at: 0,
        game_state: 0,
        play_time: 0,
        seed: 0,
        campaign_id: Default::default(),
        speed: 0,
        tick_busy: 0,
    })
}
fn setup(world: &World) {
    LEDGER.reset();
    LEDGER.prepare(world.countries * 2);
    BASE.store(world.base(), Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    ORIGINAL.store(native_write as *const () as usize, Ordering::Release);
    unsafe { boundary(clock(47)) };
    assert!(unsafe { drain(&mut Vec::new()) }.is_err());
    unsafe { boundary(clock(48)) };
}
#[test]
fn production_observers_keep_family_cause_opponent_and_refuse_saturation() {
    let _owner = ObservationOwner::new();
    let mut world = World::new(5000);
    setup(&world);
    world.u32(World::GAME + 0x800, 4999);
    world.owner(5000);
    world.invoke(0, 1);
    world.invoke(1, 6);
    world.invoke(1, 3);
    std::thread::spawn(|| {
        unsafe { boundary(clock(49)) };
        unsafe { lost_drain() };
        unsafe { lost_output(1) };
        assert!(unsafe { drain(&mut Vec::new()) }.is_err());
    })
    .join()
    .unwrap();
    let mut rows = Vec::with_capacity(capacity());
    assert_eq!(unsafe { drain(&mut rows) }.unwrap(), 2);
    assert_eq!(
        (
            rows[0].country,
            rows[0].opponent,
            rows[0].family_base,
            rows[0].amount_raw
        ),
        (5000, Some(4999), 30, 100000)
    );
    assert_eq!(
        (rows[1].opponent, rows[1].family_base, rows[1].native_cause),
        (Some(5000), 71, 6)
    );
    assert!(rows.iter().all(|row| row.daily_slot == 29));
    world.word(World::VALUES + 30 * 8, 0x53e2_d620_fb40);
    world.invoke(0, 1);
    assert!(unsafe { drain(&mut rows) }.unwrap_err().contains("saturated"));
    assert_eq!(LEDGER.refusals.load(Ordering::Relaxed), 1);
    assert_eq!(active(), 0);
}
#[test]
fn gathered_and_serial_operands_publish_identical_rows() {
    let _owner = ObservationOwner::new();
    let mut world = World::new(5000);
    world.u32(World::GAME + 0x800, 4999);
    world.owner(5000);
    let mut runs = Vec::new();
    for serial in [false, true] {
        for field in [30, 31, 71, 77] {
            world.word(World::VALUES + field * 8, 0);
        }
        FORCE_SERIAL.store(serial, Ordering::SeqCst);
        setup(&world);
        let fallbacks = SERIAL_FALLBACKS.load(Ordering::Relaxed);
        world.invoke(0, 1);
        world.invoke(1, 6);
        unsafe { boundary(clock(49)) };
        FORCE_SERIAL.store(false, Ordering::SeqCst);
        assert_eq!(
            SERIAL_FALLBACKS.load(Ordering::Relaxed) - fallbacks,
            if serial { 2 } else { 0 }
        );
        let mut rows = Vec::with_capacity(capacity());
        assert_eq!(unsafe { drain(&mut rows) }.unwrap(), 2);
        assert_eq!(
            (
                rows[0].country,
                rows[0].opponent,
                rows[0].region,
                rows[0].game_hour
            ),
            (5000, Some(4999), 23, 48)
        );
        runs.push(rows);
    }
    assert_eq!(runs[0], runs[1]);
    setup(&world);
    let fallbacks = SERIAL_FALLBACKS.load(Ordering::Relaxed);
    world.owner(5001);
    world.invoke(0, 1);
    assert_eq!(SERIAL_FALLBACKS.load(Ordering::Relaxed) - fallbacks, 1);
    assert_eq!(LEDGER.refusals.load(Ordering::Relaxed), 1);
    assert_eq!(active(), 0);
}
#[test]
#[ignore = "release per-call detour cost; run with --release --ignored --nocapture --test-threads=1"]
fn hook_cost_air_losses() {
    const COUNTRIES: usize = 1500;
    let _owner = ObservationOwner::new();
    let mut world = World::new(COUNTRIES);
    setup(&world);
    LEDGER.prepare(8 * COUNTRIES);
    unsafe { boundary(clock(49)) };
    assert_eq!(unsafe { drain(&mut Vec::new()) }, Ok(0));
    let hooked = crate::test_support::per_call_ns(COUNTRIES, || {
        for country in 1..=COUNTRIES {
            world.owner(country as u32);
            world.invoke(0, 1);
        }
    });
    unsafe { boundary(clock(50)) };
    let mut rows = Vec::with_capacity(capacity());
    assert_eq!(unsafe { drain(&mut rows) }, Ok(8 * COUNTRIES));
    assert_eq!(LEDGER.refusals.load(Ordering::Relaxed), 0);
    let native = crate::test_support::per_call_ns(COUNTRIES, || {
        for _ in 1..=COUNTRIES {
            std::hint::black_box(unsafe {
                native_write(
                    world.pointer(World::AIR),
                    world.pointer(World::WING),
                    1,
                    0,
                    world.pointer(World::REGION),
                    0,
                    1,
                )
            });
        }
    });
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let (base, game) = (world.base(), world.pointer(World::GAME));
    let reads = crate::test_support::per_call_ns(COUNTRIES, || {
        for country in 1..=COUNTRIES {
            world.owner(country as u32);
            let family =
                native::selected_family(&memory, base, world.pointer(World::WING), 0).unwrap();
            let snapshot = native::before(
                &memory,
                base,
                game,
                world.pointer(World::AIR),
                23,
                0,
                1,
                family,
            )
            .unwrap();
            std::hint::black_box(native::after(&memory, base, game, &snapshot).unwrap());
        }
    });
    println!(
        "hook_cost air_losses own loss: detour {:.1} ns/call (hooked {hooked:.1}, native {native:.1}; native reads {reads:.1})",
        hooked - native
    );
}
core::arch::global_asm!(
    ".text", ".globl air_loss_unwind_caller", ".seh_proc air_loss_unwind_caller",
    "air_loss_unwind_caller:", "sub rsp, 0x48", ".seh_stackalloc 0x48", ".seh_endprologue",
    ".globl air_loss_unwind_site", "air_loss_unwind_site:", "call {target}",
    ".globl air_loss_unwind_return", "air_loss_unwind_return:",
    ".rept 11", "nop", ".endr", "add rsp, 0x48", "ret", ".seh_endproc",
    target = sym forwarding,
);
unsafe extern "system" {
    fn air_loss_unwind_site();
    fn air_loss_unwind_return();
}
#[repr(C)]
struct RuntimeFunction {
    begin: u32,
    end: u32,
    unwind: u32,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlLookupFunctionEntry(
        pc: u64,
        image: *mut u64,
        history: *mut std::ffi::c_void,
    ) -> *mut RuntimeFunction;
    fn RtlVirtualUnwind(
        kind: u32,
        image: u64,
        pc: u64,
        entry: *const RuntimeFunction,
        context: *mut windows_sys::Win32::System::Diagnostics::Debug::CONTEXT,
        handler: *mut *mut std::ffi::c_void,
        frame: *mut u64,
        pointers: *mut std::ffi::c_void,
    ) -> usize;
}
#[repr(C, align(16))]
struct Context(windows_sys::Win32::System::Diagnostics::Debug::CONTEXT);

#[test]
fn system_unwind_crosses_assembly_and_patched_native_caller_frames() {
    let _family = crate::hook::FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _owner = ObservationOwner::new();
    let site = air_loss_unwind_site as *const () as usize;
    let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let mut hook =
        unsafe { Hook::new_call(site, air_loss_entry as *const () as usize, sentinel) }.unwrap();
    assert!(hook.set_enabled(true).unwrap());
    let mut stack = vec![0u64; (FRAME_BYTES + 0x48 + 64).div_ceil(8)];
    let shim_rsp = stack.as_mut_ptr() as usize;
    let caller_rsp = shim_rsp + FRAME_BYTES + 8;
    let caller_pc = air_loss_unwind_return as *const () as usize;
    stack[FRAME_BYTES / 8] = caller_pc as u64;
    stack[(FRAME_BYTES + 8 + 0x48) / 8] = 0x1234_5678;
    let mut context = Context(unsafe { std::mem::zeroed() });
    context.0.Rip = air_loss_original_return as *const () as u64;
    context.0.Rsp = shim_rsp as u64;
    for (expected_pc, expected_rsp) in [
        (caller_pc as u64, caller_rsp as u64),
        (0x1234_5678, (caller_rsp + 0x48 + 8) as u64),
    ] {
        let mut image = 0u64;
        let mut handler = std::ptr::null_mut();
        let mut frame = 0u64;
        let entry =
            unsafe { RtlLookupFunctionEntry(context.0.Rip, &mut image, std::ptr::null_mut()) };
        assert!(
            !entry.is_null(),
            "both frames need registered Windows unwind metadata"
        );
        unsafe {
            RtlVirtualUnwind(
                0,
                image,
                context.0.Rip,
                entry,
                &mut context.0,
                &mut handler,
                &mut frame,
                std::ptr::null_mut(),
            );
        }
        assert_eq!((context.0.Rip, context.0.Rsp), (expected_pc, expected_rsp));
    }
    assert!(hook.set_enabled(false).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
}
