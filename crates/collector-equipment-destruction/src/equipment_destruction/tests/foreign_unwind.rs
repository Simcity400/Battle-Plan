use super::*;
use crate::foreign_unwind_probe as probe;
use collector_hooks::hook::Hook;
use std::ptr;
use windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry;

static RAISE: AtomicUsize = AtomicUsize::new(0);
static NATIVE_CALLS: AtomicUsize = AtomicUsize::new(0);
static NATIVE_RETURNS: AtomicUsize = AtomicUsize::new(0);
#[repr(C)]
struct Invocation {
    reserves: usize,
    pool: usize,
    continued: usize,
}

core::arch::global_asm!(
    ".text", ".globl land_foreign_return_caller", ".seh_proc land_foreign_return_caller",
    "land_foreign_return_caller:", "push rbx", ".seh_pushreg rbx",
    "sub rsp, 0x20", ".seh_stackalloc 0x20", ".seh_endprologue",
    "mov rbx, rcx", "mov rcx, [rbx]", "mov rdx, [rbx + 8]",
    ".globl land_foreign_return_site", "land_foreign_return_site:", "call {original}",
    ".globl land_foreign_return_continuation", "land_foreign_return_continuation:",
    ".rept 11", "nop", ".endr", "inc qword ptr [rbx + 16]",
    "add rsp, 0x20", "pop rbx", "ret", ".seh_endproc",
    ".globl land_foreign_credit_original", ".seh_proc land_foreign_credit_original",
    "land_foreign_credit_original:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "call {credit}", "mov rax, qword ptr [rip + {raise}]", "test rax, rax", "jz 2f",
    "call rax", "2:", "lock inc qword ptr [rip + {returned}]",
    "add rsp, 0x28", "ret", ".seh_endproc",
    original = sym land_foreign_credit_original, credit = sym credit,
    raise = sym RAISE, returned = sym NATIVE_RETURNS,
);
unsafe extern "system-unwind" {
    fn land_foreign_return_caller(argument: usize);
    fn land_foreign_return_site();
    fn land_foreign_return_continuation();
    fn land_foreign_credit_original(reserves: usize, pool: usize);
}
core::arch::global_asm!(
    ".text", ".globl land_foreign_damage_caller", ".seh_proc land_foreign_damage_caller",
    "land_foreign_damage_caller:", "sub rsp, 0x48", ".seh_stackalloc 0x48", ".seh_endprologue",
    "mov rax, [rsp + 0x70]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x78]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0x80]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0x88]", "mov [rsp + 0x38], rax", "call {entry}",
    ".globl land_foreign_damage_return", "land_foreign_damage_return:",
    "add rsp, 0x48", "ret", ".seh_endproc",
    ".globl land_foreign_damage_target", ".seh_proc land_foreign_damage_target",
    "land_foreign_damage_target:", "mov [rsp + 0x20], rbx",
    "sub rsp, 0x68", ".seh_stackalloc 0x68", ".seh_endprologue",
    ".rept 7", "nop", ".endr",
    "mov rax, [rsp + 0x90]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x98]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0xa0]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0xa8]", "mov [rsp + 0x38], rax", "call {damage}",
    "mov rax, qword ptr [rip + {raise}]", "test rax, rax", "jz 2f", "call rax",
    "2:", "lock inc qword ptr [rip + {returned}]", "add rsp, 0x68", "ret", ".seh_endproc",
    entry = sym land_foreign_damage_target, damage = sym land_damage,
    raise = sym RAISE, returned = sym NATIVE_RETURNS,
);
unsafe extern "system-unwind" {
    fn land_foreign_damage_caller(
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        e: usize,
        f: usize,
        g: usize,
        h: usize,
    );
    fn land_foreign_damage_return();
    fn land_foreign_damage_target(
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        e: usize,
        f: usize,
        g: usize,
        h: usize,
    );
}
unsafe extern "system" fn land_damage(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    NATIVE_CALLS.fetch_add(1, Ordering::SeqCst);
    assert_eq!(ACTIVE[SiteId::Land as usize].load(Ordering::SeqCst), 1);
    assert!(OBSERVING.get());
    unsafe { land_removal(a, b, c, d, e, f, g, h) };
}
struct DamageInvocation {
    slots: [usize; 8],
    continued: usize,
}
unsafe extern "system-unwind" fn invoke_damage(argument: usize) {
    let invocation = unsafe { &mut *(argument as *mut DamageInvocation) };
    let [a, b, c, d, e, f, g, h] = invocation.slots;
    unsafe { land_foreign_damage_caller(a, b, c, d, e, f, g, h) };
    invocation.continued += 1;
}
unsafe extern "system" fn credit(reserves: usize, pool: usize) {
    NATIVE_CALLS.fetch_add(1, Ordering::SeqCst);
    let army = pool - carmy::EQUIPMENT_POOL;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let handle = native::army_identity(&memory, BASE.load(Ordering::Relaxed), army).unwrap();
    assert_eq!(ACTIVE[SiteId::HqReturn as usize].load(Ordering::SeqCst), 1);
    assert!(OBSERVING.get());
    assert_eq!(
        folded()
            .credits
            .state(handle, army, 4999)
            .unwrap_err(),
        "equipment returned source lifetime incomplete"
    );
    let enabled = ENABLED.swap(false, Ordering::SeqCst);
    let supplemental = SUPPLEMENTAL_ENABLED.swap(false, Ordering::SeqCst);
    let retained = release();
    ENABLED.store(enabled, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(supplemental, Ordering::SeqCst);
    assert_eq!(
        retained,
        Err("equipment event storage release before callback quiescence")
    );
    unsafe { transfer(reserves, pool) };
}
fn registered(address: usize) -> bool {
    let mut image = 0;
    !unsafe { RtlLookupFunctionEntry(address as u64, &raw mut image, ptr::null_mut()) }.is_null()
}

#[test]
#[ignore = "parent-only actual Land detour C++/SEH; requires shared probe and reviewed Hook"]
fn production_land_detour_foreign_unwind() {
    let _probe_guard = probe::SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _serial = serial();
    let probe = probe::Probe::load();
    let site = land_foreign_damage_target as *const () as usize;
    let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let original: ArmyFn = land_foreign_damage_target;
    let body: unsafe extern "system-unwind" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) = land;
    assert!(registered(site) && registered(destruction_land_entry as *const () as usize));
    assert!(registered(body as *const () as usize));
    let mut hook =
        unsafe { Hook::new(site, destruction_land_entry as *const () as usize, sentinel) }.unwrap();
    println!(
        "LAND damage site={site:#x} shim={:#x} original={:#x} ranges={:?}",
        destruction_land_entry as *const () as usize,
        original as *const () as usize,
        hook.executable_ranges()
    );
    for case in probe.cases() {
        setup(8, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let base = land_foreign_damage_return as *const () as usize - native::LAND_COMBAT_RETURN.0;
        BASE.store(base, Ordering::SeqCst);
        let mut world = World::new(1);
        world.rebase(base);
        world.owner(4999);
        world.amount(0, 500000);
        ORIGINAL[SiteId::Land as usize].store(hook.trampoline(), Ordering::SeqCst);
        let slots = [
            world.army(),
            world.design(0),
            100000,
            0xd004,
            0xe005,
            0xf006,
            0xa007,
            0xb008,
        ];
        let mut invocation = DamageInvocation {
            slots,
            continued: 0,
        };
        NATIVE_CALLS.store(0, Ordering::SeqCst);
        NATIVE_RETURNS.store(0, Ordering::SeqCst);
        RAISE.store(
            case.raise.map_or(0, |raise| raise as *const () as usize),
            Ordering::SeqCst,
        );
        let refusals = SITE_REFUSALS[SiteId::Land as usize].load(Ordering::Relaxed);
        let capacity = folded().owned_capacity();
        assert!(Hook::set_enabled_batch(&mut [&mut hook], true).unwrap());
        unsafe {
            probe.run(
                case,
                invoke_damage,
                std::ptr::addr_of_mut!(invocation) as usize,
            )
        };
        let normal = usize::from(case.expected == 1);
        assert_eq!(
            *FORWARDED.lock().unwrap(),
            slots,
            "all eight native operands"
        );
        assert_eq!(world.storage[(0x600 + 8) / 8], 400000);
        assert_eq!(
            (
                NATIVE_CALLS.load(Ordering::SeqCst),
                NATIVE_RETURNS.load(Ordering::SeqCst),
                invocation.continued
            ),
            (1, normal, normal)
        );
        assert_eq!(active(), 1 - normal);
        assert!(!OBSERVING.get());
        assert_eq!(folded().poisoned, normal == 0);
        assert_eq!(
            SITE_REFUSALS[SiteId::Land as usize].load(Ordering::Relaxed),
            refusals + u64::from(normal == 0)
        );
        assert_eq!(folded().owned_capacity(), capacity);
        assert!(release().is_err());
        if normal != 0 {
            let mut rows = vec![Entry::default(); 8];
            assert_eq!(drain(&mut rows).unwrap(), 1);
            assert_eq!(
                (rows[0].country, rows[0].cause, rows[0].amount_raw),
                (4999, native::Cause::LandCombat, 100000)
            );
        } else {
            assert!(drain(&mut vec![Entry::default(); 8]).is_err());
        }
        assert!(Hook::set_enabled_batch(&mut [&mut hook], false).unwrap());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        boundary(clock(10, 2));
        boundary(clock(11, 2));
        assert_eq!(active(), 1 - normal);
        assert!(!OBSERVING.get());
        ENABLED.store(false, Ordering::SeqCst);
        SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
        if normal == 0 {
            assert!(release().is_err());
            ACTIVE[SiteId::Land as usize].store(0, Ordering::SeqCst);
        }
        release().unwrap();
        println!(
            "PASS LAND damage {}: native handler, eight operands, retained callback, restored site",
            case.name
        );
    }
}

#[test]
#[ignore = "parent-only actual foreign C++/SEH; requires BATTLEPLAN_TAP_UNWIND_PROBE and reviewed shared Hook"]
fn production_return_call_foreign_unwind() {
    let _probe_guard = probe::SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _serial = serial();
    let probe = probe::Probe::load();
    let site = land_foreign_return_site as *const () as usize;
    assert_eq!(
        land_foreign_return_continuation as *const () as usize,
        site + 5
    );
    let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let callback: unsafe extern "system-unwind" fn(usize, usize) = hq_returned;
    let original: unsafe extern "system-unwind" fn(usize, usize) = land_foreign_credit_original;
    assert!(registered(site) && registered(original as *const () as usize));
    let mut hook =
        unsafe { Hook::new_call(site, callback as *const () as usize, sentinel) }.unwrap();
    println!(
        "LAND return site={site:#x} callback={:#x} original={:#x} ranges={:?}",
        callback as *const () as usize,
        original as *const () as usize,
        hook.executable_ranges()
    );
    static TRACKER: collector_primitives::raw_memory::Tracker =
        collector_primitives::raw_memory::Tracker::new();
    for case in probe.cases() {
        setup(8, 2);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(1);
        world.rebase(base);
        world.owner(4999);
        world.amount(0, 100000);
        let reserve = ReserveWorld::for_source(&world, base, 4999, 0);
        let handle = context_rebased(&world, base, native::Cause::Unknown).handle;
        ORIGINAL[SiteId::HqReturn as usize].store(original as *const () as usize, Ordering::SeqCst);
        let expected_capacity = {
            let mut store = folded();
            let capacity = store.owned_capacity();
            store._charge = collector_primitives::raw_memory::Charge::empty_on(&TRACKER);
            store._charge.set(capacity);
            capacity
        };
        NATIVE_CALLS.store(0, Ordering::SeqCst);
        NATIVE_RETURNS.store(0, Ordering::SeqCst);
        RAISE.store(
            case.raise.map_or(0, |raise| raise as *const () as usize),
            Ordering::SeqCst,
        );
        let refusals = SITE_REFUSALS[SiteId::HqReturn as usize].load(Ordering::Relaxed);
        let mut invocation = Invocation {
            reserves: reserve.manager.as_ptr() as usize,
            pool: world.army() + carmy::EQUIPMENT_POOL,
            continued: 0,
        };
        assert!(Hook::set_enabled_batch(&mut [&mut hook], true).unwrap());
        unsafe {
            probe.run(
                case,
                land_foreign_return_caller,
                std::ptr::addr_of_mut!(invocation) as usize,
            )
        };
        let normal = usize::from(case.expected == 1);
        assert_eq!(
            (
                NATIVE_CALLS.load(Ordering::SeqCst),
                NATIVE_RETURNS.load(Ordering::SeqCst),
                invocation.continued
            ),
            (1, normal, normal)
        );
        assert_eq!(reserve.world.storage[(0x600 + 8) / 8], 100000);
        assert_eq!(active(), 1 - normal);
        assert!(!OBSERVING.get());
        assert_eq!(folded().poisoned, normal == 0);
        assert_eq!(
            SITE_REFUSALS[SiteId::HqReturn as usize].load(Ordering::Relaxed),
            refusals + u64::from(normal == 0)
        );
        assert_eq!(TRACKER.snapshot().current, expected_capacity);
        assert!(release().is_err());
        if normal == 0 {
            assert_eq!(
                folded()
                    .credits
                    .state(handle, world.army(), 4999)
                    .unwrap_err(),
                "equipment returned source lifetime incomplete"
            );
            boundary(clock(20, 1));
            assert!(
                folded().poisoned,
                "foreign mutation remains unavailable across a gap"
            );
            boundary(clock(21, 1));
            assert!(drain(&mut vec![Entry::default(); 8]).is_err());
        } else {
            assert_eq!(
                folded()
                    .credits
                    .state(handle, world.army(), 4999)
                    .unwrap(),
                PoolState::Bookkeeping
            );
        }
        assert!(Hook::set_enabled_batch(&mut [&mut hook], false).unwrap());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        boundary(clock(22, 2));
        boundary(clock(23, 2));
        boundary(clock(24, 2));
        assert!(!folded().poisoned);
        assert_eq!(active(), 1 - normal);
        assert!(!OBSERVING.get());
        ENABLED.store(false, Ordering::SeqCst);
        SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
        if normal == 0 {
            assert!(release().is_err());
            ACTIVE[SiteId::HqReturn as usize].store(0, Ordering::SeqCst);
        }
        release().unwrap();
        assert_eq!(TRACKER.snapshot().current, Default::default());
        println!(
            "PASS LAND return {}: native handler, armed refusal, retained callback/storage, restored site",
            case.name
        );
    }
}
