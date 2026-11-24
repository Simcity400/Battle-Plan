use super::*;
use crate::foreign_unwind_probe as probe;
use windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry;

#[repr(C)]
struct Side {
    leader: usize,
    calls: usize,
    returned: usize,
}
struct Invocation {
    side: Side,
    vtable: [usize; 30],
    nonvolatile: [usize; 8],
    result: usize,
    continued: usize,
}
static RAISE: AtomicUsize = AtomicUsize::new(0);
core::arch::global_asm!(
    ".text", ".globl land_foreign_commander", ".seh_proc land_foreign_commander",
    "land_foreign_commander:", "push rbx", ".seh_pushreg rbx",
    "sub rsp, 0x20", ".seh_stackalloc 0x20", ".seh_endprologue",
    "mov rbx, rcx", "inc qword ptr [rbx + 8]", "call {retained}",
    "mov rax, qword ptr [rip + {raise}]", "test rax, rax", "jz 2f", "call rax",
    "2:", "inc qword ptr [rbx + 16]", "mov rax, [rbx]",
    "add rsp, 0x20", "pop rbx", "ret", ".seh_endproc",
    retained = sym retain_log, raise = sym RAISE,
);
unsafe extern "system-unwind" {
    fn land_foreign_commander(side: usize) -> usize;
}
unsafe extern "system" fn retain_log() {
    assert_eq!(LIVE.active.load(Ordering::SeqCst), 1);
    assert_eq!(
        TAP_IN_FLIGHT.load(Ordering::SeqCst),
        TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst) as usize + 1
    );
    let storage = LIVE.log.load(Ordering::SeqCst);
    let enabled = LIVE.enabled.swap(false, Ordering::SeqCst);
    let retained = unsafe { release() };
    LIVE.enabled.store(enabled, Ordering::SeqCst);
    assert_eq!(
        retained,
        Err("land event storage release before callback quiescence")
    );
    assert_eq!(LIVE.log.load(Ordering::SeqCst), storage);
}
unsafe extern "system-unwind" fn invoke(argument: usize) {
    let invocation = unsafe { &mut *(argument as *mut Invocation) };
    invocation.result = unsafe {
        tests::land_leader_tap_test_caller(
            std::ptr::addr_of_mut!(invocation.side) as usize,
            invocation.vtable.as_ptr() as usize,
            invocation.nonvolatile.as_mut_ptr(),
        )
    };
    invocation.continued += 1;
}
fn registered(address: usize) -> bool {
    let mut image = 0;
    !unsafe { RtlLookupFunctionEntry(address as u64, &raw mut image, ptr::null_mut()) }.is_null()
}

#[test]
#[ignore = "parent-only real C++/SEH run; requires BATTLEPLAN_TAP_UNWIND_PROBE and reviewed shared Hook"]
fn production_verdict_indirect_foreign_unwind() {
    let _probe_guard = probe::SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut owned = test_support::own();
    // SAFETY: Ignored probe test holds `test_support::own()` before installing its test hook.
    unsafe { test_support::reset(&mut owned) };
    let probe = probe::Probe::load();
    let site = tests::land_leader_tap_test_site as *const () as usize;
    let base = site + 6 - native::DISPATCH_LEADER_RETURNS[0].0;
    let previous_base = BASE.swap(base, Ordering::SeqCst);
    let previous_dispatch = DISPATCH.get();
    let storage = LIVE.log.load(Ordering::SeqCst);
    let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    assert_eq!(sentinel[..6], native::DISPATCH_LEADER_CALL_SENTINEL);
    let body: unsafe extern "system-unwind" fn(usize, usize, usize) -> usize = leader_tap_body;
    assert!(registered(site) && registered(leader_tap_entry as *const () as usize));
    assert!(registered(body as *const () as usize));
    let mut hook = unsafe {
        Hook::new_indirect_call(
            site,
            leader_tap_entry as *const () as usize,
            sentinel,
            tap_safety(),
        )
    }
    .unwrap();
    let mut leader = vec![0u64; (engine_native::cunit_leader::TYPE + 8).div_ceil(8)];
    leader[0] = (base + engine_native::carmy_leader::VTABLE_RVA.0) as u64;
    let reference = (0x7a11_u64 << 32) | u64::from(engine_native::cunit_leader::REFERENCE_TYPE);
    leader[engine_native::cunit_leader::TYPE / 8] = reference;
    println!(
        "LAND indirect site={site:#x} stub={:#x} body={:#x} ranges={:?}",
        leader_tap_entry as *const () as usize,
        body as *const () as usize,
        hook.executable_ranges()
    );
    let escaped = TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst);
    assert_eq!(escaped, 0, "fresh isolated diagnostic process");
    assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
    for case in probe.cases() {
        let mut invocation = Invocation {
            side: Side {
                leader: leader.as_ptr() as usize,
                calls: 0,
                returned: 0,
            },
            vtable: [0; 30],
            nonvolatile: [0; 8],
            result: 0,
            continued: 0,
        };
        invocation.vtable[29] = land_foreign_commander as *const () as usize;
        RAISE.store(
            case.raise.map_or(0, |raise| raise as *const () as usize),
            Ordering::SeqCst,
        );
        LIVE.enabled.store(true, Ordering::SeqCst);
        let side = std::ptr::addr_of_mut!(invocation.side) as usize;
        DISPATCH.set(Dispatch {
            combat: 0x7a11,
            sides: [side + 8, side],
            winner: 1,
            ..NO_DISPATCH
        });
        assert!(Hook::set_enabled_batch(&mut [&mut hook], true).unwrap());
        unsafe { probe.run(case, invoke, std::ptr::addr_of_mut!(invocation) as usize) };
        let normal = usize::from(case.expected == 1);
        assert_eq!(
            (
                invocation.side.calls,
                invocation.side.returned,
                invocation.continued
            ),
            (1, normal, normal)
        );
        assert_eq!(
            (
                LIVE.active.load(Ordering::SeqCst),
                TAP_IN_FLIGHT.load(Ordering::SeqCst)
            ),
            (1 - normal, 1 - normal)
        );
        assert_eq!(LIVE.log.load(Ordering::SeqCst), storage);
        assert_eq!(DISPATCH.get().seen, [false, normal != 0]);
        if normal != 0 {
            assert_eq!(invocation.result, leader.as_ptr() as usize);
            assert_eq!(DISPATCH.get().leaders[1], reference);
            assert_eq!(
                invocation.nonvolatile,
                [
                    0x1101, 0x2202, 0x3303, 0x4404, 0x5505, 0x6606, 0x7707, 0x8808
                ]
            );
        }
        if normal == 0 {
            assert_eq!(TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst), escaped + 1);
            assert!(!observing());
            assert!(active() != 0);
            assert!(
                !Hook::set_enabled_batch(&mut [&mut hook], false).unwrap(),
                "unwind flight retains patch"
            );
            assert_eq!(unsafe { *(site as *const u8) }, 0xe8);
            assert!(unsafe { release() }.is_err());
            let mut output = vec![0; 16];
            for (hour, load) in [(10, false), (11, true), (12, false)] {
                let refused = drain(&mut output, hour, load);
                assert!(!refused.complete);
                assert_eq!(refused.reason, TAP_UNWIND_RETENTION);
            }
            RAISE.store(0, Ordering::SeqCst);
            unsafe {
                probe.run(
                    probe.cases()[0],
                    invoke,
                    std::ptr::addr_of_mut!(invocation) as usize,
                )
            };
            assert_eq!(
                (
                    invocation.side.calls,
                    invocation.side.returned,
                    invocation.continued
                ),
                (2, 1, 1)
            );
            assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 1);
            assert_eq!(DISPATCH.get().seen, [false; 2]);
            assert!(!observing());
            assert_eq!(LIVE.active.load(Ordering::SeqCst), 1);
            LIVE.active.store(0, Ordering::SeqCst);
            LIVE.call_escaped_unwinds.store(0, Ordering::SeqCst);
            TAP_IN_FLIGHT.store(0, Ordering::SeqCst);
            TAP_ESCAPED_UNWINDS.store(escaped, Ordering::SeqCst);
        }
        assert!(Hook::set_enabled_batch(&mut [&mut hook], false).unwrap());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        if normal == 0 {
            invocation.side.calls = 0;
            invocation.side.returned = 0;
            invocation.continued = 0;
            LIVE.enabled.store(true, Ordering::SeqCst);
            DISPATCH.set(Dispatch {
                combat: 0x7a11,
                sides: [side + 8, side],
                winner: 1,
                ..NO_DISPATCH
            });
            assert!(Hook::set_enabled_batch(&mut [&mut hook], true).unwrap());
            unsafe {
                probe.run(
                    probe.cases()[0],
                    invoke,
                    std::ptr::addr_of_mut!(invocation) as usize,
                )
            };
            assert_eq!(
                (
                    invocation.side.calls,
                    invocation.side.returned,
                    invocation.continued
                ),
                (1, 1, 1)
            );
            assert_eq!(DISPATCH.get().leaders[1], reference);
            assert_eq!(
                invocation.nonvolatile,
                [
                    0x1101, 0x2202, 0x3303, 0x4404, 0x5505, 0x6606, 0x7707, 0x8808
                ]
            );
            assert_eq!(
                (
                    LIVE.active.load(Ordering::SeqCst),
                    TAP_IN_FLIGHT.load(Ordering::SeqCst)
                ),
                (0, 0)
            );
            assert!(Hook::set_enabled_batch(&mut [&mut hook], false).unwrap());
            assert_eq!(
                unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
                sentinel
            );
        }
        println!(
            "PASS LAND indirect {}: native handler, retained unwind flight/log, fixture restore and normal continuation",
            case.name
        );
    }
    drop(hook);
    BASE.store(previous_base, Ordering::SeqCst);
    DISPATCH.set(previous_dispatch);
    test_support::disable();
    unsafe { release() }.unwrap();
    assert!(LIVE.log.load(Ordering::SeqCst).is_null());
}
