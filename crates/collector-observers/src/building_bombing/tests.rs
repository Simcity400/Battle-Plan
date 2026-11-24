use super::*;
use engine_native_air::{cair_wing, cstrategic_air};
use engine_native_parts::{cair_wing_pool, cbuilding, cbuilding_template};
use engine_native::{
    cprovince::{buildings as cbuilding_status, self},
    cgame_state,
    cstate,
};
use std::{cell::UnsafeCell, hint::black_box, sync::Mutex, time::Instant};
use windows_sys::Win32::System::Diagnostics::Debug::{
    CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
};
#[cfg(feature = "tap-foreign-probe")]
mod foreign_unwind;

static SERIAL: Mutex<()> = Mutex::new(());
thread_local! {
    static OBSERVATION_OWNER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(super) fn owns_observation() -> bool {
    OBSERVATION_OWNER.get()
}
struct ObservationOwner {
    _serial: std::sync::MutexGuard<'static, ()>,
}
impl ObservationOwner {
    fn new() -> Self {
        let serial = SERIAL.lock().unwrap();
        OBSERVATION_OWNER.set(true);
        Self { _serial: serial }
    }
}
impl Drop for ObservationOwner {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::SeqCst);
        OBSERVATION_OWNER.set(false);
    }
}

static FORWARDED: Mutex<[usize; 6]> = Mutex::new([0; 6]);
const GAME: usize = 0x100;
const STATE: usize = 0x1000;
const PROVINCE: usize = 0x2000;
const BUILDING: usize = 0x3000;
const TEMPLATE: usize = 0x3800;
const STATES: usize = 0x4000;
const PROVINCES: usize = 0x4100;
const INDICES: usize = 0x4200;
const BUILDINGS: usize = 0x4300;
const SOURCE: usize = 0x5000;
const WING: usize = 0x6000;
const POOL: usize = 0x7000;
const OWNER: usize = 0x8000;
const OUTPUT: usize = 0x8500;
const TYPE: u32 = 0xfa001;

struct World {
    storage: Vec<u64>,
    root: Box<u64>,
    base: usize,
    observation: Box<UnsafeCell<Observation>>,
}
impl World {
    fn new(province_scope: bool) -> Self {
        let mut world = Self {
            storage: vec![0; 0x9000 / 8],
            root: Box::new(0),
            base: 0,
            observation: Box::new(UnsafeCell::new(Observation::default())),
        };
        world.base = (&raw const *world.root as usize) - cgame_state::SINGLETON_RVA.0;
        *world.root = world.at(GAME) as u64;
        let (object, offset, id, scope) = if province_scope {
            (
                PROVINCE,
                cprovince::BUILDING_STATUS,
                4,
                cbuilding_status::PROVINCE_FLAG,
            )
        } else {
            (
                STATE,
                cstate::BUILDING_STATUS,
                3,
                cbuilding_status::STATE_FLAG,
            )
        };
        let status = object + offset;
        for (at, value) in [
            (STATE, world.base + cstate::VTABLE_RVA.0),
            (PROVINCE, world.base + cprovince::VTABLE_RVA.0),
            (PROVINCE + cprovince::STATE, world.at(STATE)),
            (BUILDING, world.base + cbuilding::VTABLE_RVA.0),
            (BUILDING + cbuilding::BUILDING_STATUS, world.at(status)),
            (BUILDING + cbuilding::TEMPLATE, world.at(TEMPLATE)),
            (status, world.base + cbuilding_status::VTABLE_RVA.0),
            (STATES + 3 * 8, world.at(STATE)),
            (PROVINCES + 4 * 8, world.at(PROVINCE)),
            (BUILDINGS, world.at(BUILDING)),
            (SOURCE + native::SOURCE_WING, world.at(WING)),
            (WING, world.base + engine_native_parts::cair_wing_layout::VTABLE_RVA.0),
            (WING + engine_native_parts::cair_wing_layout::POOL, world.at(POOL)),
            (POOL, world.base + cair_wing_pool::VTABLE_RVA.0),
            (POOL + cair_wing_pool::OWNER, world.at(OWNER)),
            (OWNER, world.base + cstrategic_air::VTABLE_RVA.0),
        ] {
            world.word(at, value as u64);
        }
        for (at, value) in [
            (GAME + cgame_state::CURRENT_HOUR, 12345),
            (STATE + cstate::ID, 3),
            (STATE + cstate::CONTROLLER, 5001),
            (PROVINCE + cprovince::ID, 4),
            (PROVINCE + cprovince::CONTROLLER, 5002),
            (BUILDING + cbuilding::TYPE_ID, TYPE),
            (TEMPLATE + cbuilding_template::TYPE_ID, TYPE),
            (TEMPLATE + cbuilding_template::DATABASE_ORDINAL, 5),
            (status + cbuilding_status::FLAGS, scope),
            (status + cbuilding_status::ID, id),
            (status + cbuilding_status::COUNTRY, 9999),
            (SOURCE, native::AIR_SOURCE_KIND),
            (WING + cair_wing::ID, 71),
            (OWNER + cstrategic_air::COUNTRY, 6001),
        ] {
            world.dword(at, value);
        }
        world.array(GAME + cgame_state::STATES, STATES, 8);
        world.array(GAME + cgame_state::PROVINCES, PROVINCES, 8);
        world.array(status + cbuilding_status::BUILDING_INDICES, INDICES, 9);
        world.array(status + cbuilding_status::BUILDINGS, BUILDINGS, 1);
        world.health(2, 4_000_000);
        world
    }
    fn at(&self, offset: usize) -> usize {
        self.storage.as_ptr() as usize + offset
    }
    fn word(&mut self, at: usize, value: u64) {
        self.storage[at / 8] = value;
    }
    fn dword(&mut self, at: usize, value: u32) {
        let pointer = self.storage.as_mut_ptr().cast::<u8>();
        unsafe {
            std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), pointer.add(at), 4);
        }
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, self.at(data) as u64);
        self.dword(at + 8, count);
        self.dword(at + 12, count);
    }
    fn health(&mut self, healthy: i16, partial: i64) {
        self.word(
            BUILDING + cbuilding::LEVEL,
            5 | ((healthy as u16 as u64) << 16),
        );
        self.word(BUILDING + cbuilding::PARTIAL_HEALTH, partial as u64);
    }
    fn pre(&self, source: usize, request: i64, caller: native::Cause) -> usize {
        let return_pc = native::CALLERS
            .iter()
            .find(|(_, cause)| *cause == caller)
            .map_or(self.base + 0x12345, |(rva, _)| self.base + rva.0);
        unsafe {
            before(
                self.at(BUILDING),
                self.at(OUTPUT),
                request as usize,
                1,
                source,
                1,
                return_pc,
                self.observation.get(),
            )
        }
    }
    fn post(&self, token: usize) {
        unsafe {
            after(token, self.observation.get(), self.at(OUTPUT));
        }
    }
}
fn setup(capacity: usize, base: usize) {
    LEDGER.reset();
    LEDGER.prepare(capacity);
    BASE.store(base, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    UNATTRIBUTED.store(0, Ordering::Relaxed);
    ATTRIBUTION_FAILURES.store(0, Ordering::Relaxed);
    UNKNOWN_CALLERS.store(0, Ordering::Relaxed);
    for count in &CALLER_CALLS {
        count.store(0, Ordering::Relaxed);
    }
    for ticks in [
        &WRITER_TICKS,
        &INTERVAL_TICKS,
        &LAST_HOUR_TICKS,
        &MAX_HOUR_TICKS,
    ] {
        ticks.store(0, Ordering::Relaxed);
    }
    let mut frequency = 0;
    unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&raw mut frequency);
    }
    FREQUENCY.store(frequency as u64, Ordering::Relaxed);
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
fn ready() {
    unsafe { boundary(clock(1, 4)) };
    assert!(unsafe { drain(&mut Vec::new()) }.is_err());
    unsafe { boundary(clock(2, 4)) };
}
#[test]
fn production_callbacks_defer_attribution_and_preserve_unattributable_damage() {
    let _owner = ObservationOwner::new();
    let mut world = World::new(false);
    setup(8, world.base);
    ready();
    let token = world.pre(world.at(SOURCE), 1, native::Cause::AirMission);
    std::thread::spawn(|| {
        unsafe { boundary(clock(3, 4)) };
        unsafe { lost_drain() };
        unsafe { lost_output(1) };
        assert!(unsafe { drain(&mut Vec::new()) }.is_err());
    })
    .join()
    .unwrap();
    world.dword(WING + cair_wing::ID, 0);
    world.health(1, 0);
    world.word(OUTPUT, 1);
    world.post(token);
    let mut output = Vec::with_capacity(capacity());
    assert_eq!(unsafe { drain(&mut output) }.unwrap(), 1);
    assert_eq!(output[0].attacker, None);
    assert_eq!(output[0].amount_raw, 14_000_000);
    assert_eq!(output[0].credited_raw, 1);
    assert_eq!(UNATTRIBUTED.load(Ordering::Relaxed), 1);
    assert_eq!(ATTRIBUTION_FAILURES.load(Ordering::Relaxed), 1);
    assert_eq!(LEDGER.refusals.load(Ordering::Relaxed), 0);
    assert_eq!(CALLER_CALLS[0].load(Ordering::Relaxed), 1);
    world.word(SOURCE + native::SOURCE_WING, 0);
    world.health(2, 4_000_000);
    let token = world.pre(world.at(SOURCE), 1, native::Cause::AirMission);
    world.health(1, 0);
    world.post(token);
    assert_eq!(unsafe { drain(&mut output) }.unwrap(), 1);
    assert_eq!(output[0].attacker, None);
    assert_eq!(UNATTRIBUTED.load(Ordering::Relaxed), 2);
    assert_eq!(ATTRIBUTION_FAILURES.load(Ordering::Relaxed), 1);
    world.health(2, 4_000_000);
    let token = world.pre(world.at(SOURCE), 1, native::Cause::AirMission);
    world.word(STATE, 0);
    world.post(token);
    assert_eq!(unsafe { drain(&mut output) }.unwrap(), 0);
}
#[test]
fn negative_modified_damage_is_a_signed_event_even_above_total_health() {
    let _owner = ObservationOwner::new();
    let mut world = World::new(false);
    setup(8, world.base);
    ready();
    world.health(4, 10_000_000);
    let token = world.pre(world.at(SOURCE), 1, native::Cause::AirMission);
    world.health(4, 11_000_000);
    world.word(OUTPUT, 1);
    world.post(token);
    let mut output = Vec::with_capacity(capacity());
    assert_eq!(unsafe { drain(&mut output) }.unwrap(), 1);
    assert_eq!(output[0].amount_raw, -1_000_000);
    assert_eq!(output[0].credited_raw, 1);
    assert_eq!(output[0].attacker, Some(6001));
    assert_eq!(LEDGER.refusals.load(Ordering::Relaxed), 0);
}

#[test]
fn failed_pre_observation_completes_its_claim_and_refuses_once() {
    let _owner = ObservationOwner::new();
    let mut world = World::new(false);
    setup(8, world.base);
    ready();
    world.word(BUILDING, 0);
    let token = world.pre(world.at(SOURCE), 1, native::Cause::AirMission);
    assert_eq!(token, 0);
    world.post(token);
    assert_eq!(LEDGER.refusals.load(Ordering::Relaxed), 1);
    assert_eq!(LEDGER.lost_events_unknown.load(Ordering::Relaxed), 1);
    assert_eq!(active(), 0);
    assert_eq!(
        unsafe { drain(&mut Vec::new()) }.unwrap_err(),
        "building damage vtable mismatch"
    );
}
unsafe extern "system" fn forwarding(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
) -> usize {
    *FORWARDED.lock().unwrap() = [a, b, c, d, e, f];
    assert_eq!(active(), 1);
    b
}
#[test]
fn assembly_preserves_six_arguments_return_pointer_and_observer_lifetime() {
    let _owner = ObservationOwner::new();
    setup(0, 0);
    ENABLED.store(false, Ordering::SeqCst);
    ORIGINAL.store(forwarding as *const () as usize, Ordering::Release);
    let entry: unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(building_damage_entry as *const ()) };
    let result = unsafe { entry(11, 22, 33, 44, 55, 66) };
    assert_eq!(result, 22);
    assert_eq!(*FORWARDED.lock().unwrap(), [11, 22, 33, 44, 55, 66]);
    assert_eq!(active(), 0);
}

core::arch::global_asm!(
    ".text",
    ".globl building_damage_unwind_test_native",
    ".seh_proc building_damage_unwind_test_native",
    "building_damage_unwind_test_native:",
    "mov [rsp + 0x20], rbx",
    ".seh_savereg rbx, 0x68",
    "push rsi",
    ".seh_pushreg rsi",
    "push rdi",
    ".seh_pushreg rdi",
    "push r15",
    ".seh_pushreg r15",
    "sub rsp, 0x30",
    ".seh_stackalloc 0x30",
    ".byte 0x48, 0x8b, 0xf9",
    ".seh_endprologue",
    ".globl building_damage_unwind_test_body",
    "building_damage_unwind_test_body:",
    "nop",
    "add rsp, 0x30",
    "pop r15",
    "pop rdi",
    "pop rsi",
    "mov rbx, [rsp + 0x20]",
    "ret",
    ".seh_endproc",
);
unsafe extern "system" {
    fn building_damage_unwind_test_native();
    fn building_damage_unwind_test_body();
}
#[test]
fn building_damage_relocation_moves_exactly_the_five_byte_home_space_store() {
    let site = building_damage_unwind_test_native as *const () as usize;
    let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let hook =
        unsafe { Hook::new(site, building_damage_entry as *const () as usize, sentinel) }.unwrap();
    let trampoline = hook.trampoline();
    let bytes = unsafe { std::slice::from_raw_parts(trampoline as *const u8, 10) };
    assert_eq!(&bytes[..5], &sentinel[..5]);
    let target = match bytes[5] {
        0xe9 => {
            let displacement = i32::from_le_bytes(bytes[6..10].try_into().unwrap());
            trampoline
                .checked_add(10)
                .unwrap()
                .checked_add_signed(displacement as isize)
                .unwrap()
        }
        0xff if bytes[6] == 0x25 => {
            let displacement = unsafe { std::ptr::read_unaligned((trampoline + 7) as *const i32) };
            let slot = (trampoline + 11)
                .checked_add_signed(displacement as isize)
                .unwrap();
            unsafe { std::ptr::read_unaligned(slot as *const usize) }
        }
        opcode => panic!("relocation must jump after the five-byte MOV, got {opcode:02x}"),
    };
    assert_eq!(target, site + 5);
}
#[test]
fn assembly_wrapper_has_real_unwind_metadata_for_native_call_stack() {
    let pc = building_damage_native_call as *const () as u64;
    let mut base = 0;
    let entry = unsafe { RtlLookupFunctionEntry(pc, &raw mut base, std::ptr::null_mut()) };
    assert!(!entry.is_null());
    assert!(unsafe { (*entry).BeginAddress as u64 + base } <= pc);
    assert!(pc < unsafe { (*entry).EndAddress as u64 + base });
    let mut stack = [0_u64; 48];
    stack[0x138 / 8] = 0x12345678;
    let frame = stack.as_mut_ptr() as u64;
    let mut context: CONTEXT = unsafe { std::mem::zeroed() };
    context.Rsp = frame;
    context.Rip = pc;
    context.Rbx = 0xf001;
    context.Rsi = 0xf002;
    context.Rdi = 0xf003;
    let mut handler = std::ptr::null_mut();
    let mut establisher = 0;
    unsafe {
        let _ = RtlVirtualUnwind(
            0,
            base,
            pc,
            entry,
            &raw mut context,
            &raw mut handler,
            &raw mut establisher,
            std::ptr::null_mut(),
        );
    }
    assert_eq!(context.Rsp, frame + 0x140);
    assert_eq!(context.Rip, 0x12345678);
    assert_eq!(
        (context.Rbx, context.Rsi, context.Rdi),
        (0xf001, 0xf002, 0xf003)
    );
    let synthetic = unsafe {
        std::slice::from_raw_parts(
            (building_damage_unwind_test_native as *const ()).cast::<u8>(),
            16,
        )
    };
    assert_eq!(&synthetic[..5], &[0x48, 0x89, 0x5c, 0x24, 0x20]);

    let pc = building_damage_unwind_test_body as *const () as u64;
    let entry = unsafe { RtlLookupFunctionEntry(pc, &raw mut base, std::ptr::null_mut()) };
    assert!(!entry.is_null());
    let mut stack = [0_u64; 64];
    stack[0x30 / 8] = 0xf015;
    stack[0x38 / 8] = 0xf003;
    stack[0x40 / 8] = 0xf002;
    stack[0x48 / 8] = building_damage_native_return as *const () as u64;
    stack[0x68 / 8] = 0xf001;
    stack[(0x50 + 0x138) / 8] = 0x12345678;
    let native_frame = stack.as_mut_ptr() as u64;
    let wrapper_frame = native_frame + 0x50;
    context.Rsp = native_frame;
    context.Rip = pc;
    unsafe {
        let _ = RtlVirtualUnwind(
            0,
            base,
            pc,
            entry,
            &raw mut context,
            &raw mut handler,
            &raw mut establisher,
            std::ptr::null_mut(),
        );
    }
    assert_eq!(context.Rsp, wrapper_frame);
    assert_eq!(
        context.Rip,
        building_damage_native_return as *const () as u64
    );
    assert_eq!(
        (context.Rbx, context.Rsi, context.Rdi, context.R15),
        (0xf001, 0xf002, 0xf003, 0xf015)
    );
    let pc = context.Rip;
    let entry = unsafe { RtlLookupFunctionEntry(pc, &raw mut base, std::ptr::null_mut()) };
    assert!(!entry.is_null());
    unsafe {
        let _ = RtlVirtualUnwind(
            0,
            base,
            pc,
            entry,
            &raw mut context,
            &raw mut handler,
            &raw mut establisher,
            std::ptr::null_mut(),
        );
    }
    assert_eq!(
        (context.Rsp, context.Rip),
        (wrapper_frame + 0x140, 0x12345678)
    );
}

#[test]
fn game_root_change_refuses_the_event_and_withholds_the_interval() {
    let _owner = ObservationOwner::new();
    let mut world = World::new(false);
    setup(8, world.base);
    ready();
    let token = world.pre(world.at(SOURCE), 1, native::Cause::AirMission);
    world.health(1, 0);
    world.word(OUTPUT, 1);
    *world.root = 0;
    world.post(token);
    assert_eq!(active(), 0);
    assert_eq!(LEDGER.lost_events_unknown.load(Ordering::Relaxed), 1);
    assert!(
        unsafe { drain(&mut vec![Entry::default(); 8]) }
            .unwrap_err()
            .contains("game root changed")
    );
}
