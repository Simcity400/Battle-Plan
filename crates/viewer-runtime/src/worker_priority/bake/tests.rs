use super::*;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use windows_sys::Win32::System::Threading::{
    THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_IDLE,
    THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_NORMAL,
};

const BAKE_CASES: [(i32, i32); 6] = [
    (THREAD_PRIORITY_IDLE, THREAD_PRIORITY_IDLE),
    (THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_LOWEST),
    (THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_BELOW_NORMAL),
    (THREAD_PRIORITY_NORMAL, THREAD_PRIORITY_BELOW_NORMAL),
    (THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_BELOW_NORMAL),
    (THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_BELOW_NORMAL),
];

#[test]
fn bake_never_raises_and_restores_the_exact_entry_priority() {
    for (before, during) in BAKE_CASES {
        let priority = Cell::new(before);
        let sets = RefCell::new(Vec::new());
        let reads = Cell::new(0);
        let calls = Cell::new(0);
        let result = with_priority(
            || {
                reads.set(reads.get() + 1);
                Some(priority.get())
            },
            |value| {
                sets.borrow_mut().push(value);
                priority.set(value);
                true
            },
            || {
                assert_eq!(priority.get(), during);
                calls.set(calls.get() + 1);
                37
            },
        );
        assert_eq!(result, 37);
        assert_eq!(calls.get(), 1);
        assert_eq!(priority.get(), before);
        let expected_sets = if before == during {
            vec![]
        } else {
            vec![THREAD_PRIORITY_BELOW_NORMAL, before]
        };
        assert_eq!(*sets.borrow(), expected_sets);
        assert_eq!(reads.get(), if before == during { 1 } else { 2 });
    }
}

#[test]
fn an_early_error_preserves_or_restores_the_entry_priority() {
    for (before, during) in BAKE_CASES {
        let priority = Cell::new(before);
        let sets = RefCell::new(Vec::new());
        let calls = Cell::new(0);
        let result = with_priority(
            || Some(priority.get()),
            |value| {
                sets.borrow_mut().push(value);
                priority.set(value);
                true
            },
            || {
                assert_eq!(priority.get(), during);
                calls.set(calls.get() + 1);
                if priority.get() == during {
                    return Err("bake refused");
                }
                Ok(())
            },
        );
        assert_eq!(result, Err("bake refused"));
        assert_eq!(calls.get(), 1);
        assert_eq!(priority.get(), before);
        let expected_sets = if before == during {
            vec![]
        } else {
            vec![THREAD_PRIORITY_BELOW_NORMAL, before]
        };
        assert_eq!(*sets.borrow(), expected_sets);
    }
}

#[test]
fn a_panicking_bake_preserves_or_restores_before_unwinding_to_the_caller() {
    for (before, during) in BAKE_CASES {
        let priority = Cell::new(before);
        let sets = RefCell::new(Vec::new());
        let calls = Cell::new(0);
        let result = catch_unwind(AssertUnwindSafe(|| {
            with_priority(
                || Some(priority.get()),
                |value| {
                    sets.borrow_mut().push(value);
                    priority.set(value);
                    true
                },
                || {
                    assert_eq!(priority.get(), during);
                    calls.set(calls.get() + 1);
                    panic!("bake panic");
                },
            );
        }));
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        assert_eq!(priority.get(), before);
        let expected_sets = if before == during {
            vec![]
        } else {
            vec![THREAD_PRIORITY_BELOW_NORMAL, before]
        };
        assert_eq!(*sets.borrow(), expected_sets);
    }
}

#[test]
fn nested_bakes_leave_already_lowered_scopes_unchanged() {
    for (before, during) in BAKE_CASES {
        let priority = Cell::new(before);
        let sets = RefCell::new(Vec::new());
        let reads = Cell::new(0);
        let calls = Cell::new(0);
        with_priority(
            || {
                reads.set(reads.get() + 1);
                Some(priority.get())
            },
            |value| {
                sets.borrow_mut().push(value);
                priority.set(value);
                true
            },
            || {
                assert_eq!(priority.get(), during);
                calls.set(calls.get() + 1);
                with_priority(
                    || {
                        reads.set(reads.get() + 1);
                        Some(priority.get())
                    },
                    |_| panic!("an already lowered nested bake must not set priority"),
                    || {
                        assert_eq!(priority.get(), during);
                        calls.set(calls.get() + 1);
                    },
                );
                assert_eq!(priority.get(), during);
            },
        );
        assert_eq!(calls.get(), 2);
        assert_eq!(priority.get(), before);
        let expected_sets = if before == during {
            vec![]
        } else {
            vec![THREAD_PRIORITY_BELOW_NORMAL, before]
        };
        assert_eq!(*sets.borrow(), expected_sets);
        assert_eq!(reads.get(), if before == during { 2 } else { 3 });
    }
}

#[test]
fn an_unreadable_entry_or_failed_set_runs_the_work_once_without_guessing() {
    let calls = Cell::new(0);
    with_priority(
        || None,
        |_| panic!("cannot restore an unknown entry priority"),
        || calls.set(calls.get() + 1),
    );
    let sets = RefCell::new(Vec::new());
    with_priority(
        || Some(2),
        |value| {
            sets.borrow_mut().push(value);
            false
        },
        || calls.set(calls.get() + 1),
    );
    assert_eq!(calls.get(), 2);
    assert_eq!(*sets.borrow(), [THREAD_PRIORITY_BELOW_NORMAL]);
}

#[test]
fn unavailable_or_mismatched_readback_restores_before_doing_the_work() {
    for actual in [None, Some(THREAD_PRIORITY_LOWEST)] {
        let reads = RefCell::new([Some(2), actual].into_iter());
        let priority = Cell::new(2);
        let sets = RefCell::new(Vec::new());
        let calls = Cell::new(0);
        with_priority(
            || reads.borrow_mut().next().unwrap(),
            |value| {
                sets.borrow_mut().push(value);
                priority.set(value);
                true
            },
            || {
                assert_eq!(priority.get(), 2);
                calls.set(calls.get() + 1);
            },
        );
        assert_eq!(*sets.borrow(), [THREAD_PRIORITY_BELOW_NORMAL, 2]);
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn a_failed_restore_does_not_retry_or_discard_the_work_result() {
    let priority = Cell::new(2);
    let sets = RefCell::new(Vec::new());
    let result = with_priority(
        || Some(priority.get()),
        |value| {
            sets.borrow_mut().push(value);
            if value == THREAD_PRIORITY_BELOW_NORMAL {
                priority.set(value);
                true
            } else {
                false
            }
        },
        || {
            assert_eq!(priority.get(), THREAD_PRIORITY_BELOW_NORMAL);
            37
        },
    );
    assert_eq!(result, 37);
    assert_eq!(*sets.borrow(), [THREAD_PRIORITY_BELOW_NORMAL, 2]);
}

#[cfg(windows)]
#[test]
fn windows_bakes_preserve_or_restore_success_error_panic_and_nesting() {
    std::thread::spawn(|| {
        for (before, during) in BAKE_CASES {
            assert!(set_priority(before));
            assert_eq!(super::super::current_priority(), Some(before));
            let calls = Cell::new(0);
            let result = at_bake_priority(|| {
                assert_eq!(super::super::current_priority(), Some(during));
                let result = at_bake_priority(|| {
                    assert_eq!(super::super::current_priority(), Some(during));
                    calls.set(calls.get() + 1);
                    37
                });
                assert_eq!(super::super::current_priority(), Some(during));
                result
            });
            assert_eq!(result, 37);
            assert_eq!(calls.get(), 1);
            assert_eq!(super::super::current_priority(), Some(before));
            let result = at_bake_priority(|| {
                assert_eq!(super::super::current_priority(), Some(during));
                let result = at_bake_priority(|| {
                    assert_eq!(super::super::current_priority(), Some(during));
                    calls.set(calls.get() + 1);
                    Err::<(), _>("bake refused")
                });
                assert_eq!(super::super::current_priority(), Some(during));
                result
            });
            assert_eq!(result, Err("bake refused"));
            assert_eq!(calls.get(), 2);
            assert_eq!(super::super::current_priority(), Some(before));
            let result = catch_unwind(AssertUnwindSafe(|| {
                at_bake_priority(|| {
                    assert_eq!(super::super::current_priority(), Some(during));
                    at_bake_priority(|| {
                        assert_eq!(super::super::current_priority(), Some(during));
                        calls.set(calls.get() + 1);
                        panic!("bake panic");
                    });
                });
            }));
            assert!(result.is_err());
            assert_eq!(calls.get(), 3);
            assert_eq!(super::super::current_priority(), Some(before));
        }
    })
    .join()
    .unwrap();
}

#[cfg(windows)]
#[test]
fn a_bake_preserves_the_existing_waiter_priority_restore_order() {
    std::thread::spawn(|| {
        assert!(set_priority(-2));
        super::super::at_least(Some(1), || {
            assert_eq!(super::super::current_priority(), Some(1));
            at_bake_priority(|| {
                assert_eq!(super::super::current_priority(), Some(-1));
                super::super::at_least(Some(1), || {
                    assert_eq!(super::super::current_priority(), Some(1));
                });
                assert_eq!(super::super::current_priority(), Some(-1));
            });
            assert_eq!(super::super::current_priority(), Some(1));
        });
        assert_eq!(super::super::current_priority(), Some(-2));
    })
    .join()
    .unwrap();
}
