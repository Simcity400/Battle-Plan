use super::*;
use crate::foreign_unwind_probe as probe;
use std::mem::offset_of;

#[repr(C)]
#[derive(Default)]
struct NativeCall {
    arguments: [usize; 4],
    observed: [usize; 4],
    raise: Option<probe::Raise>,
    entered: usize,
    returned: usize,
    continued: usize,
    result: usize,
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

core::arch::global_asm!(
    ".text",
    ".globl engagement_foreign_caller",
    ".seh_proc engagement_foreign_caller",
    "engagement_foreign_caller:",
    "sub rsp, 0x38",
    ".seh_stackalloc 0x38",
    ".seh_endprologue",
    "mov [rsp + 0x28], rcx",
    "mov r11, rcx",
    "mov rcx, [r11]",
    "mov rdx, [r11 + 8]",
    "mov r8, [r11 + 16]",
    "mov r9, [r11 + 24]",
    "call {original}",
    "mov r11, [rsp + 0x28]",
    "inc qword ptr [r11 + {continued}]",
    "mov [r11 + {result}], rax",
    "add rsp, 0x38",
    "ret",
    ".seh_endproc",
    ".globl engagement_foreign_original",
    ".seh_proc engagement_foreign_original",
    "engagement_foreign_original:",
    "push rbx",
    ".seh_pushreg rbx",
    "push rsi",
    ".seh_pushreg rsi",
    "push rdi",
    ".seh_pushreg rdi",
    "sub rsp, 0x20",
    ".seh_stackalloc 0x20",
    ".seh_endprologue",
    "mov rbx, 0x4601",
    "mov rsi, 0x4602",
    "mov rdi, 0x4603",
    "mov r11, [rip + {current}]",
    "inc qword ptr [r11 + {entered}]",
    "mov [r11 + {observed}], rcx",
    "mov [r11 + {observed} + 8], rdx",
    "mov [r11 + {observed} + 16], r8",
    "mov [r11 + {observed} + 24], r9",
    "mov rax, [r11 + {raise}]",
    "test rax, rax",
    "jz 2f",
    "call rax",
    "ud2",
    "2:",
    "mov rax, 0x77",
    "inc qword ptr [r11 + {returned}]",
    "add rsp, 0x20",
    "pop rdi",
    "pop rsi",
    "pop rbx",
    "ret",
    ".seh_endproc",
    observed = const offset_of!(NativeCall, observed),
    entered = const offset_of!(NativeCall, entered),
    returned = const offset_of!(NativeCall, returned),
    continued = const offset_of!(NativeCall, continued),
    result = const offset_of!(NativeCall, result),
    raise = const offset_of!(NativeCall, raise),
    current = sym CURRENT,
    original = sym engagement_foreign_original,
);

unsafe extern "system-unwind" {
    fn engagement_foreign_caller(argument: usize);
    fn engagement_foreign_original(command: usize, rdx: usize, r8: usize, r9: usize) -> usize;
}

#[test]
#[ignore = "parent-only actual C++/SEH dispatch; requires the shared native probe DLL"]
fn combat_engagement_foreign_unwind_retains_flight_and_refuses_interval() {
    let _serial = probe::SERIAL.lock().unwrap();
    let _scope = TestScope::enter();
    let driver = probe::Probe::load();
    let [normal, cpp, seh] = driver.cases();
    let mut command_mem = [0u8; 0x40];
    write_u32(&mut command_mem, native::CHANGE_CONTROLLER_TARGET, 42);
    let command_ptr = command_mem.as_ptr() as usize;

    for case in [cpp, seh] {
        DEPTH.with(|depth| depth.set(0));
        FLIGHTS.store(0, Ordering::SeqCst);
        LEDGER.reset();
        ENABLED.store(true, Ordering::SeqCst);
        // SAFETY: Isolated foreign-unwind test owns observation via TestScope and probe::SERIAL.
        unsafe {
            presize(16);
            boundary(Some(clock(100)));
        }
        let mut out = Vec::new();
        // SAFETY: Isolated foreign-unwind test owns observation via TestScope and probe::SERIAL.
        assert!(unsafe { drain(&mut out) }.is_err());

        let mut call = Box::new(NativeCall {
            arguments: [command_ptr, 11, 22, 33],
            ..Default::default()
        });
        let argument = &mut *call as *mut NativeCall as usize;
        CURRENT.store(argument, Ordering::Release);
        let site = engagement_foreign_original as *const () as usize;
        // SAFETY: reading the 16-byte entry prologue of the local assembly fixture function.
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        // SAFETY: hooking the test-only local assembly function with the production callback.
        let mut hook =
            unsafe { Hook::new(site, change_controller as *const () as usize, sentinel) }.unwrap();
        ORIGINAL[3].store(hook.trampoline(), Ordering::Release);
        assert!(hook.set_enabled(true).unwrap());

        // SAFETY: invoking the native foreign-unwind probe on the test assembly caller.
        unsafe { driver.run(normal, engagement_foreign_caller, argument) };
        assert_eq!(call.observed, call.arguments);
        assert_eq!((call.entered, call.returned, call.continued, call.result), (1, 1, 1, 0x77));
        assert_eq!(active(), 0);
        // SAFETY: Isolated foreign-unwind test owns observation via TestScope and probe::SERIAL.
        unsafe { boundary(Some(clock(101))) };
        // SAFETY: Isolated foreign-unwind test owns observation via TestScope and probe::SERIAL.
        assert_eq!(unsafe { drain(&mut out) }.unwrap(), 1);
        assert_eq!(out[0].kind, CombatEngagementRefreshRow::CONTROLLER_TARGET);
        assert_eq!(out[0].target, 42);

        call.raise = case.raise;
        // SAFETY: invoking the native foreign-unwind probe to unwind through the live production hook.
        unsafe { driver.run(case, engagement_foreign_caller, argument) };
        assert_eq!((call.entered, call.returned, call.continued), (2, 1, 1));
        let retained_flights = FLIGHTS.load(Ordering::SeqCst);
        let retained_active = active();
        assert_eq!(retained_flights, 1);
        assert!(retained_active >= 1);
        // SAFETY: Isolated foreign-unwind test owns observation via TestScope and probe::SERIAL.
        unsafe { boundary(Some(clock(102))) };
        // SAFETY: Isolated foreign-unwind test owns observation via TestScope and probe::SERIAL.
        assert!(unsafe { drain(&mut out) }.is_err());

        call.raise = None;
        // SAFETY: verifying subsequent ordinary native calls still return while the unwound flight stays retained.
        unsafe { driver.run(normal, engagement_foreign_caller, argument) };
        assert_eq!((call.entered, call.returned, call.continued), (3, 2, 2));
        assert_eq!(FLIGHTS.load(Ordering::SeqCst), retained_flights);
        assert_eq!(active(), retained_active);

        ENABLED.store(false, Ordering::SeqCst);
        assert!(hook.set_enabled(false).unwrap());
        // SAFETY: verifying the restored prologue matches the saved sentinel bytes.
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        drop(hook);
        DEPTH.with(|depth| depth.set(0));
        FLIGHTS.store(0, Ordering::SeqCst);
        LEDGER.reset();
        CURRENT.store(0, Ordering::Release);
    }
}
