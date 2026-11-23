use super::*;
use crate::foreign_unwind_probe as probe;
use std::mem::offset_of;
use windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry;

#[repr(C)]
#[derive(Default)]
struct NativeCall {
    arguments: [usize; 2],
    observed: [usize; 2],
    raise: Option<probe::Raise>,
    site: usize,
    entered: usize,
    returned: usize,
    continued: usize,
    result: usize,
    caller_rsp: usize,
    rsp: usize,
    return_pc: usize,
    completed_rsp: usize,
    hooked: bool,
}
static CURRENT: AtomicUsize = AtomicUsize::new(0);

core::arch::global_asm!(
    ".text", ".globl naval_foreign_caller", ".seh_proc naval_foreign_caller",
    "naval_foreign_caller:", "sub rsp, 0x38", ".seh_stackalloc 0x38",
    ".seh_endprologue", "mov [rsp + 0x20], rcx", "mov [rcx + {caller_rsp}], rsp",
    "mov r11, rcx", "mov rcx, [r11]", "mov rdx, [r11 + 8]",
    ".globl naval_foreign_site", "naval_foreign_site:", "call {original}",
    ".rept 11", "nop", ".endr",
    "mov r11, [rsp + 0x20]", "inc qword ptr [r11 + {continued}]",
    "mov [r11 + {result}], rax", "mov [r11 + {completed_rsp}], rsp",
    "add rsp, 0x38", "ret", ".seh_endproc",
    ".globl naval_foreign_original", ".seh_proc naval_foreign_original",
    "naval_foreign_original:", "mov [rsp + 8], rbx", "sub rsp, 0x48",
    ".seh_stackalloc 0x48", ".seh_savereg rbx, 0x50",
    ".rept 7", "nop", ".endr", ".seh_endprologue",
    "lea r8, [rsp + 0x48]", "mov r9, [rsp + 0x48]", "call {record}",
    "mov rbx, [rip + {current}]", "mov rax, [rbx + {raise}]",
    "test rax, rax", "jz 2f", "call rax", "ud2", "2:",
    "inc qword ptr [rbx + {returned}]", "mov rax, [rbx]",
    "mov rbx, [rsp + 0x50]", "add rsp, 0x48", "ret", ".seh_endproc",
    caller_rsp = const offset_of!(NativeCall, caller_rsp),
    continued = const offset_of!(NativeCall, continued),
    result = const offset_of!(NativeCall, result),
    completed_rsp = const offset_of!(NativeCall, completed_rsp),
    returned = const offset_of!(NativeCall, returned),
    raise = const offset_of!(NativeCall, raise), current = sym CURRENT,
    original = sym naval_foreign_original, record = sym record_original,
);
unsafe extern "system-unwind" {
    fn naval_foreign_caller(argument: usize);
    fn naval_foreign_original(first: usize, second: usize) -> usize;
    fn naval_foreign_site();
}

unsafe extern "system" fn record_original(first: usize, second: usize, rsp: usize, pc: usize) {
    let call = unsafe { &mut *(CURRENT.load(Ordering::Acquire) as *mut NativeCall) };
    call.entered += 1;
    call.observed = [first, second];
    call.rsp = rsp;
    call.return_pc = pc;
    if call.hooked {
        assert_eq!(active(), 1);
        assert!(active() != 0);
        assert!(store().try_lock().is_ok());
        let enabled = ENABLED.swap(false, Ordering::SeqCst);
        let installed = SITE_ENABLED[call.site].swap(false, Ordering::SeqCst);
        assert_eq!(
            release(),
            Err("naval result source release before callback quiescence")
        );
        SITE_ENABLED[call.site].store(installed, Ordering::SeqCst);
        ENABLED.store(enabled, Ordering::SeqCst);
    }
    if call.site < 2 {
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let hour = native::creation_hour(&memory, BASE.load(Ordering::Relaxed)).unwrap();
        unsafe { *((first + cnaval_combat_results::RESULT_HOUR) as *mut i32) = hour };
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Retained {
    entries: Vec<Entry>,
    entries_pointer: usize,
    entries_capacity: usize,
    births_capacity: usize,
    next_id: u64,
    poisoned: bool,
}
fn retained() -> Retained {
    let mut current = store().try_lock().expect("foreign cleanup released STORE");
    current.fold();
    Retained {
        entries: current.entries.clone(),
        entries_pointer: current.entries.as_ptr() as usize,
        entries_capacity: current.entries.capacity(),
        births_capacity: current.births.capacity(),
        next_id: current.next_id,
        poisoned: current.poisoned,
    }
}
fn registered(pc: usize) {
    let mut image_base = 0;
    assert!(
        !unsafe { RtlLookupFunctionEntry(pc as u64, &mut image_base, std::ptr::null_mut()) }
            .is_null()
    );
}
fn normal_return(call: &NativeCall, count: usize) {
    assert_eq!(call.observed[0], call.arguments[0]);
    if call.site < 2 {
        assert_eq!(call.observed, call.arguments);
    }
    assert_eq!(
        (call.entered, call.returned, call.continued),
        (count, count, count)
    );
    assert_eq!(call.result, call.arguments[0]);
    assert_eq!(call.completed_rsp, call.caller_rsp);
    assert_eq!(call.rsp & 15, 8);
    assert_eq!(active(), 0);
}

#[test]
#[ignore = "parent-only native C++/SEH dispatch; requires BATTLEPLAN_TAP_UNWIND_PROBE"]
fn naval_original_cpp_seh_preserves_cleanup_storage_and_restores_hooks() {
    let _serial = probe::SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _observation = Observation::new();
    let driver = probe::Probe::load();
    let [normal, cpp, seh] = driver.cases();
    let caller_site = naval_foreign_site as *const () as usize;
    for site_index in 0..3 {
        for case in [cpp, seh] {
            assert_eq!(active(), 0, "a stranded writer must retain its storage");
            assert!(SITE_ENABLED.iter().all(|site| !site.load(Ordering::SeqCst)));
            let world = World::new();
            let base = if site_index == 2 {
                caller_site + 5 - 0xc0dc19
            } else {
                world.base()
            };
            setup(2, base);
            boundary(clock(10, 1));
            boundary(clock(11, 1));
            let mut call = Box::new(NativeCall {
                arguments: [
                    if site_index == 2 {
                        world.combat()
                    } else {
                        world.result()
                    },
                    world.combat(),
                ],
                site: site_index,
                hooked: true,
                ..Default::default()
            });
            let argument = &mut *call as *mut NativeCall as usize;
            CURRENT.store(argument, Ordering::Release);
            let target = if site_index == 2 {
                naval_foreign_original as *const () as usize
            } else {
                caller_site
            };
            let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(target as *const u8, 16) }
                .try_into()
                .unwrap();
            let callback = [
                fill as *const () as usize,
                merge as *const () as usize,
                constructor_entry as *const () as usize,
            ][site_index];
            let mut hook = if site_index == 2 {
                unsafe { Hook::new(target, callback, sentinel) }.unwrap()
            } else {
                unsafe {
                    Hook::new_call_span(
                        target,
                        callback,
                        sentinel,
                        if site_index == 0 { 16 } else { 13 },
                    )
                }
                .unwrap()
            };
            ORIGINAL[site_index].store(
                if site_index == 2 {
                    hook.trampoline()
                } else {
                    naval_foreign_original as *const () as usize
                },
                Ordering::Release,
            );
            let ranges = hook.executable_ranges();
            let owned = ranges
                .clone()
                .map(|range| unsafe { std::ptr::read_unaligned(range.start as *const [u8; 16]) });
            assert!(hook.set_enabled(true).unwrap());
            SITE_ENABLED[site_index].store(true, Ordering::SeqCst);
            for function in [
                naval_foreign_caller as *const () as usize,
                naval_foreign_original as *const () as usize,
                constructor as *const () as usize,
                result_call as *const () as usize,
            ] {
                registered(function);
            }
            let calls = CALLS[site_index].load(Ordering::Relaxed);
            unsafe { driver.run(normal, naval_foreign_caller, argument) };
            normal_return(&call, 1);
            assert!(!retained().poisoned);
            assert!(call.rsp < call.caller_rsp);
            assert_ne!(call.return_pc, caller_site + 5);
            registered(call.return_pc);
            let original_state = (call.caller_rsp - call.rsp, call.return_pc);
            let mut rows = vec![Entry::default(); 2];
            assert_eq!(drain(&mut rows).unwrap(), usize::from(site_index < 2));
            if site_index < 2 {
                assert_eq!(
                    (
                        rows[0].game_hour,
                        rows[0].result,
                        rows[0].combat,
                        rows[0].merged
                    ),
                    (
                        11,
                        world.result[cnaval_combat_results::HANDLE / 8],
                        world.combat[cnaval_combat::HANDLE / 8],
                        site_index == 1,
                    )
                );
            }
            unsafe { driver.run(normal, naval_foreign_caller, argument) };
            normal_return(&call, 2);
            let storage = retained();
            assert_eq!(storage.entries.len(), usize::from(site_index < 2));
            let before = UNWINDS.load(Ordering::Relaxed);
            call.raise = case.raise;
            eprintln!(
                "Naval {site_index} {} caller={:#x} site={target:#x} bridge={callback:#x} original={:#x} original-PC={:#x} original-RSP={:#x} ranges={ranges:x?}",
                case.name,
                naval_foreign_caller as *const () as usize,
                naval_foreign_original as *const () as usize,
                call.return_pc,
                call.rsp
            );
            unsafe { driver.run(case, naval_foreign_caller, argument) };
            assert_eq!(call.observed[0], call.arguments[0]);
            if site_index < 2 {
                assert_eq!(call.observed, call.arguments);
            }
            assert!(call.rsp < call.caller_rsp);
            assert_eq!(call.rsp & 15, 8);
            assert_eq!((call.caller_rsp - call.rsp, call.return_pc), original_state);
            assert_eq!((call.entered, call.returned, call.continued), (3, 2, 2));
            assert_eq!(UNWINDS.load(Ordering::Relaxed), before + 1);
            assert_eq!(
                active(),
                0,
                "foreign cleanup must retire the actual Call guard"
            );
            assert_eq!(
                drain(&mut rows).unwrap_err(),
                "naval result sources native writer unwound"
            );
            let storage = Retained {
                poisoned: true,
                ..storage
            };
            assert_eq!(retained(), storage);
            ENABLED.store(false, Ordering::SeqCst);
            assert_eq!(
                release(),
                Err("naval result source release before callback quiescence")
            );
            assert_eq!(retained(), storage);
            ENABLED.store(true, Ordering::SeqCst);
            call.raise = None;
            unsafe { driver.run(normal, naval_foreign_caller, argument) };
            assert_eq!((call.entered, call.returned, call.continued), (4, 3, 3));
            assert_eq!(call.result, call.arguments[0]);
            assert_eq!(active(), 0);
            assert_eq!(CALLS[site_index].load(Ordering::Relaxed), calls + 4);
            assert_eq!(UNWINDS.load(Ordering::Relaxed), before + 1);
            assert_eq!(
                retained(),
                storage,
                "later native return cannot repair the failed observation"
            );
            assert!(hook.set_enabled(false).unwrap());
            SITE_ENABLED[site_index].store(false, Ordering::SeqCst);
            ENABLED.store(false, Ordering::SeqCst);
            assert_eq!(
                unsafe { std::slice::from_raw_parts(target as *const u8, 16) },
                sentinel
            );
            assert_eq!(hook.executable_ranges(), ranges);
            assert_eq!(
                ranges.clone().map(|range| unsafe {
                    std::ptr::read_unaligned(range.start as *const [u8; 16])
                }),
                owned
            );
            call.hooked = false;
            unsafe { driver.run(normal, naval_foreign_caller, argument) };
            assert_eq!((call.entered, call.returned, call.continued), (5, 4, 4));
            assert_eq!(call.result, call.arguments[0]);
            assert_eq!(call.rsp, call.caller_rsp - 8);
            assert_eq!(call.return_pc, caller_site + 5);
            assert_eq!(call.completed_rsp, call.caller_rsp);
            assert_eq!(CALLS[site_index].load(Ordering::Relaxed), calls + 4);
            assert_eq!(retained(), storage);
            release().unwrap();
            assert_eq!(store().lock().unwrap().entries.capacity(), 0);
            assert!(LOG.is_empty());
            drop(hook);
            CURRENT.store(0, Ordering::Release);
            eprintln!("Naval {site_index} {} PASS", case.name);
        }
    }
}
