use super::*;
unsafe extern "system" fn original() -> usize {
    7
}
unsafe extern "system" fn callback() -> usize {
    19
}
unsafe fn invoke(address: usize) -> usize {
    let function: unsafe extern "system" fn() -> usize = unsafe { std::mem::transmute(address) };
    unsafe { function() }
}

#[test]
fn adjacent_call_prefixes_publish_recheck_and_restore_together() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for spacing in [7, 10] {
        let code = TestCode::new();
        let base = code.0 as usize;
        let mut bytes = [0x90_u8; 64];
        for offset in [0, spacing] {
            bytes[offset] = 0xe8;
            bytes[offset + 1..offset + 5]
                .copy_from_slice(&((48 - offset - 5) as i32).to_le_bytes());
        }
        bytes[48] = 0xc3;
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
        }
        let mut hooks: Vec<_> = [0, spacing]
            .into_iter()
            .map(|offset| {
                let original = bytes[offset..offset + 16].try_into().unwrap();
                unsafe {
                    Hook::new_call_span(
                        base + offset,
                        callback as *const () as usize,
                        original,
                        if offset == 0 { spacing } else { 16 },
                    )
                }
                .unwrap()
            })
            .collect();
        assert_eq!(Hook::set_family_enabled(&mut hooks, true), Ok(true));
        assert!(hooks.iter().all(|hook| hook.snapshot_failure().is_none()));
        assert_eq!(Hook::set_family_enabled(&mut hooks, true), Ok(true));
        assert_eq!(Hook::set_family_enabled(&mut hooks, false), Ok(true));
        assert_eq!(
            unsafe { std::slice::from_raw_parts(code.0.cast::<u8>(), bytes.len()) },
            bytes
        );
    }
}
#[test]
fn family_refuses_every_write_if_one_sentinel_changed_and_checks_retries() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let sites: Vec<_> = [0, 64]
        .into_iter()
        .map(|offset| code.stub(offset, original as *const () as usize))
        .collect();
    let mut hooks: Vec<_> = sites
        .iter()
        .map(|&(site, bytes)| {
            unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()
        })
        .collect();
    unsafe {
        ptr::write((sites[1].0 + 15) as *mut u8, 0xcc);
    }
    assert_eq!(
        Hook::set_family_enabled(&mut hooks, true),
        Err(BYTE_VERIFICATION_FAILURE.into())
    );
    assert!(hooks.iter().all(|hook| !hook.enabled));
    assert_eq!(unsafe { invoke(sites[0].0) }, 7);
    assert_eq!(Hook::set_family_enabled(&mut hooks, false), Ok(true));
    assert_eq!(unsafe { ptr::read((sites[1].0 + 15) as *const u8) }, 0xcc);
    unsafe {
        ptr::write((sites[1].0 + 15) as *mut u8, sites[1].1[15]);
    }
    assert_eq!(Hook::set_family_enabled(&mut hooks, true), Ok(true));
    for hook in &hooks {
        assert_eq!(unsafe { invoke(hook.site) }, 19);
        assert_eq!(unsafe { invoke(hook.trampoline()) }, 7);
    }
    assert_eq!(Hook::set_family_enabled(&mut hooks, true), Ok(true));
    assert_eq!(Hook::set_family_enabled(&mut hooks, false), Ok(true));
    for &(site, bytes) in &sites {
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            bytes
        );
        assert_eq!(unsafe { invoke(site) }, 7);
    }
}
#[test]
fn enabled_drop_retains_relay_and_relocation_without_panicking() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, original as *const () as usize);
    let mut hook = unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap();
    assert_eq!(hook.set_enabled(true), Ok(true));
    let trampoline = hook.trampoline();
    drop(hook);
    assert_eq!(unsafe { invoke(site) }, 19);
    assert_eq!(unsafe { invoke(trampoline) }, 7);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), site as *mut u8, 16);
    }
    assert_ne!(
        unsafe { FlushInstructionCache(GetCurrentProcess(), site as *const c_void, 16) },
        0
    );
}

#[test]
fn failed_rollback_flush_retains_code_until_detach_republishes_originals() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, original as *const () as usize);
    let mut hook = unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap();
    fail_flushes(2);
    let error = hook.set_enabled(true).unwrap_err();
    assert!(error.contains("restored bytes remain unflushed"));
    assert!(!hook.enabled);
    assert!(hook.is_enabled());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        bytes
    );
    assert_eq!(hook.set_enabled(false), Ok(true));
    assert!(!hook.is_enabled());
    assert_eq!(unsafe { invoke(site) }, 7);
}

#[test]
fn disabling_flush_failure_rolls_back_to_owned_patch_and_can_retry() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, original as *const () as usize);
    let mut hook = unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap();
    assert_eq!(hook.set_enabled(true), Ok(true));
    fail_flushes(1);
    assert!(hook.set_enabled(false).is_err());
    assert!(hook.is_enabled());
    assert_eq!(unsafe { invoke(site) }, 19);
    assert_eq!(hook.set_enabled(false), Ok(true));
    assert!(!hook.is_enabled());
    assert_eq!(unsafe { invoke(site) }, 7);
}

#[test]
fn installation_keeps_the_first_native_rollback_failure_after_successful_disable() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, original as *const () as usize);
    let mut hook = unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap();
    let (_, report) = installation::capture(|| {
        fail_flushes(2);
        assert!(hook.set_enabled(true).is_err());
        assert_eq!(hook.set_enabled(false), Ok(true));
    });
    let failure = report.failure.unwrap();
    assert_eq!(failure.kind, installation::Kind::Rollback);
    assert_eq!(failure.site, Some(site));
    assert!(!hook.is_enabled());
    assert_eq!(unsafe { invoke(site) }, 7);
}

#[test]
fn owned_family_restores_after_capture_stop_without_clearing_it() {
    const CHILD: &str = "BATTLEPLAN_HOOK_RESTORE_AFTER_CAPTURE_STOP";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hook::tests::owned_family_restores_after_capture_stop_without_clearing_it",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "restore child failed: {:?}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let sites: Vec<_> = [0, 64]
        .into_iter()
        .map(|offset| code.stub(offset, original as *const () as usize))
        .collect();
    let mut hooks: Vec<_> = sites
        .iter()
        .map(|&(site, bytes)| {
            unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()
        })
        .collect();
    assert_eq!(Hook::set_family_enabled(&mut hooks, true), Ok(true));
    engine_allocation::set_stop_hook(Some(|| true));
    assert!(engine_allocation::capture_stopped());
    assert_eq!(
        engine_allocation::vec::<u8>(1).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert!(
        Hook::set_family_enabled(&mut hooks, true).is_err(),
        "stopped capture cannot enable"
    );
    assert!(
        publication_storage::<&mut Hook>(hooks.len(), true).is_err(),
        "stopped capture cannot prepare publication adapters"
    );
    let mut threads = Threads::open_excluding(&[unsafe { GetCurrentThreadId() }]).unwrap();
    threads.suspend().unwrap();
    threads.resume().unwrap();
    drop(threads);
    let mut family = publication_storage(hooks.len(), false).unwrap();
    family.extend(hooks.iter_mut());
    assert_eq!(Hook::set_enabled_batch(&mut family, false), Ok(true));
    drop(family);
    assert!(
        engine_allocation::capture_stopped(),
        "restore must not clear the terminal gate"
    );
    assert!(hooks.iter().all(|hook| !hook.is_enabled()));
    for &(site, bytes) in &sites {
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            bytes
        );
        assert_eq!(unsafe { invoke(site) }, 7);
    }
}

#[test]
fn publication_holds_the_process_lock_through_its_writes() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, original as *const () as usize);
    let mut hook = unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap();
    let mut held = Vec::new();
    for enabled in [true, false] {
        assert_eq!(
            Hook::set_family_enabled_with_flush(&mut [&mut hook], enabled, || {
                held.push(PUBLICATION.try_lock().is_err());
                if unsafe { FlushInstructionCache(GetCurrentProcess(), ptr::null(), 0) } == 0 {
                    Err(unsafe { GetLastError() })
                } else {
                    Ok(())
                }
            }),
            Ok(true)
        );
    }
    assert_eq!(held, [true, true]);
    assert!(PUBLICATION.try_lock().is_ok());
    assert_eq!(unsafe { invoke(site) }, 7);
}

#[test]
fn a_reused_thread_id_of_another_process_is_not_opened() {
    let mut child = std::process::Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut foreign = None;
    for _ in 0..200 {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        assert_ne!(snapshot, INVALID_HANDLE_VALUE);
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut found = unsafe { Thread32First(snapshot, &raw mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == pid {
                foreign = Some(entry.th32ThreadID);
                break;
            }
            found = unsafe { Thread32Next(snapshot, &raw mut entry) };
        }
        unsafe {
            CloseHandle(snapshot);
        }
        if foreign.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let foreign = foreign.expect("the child process has a thread");
    let opened = open_thread(foreign);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(opened, Ok(None));
    let own = std::thread::spawn(|| unsafe { GetCurrentThreadId() });
    let id = unsafe { GetCurrentThreadId() };
    drop(own.join());
    let handle = open_thread(id).unwrap().expect("an own thread opens");
    unsafe {
        CloseHandle(handle);
    }
}

#[test]
fn a_terminating_thread_is_left_unheld_and_other_refusals_still_fail() {
    assert_eq!(refused_suspension(ERROR_ACCESS_DENIED, false), Ok(false));
    assert_eq!(refused_suspension(ERROR_INVALID_PARAMETER, true), Ok(false));
    assert_eq!(
        refused_suspension(ERROR_INVALID_PARAMETER, false),
        Err(ERROR_INVALID_PARAMETER)
    );
}

#[test]
fn a_thread_at_any_site_holds_its_whole_family_under_one_suspension() {
    assert_busy_family_publication(true);
}

#[test]
fn untracked_family_keeps_markers_untouched_and_uses_only_one_scan() {
    assert_busy_family_publication(false);
}

fn assert_busy_family_publication(track_busy: bool) {
    use std::os::windows::io::AsRawHandle;
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (first, _) = code.stub(0, original as *const () as usize);
    let held = code.0 as usize + 64;
    let (_, held_bytes) = code.stub(64, held);
    let (other, _) = code.stub(128, original as *const () as usize);
    let new = |(site, bytes): (usize, [u8; 16])| {
        unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()
    };
    let mut family = vec![new((first, code_bytes(first))), new((held, held_bytes))];
    family[0]
        .publication_busy
        .store(!track_busy, Ordering::Relaxed);
    let publish = |family: &mut [Hook], enabled| {
        if track_busy {
            Hook::set_enabled_batch_tracking_busy(
                &mut family.iter_mut().collect::<Vec<_>>(),
                enabled,
            )
        } else {
            Hook::set_family_enabled(family, enabled)
        }
    };
    let mut other_family = vec![new((other, code_bytes(other)))];
    let worker = std::thread::spawn(move || unsafe { invoke(held) });
    let handle = worker.as_raw_handle() as HANDLE;
    let started = std::time::Instant::now();
    loop {
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_ne!(unsafe { SuspendThread(handle) }, u32::MAX);
        let mut context = AlignedContext(unsafe { std::mem::zeroed() });
        context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
        assert_ne!(unsafe { GetThreadContext(handle, &raw mut context.0) }, 0);
        assert_ne!(unsafe { ResumeThread(handle) }, u32::MAX);
        if (held..held + 16).contains(&(context.0.Rip as usize)) {
            break;
        }
        std::thread::yield_now();
    }
    let suspensions = || SUSPENSIONS.with(std::cell::Cell::get);
    let before = suspensions();
    IP_SCANS.with(|count| count.set(0));
    assert_eq!(publish(&mut family, true), Ok(false));
    assert_eq!(
        IP_SCANS.with(std::cell::Cell::get),
        1,
        "one suspended-thread scan"
    );
    assert_eq!(family[0].publication_busy(), !track_busy);
    assert_eq!(family[1].publication_busy(), track_busy);
    assert_eq!(suspensions() - before, 1, "one suspension for the family");
    assert!(
        family.iter().all(|hook| !hook.is_enabled()),
        "no site written"
    );
    assert_eq!(unsafe { invoke(first) }, 7);
    assert_eq!(Hook::set_family_enabled(&mut other_family, true), Ok(true));
    assert_eq!(unsafe { invoke(other) }, 19);
    let target = held + 11;
    let flush = |address: usize| {
        assert_ne!(
            unsafe { FlushInstructionCache(GetCurrentProcess(), address as *const c_void, 8) },
            0
        );
    };
    unsafe { ptr::write_unaligned(target as *mut u64, original as *const () as u64) };
    flush(target);
    assert_eq!(worker.join().unwrap(), 7);
    unsafe { ptr::write_unaligned(target as *mut u64, held as u64) };
    flush(target);
    let before = suspensions();
    assert_eq!(publish(&mut family, true), Ok(true));
    assert_eq!(
        family[0].publication_busy(),
        !track_busy,
        "ordinary publication never resets diagnostic markers"
    );
    assert!(
        !family[1].publication_busy(),
        "tracked retries clear the previous collision"
    );
    assert_eq!(suspensions() - before, 1, "both sites under one suspension");
    assert_eq!(unsafe { invoke(first) }, 19);
    assert_eq!(unsafe { invoke(held) }, 19);
    for hooks in [&mut family, &mut other_family] {
        assert_eq!(Hook::set_family_enabled(hooks, false), Ok(true));
    }
    assert_eq!(code_bytes(held), held_bytes);
    assert_eq!(unsafe { invoke(first) }, 7);
}

fn code_bytes(site: usize) -> [u8; 16] {
    unsafe { ptr::read_unaligned(site as *const [u8; 16]) }
}

#[test]
fn a_busy_tap_names_only_its_site_and_clears_after_retry() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    static FLIGHTS: AtomicUsize = AtomicUsize::new(0);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, original as *const () as usize);
    let callback = callback as *const () as usize;
    let mut hook = unsafe {
        Hook::new_with_safety(
            site,
            callback,
            bytes,
            TapSafety {
                in_flight: &FLIGHTS,
                stub: callback..callback + 1,
            },
        )
    }
    .unwrap();
    FLIGHTS.store(1, Ordering::SeqCst);
    assert_eq!(
        Hook::set_enabled_batch_tracking_busy(&mut [&mut hook], true),
        Ok(false)
    );
    assert!(hook.publication_busy());
    assert!(!hook.is_enabled());
    FLIGHTS.store(0, Ordering::SeqCst);
    assert_eq!(
        Hook::set_enabled_batch_tracking_busy(&mut [&mut hook], true),
        Ok(true)
    );
    assert!(!hook.publication_busy());
    assert_eq!(
        Hook::set_enabled_batch_tracking_busy(&mut [&mut hook], false),
        Ok(true)
    );
}

#[test]
fn push_frame_trampoline_unwinds_every_instruction_boundary_and_restores_its_span() {
    use windows_sys::Win32::System::Diagnostics::Debug::{
        RtlLookupFunctionEntry, RtlVirtualUnwind,
    };
    #[repr(align(16))]
    struct Aligned(CONTEXT);
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes = [0x90; 32];
    bytes[..11].copy_from_slice(&[
        0x40, 0x53, 0x41, 0x57, 0x48, 0x83, 0xec, 0x58, 0x48, 0x8b, 0x02,
    ]);
    bytes[11..19].copy_from_slice(&[0x48, 0x83, 0xc4, 0x58, 0x41, 0x5f, 0x5b, 0xc3]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = bytes[..16].try_into().unwrap();
    let site = code.0 as usize;
    let mut hook =
        unsafe { Hook::new_push_frame(site, callback as *const () as usize, sentinel) }.unwrap();
    let trampoline = hook.trampoline();
    let mut stack = [0_u64; 32];
    let native_rsp = unsafe { stack.as_mut_ptr().add(24) } as usize;
    unsafe {
        ptr::write(native_rsp as *mut u64, 0x1234);
        ptr::write((native_rsp - 8) as *mut u64, 0x3333);
        ptr::write((native_rsp - 16) as *mut u64, 0xffff);
    }
    for (offset, allocated) in [(0, 0), (2, 8), (4, 16), (8, 104), (18, 104)] {
        let mut context = Aligned(CONTEXT {
            Rip: (trampoline + offset) as u64,
            Rsp: (native_rsp - allocated) as u64,
            Rbx: if offset >= 2 { 0xdead } else { 0x3333 },
            R15: if offset >= 4 { 0xbeef } else { 0xffff },
            ..Default::default()
        });
        let mut base = 0;
        let entry = unsafe { RtlLookupFunctionEntry(context.0.Rip, &raw mut base, ptr::null_mut()) };
        assert!(!entry.is_null());
        let mut data = ptr::null_mut();
        let mut frame = 0;
        unsafe {
            RtlVirtualUnwind(
                0,
                base,
                context.0.Rip,
                entry,
                &raw mut context.0,
                &raw mut data,
                &raw mut frame,
                ptr::null_mut(),
            );
        }
        assert_eq!(
            (context.0.Rip, context.0.Rsp, context.0.Rbx, context.0.R15),
            (0x1234, native_rsp as u64 + 8, 0x3333, 0xffff),
            "offset {offset}"
        );
    }
    let function: unsafe extern "system" fn(usize, *const u64) -> u64 =
        unsafe { std::mem::transmute(trampoline) };
    assert_eq!(unsafe { function(0, &73) }, 73);
    assert!(hook.set_enabled(true).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts((site + 5) as *const u8, 3) },
        [0x90; 3]
    );
    assert!(hook.set_enabled(false).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
    drop(hook);
    let mut base = 0;
    assert!(
        unsafe { RtlLookupFunctionEntry(trampoline as u64, &raw mut base, ptr::null_mut()) }.is_null()
    );
}

#[test]
fn push_frame_refuses_unknown_instructions_before_allocating_or_reading_the_site() {
    for (offset, replacement) in [(0, 0x41), (3, 0x56), (7, 0x80), (7, 0x57), (10, 0x01)] {
        let mut bytes = [0x90; 16];
        bytes[..11].copy_from_slice(&[
            0x40, 0x53, 0x41, 0x57, 0x48, 0x83, 0xec, 0x58, 0x48, 0x8b, 0x02,
        ]);
        bytes[offset] = replacement;
        let result = unsafe { Hook::new_push_frame(1, 2, bytes) };
        assert_eq!(
            result.err().as_deref(),
            Some("hook push-frame prologue unsupported")
        );
    }
}

#[test]
fn a_held_idle_thread_is_raised_while_held_and_restored_before_it_resumes() {
    use std::os::windows::io::AsRawHandle;
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (ready, lowered) = std::sync::mpsc::channel();
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        assert_ne!(
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE) },
            0
        );
        ready.send(()).unwrap();
        let _ = stopped.recv();
    });
    lowered.recv().unwrap();
    let handle = worker.as_raw_handle() as HANDLE;
    let priority = || unsafe { GetThreadPriority(handle) };
    assert_eq!(priority(), THREAD_PRIORITY_IDLE);
    let mut threads = Threads::open().unwrap();
    threads.suspend().unwrap();
    let held = priority();
    threads.resume().unwrap();
    let failures = threads.diagnostics.sample.priority_restore_failures;
    drop(threads);
    assert_eq!(held, THREAD_PRIORITY_TIME_CRITICAL);
    assert_eq!(priority(), THREAD_PRIORITY_IDLE);
    assert_eq!(failures, 0);
    stop.send(()).unwrap();
    worker.join().unwrap();
}
