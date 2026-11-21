use super::*;
use crate::foreign_unwind_probe as probe;
use std::mem::offset_of;

#[repr(C)]
#[derive(Default)]
struct NativeCall {
    arguments: [usize; 6],
    observed: [usize; 6],
    raise: Option<probe::Raise>,
    entered: usize,
    returned: usize,
    continued: usize,
    result: usize,
    caller_rsp: usize,
    rsp: usize,
    return_pc: usize,
}
static CURRENT: AtomicUsize = AtomicUsize::new(0);

core::arch::global_asm!(
    ".text", ".globl bombing_foreign_caller", ".seh_proc bombing_foreign_caller",
    "bombing_foreign_caller:", "sub rsp, 0x48", ".seh_stackalloc 0x48",
    ".seh_endprologue", "mov [rsp + 0x38], rcx", "mov [rcx + {caller_rsp}], rsp", "mov r11, rcx",
    "mov rcx, [r11]", "mov rdx, [r11 + 8]",
    "mov r8, [r11 + 16]", "mov r9, [r11 + 24]",
    "mov rax, [r11 + 32]", "mov [rsp + 0x20], rax",
    "mov rax, [r11 + 40]", "mov [rsp + 0x28], rax",
    "call {original}", ".globl bombing_foreign_caller_return", "bombing_foreign_caller_return:",
    "mov r11, [rsp + 0x38]",
    "inc qword ptr [r11 + {continued}]", "mov [r11 + {result}], rax",
    "add rsp, 0x48", "ret", ".seh_endproc",
    ".globl bombing_foreign_original", ".seh_proc bombing_foreign_original",
    "bombing_foreign_original:", "mov [rsp + 0x20], rbx", ".seh_savereg rbx, 0x68",
    "push rsi", ".seh_pushreg rsi", "push rdi", ".seh_pushreg rdi",
    "push r15", ".seh_pushreg r15", "sub rsp, 0x30", ".seh_stackalloc 0x30",
    ".byte 0x48, 0x8b, 0xf9", ".seh_endprologue",
    "mov rbx, 0x4501", "mov rsi, 0x4502", "mov r15, 0x450f",
    "mov r11, [rip + {current}]", "inc qword ptr [r11 + {entered}]",
    "mov [r11 + {observed}], rcx", "mov [r11 + {observed} + 8], rdx",
    "mov [r11 + {observed} + 16], r8", "mov [r11 + {observed} + 24], r9",
    "mov r10, [rsp + 0x70]", "mov [r11 + {observed} + 32], r10",
    "mov r10, [rsp + 0x78]", "mov [r11 + {observed} + 40], r10",
    "lea rax, [rsp + 0x48]", "mov [r11 + {rsp}], rax",
    "mov rax, [rsp + 0x48]", "mov [r11 + {return_pc}], rax",
    "mov rax, [r11 + {raise}]", "test rax, rax", "jz 2f", "call rax", "ud2", "2:",
    "mov dword ptr [rdi + {level}], 0x10005", "mov qword ptr [rdi + {partial}], 0",
    "mov qword ptr [rdx], 1", "mov rax, rdx", "inc qword ptr [r11 + {returned}]",
    "add rsp, 0x30", "pop r15", "pop rdi", "pop rsi", "mov rbx, [rsp + 0x20]",
    "ret", ".seh_endproc",
    observed = const offset_of!(NativeCall, observed), entered = const offset_of!(NativeCall, entered),
    returned = const offset_of!(NativeCall, returned), continued = const offset_of!(NativeCall, continued),
    result = const offset_of!(NativeCall, result), caller_rsp = const offset_of!(NativeCall, caller_rsp),
    rsp = const offset_of!(NativeCall, rsp),
    return_pc = const offset_of!(NativeCall, return_pc), raise = const offset_of!(NativeCall, raise),
    current = sym CURRENT, original = sym bombing_foreign_original,
    level = const cbuilding::LEVEL, partial = const cbuilding::PARTIAL_HEALTH,
);
unsafe extern "system-unwind" {
    fn bombing_foreign_caller(argument: usize);
    fn bombing_foreign_original(
        building: usize,
        output: usize,
        request: usize,
        notify: usize,
        source: usize,
        extra: usize,
    ) -> usize;
    fn bombing_foreign_caller_return();
}

#[test]
#[ignore = "parent-only actual C++/SEH dispatch; requires the shared native probe DLL"]
fn building_bombing_foreign_unwind_retains_stranded_observation_and_restores_call() {
    let _serial = probe::SERIAL.lock().unwrap();
    let _owner = ObservationOwner::new();
    let driver = probe::Probe::load();
    let [normal, cpp, seh] = driver.cases();
    for case in [cpp, seh] {
        let mut world = World::new(false);
        setup(8, world.base);
        ready();
        let mut call = Box::new(NativeCall {
            arguments: [
                world.at(BUILDING),
                world.at(OUTPUT),
                1,
                1,
                world.at(SOURCE),
                1,
            ],
            ..Default::default()
        });
        let argument = &mut *call as *mut NativeCall as usize;
        CURRENT.store(argument, Ordering::Release);
        let site = bombing_foreign_original as *const () as usize;
        eprintln!(
            "building bombing {} caller={:#x} site={site:#x} bridge={:#x}",
            case.name,
            bombing_foreign_caller as *const () as usize,
            building_damage_entry as *const () as usize
        );
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        let mut hook =
            unsafe { Hook::new(site, building_damage_entry as *const () as usize, sentinel) }
                .unwrap();
        ORIGINAL.store(hook.trampoline(), Ordering::Release);
        let ranges = hook.executable_ranges();
        let retained_bytes = ranges
            .clone()
            .map(|range| unsafe { std::ptr::read_unaligned(range.start as *const [u8; 16]) });
        assert!(hook.set_enabled(true).unwrap());
        unsafe { driver.run(normal, bombing_foreign_caller, argument) };
        assert_eq!(call.observed, call.arguments);
        assert_eq!(
            call.return_pc,
            building_damage_native_return as *const () as usize
        );
        assert_eq!(call.rsp & 15, 8);
        assert_eq!(call.rsp, call.caller_rsp - 0x138 - 16);
        assert_eq!(
            (call.entered, call.returned, call.continued, call.result),
            (1, 1, 1, world.at(OUTPUT))
        );
        assert_eq!(active(), 0);
        assert_eq!(LEDGER.foreign_probe_state().0, None);
        let mut rows = Vec::with_capacity(capacity());
        assert_eq!(drain(&mut rows).unwrap(), 1);
        assert_eq!(
            (
                rows[0].attacker,
                rows[0].victim,
                rows[0].amount_raw,
                rows[0].credited_raw
            ),
            (Some(6001), 5001, 14_000_000, 1)
        );
        assert_eq!(
            rows[0].cause,
            native::Cause::Unknown,
            "synthetic return PC is not a game caller"
        );
        assert_eq!(
            (rows[0].game_hour, rows[0].state, rows[0].building_type),
            (12345, 3, TYPE)
        );
        world.health(2, 4_000_000);
        unsafe { driver.run(normal, bombing_foreign_caller, argument) };
        let (_, storage, capacity) = LEDGER.foreign_probe_state();
        assert!(capacity > 0);

        world.health(2, 4_000_000);
        call.raise = case.raise;
        unsafe { driver.run(case, bombing_foreign_caller, argument) };
        assert_eq!(call.observed, call.arguments);
        assert_eq!(
            call.return_pc,
            building_damage_native_return as *const () as usize
        );
        assert_eq!(call.rsp & 15, 8);
        assert_eq!(call.rsp, call.caller_rsp - 0x138 - 16);
        assert_eq!((call.entered, call.returned, call.continued), (3, 2, 2));
        assert_eq!(
            active(),
            1,
            "native unwind skipped the production post callback"
        );
        let pending = Some((call.arguments[0], 0));
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        boundary(clock(3, 4));
        assert!(LEDGER.stranded.load(Ordering::SeqCst));
        assert!(LEDGER.poisoned.load(Ordering::SeqCst));
        assert_eq!(
            drain(&mut rows).unwrap_err(),
            "event ledger active or unwound native writer at drain"
        );
        boundary(clock(4, 4));
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        assert_eq!(active(), 1);
        assert_eq!(LEDGER.lost_drains.load(Ordering::Relaxed), 3);
        assert!(LEDGER.lost_events_unknown.load(Ordering::Relaxed) > 0);

        call.raise = None;
        unsafe { driver.run(normal, bombing_foreign_caller, argument) };
        assert_eq!((call.entered, call.returned, call.continued), (4, 3, 3));
        assert_eq!(call.result, world.at(OUTPUT));
        assert_eq!(
            active(),
            1,
            "a later native return does not release the stranded claim"
        );
        assert_eq!(LEDGER.calls.load(Ordering::SeqCst), 4);
        assert_eq!(LEDGER.events.load(Ordering::Relaxed), 2);
        assert_eq!(LEDGER.overlaps.load(Ordering::Relaxed), 1);
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        assert!(LEDGER.lost_events.load(Ordering::Relaxed) > 0);
        ENABLED.store(false, Ordering::SeqCst);
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        assert_eq!(
            hook.executable_ranges(),
            ranges,
            "disabled storage stays owned while stranded"
        );
        assert_eq!(
            ranges
                .clone()
                .map(|range| unsafe { std::ptr::read_unaligned(range.start as *const [u8; 16]) }),
            retained_bytes
        );
        unsafe { driver.run(normal, bombing_foreign_caller, argument) };
        assert_eq!((call.entered, call.returned, call.continued), (5, 4, 4));
        assert_eq!(
            call.return_pc,
            bombing_foreign_caller_return as *const () as usize
        );
        assert_eq!(call.rsp, call.caller_rsp - 8);
        assert_eq!(call.observed, call.arguments);
        assert_eq!(call.result, world.at(OUTPUT));
        assert_eq!(LEDGER.calls.load(Ordering::SeqCst), 4);
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        eprintln!(
            "PASS building bombing {}: actual handler, stranded capture, restored native call",
            case.name
        );
        drop(hook);
        LEDGER.reset();
        CURRENT.store(0, Ordering::Release);
    }
}
