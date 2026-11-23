use super::*;
use std::cell::Cell;

struct DropProbe<'a>(&'a Cell<usize>);
impl Drop for DropProbe<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn equipment_destruction_detach_restores_other_families_after_a_refusal() {
    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    let mut calls = Vec::new();
    assert_eq!(
        restore_round(
            &mut restored,
            &mut previous_errors,
            || false,
            |family| {
                calls.push(family);
                match family {
                    RestoreFamily::Hourly | RestoreFamily::EquipmentDestruction => {
                        Err("synthetic enabled patch mismatch")
                    }
                    RestoreFamily::Portrait => Ok(false),
                    _ => Ok(true),
                }
            }
        ),
        Ok(false)
    );
    assert_eq!(
        calls,
        RestoreFamily::ALL[..RestoreFamily::WorldLifetime as usize]
    );
    assert!(!restored[RestoreFamily::WorldLifetime as usize]);
    assert!(restored[RestoreFamily::History as usize]);
    assert!(restored[RestoreFamily::Relation as usize]);
    for family in [
        RestoreFamily::Land,
        RestoreFamily::Experience,
        RestoreFamily::Fuel,
        RestoreFamily::OperationOutcomes,
        RestoreFamily::Politics,
        RestoreFamily::Logistics,
        RestoreFamily::Peace,
    ] {
        assert!(
            restored[family as usize],
            "{family:?} restored despite earlier refusal"
        );
    }
    assert!(!restored[RestoreFamily::Hourly as usize]);

    calls.clear();
    assert_eq!(
        restore_round(
            &mut restored,
            &mut previous_errors,
            || false,
            |family| {
                calls.push(family);
                match family {
                    RestoreFamily::Hourly => Err("synthetic enabled patch mismatch"),
                    _ => Ok(true),
                }
            }
        ),
        Err("synthetic enabled patch mismatch")
    );
    assert_eq!(
        calls,
        [
            RestoreFamily::Hourly,
            RestoreFamily::EquipmentDestruction,
            RestoreFamily::Portrait,
        ]
    );
    assert_eq!(
        restored.iter().filter(|complete| **complete).count(),
        RestoreFamily::ALL.len() - 2
    );

    calls.clear();
    assert_eq!(
        restore_round(
            &mut restored,
            &mut previous_errors,
            || false,
            |family| {
                calls.push(family);
                Ok(true)
            }
        ),
        Ok(true)
    );
    assert_eq!(calls, [RestoreFamily::Hourly, RestoreFamily::WorldLifetime]);
}

#[test]
fn equipment_destruction_detach_skips_the_already_restored_hourly_hook() {
    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    restored[RestoreFamily::Hourly as usize] = true;
    let mut calls = Vec::new();
    assert_eq!(
        restore_round(
            &mut restored,
            &mut previous_errors,
            || false,
            |family| {
                calls.push(family);
                Ok(true)
            }
        ),
        Ok(true)
    );
    assert_eq!(calls, &RestoreFamily::ALL[1..]);
    assert_eq!(
        restore_round(
            &mut restored,
            &mut previous_errors,
            || false,
            |_| panic!("already restored")
        ),
        Ok(true)
    );
}

#[test]
fn equipment_destruction_detach_transient_busy_is_not_a_persistent_error() {
    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    for attempt in 0..4 {
        assert_eq!(
            restore_round(
                &mut restored,
                &mut previous_errors,
                || false,
                |family| {
                    if family != RestoreFamily::EquipmentDestruction {
                        return Ok(true);
                    }
                    match attempt {
                        0 | 2 => Err("transient OS error"),
                        _ => Ok(false),
                    }
                }
            ),
            Ok(false)
        );
    }
    assert_eq!(
        restore_round(&mut restored, &mut previous_errors, || false, |_| Ok(true)),
        Ok(true)
    );
}

#[test]
fn equipment_destruction_persistent_error_still_visits_every_unfinished_family() {
    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    for result in [Ok(false), Err("synthetic persistent patch error")] {
        let mut calls = Vec::new();
        assert_eq!(
            restore_round(
                &mut restored,
                &mut previous_errors,
                || false,
                |family| {
                    calls.push(family);
                    if family == RestoreFamily::Hourly {
                        Err("synthetic persistent patch error")
                    } else {
                        Ok(false)
                    }
                }
            ),
            result
        );
        assert_eq!(
            calls,
            RestoreFamily::ALL[..RestoreFamily::WorldLifetime as usize]
        );
    }
}

#[test]
fn equipment_destruction_hourly_restoration_is_terminal_on_exactly_two_errors() {
    for busy in [false, true] {
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut previous_errors = [false; RestoreFamily::ALL.len()];
        let hourly_calls = Cell::new(0);
        let waits = Cell::new(0);
        restore_hourly(
            &mut restored,
            &mut previous_errors,
            || Duration::ZERO,
            || {
                hourly_calls.set(hourly_calls.get() + 1);
                if busy && hourly_calls.get() == 1 {
                    Ok(false)
                } else {
                    Err("synthetic persistent hourly error")
                }
            },
            || waits.set(waits.get() + 1),
        );
        assert_eq!(waits.get(), usize::from(busy));
        assert_eq!(hourly_calls.get(), 1 + usize::from(busy));
        let mut families = Vec::new();
        assert_eq!(
            restore_round(
                &mut restored,
                &mut previous_errors,
                || false,
                |family| {
                    families.push(family);
                    if family == RestoreFamily::Hourly {
                        hourly_calls.set(hourly_calls.get() + 1);
                        Err("synthetic persistent hourly error")
                    } else {
                        Ok(true)
                    }
                }
            ),
            Err("synthetic persistent hourly error")
        );
        assert_eq!(hourly_calls.get() - waits.get(), 2);
        assert_eq!(
            families,
            RestoreFamily::ALL[..RestoreFamily::WorldLifetime as usize]
        );
    }
}

#[test]
fn equipment_destruction_successful_run_exit_quiesces_releases_and_unloads() {
    let shared = SharedSlot::retained_for_test();
    let pointer = shared.identity_for_test();
    let drops = Cell::new(0);
    let waits = Cell::new(0);
    let hooks = HookLifetime {
        restored: true,
        shared: &shared,
    };
    hooks.wait_quiet(|| waits.set(waits.get() + 1));
    hooks.release((DropProbe(&drops), DropProbe(&drops)), || {
        assert_eq!(shared.identity_for_test(), pointer);
        assert_eq!(drops.get(), 0);
        waits.set(waits.get() + 1);
    });
    assert_eq!(waits.get(), 2);
    assert_eq!(drops.get(), 2);
    assert!(shared.identity_for_test().is_null());
    let unloaded = Cell::new(false);
    assert_eq!(
        finish_worker(&shared, DropProbe(&drops), |witness| {
            unloaded.set(true);
            drop(witness);
            0
        }),
        0
    );
    assert!(unloaded.get());
    assert_eq!(drops.get(), 3);
}

#[test]
fn equipment_destruction_capture_admission_drains_reads_and_refuses_later_capture() {
    let admission = CaptureAdmission::new();
    let lease =
    // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
    // hourly exit.
    unsafe { admission.enter() }.unwrap();
    // SAFETY: Detach closes admission before waiting for admitted capture to complete.
    unsafe { admission.close() };
    assert!(!admission.quiet());
    assert!(
        // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
        // hourly exit.
        unsafe { admission.enter() }.is_none()
    );
    let (lease, polls) = (std::cell::RefCell::new(Some(lease)), Cell::new(0));
    let waited = wait_acquisitions(&admission, || {
        polls.set(polls.get() + 1);
        if polls.get() == 1 {
            drop(lease.borrow_mut().take());
            Duration::ZERO
        } else {
            DETACH_BUDGET
        }
    });
    assert_eq!(waited, Ok(()), "releasing the last lease quiets admission");
    assert_eq!(polls.get(), 1);
    assert!(
        // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
        // hourly exit.
        unsafe { admission.enter() }.is_none()
    );
}

#[test]
fn equipment_destruction_pinned_status_preserves_closed_spool_metadata_and_unlocks_before_done() {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::{Record, recover_prefix};
    struct BusyObserver;
    impl Drop for BusyObserver {
        fn drop(&mut self) {
            collector_characters::portrait_changes::ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    collector_characters::portrait_changes::ACTIVE.fetch_add(1, Ordering::SeqCst);
    let _observer = BusyObserver;
    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    let mut restore = |family| {
        if family == RestoreFamily::Portrait {
            Err("synthetic persistent patch error")
        } else {
            Ok(true)
        }
    };
    assert_eq!(
        restore_round(
            &mut restored,
            &mut previous_errors,
            observers_active,
            &mut restore
        ),
        Ok(false)
    );
    let restore_result = restore_round(
        &mut restored,
        &mut previous_errors,
        observers_active,
        &mut restore,
    );
    let restoration_error = restore_result.unwrap_err();
    let admission = CaptureAdmission::new();
    // SAFETY: Detach closes admission before waiting for admitted capture to complete.
    unsafe { admission.close() };
    wait_acquisitions(&admission, || Duration::ZERO).unwrap();
    assert!(observers_active());
    let retained_shared = SharedSlot::retained_for_test();
    let pointer = retained_shared.identity_for_test();
    let hooks = HookLifetime {
        restored: restore_result.is_ok(),
        shared: &retained_shared,
    };
    hooks.wait_quiet(|| panic!("pinned run must not wait for native observers"));
    let root = TempDir(std::env::temp_dir().join(format!(
        "hoi4-pinned-terminal-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    fs::create_dir_all(&root.0).unwrap();
    let session_dir = root.0.join("session");
    let mut processor =
    // SAFETY: The isolated test owns the spool's failure state.
    unsafe { collector_spool::spooling::spool_failure_fixture(&session_dir) }.unwrap();
    let frame = collector_spool::spooling::allocation_failure_frame(2);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.prepare(&frame) }.unwrap();
    let prepared =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.prepare_frame(&frame) }.unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.write_prepared(prepared) }.unwrap();
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let directories = processor.spool_directories().unwrap();
    let bytes = fs::read(directories[0].join("spool.0000.h4s")).unwrap();
    let recovered = recover_prefix(&bytes);
    assert!(recovered.damage.is_none(), "{:?}", recovered.damage);
    assert!(recovered.records.iter().any(|record| matches!(
        record,
        Record::Control(ControlRecord::Detach {
            last_hour: Some(2),
            ..
        })
    )));
    assert!(!recovered.records.iter().any(|record| matches!(
        record,
        Record::Control(ControlRecord::Partial { .. } | ControlRecord::GapSummary { .. })
    )));
    let session = recovered
        .records
        .iter()
        .find_map(|record| match record {
            Record::SessionStart(session) => Some(session.clone()),
            _ => None,
        })
        .unwrap();
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, session.qpc_frequency) };

    // SAFETY: The current runtime or its spool owns the session's terminal stop.
    unsafe { shared.recording() }.store(false, Ordering::Release);
    let mut writer = crate::status_writer::StatusWriter::start(|_, _| Ok(())).unwrap();
    let status = collector_status::status::serialize(
        &spool_status(
            &processor,
            &shared,
            &session,
            "detached",
            None,
            None,
            true,
            true,
            TeardownStatus {
                restoration_error: Some(restoration_error),
                installation_error: Some("synthetic setup error"),
            },
        )
        .unwrap(),
    )
    .unwrap();
    writer.finish();
    let status: Value = serde_json::from_slice(&status).unwrap();
    assert_eq!(status["hours"], 1);
    assert_eq!(status["campaign"], "01234567-89ab-cdef-0123-456789abcdef");
    assert_eq!(status["state"], "pinned");
    assert_eq!(status["drain_complete"], true);
    assert_eq!(status["error"], "synthetic setup error");
    assert_eq!(
        status["restoration_error"],
        "synthetic persistent patch error"
    );

    let lock_path = root.0.join("finalize.lock");
    let open_lock = || {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&lock_path)
    };
    let output_lock = open_lock().unwrap();
    let done = Event::create(&format!(
        "Local\\hoi4-destroyed-pinned-done-{}",
        std::process::id()
    ))
    .unwrap();
    assert!(!done.signaled());
    assert_eq!(open_lock().unwrap_err().raw_os_error(), Some(32));
    signal_done(output_lock, &done);
    assert!(done.signaled());
    drop(open_lock().unwrap());
    assert!(observers_active());
    let drops = Cell::new(0);
    hooks.release((DropProbe(&drops), DropProbe(&drops)), || {
        panic!("pinned run must skip the final relay quiescence wait");
    });
    assert_eq!(
        drops.get(),
        0,
        "pinned relay/shared storage must survive run"
    );
    assert_eq!(retained_shared.identity_for_test(), pointer);
    assert_eq!(
        finish_worker(&retained_shared, DropProbe(&drops), |_| {
            panic!("pinned worker must not clear allocation hooks or unload");
        }),
        1
    );
    assert_eq!(drops.get(), 0, "pinned worker must retain its witness");
    drop(processor);
    fs::remove_dir_all(&root.0).unwrap();
    assert!(!root.0.exists());
}
