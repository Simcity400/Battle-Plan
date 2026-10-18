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
    // A refused family never owned these bytes; disable must leave them alone.
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
    // The deliberate leak is confined to this test process. Restore the
    // owned stub before freeing it; no game memory is involved.
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
fn owned_family_restores_after_capture_stop_without_clearing_it() {
    const CHILD: &str = "BATTLE_PLAN_HOOK_RESTORE_AFTER_CAPTURE_STOP";
    if std::env::var_os(CHILD).is_none() {
        // Isolate the process-global capture stop hook from other fixture threads.
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

/// A publication holds the process-wide publication lock from its first
/// page-protection change through its flush and restore, so a second
/// publisher on another thread (whose sites may share the code page) waits
/// instead of writing into a page the first has already made read-only.
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

/// A census thread id that, by the time it is opened, names another
/// process's thread (the listed thread exited and the id was reused) is not
/// opened: suspending it would hold a foreign process, and a protected one
/// refuses with ERROR_ACCESS_DENIED, which failed publication with "Windows
/// error 5". A child process's thread stands in for the reused id.
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
        let mut found = unsafe { Thread32First(snapshot, &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == pid {
                foreign = Some(entry.th32ThreadID);
                break;
            }
            found = unsafe { Thread32Next(snapshot, &mut entry) };
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
    // This process's own threads still open.
    let own = std::thread::spawn(|| unsafe { GetCurrentThreadId() });
    let id = unsafe { GetCurrentThreadId() };
    drop(own.join());
    let handle = open_thread(id).unwrap().expect("an own thread opens");
    unsafe {
        CloseHandle(handle);
    }
}

/// A thread that is terminating when publication suspends it refuses with
/// ERROR_ACCESS_DENIED (STATUS_THREAD_IS_TERMINATING) before its handle is
/// signalled, which failed the whole publication with "Windows error 5"
/// whenever a test or game thread happened to exit at that moment. It never
/// runs user code again and is left unheld; other refusals still fail.
#[test]
fn a_terminating_thread_is_left_unheld_and_other_refusals_still_fail() {
    assert_eq!(refused_suspension(ERROR_ACCESS_DENIED, false), Ok(false));
    assert_eq!(refused_suspension(ERROR_INVALID_PARAMETER, true), Ok(false));
    assert_eq!(
        refused_suspension(ERROR_INVALID_PARAMETER, false),
        Err(ERROR_INVALID_PARAMETER)
    );
}

/// Families publish under one census and suspension, so a thread executing
/// any one site of a family holds back the whole family (no site of it is
/// written, the attempt is not ready and is retried), while another family
/// publishes. Once the thread leaves, the family publishes all its sites under
/// one suspension.
#[test]
fn a_thread_at_any_site_holds_its_whole_family_under_one_suspension() {
    use std::os::windows::io::AsRawHandle;
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (first, _) = code.stub(0, original as *const () as usize);
    // The second site jumps to itself: a thread entering it stays inside.
    let held = code.0 as usize + 64;
    let (_, held_bytes) = code.stub(64, held);
    let (other, _) = code.stub(128, original as *const () as usize);
    let new = |(site, bytes): (usize, [u8; 16])| {
        unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()
    };
    let mut family = vec![new((first, code_bytes(first))), new((held, held_bytes))];
    let mut other_family = vec![new((other, code_bytes(other)))];
    let worker = std::thread::spawn(move || unsafe { invoke(held) });
    // Wait until the worker executes inside the held site.
    let handle = worker.as_raw_handle() as HANDLE;
    let started = std::time::Instant::now();
    loop {
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_ne!(unsafe { SuspendThread(handle) }, u32::MAX);
        let mut context = AlignedContext(unsafe { std::mem::zeroed() });
        context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
        assert_ne!(unsafe { GetThreadContext(handle, &mut context.0) }, 0);
        assert_ne!(unsafe { ResumeThread(handle) }, u32::MAX);
        if (held..held + 16).contains(&(context.0.Rip as usize)) {
            break;
        }
        std::thread::yield_now();
    }
    let suspensions = || SUSPENSIONS.with(std::cell::Cell::get);
    let before = suspensions();
    assert_eq!(Hook::set_family_enabled(&mut family, true), Ok(false));
    assert_eq!(suspensions() - before, 1, "one suspension for the family");
    assert!(
        family.iter().all(|hook| !hook.is_enabled()),
        "no site written"
    );
    assert_eq!(unsafe { invoke(first) }, 7);
    // Another family is not held back by it.
    assert_eq!(Hook::set_family_enabled(&mut other_family, true), Ok(true));
    assert_eq!(unsafe { invoke(other) }, 19);
    // Let the worker out, then put the held site's bytes back as they were.
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
    assert_eq!(Hook::set_family_enabled(&mut family, true), Ok(true));
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
