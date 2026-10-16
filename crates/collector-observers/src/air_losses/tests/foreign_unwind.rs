//! Actual native handler dispatch through the production CALL tap and bridge.
use super::*;
use crate::foreign_unwind_probe as probe;
use std::mem::offset_of;

#[repr(C)]
#[derive(Default)]
struct NativeCall {
    arguments: [usize; 7],
    observed: [usize; 7],
    tags: [usize; 2],
    observed_tags: [usize; 2],
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
const NATIVE_RETURN: usize = 0x7450_a17f_1234_5678;

// Authored native caller and original. Foreign raises happen only after pre
// returned, in assembly; neither a Rust catcher nor a non-unwind Rust frame
// surrounds the raise. Windows uses these real .seh frames during dispatch.
core::arch::global_asm!(
    ".text", ".globl air_foreign_caller", ".seh_proc air_foreign_caller",
    "air_foreign_caller:", "push rsi", ".seh_pushreg rsi",
    "push rdi", ".seh_pushreg rdi", "sub rsp, 0x48", ".seh_stackalloc 0x48",
    ".seh_endprologue", "mov [rsp + 0x38], rcx", "mov [rcx + {caller_rsp}], rsp", "mov r11, rcx",
    "mov rsi, [r11 + {tags}]", "mov rdi, [r11 + {tags} + 8]",
    "mov rcx, [r11]", "mov rdx, [r11 + 8]",
    "mov r8, [r11 + 16]", "mov r9, [r11 + 24]",
    "mov rax, [r11 + 32]", "mov [rsp + 0x20], rax",
    "mov rax, [r11 + 40]", "mov [rsp + 0x28], rax",
    "mov rax, [r11 + 48]", "mov [rsp + 0x30], rax",
    ".globl air_foreign_site", "air_foreign_site:", "call {original}",
    ".rept 11", "nop", ".endr",
    "mov r11, [rsp + 0x38]", "inc qword ptr [r11 + {continued}]",
    "mov [r11 + {result}], rax",
    "add rsp, 0x48", "pop rdi", "pop rsi", "ret", ".seh_endproc",
    ".globl air_foreign_original", ".seh_proc air_foreign_original",
    "air_foreign_original:", "sub rsp, 0x48", ".seh_stackalloc 0x48",
    ".seh_endprologue", "mov r11, [rip + {current}]",
    "inc qword ptr [r11 + {entered}]",
    "mov [r11 + {observed}], rcx", "mov [r11 + {observed} + 8], rdx",
    "mov [r11 + {observed} + 16], r8", "mov [r11 + {observed} + 24], r9",
    "mov r10, [rsp + 0x70]", "mov [r11 + {observed} + 32], r10",
    "mov [rsp + 0x20], r10",
    "mov r10, [rsp + 0x78]", "mov [r11 + {observed} + 40], r10",
    "mov [rsp + 0x28], r10",
    "mov r10, [rsp + 0x80]", "mov [r11 + {observed} + 48], r10",
    "mov [rsp + 0x30], r10",
    "mov [r11 + {observed_tags}], rsi", "mov [r11 + {observed_tags} + 8], rdi",
    "lea rax, [rsp + 0x48]", "mov [r11 + {rsp}], rax",
    "mov rax, [rsp + 0x48]", "mov [r11 + {return_pc}], rax",
    "mov rax, [r11 + {raise}]", "test rax, rax", "jz 2f",
    "call rax", "ud2", "2:", "call {write}",
    "mov r11, [rip + {current}]", "inc qword ptr [r11 + {returned}]",
    "mov rax, {native_return}",
    "add rsp, 0x48", "ret", ".seh_endproc",
    tags = const offset_of!(NativeCall, tags), observed = const offset_of!(NativeCall, observed),
    observed_tags = const offset_of!(NativeCall, observed_tags),
    entered = const offset_of!(NativeCall, entered), returned = const offset_of!(NativeCall, returned),
    continued = const offset_of!(NativeCall, continued), result = const offset_of!(NativeCall, result),
    caller_rsp = const offset_of!(NativeCall, caller_rsp), rsp = const offset_of!(NativeCall, rsp),
    return_pc = const offset_of!(NativeCall, return_pc),
    raise = const offset_of!(NativeCall, raise), current = sym CURRENT,
    native_return = const NATIVE_RETURN,
    original = sym air_foreign_original, write = sym native_write,
);
unsafe extern "system-unwind" {
    fn air_foreign_caller(argument: usize);
    fn air_foreign_original(
        receiver: usize,
        wing: usize,
        count: usize,
        enemy: usize,
        region: usize,
        selection: usize,
        cause: usize,
    ) -> usize;
    fn air_foreign_site();
}

#[test]
#[ignore = "parent-only actual C++/SEH dispatch; requires the shared native probe DLL"]
fn air_loss_foreign_unwind_retains_stranded_observation_and_restores_call() {
    let _serial = probe::SERIAL.lock().unwrap();
    let _owner = ObservationOwner::new();
    let driver = probe::Probe::load();
    let [normal, cpp, seh] = driver.cases();
    for case in [cpp, seh] {
        let mut world = World::new(2);
        world.u32(World::GAME + 0x800, 2);
        setup(&world);
        let mut call = Box::new(NativeCall {
            arguments: [
                world.pointer(World::AIR),
                world.pointer(World::WING),
                1,
                0,
                world.pointer(World::REGION),
                0,
                1,
            ],
            tags: [
                world.pointer(World::AIR + cstrategic_air::COUNTRY),
                world.pointer(World::GAME + 0x800),
            ],
            ..Default::default()
        });
        let argument = &mut *call as *mut NativeCall as usize;
        CURRENT.store(argument, Ordering::Release);
        ORIGINAL.store(
            air_foreign_original as *const () as usize,
            Ordering::Release,
        );
        let site = air_foreign_site as *const () as usize;
        eprintln!(
            "air loss {} caller={:#x} site={site:#x} bridge={:#x} original={:#x}",
            case.name,
            air_foreign_caller as *const () as usize,
            air_loss_entry as *const () as usize,
            air_foreign_original as *const () as usize
        );
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        let mut hook =
            unsafe { Hook::new_call(site, air_loss_entry as *const () as usize, sentinel) }
                .unwrap();
        let ranges = hook.executable_ranges();
        let retained_bytes = ranges
            .clone()
            .map(|range| unsafe { std::ptr::read_unaligned(range.start as *const [u8; 16]) });
        assert!(hook.set_enabled(true).unwrap());
        unsafe { driver.run(normal, air_foreign_caller, argument) };
        assert_eq!(call.observed, call.arguments);
        assert_eq!(call.observed_tags, call.tags);
        assert_eq!(
            TAPPED_TAGS
                .each_ref()
                .map(|tag| tag.load(Ordering::Relaxed)),
            call.tags
        );
        assert_eq!(
            call.return_pc,
            air_loss_original_return as *const () as usize
        );
        assert_eq!(call.rsp & 15, 8);
        assert_eq!(call.rsp, call.caller_rsp - FRAME_BYTES - 16);
        assert_eq!(
            (call.entered, call.returned, call.continued, call.result),
            (1, 1, 1, NATIVE_RETURN)
        );
        assert_eq!(active(), 0);
        assert_eq!(LEDGER.foreign_probe_state().0, None);
        let mut rows = Vec::with_capacity(capacity());
        assert_eq!(drain(&mut rows).unwrap(), 1);
        assert_eq!(
            (rows[0].country, rows[0].opponent, rows[0].amount_raw),
            (1, Some(2), 100000)
        );
        assert_eq!(
            (rows[0].game_hour, rows[0].native_cause, rows[0].daily_slot),
            (48, 1, 29)
        );
        unsafe { driver.run(normal, air_foreign_caller, argument) };
        let (_, storage, capacity) = LEDGER.foreign_probe_state();
        assert!(capacity > 0);

        call.raise = case.raise;
        unsafe { driver.run(case, air_foreign_caller, argument) };
        assert_eq!(call.observed, call.arguments);
        assert_eq!(
            call.return_pc,
            air_loss_original_return as *const () as usize
        );
        assert_eq!(call.rsp & 15, 8);
        assert_eq!(call.rsp, call.caller_rsp - FRAME_BYTES - 16);
        assert_eq!((call.entered, call.returned, call.continued), (3, 2, 2));
        assert_eq!(
            active(),
            1,
            "native unwind skipped the production post callback"
        );
        let pending = Some((call.arguments[0], 23));
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        boundary(clock(49));
        assert!(LEDGER.stranded.load(Ordering::SeqCst));
        assert!(LEDGER.poisoned.load(Ordering::SeqCst));
        assert_eq!(
            drain(&mut rows).unwrap_err(),
            "event ledger active or unwound native writer at drain"
        );
        boundary(clock(50));
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        assert_eq!(active(), 1);
        assert_eq!(LEDGER.lost_drains.load(Ordering::Relaxed), 3);
        assert!(LEDGER.lost_events_unknown.load(Ordering::Relaxed) > 0);

        call.raise = None;
        unsafe { driver.run(normal, air_foreign_caller, argument) };
        assert_eq!((call.entered, call.returned, call.continued), (4, 3, 3));
        assert_eq!(call.result, NATIVE_RETURN);
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
        unsafe { driver.run(normal, air_foreign_caller, argument) };
        assert_eq!((call.entered, call.returned, call.continued), (5, 4, 4));
        assert_eq!(call.return_pc, site + 5);
        assert_eq!(call.rsp, call.caller_rsp - 8);
        assert_eq!(call.observed, call.arguments);
        assert_eq!(call.result, NATIVE_RETURN);
        assert_eq!(LEDGER.calls.load(Ordering::SeqCst), 4);
        assert_eq!(LEDGER.foreign_probe_state(), (pending, storage, capacity));
        eprintln!(
            "PASS air loss {}: actual handler, stranded capture, restored native call",
            case.name
        );
        // The synthetic call and native handler have returned; no live game or
        // runtime detach is involved. This reset is diagnostic teardown only.
        drop(hook);
        LEDGER.reset();
        CURRENT.store(0, Ordering::Release);
    }
}
