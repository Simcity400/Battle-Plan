use super::*;
use std::sync::{atomic::AtomicUsize, mpsc};

type Source = Mailbox<i32, Vec<u8>, u64>;

fn source() -> (Arc<Source>, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let count = wakes.clone();
    (
        Source::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            true
        }),
        wakes,
    )
}

#[test]
fn cancelled_pending_reads_acknowledge_once_without_more_wakes() {
    let (source, wakes) = source();
    let (send, received) = mpsc::channel();
    let first = source.post(1, 10, move |id, reply| send.send((id, reply)).unwrap());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    first.cancel();
    assert!(matches!(
        received.try_recv().unwrap(),
        (1, Reply::Cancelled)
    ));
    first.cancel();
    assert!(received.try_recv().is_err());
    let (send, received) = mpsc::channel();
    let _second = source.post(2, 11, move |id, reply| send.send((id, reply)).unwrap());
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "reuse already queued wake");
    let read = source.take().unwrap();
    assert_eq!(read.id, 2);
    assert_eq!(read.key, 11);
    read.ready(vec![1, 2, 3], Some(9));
    assert!(matches!(received.try_recv().unwrap(),
        (2, Reply::Ready { value, sample: Some(9) }) if value == [1, 2, 3]));
    assert!(source.take().is_none());
}

#[test]
fn active_read_credit_is_not_acknowledged_before_service_finishes() {
    let (source, wakes) = source();
    let (send, received) = mpsc::channel();
    let first = source.post(1, 10, move |id, reply| send.send((id, reply)).unwrap());
    let active = source.take().unwrap();
    first.cancel();
    assert!(active.cancelled());
    assert!(
        received.try_recv().is_err(),
        "active decode has not returned"
    );
    active.ready(vec![10], None);
    assert!(matches!(
        received.try_recv().unwrap(),
        (1, Reply::Cancelled)
    ));
    let (send, received) = mpsc::channel();
    let _next = source.post(2, 12, move |id, reply| send.send((id, reply)).unwrap());
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    drop(source.take());
    assert!(matches!(received.try_recv().unwrap(), (2, Reply::Stopped)));
}

#[test]
fn replacement_and_stop_keep_one_terminal_reply_per_read() {
    let (source, wakes) = source();
    let (send, received) = mpsc::channel();
    let notify = send.clone();
    let _first = source.post(1, 10, move |id, reply| notify.send((id, reply)).unwrap());
    let _second = source.post(2, 11, move |id, reply| send.send((id, reply)).unwrap());
    assert!(matches!(
        received.try_recv().unwrap(),
        (1, Reply::Cancelled)
    ));
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    source.stop();
    assert!(matches!(received.try_recv().unwrap(), (2, Reply::Stopped)));
    source.stop();
    assert!(received.try_recv().is_err());
}

#[test]
fn stopped_actor_and_callback_panic_do_not_strand_later_requests() {
    let source: Arc<Source> = Source::new(|_| false);
    let (send, received) = mpsc::channel();
    let _read = source.post(1, 10, move |id, reply| send.send((id, reply)).unwrap());
    assert!(matches!(received.try_recv().unwrap(), (1, Reply::Stopped)));
    let _read = source.post(2, 11, |_, _| panic!("synthetic event receiver failure"));
    assert!(source.take().is_none());
    let (send, received) = mpsc::channel();
    let _read = source.post(3, 12, move |id, reply| send.send((id, reply)).unwrap());
    assert!(matches!(received.try_recv().unwrap(), (3, Reply::Stopped)));
}

#[test]
fn terminal_callback_can_reenter_the_mailbox_without_its_lock() {
    let (source, _) = source();
    let reentrant = Arc::downgrade(&source);
    let _read = source.post(1, 10, move |_, reply| {
        assert!(matches!(reply, Reply::Cancelled));
        let source = reentrant.upgrade().unwrap();
        assert_eq!(source.pending_key(), Some(11));
    });
    let (send, received) = mpsc::channel();
    let _read = source.post(2, 11, move |id, reply| send.send((id, reply)).unwrap());
    let active = source.take().unwrap();
    active.ready(vec![11], Some(44));
    assert!(matches!(
        received.try_recv().unwrap(),
        (
            2,
            Reply::Ready {
                sample: Some(44),
                ..
            }
        )
    ));
}

#[test]
fn stopped_mailbox_refuses_future_requests_even_if_its_wake_transport_is_alive() {
    let (source, wakes) = source();
    source.stop();
    let (send, received) = mpsc::channel();
    let _handle = source.post(1, 10, move |id, reply| send.send((id, reply)).unwrap());
    assert!(matches!(received.try_recv().unwrap(), (1, Reply::Stopped)));
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
    assert!(source.take().is_none());
}

#[test]
fn stale_handle_cannot_cancel_a_replacement_with_the_same_target_ticket() {
    let (source, _) = source();
    let (send, received) = mpsc::channel();
    let first = source.post(1, 10, |_, reply| assert!(matches!(reply, Reply::Cancelled)));
    let _next = source.post(1, 11, move |id, reply| send.send((id, reply)).unwrap());
    first.cancel();
    assert_eq!(source.pending_key(), Some(11));
    source.take().unwrap().ready(vec![11], None);
    assert!(
        matches!(received.try_recv().unwrap(), (1, Reply::Ready { value, .. }) if value == [11])
    );
}
