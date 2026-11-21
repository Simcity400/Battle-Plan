use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn counted(capacity: &Arc<Capacity>) -> (CapacityListener, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    (
        capacity.listen(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }),
        calls,
    )
}

#[test]
fn capacity_generation_coalesces_until_acknowledged() {
    let capacity = Arc::new(Capacity::default());
    let (listener, calls) = counted(&capacity);
    assert_eq!(listener.acknowledge(), 0);
    capacity.dequeued();
    capacity.dequeued();
    assert_eq!(listener.generation(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acknowledge(), 2);
    capacity.dequeued();
    assert_eq!(listener.generation(), 3);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn capacity_full_handshake_covers_all_dequeue_interleavings() {
    for point in 0..4 {
        let capacity = Arc::new(Capacity::default());
        let (notify, wakes) = mpsc::channel();
        let listener = capacity.listen(move || {
            notify.send(()).unwrap();
        });
        let (jobs, inbox) = mpsc::sync_channel(1);
        jobs.try_send(()).unwrap();
        let dequeue_and_refill = || {
            inbox.try_recv().unwrap();
            capacity.dequeued();
            jobs.try_send(()).unwrap();
        };
        if point == 0 {
            dequeue_and_refill();
            wakes.try_recv().unwrap();
        }
        let before = listener.acknowledge();
        if point == 1 {
            dequeue_and_refill();
        }
        assert!(matches!(
            jobs.try_send(()),
            Err(mpsc::TrySendError::Full(()))
        ));
        if point == 2 {
            dequeue_and_refill();
        }
        let changed = listener.generation() != before;
        if point == 3 {
            dequeue_and_refill();
        }
        if point == 0 {
            assert!(!changed);
            assert!(matches!(wakes.try_recv(), Err(mpsc::TryRecvError::Empty)));
            inbox.try_recv().unwrap();
            capacity.dequeued();
            wakes.try_recv().unwrap();
            jobs.try_send(()).unwrap();
        } else {
            assert!(changed || point == 3);
            wakes.try_recv().unwrap();
        }
    }
}

#[test]
fn capacity_two_subscribers_rearm_independently_and_teardown() {
    let capacity = Arc::new(Capacity::default());
    let (first, a) = counted(&capacity);
    let (second, b) = counted(&capacity);
    capacity.dequeued();
    first.acknowledge();
    capacity.dequeued();
    assert_eq!(a.load(Ordering::SeqCst), 2);
    assert_eq!(b.load(Ordering::SeqCst), 1);
    drop(first);
    second.acknowledge();
    capacity.dequeued();
    assert_eq!(a.load(Ordering::SeqCst), 2);
    assert_eq!(b.load(Ordering::SeqCst), 2);
    drop(second);
    capacity.dequeued();
    assert_eq!(b.load(Ordering::SeqCst), 2);
}

#[test]
fn capacity_callbacks_can_subscribe_and_unsubscribe_outside_registry_lock() {
    let capacity = Arc::new(Capacity::default());
    let inner = capacity.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let listener = capacity.listen(move || {
        let temporary = inner.listen(|| {});
        drop(temporary);
        count.fetch_add(1, Ordering::SeqCst);
    });
    capacity.dequeued();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(listener);
    let generation = capacity.listen(|| {});
    let before = generation.generation();
    drop(generation);
    capacity.dequeued();
    let again = capacity.listen(|| {});
    assert_eq!(again.generation(), before);
}

#[test]
fn capacity_panicking_callback_does_not_suppress_an_accepted_pool_job() {
    let pool = Pool::new(1).unwrap();
    let _listener = pool.listen_capacity(|| panic!("synthetic wake failure"));
    let (done, completed) = mpsc::channel();
    pool.submit(Box::new(move || {
        done.send(17).unwrap();
    }));
    assert_eq!(completed.recv().unwrap(), 17);
}

#[test]
fn capacity_listener_fires_on_dequeue_before_job_execution() {
    let pool = Pool::new(1).unwrap();
    let (order, received) = mpsc::channel();
    let notify = order.clone();
    let _listener = pool.listen_capacity(move || {
        notify.send("dequeue").unwrap();
    });
    pool.submit(Box::new(move || {
        order.send("job").unwrap();
    }));
    assert_eq!(received.recv().unwrap(), "dequeue");
    assert_eq!(received.recv().unwrap(), "job");
}

#[test]
fn capacity_status_preserves_full_and_disconnected_jobs_without_running_them() {
    let (jobs, inbox) = mpsc::sync_channel::<Job>(1);
    let pool = Pool {
        jobs: Some(jobs),
        threads: Vec::new(),
        capacity: Arc::new(Capacity::default()),
    };
    pool.submit(Box::new(|| {}));
    let (done, received) = mpsc::channel();
    let notify = done.clone();
    let full = match pool.try_submit_status(Box::new(move || {
        notify.send(1).unwrap();
    })) {
        Err(TrySubmitError::Full(job)) => job,
        _ => panic!("full must preserve its job"),
    };
    assert!(matches!(
        received.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    full();
    assert_eq!(received.recv().unwrap(), 1);
    drop(inbox);
    let disconnected = match pool.try_submit_status(Box::new(move || {
        done.send(2).unwrap();
    })) {
        Err(TrySubmitError::Disconnected(job)) => job,
        _ => panic!("disconnected must preserve its job"),
    };
    disconnected();
    assert_eq!(received.recv().unwrap(), 2);
}
