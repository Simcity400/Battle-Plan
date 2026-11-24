use super::*;
use std::cell::{Cell, RefCell};

type ContextState = (Vec<Result<u64, u32>>, Vec<usize>, Vec<usize>);
thread_local! {
    static CONTEXTS: RefCell<ContextState> = RefCell::default();
}
pub(super) fn acquire(handle: HANDLE, context: &mut AlignedContext) -> Option<Result<(), u32>> {
    CONTEXTS.with(|state| {
        let mut state = state.borrow_mut();
        let (ips, calls, _) = &mut *state;
        if ips.is_empty() {
            return None;
        }
        assert_eq!(std::ptr::from_mut(context) as usize % 16, 0);
        assert_eq!(context.0.ContextFlags, CONTEXT_CONTROL_AMD64);
        let index = handle as usize - 1;
        calls[index] += 1;
        Some(ips[index].map(|ip| context.0.Rip = ip))
    })
}
pub(super) fn exited(handle: HANDLE) -> Option<bool> {
    CONTEXTS.with(|state| {
        let state = state.borrow();
        (!state.0.is_empty()).then(|| state.2.contains(&(handle as usize)))
    })
}
pub(super) fn counts() -> Vec<usize> {
    CONTEXTS.with(|state| state.borrow().1.clone())
}
pub(super) fn set_ips(ips: &[Result<u64, u32>]) {
    CONTEXTS.with(|state| state.borrow_mut().0.copy_from_slice(ips));
}
fn thread(id: u32, handle: usize) -> Thread {
    Thread {
        id,
        handle: handle as HANDLE,
        suspended: false,
        ip: None,
        sp: 0,
        exited: false,
        priority: None,
    }
}
pub(super) fn fixture(count: usize, ips: &[Result<u64, u32>]) -> ManuallyDrop<Threads> {
    CONTEXTS.with(|state| *state.borrow_mut() = (ips.to_vec(), vec![0; ips.len()], Vec::new()));
    let mut threads = Vec::with_capacity(count + count / 2 + 16);
    threads.extend((1..=count).map(|id| thread(id as u32, id)));
    ManuallyDrop::new(Threads {
        threads,
        excluded: Vec::new(),
        census: Vec::new(),
        census_required: 0,
        census_short: false,
        census_refusal: None,
        context: AlignedContext(unsafe { std::mem::zeroed() }),
        snapshot_valid: false,
        diagnostics: publication::Attempt::new(),
    })
}
pub(super) fn release(mut threads: ManuallyDrop<Threads>) {
    threads
        .resume_with(|_| Ok(()), |_| false, || unreachable!())
        .unwrap();
    threads.threads.clear();
    unsafe { ManuallyDrop::drop(&mut threads) };
    CONTEXTS.with(|state| *state.borrow_mut() = Default::default());
}
pub(super) fn stable(threads: &mut Threads) -> Result<(), u32> {
    threads.suspend_with(
        |handle, clock| suspension_with(handle, clock, thread_exited, |_| Ok(())),
        |threads| threads.discover_with(Threads::synchronize, |_| Ok(false)),
        |_| unreachable!(),
    )
}
fn ready(threads: &mut Threads, hooks: &mut [Hook], enabled: bool) -> Result<bool, u32> {
    let selected: Vec<_> = hooks
        .iter()
        .map(|hook| enabled || hook.is_enabled())
        .collect();
    let ranges: Vec<_> = hooks
        .iter()
        .zip(&selected)
        .filter(|(_, selected)| **selected)
        .map(|(hook, _)| hook.site..hook.site + hook.sentinel_len)
        .collect();
    let mut hooks: Vec<_> = hooks.iter_mut().collect();
    Hook::family_ready::<false>(
        &mut hooks,
        enabled,
        |index| selected[index],
        &ranges,
        threads,
        &mut false,
    )
}
fn check(count: usize, ip: u64, hooks: &mut [Hook], enabled: bool, expected: bool) {
    let mut ips = vec![Ok(1); count];
    ips[0] = Ok(ip);
    let mut threads = fixture(count, &ips);
    stable(&mut threads).unwrap();
    assert_eq!(ready(&mut threads, hooks, enabled), Ok(expected));
    assert_eq!(counts(), vec![1; count]);
    release(threads);
}
fn set_owned(hook: &mut Hook, enabled: bool, unflushed: bool) {
    hook.enabled = enabled;
    hook.needs_flush = unflushed;
    let bytes = if enabled { &hook.patch } else { &hook.original };
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), hook.site as *mut u8, hook.patch_len) };
}
fn unsafe_ip(hook: &Hook, enabled: bool, ip: usize) -> bool {
    let site = hook.site..hook.site + hook.sentinel_len;
    let mut ranges = hook.executable_ranges().to_vec();
    ranges.push(hook.tap.as_ref().unwrap().stub.clone());
    (enabled || hook.is_enabled())
        && (site.contains(&ip)
            || (!enabled || !hook.enabled) && ranges.iter().any(|range| range.contains(&ip)))
}
fn range_decision(ip: Result<u64, u32>) -> Result<bool, u32> {
    ip.map(|ip| ip == 100).map_err(|_| ERROR_RETRY)
}
fn private(family: &str) {
    assert!(
        !publication::snapshot()
            .0
            .iter()
            .any(|(name, _)| *name == family)
    );
}

#[test]
fn ten_tap_family_uses_one_stable_context_per_thread_for_all_ranges_and_flights() {
    static FLIGHTS: [AtomicUsize; 10] = [const { AtomicUsize::new(0) }; 10];
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut hooks: Vec<_> = FLIGHTS
        .iter()
        .enumerate()
        .map(|(index, flight)| {
            let (site, bytes) = code.stub(index * 64, code.0 as usize + 2048);
            let callback = code.0 as usize + 1024 + index * 64;
            unsafe {
                Hook::new_with_safety(
                    site,
                    callback,
                    bytes,
                    TapSafety {
                        in_flight: flight,
                        stub: callback..callback + 32,
                    },
                )
            }
            .unwrap()
        })
        .collect();
    let before = SUSPENSIONS.with(Cell::get);
    assert!(Hook::set_family_enabled(&mut hooks, false).unwrap());
    assert_eq!(
        SUSPENSIONS.with(Cell::get),
        before,
        "unowned restore needs no controller"
    );
    for ownership in 0..3 {
        for (index, hook) in hooks.iter_mut().enumerate() {
            set_owned(
                hook,
                ownership == 1 || ownership == 2 && index % 3 == 0,
                ownership == 2 && index % 3 == 1,
            );
        }
        for enabled in [true, false] {
            for count in [7, 5000] {
                check(count, 1, &mut hooks, enabled, true);
            }
            for (index, flight) in FLIGHTS.iter().enumerate() {
                let hook = &hooks[index];
                let [relay, trampoline] = hook.executable_ranges();
                let ranges = [
                    hook.site..hook.site + hook.sentinel_len,
                    relay,
                    trampoline,
                    hook.tap.as_ref().unwrap().stub.clone(),
                ];
                for range in ranges {
                    for ip in [range.start, range.end - 1, range.end] {
                        let expected = hooks.iter().any(|hook| unsafe_ip(hook, enabled, ip));
                        check(7, ip as u64, &mut hooks, enabled, !expected);
                    }
                }
                let expected =
                    (enabled || hooks[index].is_enabled()) && (!enabled || !hooks[index].enabled);
                flight.store(1, Ordering::SeqCst);
                check(7, 1, &mut hooks, enabled, !expected);
                flight.store(0, Ordering::SeqCst);
            }
        }
    }
    for hook in &mut hooks {
        set_owned(hook, false, false);
    }
}

#[test]
fn newcomer_is_acquired_before_any_healthy_busy_or_error_decision() {
    for newcomer in [Ok(1), Ok(100), Err(87)] {
        let mut threads = fixture(1, &[Ok(1), newcomer]);
        let mut censuses = 0;
        let result = threads.suspend_with(
            |_, clock| Ok(Some(clock.now())),
            |threads| {
                threads.discover_with(Threads::synchronize, |threads| {
                    censuses += 1;
                    if censuses == 1 {
                        assert_eq!(counts(), [1, 0]);
                        assert_eq!(
                            threads.contains_ip(std::slice::from_ref(&(100..116))),
                            Err(ERROR_RETRY)
                        );
                        threads.threads.push(thread(2, 2));
                        Ok(true)
                    } else {
                        assert_eq!(counts(), [2, 1]);
                        Ok(false)
                    }
                })
            },
            |_| unreachable!(),
        );
        assert_eq!(counts(), [2, 1]);
        assert_eq!(result, newcomer.map(|_| ()));
        assert_eq!(censuses, if newcomer.is_ok() { 2 } else { 1 });
        for _ in 0..11 {
            assert_eq!(
                threads.contains_ip(std::slice::from_ref(&(100..116))),
                range_decision(newcomer)
            );
        }
        assert_eq!(counts(), [2, 1], "decisions add no context acquisition");
        release(threads);
    }
}

#[test]
fn exits_preserve_errors_recapture_survivors_and_bind_replacements_to_handles() {
    for cached_exit in [false, true] {
        for survivor in [Ok(1), Ok(100), Err(87)] {
            let mut threads = fixture(2, &[Ok(100), Ok(1), Ok(105)]);
            stable(&mut threads).unwrap();
            CONTEXTS.with(|state| state.borrow_mut().2.push(1));
            if cached_exit {
                assert_eq!(
                    threads.contains_ip(std::slice::from_ref(&(100..116))),
                    Ok(false)
                );
                assert!(threads.threads[0].suspended);
                let mut resumed = Vec::new();
                assert_eq!(
                    threads.resume_with(
                        |handle| {
                            resumed.push(handle as usize);
                            if handle as usize == 1 { Err(6) } else { Ok(()) }
                        },
                        thread_exited,
                        || unreachable!()
                    ),
                    Err(6)
                );
                assert_eq!(
                    resumed,
                    [2, 1],
                    "cached exit retains the original resume/error path"
                );
                set_ips(&[Ok(100), survivor, Ok(105)]);
            } else {
                set_ips(&[Err(6), survivor, Ok(105)]);
            }
            let result = threads.suspend_with(
                |handle, clock| suspension_with(handle, clock, thread_exited, |_| Ok(())),
                |threads| {
                    threads.discover_with(Threads::synchronize, |threads| {
                        let mut closed = Vec::new();
                        threads.remove_exited_with(thread_exited, |handle| {
                            closed.push(handle as usize)
                        });
                        assert_eq!(closed, [1]);
                        Ok(false)
                    })
                },
                |_| unreachable!(),
            );
            assert_eq!(result, survivor.map(|_| ()));
            assert_eq!(
                threads.contains_ip(std::slice::from_ref(&(100..116))),
                range_decision(survivor)
            );
            let first_calls = if cached_exit { 1 } else { 2 };
            assert_eq!(counts(), [first_calls, 2, 0]);
            if survivor.is_ok() {
                set_ips(&[Ok(1), Ok(1), Ok(105)]);
                threads.threads.push(thread(1, 3));
                stable(&mut threads).unwrap();
                assert_eq!(
                    threads.contains_ip(std::slice::from_ref(&(105..106))),
                    Ok(true)
                );
                assert_eq!(counts(), [first_calls, 3, 1]);
            }
            release(threads);
        }
    }
    let mut threads = fixture(1, &[Ok(100)]);
    stable(&mut threads).unwrap();
    set_ips(&[Err(6)]);
    let mut censuses = 0;
    assert_eq!(
        threads.discover_with(Threads::synchronize, |_| {
            censuses += 1;
            Ok(false)
        }),
        Err(6)
    );
    assert_eq!(censuses, 0, "unproved exit preserves the acquisition error");
    assert_eq!(
        threads.contains_ip(std::slice::from_ref(&(100..116))),
        Err(ERROR_RETRY)
    );
    assert_eq!(counts(), [2]);
    release(threads);
}

#[test]
fn hold_measurement_excludes_off_hold_work_and_publishes_only_after_resolution() {
    for (family, off_hold, precheck, held_stall, proved_exit) in [
        ("synthetic_off_hold", 1000, 0, 0, false),
        ("synthetic_precheck", 0, 1000, 0, false),
        ("synthetic_held", 0, 0, 1000, false),
        ("synthetic_exit", 1000, 0, 0, true),
    ] {
        publication::select_family(Some(family));
        let mut threads = fixture(2, &[Ok(1); 2]);
        let time = std::rc::Rc::new(Cell::new(off_hold));
        threads.diagnostics.clock = publication::Clock::Fake(time.clone());
        let events = RefCell::new(Vec::new());
        for interval in 0..2 {
            let precheck_delay = Cell::new(if interval == 0 { precheck } else { 0 });
            let suspend = |handle, clock: &publication::Attempt| {
                suspension_with(
                    handle,
                    clock,
                    |handle| {
                        time.set(time.get() + precheck_delay.replace(0));
                        events.borrow_mut().push(("precheck", time.get()));
                        proved_exit && interval == 1 && handle as usize == 2
                    },
                    |_| {
                        events.borrow_mut().push(("suspend", time.get()));
                        time.set(time.get() + 10);
                        Ok(())
                    },
                )
            };
            let discover = |threads: &mut Threads| {
                let context = |handle: HANDLE, context: &mut AlignedContext| {
                    let stall = if interval == 0 && handle as usize == 2 {
                        held_stall
                    } else {
                        0
                    };
                    time.set(time.get() + 5 + stall);
                    context.0.Rip = 1;
                    Ok(())
                };
                threads.discover_with(
                    |threads| threads.synchronize_with(context, |_| false, |_| (0, None)),
                    |_| Ok(false),
                )
            };
            threads
                .suspend_with(suspend, discover, |_| unreachable!())
                .unwrap();
            private(family);
            let failed = Cell::new(false);
            let resumed = RefCell::new(Vec::new());
            let result = threads.resume_with(
                |handle| {
                    private(family);
                    resumed.borrow_mut().push(handle as usize);
                    time.set(time.get() + 10);
                    if interval == 0 && handle as usize == 2 && !failed.replace(true) {
                        Err(6)
                    } else {
                        Ok(())
                    }
                },
                |_| proved_exit,
                || time.set(time.get() + 5),
            );
            assert_eq!(result, if interval == 0 { Err(6) } else { Ok(()) });
            if interval == 0 {
                assert_eq!(
                    *resumed.borrow(),
                    if proved_exit {
                        vec![2, 1]
                    } else {
                        vec![2, 1, 2]
                    }
                );
                private(family);
                let grown = threads.census.len() + 1;
                threads.census.resize(grown, 0);
                time.set(time.get() + 1000);
            }
        }
        let boundary = off_hold + precheck;
        assert_eq!(
            &events.borrow()[..2],
            &[("precheck", boundary), ("suspend", boundary)]
        );
        let sample = threads.diagnostics.sample;
        release(threads);
        let mut second = publication::Attempt::new();
        second.finish();
        publication::select_family(None);
        let json = serde_json::to_value(publication::status()).unwrap();
        let summary = &json["families"][family];
        assert_eq!(summary["attempts"], 2);
        assert_eq!(
            sample.total_hold_ns,
            if proved_exit { 75 } else { 115 + held_stall }
        );
        if !proved_exit {
            assert_eq!(
                time.get(),
                2115,
                "equal wall stalls have different held totals"
            );
        }
        assert_eq!(summary["total_hold_ns"], sample.total_hold_ns);
        assert_eq!(
            summary["longest_hold_ns"],
            if proved_exit { 50 } else { 65 + held_stall }
        );
        assert_eq!(summary["longest_synchronization_ns"], 10 + held_stall);
        assert_eq!(summary["slowest_context_ns"], 5 + held_stall);
        assert_eq!(
            summary["slowest_context_thread_id"],
            if held_stall == 0 { 1 } else { 2 }
        );
        assert_eq!(summary["slowest_context_priority"], 0);
        assert_eq!(summary["priority_restore_failures"], 0);
        assert_eq!(summary.as_object().unwrap().len(), 8);
        assert_eq!(json.as_object().unwrap().len(), 2);
    }
}

#[test]
fn installation_diagnostic_identifies_the_mismatching_member_snapshot() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, bytes) = code.stub(0, code.0 as usize + 2048);
    let hook = unsafe { Hook::new(site, code.0 as usize + 1024, bytes) }.unwrap();
    assert!(hook.matches_published_snapshot());
    unsafe {
        *(site as *mut u8) = 0xcc;
    }
    assert!(!hook.matches_published_snapshot());
    unsafe {
        *(site as *mut u8) = bytes[0];
    }
    assert!(hook.matches_published_snapshot());
}
