use super::*;
use crate::movement_timing::{EtaCache, Report};
use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    Error,
    Panic,
}

#[derive(Debug, PartialEq, Eq)]
struct Refusal;

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("synthetic ETA refusal")
    }
}

impl std::error::Error for Refusal {}

#[derive(Debug, PartialEq, Eq)]
struct Interrupted;

fn value() -> Etas {
    Etas {
        by_division: [(7, (25_000, 125_000))].into(),
        progress: [(7, 50_000)].into(),
        report: Report {
            estimated: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn build(outcome: Outcome) -> io::Result<Arc<Etas>> {
    match outcome {
        Outcome::Success => Ok(Arc::new(value())),
        Outcome::Error => Err(io::Error::new(io::ErrorKind::InvalidData, Refusal)),
        Outcome::Panic => std::panic::panic_any(Interrupted),
    }
}

fn expect_success(value: &Etas) {
    assert_eq!(value.get(7), Some((25_000, 125_000)));
    assert_eq!(value.progress(7), Ok(50_000));
    assert_eq!(value.report().estimated, 1);
    assert_eq!(value.report().refused_total(), 0);
}

fn expect_outcome(
    result: thread::Result<io::Result<Arc<Etas>>>,
    outcome: Outcome,
) -> Option<Arc<Etas>> {
    match outcome {
        Outcome::Success => {
            let value = result.unwrap().unwrap();
            expect_success(&value);
            Some(value)
        }
        Outcome::Error => {
            let error = result.unwrap().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "synthetic ETA refusal");
            assert_eq!(
                error.get_ref().unwrap().downcast_ref::<Refusal>(),
                Some(&Refusal)
            );
            None
        }
        Outcome::Panic => {
            assert_eq!(
                *result.unwrap_err().downcast::<Interrupted>().unwrap(),
                Interrupted
            );
            None
        }
    }
}

fn collision(outcome: Outcome) {
    let gate = WorkerEtas::default();
    let (waiting, waited) = mpsc::sync_channel(1);
    gate.waiting.set(waiting).unwrap();
    let (entered, entry) = mpsc::sync_channel(0);
    let (release, released) = mpsc::sync_channel(0);
    let builds = AtomicUsize::new(0);
    let mut published = None;
    thread::scope(|scope| {
        let gate_ref = &gate;
        let builds_ref = &builds;
        let leader = scope.spawn(move || {
            gate_ref.get_or_try_init(|| {
                builds_ref.fetch_add(1, Ordering::Relaxed);
                entered.send(()).unwrap();
                released.recv().unwrap();
                build(outcome)
            })
        });
        entry.recv().unwrap();
        let waiter = scope.spawn(|| {
            gate.get_or_try_init(|| {
                builds.fetch_add(1, Ordering::Relaxed);
                build(outcome)
            })
        });
        // The notification follows a real WouldBlock, not a scheduling assumption.
        waited.recv().unwrap();
        assert!(gate.ready.get().is_none());
        release.send(()).unwrap();
        let leader = expect_outcome(leader.join(), outcome);
        let waiter = expect_outcome(waiter.join(), outcome);
        if let (Some(leader), Some(waiter)) = (leader, waiter) {
            assert!(Arc::ptr_eq(&leader, &waiter));
            published = Some(leader);
        }
    });
    assert_eq!(
        builds.load(Ordering::Relaxed),
        if matches!(outcome, Outcome::Success) {
            1
        } else {
            2
        }
    );
    assert_eq!(gate.ready.get().is_some(), published.is_some());
    let recovery = gate
        .get_or_try_init(|| {
            builds.fetch_add(1, Ordering::Relaxed);
            build(Outcome::Success)
        })
        .unwrap();
    expect_success(&recovery);
    if let Some(published) = published {
        assert!(Arc::ptr_eq(&published, &recovery));
    }
    assert_eq!(
        builds.load(Ordering::Relaxed),
        if matches!(outcome, Outcome::Success) {
            1
        } else {
            3
        }
    );
    let warm = gate
        .get_or_try_init(|| -> io::Result<_> { panic!("ready must not build") })
        .unwrap();
    assert!(Arc::ptr_eq(&warm, &recovery));
}

#[test]
fn same_ticket_collision_publishes_one_success() {
    collision(Outcome::Success);
}

#[test]
fn same_ticket_collision_retries_the_same_error_without_erasing_its_payload() {
    collision(Outcome::Error);
}

#[test]
fn same_ticket_collision_propagates_both_panics_then_recovers() {
    collision(Outcome::Panic);
}

#[test]
fn separate_tickets_do_not_wait_on_each_other_or_an_unstarted_owner() {
    let first = WorkerEtas::default();
    let second = WorkerEtas::default();
    let unstarted = WorkerEtas::default();
    let (entered, entry) = mpsc::sync_channel(0);
    let (release, released) = mpsc::sync_channel(0);
    thread::scope(|scope| {
        let first_ref = &first;
        let leader = scope.spawn(move || {
            first_ref.get_or_try_init(|| {
                entered.send(()).unwrap();
                released.recv().unwrap();
                build(Outcome::Success)
            })
        });
        entry.recv().unwrap();
        let other = second.get_or_try_init(|| build(Outcome::Success)).unwrap();
        let lazy = unstarted
            .get_or_try_init(|| build(Outcome::Success))
            .unwrap();
        release.send(()).unwrap();
        let leader = leader.join().unwrap().unwrap();
        expect_success(&leader);
        expect_success(&other);
        expect_success(&lazy);
        assert!(!Arc::ptr_eq(&leader, &other));
        assert!(!Arc::ptr_eq(&other, &lazy));
    });
}

#[test]
fn the_public_cache_does_not_wait_on_a_worker_and_its_first_publication_wins() {
    let gate = WorkerEtas::default();
    let cache = EtaCache::default();
    let (entered, entry) = mpsc::sync_channel(0);
    let (release, released) = mpsc::sync_channel(0);
    thread::scope(|scope| {
        let gate_ref = &gate;
        let cache_ref = &cache;
        let leader = scope.spawn(move || {
            gate_ref.get_or_try_init(|| {
                cache_ref.get_or_insert_with((1, 2), || {
                    entered.send(()).unwrap();
                    released.recv().unwrap();
                    Ok::<_, io::Error>(Etas::default())
                })
            })
        });
        entry.recv().unwrap();
        let public = cache
            .get_or_insert_with((1, 2), || Ok::<_, io::Error>(value()))
            .unwrap();
        release.send(()).unwrap();
        let worker = leader.join().unwrap().unwrap();
        expect_success(&worker);
        assert!(Arc::ptr_eq(&public, &worker));
        let joined = gate
            .get_or_try_init(|| -> io::Result<_> { panic!("already published") })
            .unwrap();
        assert!(Arc::ptr_eq(&public, &joined));
    });
}

#[test]
fn a_ticket_keeps_its_success_and_the_public_cache_its_held_hour() {
    let gate = WorkerEtas::default();
    let cache = EtaCache::default();
    let worker = gate
        .get_or_try_init(|| cache.get_or_insert_with((1, 0), || Ok::<_, io::Error>(value())))
        .unwrap();
    for hour in 1..=8 {
        cache
            .get_or_insert_with((1, hour), || Ok::<_, io::Error>(Etas::default()))
            .unwrap();
    }
    // The ticket holds hour 0, so the cache still answers it with the same derivation.
    assert!(Arc::ptr_eq(&worker, &cache.get((1, 0)).unwrap()));
    let joined = gate
        .get_or_try_init(|| -> io::Result<_> { panic!("same ticket already published") })
        .unwrap();
    expect_success(&joined);
    assert!(Arc::ptr_eq(&worker, &joined));
}
