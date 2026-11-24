use super::*;
use crate::test_support::{ATTEMPTS, with_allocations_denied};

unsafe extern "system" fn target() -> usize {
    7
}
unsafe extern "system" fn callback() -> usize {
    19
}

fn child(name: &str) -> bool {
    const CHILD: &str = "BATTLEPLAN_PREPARED_RESTORATION";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("hook::restoration::tests::{name}"),
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn hook(code: &TestCode, offset: usize) -> Hook {
    let (site, bytes) = code.stub(offset, target as *const () as usize);
    unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()
}

#[test]
fn prepared_restore_and_final_quiet_need_no_allocations_after_capture_stop() {
    if !child("prepared_restore_and_final_quiet_need_no_allocations_after_capture_stop") {
        return;
    }
    let code = TestCode::new();
    let mut hooks: Vec<_> = (0..4).map(|index| hook(&code, index * 64)).collect();
    assert_eq!(Hook::set_family_enabled(&mut hooks, true), Ok(true));
    let scratch = Scratch::prepare(hooks.len()).unwrap().enter();
    engine_allocation::set_stop_hook(Some(|| true));
    let results = with_allocations_denied(|| {
        let a = hooks[0].set_enabled(false);
        let b = Hook::set_enabled_iter(hooks[1..2].iter_mut(), false);
        let c = Hook::set_enabled_iter(hooks[2..3].iter_mut(), false);
        let d = Hook::set_enabled_iter_tracking_busy(hooks[3..].iter_mut(), false);
        #[expect(
            clippy::single_range_in_vec_init,
            reason = "quiet takes a slice of ranges and this test names one"
        )]
        let quiet = quiet(&[code.0 as usize..code.0 as usize + 4096], &[], || false);
        drop(scratch);
        (a, b, c, d, quiet)
    });
    assert_eq!(
        results,
        (Ok(true), Ok(true), Ok(true), Ok(true), Some(Ok(true)))
    );
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
    assert!(engine_allocation::capture_stopped());
    assert!(
        hooks
            .iter()
            .all(|hook| !hook.is_enabled() && hook.bytes_match(false))
    );
}

#[test]
fn a_batch_restores_every_family_under_one_suspension_without_allocating() {
    if !child("a_batch_restores_every_family_under_one_suspension_without_allocating") {
        return;
    }
    let code = TestCode::new();
    let mut first: Vec<_> = (0..2).map(|index| hook(&code, index * 64)).collect();
    let mut later = hook(&code, 128);
    let mut other = hook(&code, 192);
    assert_eq!(Hook::set_family_enabled(&mut first, true), Ok(true));
    assert_eq!(later.set_enabled(true), Ok(true));
    assert_eq!(other.set_enabled(true), Ok(true));
    let _scratch = Scratch::prepare(4).unwrap().enter();
    engine_allocation::set_stop_hook(Some(|| true));
    let family = |first: &mut Vec<Hook>, later: &mut Hook| -> Result<bool, String> {
        if !Hook::set_family_enabled(first, false)? {
            return Ok(false);
        }
        later.set_enabled(false)
    };
    let suspensions = || SUSPENSIONS.with(std::cell::Cell::get);
    let before = suspensions();
    let results = with_allocations_denied(|| {
        let mut batch = Batch::collect().unwrap();
        let collected = (family(&mut first, &mut later), other.set_enabled(false));
        let restored = batch.publish();
        let reported = (
            attempt(|| family(&mut first, &mut later)).0,
            other.set_enabled(false),
        );
        drop(batch);
        let mut batch = Batch::collect().unwrap();
        let again = family(&mut first, &mut later);
        let next = batch.publish();
        let finished = family(&mut first, &mut later);
        (collected, restored, reported, again, next, finished)
    });
    assert_eq!(
        results,
        (
            (Ok(false), Ok(false)),
            2,
            (Ok(false), Ok(true)),
            Ok(false),
            1,
            Ok(true)
        )
    );
    assert_eq!(suspensions() - before, 2, "one suspension per batch");
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
    assert!(
        first
            .iter()
            .chain([&later, &other])
            .all(|hook| !hook.is_enabled() && hook.bytes_match(false))
    );
}

#[test]
fn failed_flush_retains_the_patch_and_retries_without_allocating() {
    if !child("failed_flush_retains_the_patch_and_retries_without_allocating") {
        return;
    }
    let code = TestCode::new();
    let mut hook = hook(&code, 0);
    assert_eq!(hook.set_enabled(true), Ok(true));
    let _scratch = Scratch::prepare(1).unwrap().enter();
    engine_allocation::set_stop_hook(Some(|| true));
    fail_flushes(1);
    let failed = with_allocations_denied(|| hook.set_enabled(false));
    assert_eq!(failed, Ok(false));
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
    assert!(hook.is_enabled() && hook.bytes_match(true));
    let restored = with_allocations_denied(|| hook.set_enabled(false));
    assert_eq!(restored, Ok(true));
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
    assert!(!hook.is_enabled() && hook.bytes_match(false));
}

#[test]
fn unprovable_census_retains_executable_storage_instead_of_unloading() {
    if !child("unprovable_census_retains_executable_storage_instead_of_unloading") {
        return;
    }
    let code = TestCode::new();
    let mut hook = hook(&code, 0);
    assert_eq!(hook.set_enabled(true), Ok(true));
    let mut scratch = Scratch::prepare(1).unwrap();
    scratch.threads.census = Vec::new();
    let _scratch = scratch.enter();
    engine_allocation::set_stop_hook(Some(|| true));
    let failed = with_allocations_denied(|| hook.set_enabled(false));
    assert_eq!(failed, Ok(false));
    assert!(hook.is_enabled() && hook.bytes_match(true));
    assert_eq!(refusal().unwrap().code, ERROR_NOT_ENOUGH_MEMORY);
    SCRATCH
        .with_borrow_mut(|scratch| scratch.as_mut().unwrap().threads.prepare_census())
        .unwrap();
    let restored = with_allocations_denied(|| hook.set_enabled(false));
    assert_eq!(restored, Ok(true));
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
}

#[test]
fn prepared_restore_preserves_native_flight_and_byte_ownership_guards() {
    if !child("prepared_restore_preserves_native_flight_and_byte_ownership_guards") {
        return;
    }
    static FLIGHTS: AtomicUsize = AtomicUsize::new(0);
    let code = TestCode::new();
    let mut other = hook(&code, 64);
    let mut hook = hook(&code, 0);
    let callback = callback as *const () as usize;
    hook.tap = Some(TapSafety {
        in_flight: &FLIGHTS,
        stub: callback..callback + 1,
    });
    assert_eq!(hook.set_enabled(true), Ok(true));
    assert_eq!(other.set_enabled(true), Ok(true));
    let _scratch = Scratch::prepare(2).unwrap().enter();
    engine_allocation::set_stop_hook(Some(|| true));
    FLIGHTS.store(1, Ordering::SeqCst);
    let (busy, diagnostic) =
        with_allocations_denied(|| attempt(|| (hook.set_enabled(false), other.set_enabled(false))));
    assert_eq!(busy, (Ok(false), Ok(true)));
    assert_eq!(
        diagnostic.unwrap().reason,
        "restore site or native flight remains busy"
    );
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
    assert!(hook.is_enabled() && hook.bytes_match(true));
    FLIGHTS.store(0, Ordering::SeqCst);
    let byte = (hook.site + hook.sentinel_len - 1) as *mut u8;
    let previous = unsafe { byte.read() };
    unsafe { byte.write(previous ^ 1) };
    let changed = with_allocations_denied(|| hook.set_enabled(false));
    assert_eq!(changed, Ok(false));
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
    assert!(hook.is_enabled());
    assert_eq!(refusal().unwrap().reason, "restore hook bytes differ");
    unsafe { byte.write(previous) };
    let restored = with_allocations_denied(|| hook.set_enabled(false));
    assert_eq!(restored, Ok(true));
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
}
