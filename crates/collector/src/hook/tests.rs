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
    let _guard = FAMILY_TEST.lock().unwrap();
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
    let _guard = FAMILY_TEST.lock().unwrap();
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
    let _guard = FAMILY_TEST.lock().unwrap();
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
    let _guard = FAMILY_TEST.lock().unwrap();
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
    let _guard = FAMILY_TEST.lock().unwrap();
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
    engine::allocation::set_stop_hook(Some(|| true));
    assert!(engine::allocation::capture_stopped());
    assert_eq!(
        engine::allocation::vec::<u8>(1).unwrap_err().kind(),
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
        engine::allocation::capture_stopped(),
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
