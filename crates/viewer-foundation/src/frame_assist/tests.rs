use super::*;
use crate::{builder_pool::Pool, parallel::usable_processors};
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

struct Busy;
impl Spawn for Busy {
    fn try_spawn(&self, job: Job) -> Result<(), Job> {
        Err(job)
    }
    fn threads(&self) -> usize {
        3
    }
}

#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn wait(&self) {
        let mut open = self.0.lock().unwrap();
        while !*open {
            open = self.1.wait(open).unwrap();
        }
    }
    fn release(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[test]
fn capacity_uses_headroom_without_increasing_advertised_width() {
    for (p, b, f) in [
        (1, 2, 0),
        (2, 2, 1),
        (4, 2, 1),
        (8, 2, 2),
        (16, 8, 3),
        (32, 24, 3),
    ] {
        assert_eq!(helper_count(p, b), f);
    }
    for p in 0usize..128 {
        for b in 0usize..128 {
            assert!(helper_count(p, b) <= p.saturating_sub(1));
        }
    }
    let service = FrameAssist::with_helpers(Arc::new(Busy), 1).unwrap();
    assert_eq!(service.threads(), 3);
    assert_eq!(
        Workers::new(Arc::new(service)).threads(),
        4.min(usable_processors())
    );
}

#[test]
fn protected_turn_progresses_while_every_builder_is_blocked() {
    if usable_processors() < 2 {
        return;
    }
    let builders = Arc::new(Pool::new(2).unwrap());
    let gate = Arc::new(Gate::default());
    let release = Release(Arc::clone(&gate));
    let entered = Arc::new(Barrier::new(3));
    for _ in 0..2 {
        let gate = Arc::clone(&gate);
        let entered = Arc::clone(&entered);
        builders.submit(Box::new(move || {
            entered.wait();
            gate.wait();
        }));
    }
    entered.wait();
    let service = Arc::new(FrameAssist::with_helpers(builders.clone(), 1).unwrap());
    let workers = Workers::new(service.clone()).limited(2);
    let both = Barrier::new(2);
    let seen = Mutex::new(Vec::new());
    workers.run(2, &|index| {
        both.wait();
        seen.lock().unwrap().push((index, thread::current().id()));
    });
    let mut seen = seen.into_inner().unwrap();
    seen.sort_by_key(|v| v.0);
    assert_eq!(seen.iter().map(|v| v.0).collect::<Vec<_>>(), [0, 1]);
    assert_ne!(seen[0].1, seen[1].1);
    service.wait_idle_for_tests();
    drop(release);
}

#[test]
fn occupied_protected_slot_uses_builders_then_returns_the_untouched_job() {
    let builders = crate::parallel::tests::threads(1);
    let service = FrameAssist::with_helpers(builders, 1).unwrap();
    let gate = Arc::new(Gate::default());
    let release = Release(gate.clone());
    let (sent, entered) = mpsc::channel();
    assert_eq!(
        service
            .try_spawn_classified(Box::new(move || {
                sent.send(()).unwrap();
                gate.wait();
            }))
            .ok(),
        Some(HelperSource::Protected)
    );
    entered.recv().unwrap();
    let (sent, ran) = mpsc::channel();
    assert_eq!(
        service
            .try_spawn_classified(Box::new(move || {
                sent.send(()).unwrap();
            }))
            .ok(),
        Some(HelperSource::Builder)
    );
    ran.recv().unwrap();
    drop(release);
    service.wait_idle_for_tests();

    let service = Arc::new(FrameAssist::with_helpers(Arc::new(Busy), 1).unwrap());
    let gate = Arc::new(Gate::default());
    let release = Release(gate.clone());
    assert!(service.try_spawn(Box::new(move || gate.wait())).is_ok());
    let caller = thread::current().id();
    let seen: Vec<_> = (0..257).map(|_| AtomicUsize::new(0)).collect();
    Workers::new(service.clone()).run(257, &|i| {
        assert_eq!(thread::current().id(), caller);
        seen[i].fetch_add(1, Ordering::Relaxed);
    });
    assert!(seen.iter().all(|n| n.load(Ordering::Relaxed) == 1));
    let called = Arc::new(AtomicUsize::new(0));
    let owned = called.clone();
    let job = service
        .try_spawn(Box::new(move || {
            owned.fetch_add(1, Ordering::Relaxed);
        }))
        .err()
        .unwrap();
    assert_eq!(called.load(Ordering::Relaxed), 0);
    job();
    assert_eq!(called.load(Ordering::Relaxed), 1);
    drop(release);
    service.wait_idle_for_tests();
}

#[test]
fn hybrid_batches_keep_exact_once_and_indexed_result_order() {
    let builders = crate::parallel::tests::threads(3);
    let service = Arc::new(FrameAssist::with_helpers(builders, 1).unwrap());
    for limit in 1..=4 {
        let workers = Workers::new(service.clone()).limited(limit);
        for count in [0, 1, 2, 7, 4096] {
            let seen: Vec<_> = (0..count).map(|_| AtomicUsize::new(0)).collect();
            workers.run(count, &|i| {
                seen[i].fetch_add(i + 1, Ordering::Relaxed);
            });
            assert!(
                seen.iter()
                    .enumerate()
                    .all(|(i, n)| n.load(Ordering::Relaxed) == i + 1)
            );
        }
    }
}

#[derive(Default)]
struct Queued(Mutex<Vec<Job>>);
impl Spawn for Queued {
    fn try_spawn(&self, job: Job) -> Result<(), Job> {
        self.0.lock().unwrap().push(job);
        Ok(())
    }
    fn threads(&self) -> usize {
        3
    }
}
impl Queued {
    fn drain(&self) {
        for job in std::mem::take(&mut *self.0.lock().unwrap()) {
            job();
        }
    }
}

#[test]
fn late_builder_jobs_never_touch_expired_run_or_admission_borrows() {
    let builders = Arc::new(Queued::default());
    let service = Arc::new(FrameAssist::with_helpers(builders.clone(), 0).unwrap());
    let workers = Workers::new(service);
    let seen = AtomicUsize::new(0);
    workers.run_beside(
        9,
        &|_| {
            seen.fetch_add(1, Ordering::Relaxed);
        },
        Some(&|_| panic!("late beside")),
    );
    assert_eq!(seen.load(Ordering::Relaxed), 9);
    builders.drain();
    workers.with_admitted(4, |a| {
        assert_eq!(
            a.run(4, &|_, _| {
                seen.fetch_add(1, Ordering::Relaxed);
            }),
            1
        );
    });
    builders.drain();
    assert_eq!(seen.load(Ordering::Relaxed), 10);
    workers.with_admitted(4, |_| {});
    builders.drain();
}

#[test]
fn admission_abandon_and_panics_release_protected_capacity_for_later_batches() {
    if usable_processors() < 2 {
        return;
    }
    let service = Arc::new(FrameAssist::with_helpers(Arc::new(Busy), 1).unwrap());
    let workers = Workers::new(service.clone()).limited(2);
    for panic in [false, true] {
        let result = catch_unwind(AssertUnwindSafe(|| {
            workers.with_admitted(2, |a| {
                a.wait_for_helpers(1);
                assert!(!panic, "preparation refused");
            });
        }));
        assert_eq!(result.is_err(), panic);
        service.wait_idle_for_tests();
    }
    for helper_panics in [false, true] {
        let caller = thread::current().id();
        let both = Barrier::new(2);
        let result = catch_unwind(AssertUnwindSafe(|| {
            workers.with_admitted(2, |a| {
                a.wait_for_helpers(1);
                a.run(2, &|_, count| {
                    assert_eq!(count, 2);
                    both.wait();
                    assert_ne!(
                        thread::current().id() != caller,
                        helper_panics,
                        "task refused"
                    );
                });
            });
        }));
        assert!(result.is_err());
        service.wait_idle_for_tests();
    }
    let seen = AtomicUsize::new(0);
    workers.run(17, &|_| {
        seen.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(seen.load(Ordering::Relaxed), 17);
}

#[test]
fn protected_beside_is_helper_only_and_finishes_before_scope_return() {
    if usable_processors() < 2 {
        return;
    }
    let service = Arc::new(FrameAssist::with_helpers(Arc::new(Busy), 1).unwrap());
    let workers = Workers::new(service.clone()).limited(2);
    let caller = thread::current().id();
    let (sent, received) = mpsc::channel();
    let received = Mutex::new(received);
    let hits = AtomicUsize::new(0);
    workers.with_admitted(2, |a| {
        a.wait_for_helpers(1);
        assert_eq!(
            a.run_beside(
                2,
                &|index, _| {
                    if index == 0 {
                        received.lock().unwrap().recv().unwrap();
                    }
                },
                Some(&|done: &dyn Fn() -> bool| {
                    assert_ne!(thread::current().id(), caller);
                    hits.fetch_add(1, Ordering::Relaxed);
                    sent.send(()).unwrap();
                    while !done() {
                        thread::yield_now();
                    }
                })
            ),
            2
        );
    });
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    service.wait_idle_for_tests();
}

#[test]
fn protected_transport_panic_does_not_strand_the_slot_and_drop_drains_owned_work() {
    let service = FrameAssist::with_helpers(Arc::new(Busy), 1).unwrap();
    assert!(
        service
            .try_spawn(Box::new(|| panic!("owned transport")))
            .is_ok()
    );
    service.wait_idle_for_tests();
    let ran = Arc::new(AtomicUsize::new(0));
    let owned = ran.clone();
    assert!(
        service
            .try_spawn(Box::new(move || {
                owned.fetch_add(1, Ordering::Relaxed);
            }))
            .is_ok()
    );
    drop(service);
    assert_eq!(ran.load(Ordering::Relaxed), 1);
}

#[test]
fn protected_threads_initialize_normal_priority_and_restore_borrowed_boosts() {
    let service = FrameAssist::with_helpers(Arc::new(Busy), 1).unwrap();
    let (sent, received) = mpsc::channel();
    assert!(
        service
            .try_spawn(Box::new(move || {
                let before = crate::worker_priority::current_priority();
                crate::worker_priority::at_least(Some(1), || {
                    #[cfg(windows)]
                    assert_eq!(crate::worker_priority::current_priority(), Some(1));
                });
                sent.send((before, crate::worker_priority::current_priority()))
                    .unwrap();
            }))
            .is_ok()
    );
    let (before, after) = received.recv().unwrap();
    #[cfg(windows)]
    assert_eq!(before, Some(0));
    assert_eq!(before, after);
}
