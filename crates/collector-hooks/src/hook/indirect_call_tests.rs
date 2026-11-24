use super::*;
use std::sync::atomic::AtomicBool;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static ENTERED: AtomicBool = AtomicBool::new(false);
static RELEASE: AtomicBool = AtomicBool::new(false);
static CALLER: AtomicUsize = AtomicUsize::new(0);

core::arch::global_asm!(
    ".text", ".globl hook_indirect_caller", ".seh_proc hook_indirect_caller",
    "hook_indirect_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "mov rax, rdx", ".globl hook_indirect_site", "hook_indirect_site:",
    ".byte 0xff, 0x90", ".long 8", ".rept 10", "nop", ".endr",
    "add rsp, 0x28", "ret", ".seh_endproc",
    ".globl hook_indirect_stub", ".seh_proc hook_indirect_stub",
    "hook_indirect_stub:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "mov r8, [rax + 8]", "lock inc qword ptr [rip + {count}]",
    "mov rdx, [rsp + 0x28]", "lea rdx, [rdx + 1]", "call {body}",
    "lock dec qword ptr [rip + {count}]", "add rsp, 0x28", "ret", ".seh_endproc",
    ".globl hook_indirect_stub_end", "hook_indirect_stub_end:",
    ".globl hook_indirect_ip_stub", ".seh_proc hook_indirect_ip_stub",
    "hook_indirect_ip_stub:", ".seh_endprologue",
    "mov byte ptr [rip + {entered}], 1", ".Lhook_indirect_wait:",
    "cmp byte ptr [rip + {release}], 0", "je .Lhook_indirect_wait", "ret", ".seh_endproc",
    ".globl hook_indirect_ip_stub_end", "hook_indirect_ip_stub_end:",
    count = sym IN_FLIGHT, body = sym forwarded,
    entered = sym ENTERED, release = sym RELEASE,
);
unsafe extern "system-unwind" {
    fn hook_indirect_caller(value: usize, vtable: *const usize) -> usize;
    fn hook_indirect_site();
    fn hook_indirect_stub();
    fn hook_indirect_stub_end();
    fn hook_indirect_ip_stub();
    fn hook_indirect_ip_stub_end();
}
unsafe extern "system-unwind" fn forwarded(value: usize, caller: usize, target: usize) -> usize {
    CALLER.store(caller, Ordering::SeqCst);
    let target: unsafe extern "system-unwind" fn(usize) -> usize =
        unsafe { std::mem::transmute(target) };
    unsafe { target(value) }
}
unsafe extern "system-unwind" fn target(value: usize) -> usize {
    value.wrapping_add(1)
}
unsafe extern "system-unwind" fn other_target(value: usize) -> usize {
    value.rotate_left(17)
}
unsafe extern "system-unwind" fn waiting_target(value: usize) -> usize {
    ENTERED.store(true, Ordering::Release);
    while !RELEASE.load(Ordering::Acquire) {
        std::hint::spin_loop();
    }
    value.wrapping_add(1)
}
fn safety() -> TapSafety {
    TapSafety {
        in_flight: &IN_FLIGHT,
        stub: hook_indirect_stub as *const () as usize
            ..hook_indirect_stub_end as *const () as usize,
    }
}
fn snapshot(site: usize) -> [u8; 16] {
    unsafe { ptr::read_unaligned(site as *const [u8; 16]) }
}
fn synthetic_site(code: &TestCode, offset: usize, indirect: bool) -> (usize, [u8; 16]) {
    let site = code.0 as usize + offset;
    let mut bytes = [0x90; 16];
    if indirect {
        bytes[..6].copy_from_slice(&[0xff, 0x90, 8, 0, 0, 0]);
    } else {
        bytes[..5].copy_from_slice(&[0xe8, 123, 0, 0, 0]);
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), site as *mut u8, bytes.len());
    }
    (site, bytes)
}

#[test]
fn call_spans_validate_boundaries_and_ignore_the_unowned_neighbour() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = synthetic_site(&code, 0, false);
    let callback = target as *const () as usize;
    for length in [0, 4, 17, usize::MAX] {
        assert!(unsafe { Hook::new_call_span(site, callback, bytes, length) }.is_err());
    }
    assert!(unsafe { Hook::new_call_span(usize::MAX - 4, callback, bytes, 5) }.is_err());
    for length in [5, 13, 16] {
        let mut hook = unsafe { Hook::new_call_span(site, callback, bytes, length) }.unwrap();
        assert!(hook.set_enabled(true).unwrap());
        if length < 16 {
            unsafe {
                ptr::write_bytes((site + length) as *mut u8, 0xcc, 16 - length);
            }
        }
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(&snapshot(site)[..length], &bytes[..length]);
        assert_eq!(&snapshot(site)[length..], &vec![0xcc; 16 - length]);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), site as *mut u8, 16);
        }
    }
    let (site, bytes) = synthetic_site(&code, 256, true);
    assert!(unsafe { Hook::new_indirect_call(site, callback, [0x90; 16], safety()) }.is_err());
    assert!(
        unsafe {
            Hook::new_indirect_call(
                site,
                callback,
                bytes,
                TapSafety {
                    in_flight: &IN_FLIGHT,
                    stub: site..site,
                },
            )
        }
        .is_err()
    );
    assert!(unsafe { Hook::new_indirect_call(site, callback, bytes, safety()) }.is_err());
}

#[test]
fn nine_member_family_is_atomic_and_partial_ownership_never_claims_foreign_bytes() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut hooks = Vec::new();
    for index in 0..9 {
        let (site, bytes) = if index < 7 {
            code.stub(index * 64, target as *const () as usize)
        } else {
            synthetic_site(&code, index * 64, true)
        };
        hooks.push(if index < 7 {
            unsafe { Hook::new(site, target as *const () as usize, bytes) }.unwrap()
        } else {
            unsafe {
                Hook::new_indirect_call(
                    site,
                    hook_indirect_stub as *const () as usize,
                    bytes,
                    safety(),
                )
            }
            .unwrap()
        });
    }
    let foreign = hooks[8].site + 15;
    unsafe {
        ptr::write(foreign as *mut u8, 0xcc);
    }
    assert_eq!(
        Hook::set_family_enabled(&mut hooks, true),
        Err(BYTE_VERIFICATION_FAILURE.into())
    );
    assert!(hooks.iter().all(|hook| !hook.is_enabled()));
    assert!(
        hooks
            .iter()
            .filter(|hook| hook.site + 15 != foreign)
            .all(|hook| hook.bytes_match(false))
    );
    for hook in &mut hooks[..3] {
        assert!(hook.set_enabled(true).unwrap());
    }
    assert_ne!(
        Hook::set_family_enabled(&mut hooks, true).unwrap_err(),
        BYTE_VERIFICATION_FAILURE
    );
    assert!(hooks[..3].iter().all(Hook::is_enabled));
    assert!(Hook::set_family_enabled(&mut hooks, false).unwrap());
    assert!(hooks.iter().all(|hook| !hook.is_enabled()));
    assert_eq!(unsafe { ptr::read(foreign as *const u8) }, 0xcc);
    unsafe {
        ptr::write(foreign as *mut u8, hooks[8].original[15]);
    }
    fail_flushes(2);
    assert!(Hook::set_family_enabled(&mut hooks, true).is_err());
    assert!(
        hooks
            .iter()
            .all(|hook| !hook.enabled && hook.is_enabled() && hook.bytes_match(false))
    );
    IN_FLIGHT.store(1, Ordering::SeqCst);
    let refused = Hook::set_family_enabled(&mut hooks, false);
    IN_FLIGHT.store(0, Ordering::SeqCst);
    assert_eq!(refused, Ok(false));
    assert!(hooks.iter().all(Hook::is_enabled));
    assert!(Hook::set_family_enabled(&mut hooks, false).unwrap());
    assert!(Hook::set_family_enabled(&mut hooks, true).unwrap());
    IN_FLIGHT.store(1, Ordering::SeqCst);
    let refused = Hook::set_family_enabled(&mut hooks, false);
    IN_FLIGHT.store(0, Ordering::SeqCst);
    assert_eq!(refused, Ok(false));
    assert!(
        hooks
            .iter()
            .all(|hook| hook.enabled && hook.bytes_match(true))
    );
    assert!(Hook::set_family_enabled(&mut hooks, false).unwrap());
    assert!(
        hooks
            .iter()
            .all(|hook| !hook.is_enabled() && snapshot(hook.site) == hook.original)
    );
}

#[test]
fn restored_verification_failure_retains_ownership_through_retry_and_rollback() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for rollback in [false, true] {
        let code = TestCode::new();
        let (site, bytes) = code.stub(0, target as *const () as usize);
        let mut hook = unsafe { Hook::new(site, target as *const () as usize, bytes) }.unwrap();
        if !rollback {
            assert!(hook.set_enabled(true).unwrap());
        }
        let mut attempts = 0;
        let result = Hook::set_family_enabled_with_flush(&mut [&mut hook], rollback, || {
            attempts += 1;
            if rollback && attempts == 1 {
                return Err(31);
            }
            unsafe {
                ptr::write((site + 8) as *mut u8, 0xcc);
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(!hook.enabled && hook.is_enabled());
        assert_ne!(
            hook.set_enabled(false).unwrap_err(),
            BYTE_VERIFICATION_FAILURE
        );
        assert!(hook.is_enabled());
        unsafe {
            ptr::write((site + 8) as *mut u8, bytes[8]);
        }
        assert!(hook.set_enabled(false).unwrap());
        assert!(!hook.is_enabled());
    }
}

#[test]
fn indirect_target_and_buried_return_survive_until_the_counted_call_finishes() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let site = hook_indirect_site as *const () as usize;
    let bytes = snapshot(site);
    let mut hook = unsafe {
        Hook::new_indirect_call(
            site,
            hook_indirect_stub as *const () as usize,
            bytes,
            safety(),
        )
    }
    .unwrap();
    assert!(hook.set_enabled(true).unwrap());
    assert_eq!(&snapshot(site)[..6], &hook.patch[..6]);
    assert_eq!(snapshot(site)[5], 0x90);
    for target in [
        target as *const () as usize,
        other_target as *const () as usize,
    ] {
        let native: unsafe extern "system-unwind" fn(usize) -> usize =
            unsafe { std::mem::transmute(target) };
        assert_eq!(
            unsafe { hook_indirect_caller(usize::MAX, [0, target].as_ptr()) },
            unsafe { native(usize::MAX) }
        );
        assert_eq!(CALLER.load(Ordering::SeqCst), site + 6);
        assert_eq!(IN_FLIGHT.load(Ordering::SeqCst), 0);
    }
    ENTERED.store(false, Ordering::SeqCst);
    RELEASE.store(false, Ordering::SeqCst);
    let worker = std::thread::spawn(|| unsafe {
        hook_indirect_caller(41, [0, waiting_target as *const () as usize].as_ptr())
    });
    while !ENTERED.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    let refused = hook.set_enabled(false);
    let retained = hook.is_enabled() && hook.bytes_match(true);
    RELEASE.store(true, Ordering::Release);
    let result = worker.join().unwrap();
    assert_eq!(refused, Ok(false));
    assert!(retained);
    assert_eq!(result, 42);
    assert_eq!(IN_FLIGHT.load(Ordering::SeqCst), 0);
    assert!(hook.set_enabled(false).unwrap());
    assert_eq!(snapshot(site), bytes);
}

#[test]
fn relay_and_stub_ips_refuse_restoration_even_before_the_counter_increments() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = synthetic_site(&code, 0, true);
    let stub = hook_indirect_ip_stub as *const () as usize;
    let mut hook = unsafe {
        Hook::new_indirect_call(
            site,
            stub,
            bytes,
            TapSafety {
                in_flight: &IN_FLIGHT,
                stub: stub..hook_indirect_ip_stub_end as *const () as usize,
            },
        )
    }
    .unwrap();
    let relay_loop = hook.executable_ranges()[0].start + 32;
    let mut loop_bytes = [0; 29];
    loop_bytes[..2].copy_from_slice(&[0x48, 0xb8]);
    loop_bytes[2..10].copy_from_slice(&((&raw const ENTERED as usize) as u64).to_le_bytes());
    loop_bytes[10..15].copy_from_slice(&[0xc6, 0, 1, 0x48, 0xb8]);
    loop_bytes[15..23].copy_from_slice(&((&raw const RELEASE as usize) as u64).to_le_bytes());
    loop_bytes[23..].copy_from_slice(&[0x80, 0x38, 0, 0x74, 0xfb, 0xc3]);
    let mut previous = 0;
    assert_ne!(
        unsafe {
            VirtualProtect(
                relay_loop as *const _,
                loop_bytes.len(),
                PAGE_READWRITE,
                &raw mut previous,
            )
        },
        0
    );
    unsafe {
        ptr::copy_nonoverlapping(loop_bytes.as_ptr(), relay_loop as *mut u8, loop_bytes.len());
    }
    let mut ignored = 0;
    assert_ne!(
        unsafe {
            VirtualProtect(
                relay_loop as *const _,
                loop_bytes.len(),
                previous,
                &raw mut ignored,
            )
        },
        0
    );
    assert_ne!(
        unsafe {
            FlushInstructionCache(
                GetCurrentProcess(),
                relay_loop as *const _,
                loop_bytes.len(),
            )
        },
        0
    );
    for address in [relay_loop, stub] {
        ENTERED.store(false, Ordering::SeqCst);
        RELEASE.store(false, Ordering::SeqCst);
        assert!(hook.set_enabled(true).unwrap());
        let worker = std::thread::spawn(move || {
            let wait: unsafe extern "system-unwind" fn() = unsafe { std::mem::transmute(address) };
            unsafe {
                wait();
            }
        });
        while !ENTERED.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        let refused = hook.set_enabled(false);
        let retained = hook.is_enabled() && hook.bytes_match(true);
        RELEASE.store(true, Ordering::Release);
        worker.join().unwrap();
        assert_eq!(IN_FLIGHT.load(Ordering::SeqCst), 0);
        assert_eq!(refused, Ok(false));
        assert!(retained);
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(snapshot(site), bytes);
    }
}
