use super::pump::{Delivery, deliver, emit_error};
use super::*;
use crate::transport::ReplyChunks;
use crate::{
    SourceKey,
    transport::{ProductRequest, SourceReply, bounded_json},
};
use replay::{
    container::Replay,
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    spool::Record,
};
use std::time::Duration;

/// How long a test waits on a service thread before failing instead of
/// hanging; only a hang guard, never a timing check.
const HANG_GUARD: Duration = Duration::from_secs(120);

/// A valid in-memory source needs a session identity, but no map assets,
/// recorded tables or file I/O for these ownership tests.
fn source() -> Arc<Source> {
    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    };
    let replay = Arc::new(
        Replay::from_records(
            b"dashboard-lease-test".to_vec(),
            &[vec![Record::SessionStart(session)]],
        )
        .unwrap(),
    );
    let key = SourceKey {
        source_id: "source-test".into(),
        revision: "revision-test".into(),
    };
    Arc::new(
        Source::from_replay(
            key,
            PathBuf::from("memory.battleplan"),
            replay,
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
}

#[test]
fn air_superiority_source_open_inherits_only_a_verified_same_path_append() {
    use crate::{products::country_groups, test_fixtures};
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("append.battleplan");
    let first = Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(
            1,
            vec![test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, first.bytes()).unwrap();
    let cancelled = AtomicBool::new(false);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let previous = service.inner.open(&path, None, &cancelled).unwrap();
    let cached = country_groups::cached_scan(&previous, 1, &cancelled).unwrap();
    let duplicate = service.inner.open(&path, None, &cancelled).unwrap();
    assert!(Arc::ptr_eq(&previous, &duplicate));
    let added =
        test_fixtures::continuation_records(&first, 1, vec![test_fixtures::source(1, 1, Some(2))]);
    let appended = first.append(&[added]).unwrap();
    std::fs::write(&path, appended.bytes()).unwrap();
    assert!(
        service
            .inner
            .open(&path, Some(&previous.key().revision), &cancelled)
            .is_err()
    );
    let next = service.inner.open(&path, None, &cancelled).unwrap();
    assert!(!Arc::ptr_eq(&previous, &next));
    let inherited = next.state::<country_groups::GroupCache>();
    assert!(Arc::ptr_eq(
        &cached,
        inherited.lock().unwrap().entries[0]
            .1
            .scan
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
    ));
    let extended = country_groups::cached_scan(&next, 1, &cancelled).unwrap();
    assert!(!Arc::ptr_eq(&cached, &extended));
    assert!(Arc::ptr_eq(
        &cached,
        &country_groups::cached_scan(&previous, 1, &cancelled).unwrap()
    ));
    let replacement = Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(
            1,
            vec![test_fixtures::source(1, 0, Some(999))],
        )],
    )
    .unwrap();
    let replacement_modified =
        std::fs::metadata(&path).unwrap().modified().unwrap() + std::time::Duration::from_secs(1);
    std::fs::write(&path, replacement.bytes()).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(replacement_modified)
        .unwrap();
    let replaced = service.inner.open(&path, None, &cancelled).unwrap();
    assert!(
        replaced
            .state::<country_groups::GroupCache>()
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    service.shutdown();
}

#[test]
fn source_open_cancellation_keeps_the_gate_discoverable_for_waiters_and_retries() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("cancelled-open.battleplan");
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![crate::test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, replay.bytes()).unwrap();
    let path = path.canonicalize().unwrap();
    let key = (
        path.clone(),
        crate::protocol::replay_revision(&path).unwrap(),
    );
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let waiter = service.inner.sources.lock().unwrap().open_gate(key.clone());
    let held = waiter.lock().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let request = {
        let inner = Arc::clone(&service.inner);
        let cancelled = Arc::clone(&cancelled);
        std::thread::spawn(move || inner.open(&path, None, &cancelled))
    };
    // Wait on ownership, not elapsed time: cancellation must reach the second
    // check, after this request has joined the already-held revision gate.
    let deadline = std::time::Instant::now() + HANG_GUARD;
    while Arc::strong_count(&waiter) < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "opener did not join the gate"
        );
        std::thread::yield_now();
    }
    cancelled.store(true, Ordering::Release);
    drop(held);
    assert!(request.join().unwrap().err().unwrap().contains("cancelled"));
    let retry = service.inner.sources.lock().unwrap().open_gate(key);
    assert!(
        Arc::ptr_eq(&waiter, &retry),
        "retry detached the pending waiter's gate"
    );
    let weak = Arc::downgrade(&retry);
    drop(retry);
    drop(waiter);
    assert!(
        weak.upgrade().is_none(),
        "idle revision gates must not be retained"
    );
    service.shutdown();
}

#[test]
fn cancellation_survives_serialized_output_tombstone_eviction_and_id_reuse() {
    let mut pending = Pending::default();
    let request = pending.begin("open-1".into()).unwrap();
    let job = OpenJob {
        request: Arc::clone(&request),
        path: "unused".into(),
        revision: "revision".into(),
    };
    let mut chunks = ReplyChunks::new(request.id.clone(), "{\"ready\":true}".into()).unwrap();
    drop(pending.cancel(&request.id));
    assert!(pending.begin(request.id.clone()).is_none());
    for index in 0..MAX_CANCELLED + 1 {
        drop(pending.cancel(&format!("unrelated-{index}")));
    }
    assert_eq!(pending.cancelled.len(), MAX_CANCELLED);
    let replacement = pending.begin(request.id.clone()).unwrap();
    assert!(job.request.cancelled());
    assert!(!pending.current(&job.request));
    drop(pending.finish(&job.request, false));
    assert!(pending.current(&replacement));
    assert_eq!(
        deliver(
            &request,
            &AtomicBool::new(false),
            chunks.next().unwrap(),
            &|_| { panic!("A cancelled serialized reply reached the callback") }
        ),
        Delivery::Cancelled
    );
}

#[test]
fn completed_replies_still_occupy_admission_until_delivery() {
    let mut pending = Pending::default();
    let requests: Vec<_> = (0..MAX_PENDING)
        .map(|index| pending.begin(format!("reply-{index}")).unwrap())
        .collect();
    assert!(pending.begin("over-capacity".into()).is_none());
    let (rejection, message) = pending.errors.pop_front().unwrap();
    assert_eq!(message, crate::transport::CAPACITY_REACHED);
    assert!(crate::transport::transient_error(&message));
    drop(pending.finish(&rejection, false));
    drop(pending.finish(&requests[0], true));
    assert!(pending.begin("replacement".into()).is_some());
    for index in 0..MAX_PENDING * 3 {
        let _ = pending.begin(format!("overflow-{index}"));
    }
    assert_eq!(pending.requests.len(), MAX_PENDING * 2);
    assert_eq!(pending.errors.len(), MAX_PENDING);
}

#[test]
fn late_results_for_cancelled_requests_are_dropped() {
    let mut pending = Pending::default();
    let request = pending.begin("product-1".into()).unwrap();
    drop(pending.cancel(&request.id));
    pending.complete(Arc::clone(&request), Ok("{}".into()));
    assert!(pending.ready.is_empty());
    let current = pending.begin("product-2".into()).unwrap();
    pending.complete(Arc::clone(&current), Err("failed".into()));
    assert_eq!(pending.errors.len(), 1);
    assert!(pending.ready.is_empty());
}

#[test]
fn cancellation_during_handoff_stops_following_fragments() {
    let request = Request::new("reply-1".into());
    let json = bounded_json(&"x".repeat(20_000), &request.cancelled).unwrap();
    let mut chunks = ReplyChunks::new(request.id.clone(), json).unwrap();
    let closed = AtomicBool::new(false);
    assert_eq!(
        deliver(&request, &closed, chunks.next().unwrap(), &|_| {
            request.cancel();
            true
        }),
        Delivery::Cancelled
    );
    assert_eq!(
        deliver(&request, &closed, chunks.next().unwrap(), &|_| {
            panic!("A later fragment followed cancellation during handoff")
        }),
        Delivery::Cancelled
    );
}

#[test]
fn stale_release_cannot_remove_a_lease_from_a_restarted_service() {
    let old = Service::new(Arc::new(|_| true)).unwrap();
    let stale_id = old.inner.lease_id(1);
    old.shutdown();
    let restarted = Service::new(Arc::new(|_| true)).unwrap();
    let current_id = restarted.inner.lease_id(1);
    assert_ne!(stale_id, current_id);
    let source = source();
    let request = restarted.inner.begin("new-open".into()).unwrap();
    restarted
        .inner
        .pending
        .lock()
        .unwrap()
        .source_requests
        .insert(request.id.clone(), current_id.clone());
    restarted
        .inner
        .pending
        .lock()
        .unwrap()
        .leases
        .insert(current_id.clone(), Lease { request, source });
    restarted.dispatch(DataCommand::Release {
        source_id: stale_id,
    });
    assert!(restarted.source(&current_id).is_ok());
    restarted.dispatch(DataCommand::Release {
        source_id: current_id.clone(),
    });
    assert!(restarted.source(&current_id).is_err());
    restarted.shutdown();
}

#[test]
fn unknown_products_and_unleased_sources_fail_before_loading() {
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.dispatch(DataCommand::Product {
        id: "p-1".into(),
        request: ProductRequest {
            source_id: "lease-missing".into(),
            revision: "r".into(),
            product: "national-focus".into(),
            params: serde_json::json!({}),
        },
    });
    service.dispatch(DataCommand::Product {
        id: "p-2".into(),
        request: ProductRequest {
            source_id: "lease-missing".into(),
            revision: "r".into(),
            product: "no-such-product".into(),
            params: serde_json::json!({}),
        },
    });
    let mut errors = Vec::new();
    for _ in 0..2 {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Error { id, message } => errors.push((id, message)),
            other => panic!("unexpected event {other:?}"),
        }
    }
    errors.sort();
    assert!(errors[0].1.contains("unavailable or changed"));
    assert!(errors[1].1.contains("Unknown data product"));
    service.shutdown();
}

#[test]
fn failed_source_reply_rolls_back_before_callback_and_preserves_sibling_lease() {
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let source = source();
    let source_witness = Arc::downgrade(&source);
    let request = service.inner.begin("open-1".into()).unwrap();
    let sibling = service.inner.begin("open-2".into()).unwrap();
    {
        let mut pending = service.inner.pending.lock().unwrap();
        pending
            .source_requests
            .insert(request.id.clone(), "lease-1".into());
        pending.leases.insert(
            "lease-1".into(),
            Lease {
                request: Arc::clone(&request),
                source: Arc::clone(&source),
            },
        );
        pending
            .source_requests
            .insert(sibling.id.clone(), "lease-2".into());
        pending.leases.insert(
            "lease-2".into(),
            Lease {
                request: Arc::clone(&sibling),
                source,
            },
        );
    }
    assert!(emit_error(
        &service.inner,
        &request,
        "Serialization failed".into(),
        &|event| {
            assert!(matches!(event, DataEvent::Error { .. }));
            assert_eq!(
                source_witness.strong_count(),
                1,
                "only the sibling lease may remain at handoff"
            );
            true
        }
    ));
    assert!(
        !service
            .inner
            .pending
            .lock()
            .unwrap()
            .source_requests
            .contains_key(&request.id)
    );
    assert!(service.source("lease-1").is_err());
    assert!(service.source("lease-2").is_ok());
    // Explicitly releasing a not-yet-delivered source also cancels its reply.
    service.dispatch(DataCommand::Release {
        source_id: "lease-2".into(),
    });
    assert!(sibling.cancelled());
    assert!(service.source("lease-2").is_err());
    assert_eq!(source_witness.strong_count(), 0);
    service.shutdown();
}

#[test]
fn failed_delivery_stops_admission_and_shutdown_still_joins() {
    let (called, receive) = mpsc::sync_channel(1);
    let service = Service::new(Arc::new(move |_| {
        called.send(()).unwrap();
        false
    }))
    .unwrap();
    let source = source();
    let request = service.inner.begin("open-1".into()).unwrap();
    let reply = SourceReply {
        id: "lease-1".into(),
        path: "memory.battleplan".into(),
        revision: source.key().revision.clone(),
        metadata: source.metadata(),
    };
    {
        let mut pending = service.inner.pending.lock().unwrap();
        pending
            .source_requests
            .insert(request.id.clone(), reply.id.clone());
        pending.leases.insert(
            reply.id.clone(),
            Lease {
                request: Arc::clone(&request),
                source,
            },
        );
        pending
            .ready
            .push_back((Arc::clone(&request), serde_json::to_string(&reply).unwrap()));
    }
    service.inner.signal.notify();
    receive.recv_timeout(HANG_GUARD).unwrap();
    {
        let guard = service.inner.signal.mutex.lock().unwrap();
        let (guard, _) = service
            .inner
            .signal
            .changed
            .wait_timeout_while(guard, HANG_GUARD, |_| {
                !service.inner.closed.load(Ordering::Acquire)
            })
            .unwrap();
        assert!(service.inner.closed.load(Ordering::Acquire));
        drop(guard);
    }
    assert!(service.inner.begin("after-close".into()).is_none());
    service.shutdown();
    assert!(request.cancelled());
    assert!(service.threads.lock().unwrap().is_empty());
    assert!(service.inner.loader.lock().unwrap().is_none());
    let pending = service.inner.pending.lock().unwrap();
    assert!(pending.requests.is_empty());
    assert!(pending.leases.is_empty());
    assert!(pending.source_requests.is_empty());
    assert!(pending.ready.is_empty());
    assert!(pending.errors.is_empty());
}

#[test]
fn air_superiority_source_cache_fork_never_blocks_another_path() {
    use crate::test_fixtures;
    use std::sync::Condvar;
    #[derive(Default)]
    struct BlockedClone {
        entered: Option<std::sync::mpsc::Sender<()>>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }
    impl Clone for BlockedClone {
        fn clone(&self) -> Self {
            if let Some(entered) = &self.entered {
                entered.send(()).unwrap();
            }
            let (lock, ready) = &*self.release;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
            Self {
                entered: None,
                release: Arc::clone(&self.release),
            }
        }
    }
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("blocked-append.battleplan");
    let other_path = folder.0.join("other-source.battleplan");
    let first = Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(
            1,
            vec![test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, first.bytes()).unwrap();
    std::fs::write(&other_path, first.bytes()).unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let previous = service
        .inner
        .open(&path, None, &AtomicBool::new(false))
        .unwrap();
    let (entered, seen) = std::sync::mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    *previous.append_state::<BlockedClone>().lock().unwrap() = BlockedClone {
        entered: Some(entered),
        release: Arc::clone(&release),
    };
    let appended = first
        .append(&[test_fixtures::continuation_records(
            &first,
            1,
            vec![test_fixtures::source(1, 1, Some(2))],
        )])
        .unwrap();
    std::fs::write(&path, appended.bytes()).unwrap();
    let inner = Arc::clone(&service.inner);
    let blocked = std::thread::spawn(move || inner.open(&path, None, &AtomicBool::new(false)));
    seen.recv_timeout(HANG_GUARD).unwrap();
    let inner = Arc::clone(&service.inner);
    let (sent, result) = std::sync::mpsc::channel();
    let other = std::thread::spawn(move || {
        sent.send(inner.open(&other_path, None, &AtomicBool::new(false)))
            .unwrap();
    });
    let opened = result.recv_timeout(HANG_GUARD);
    *release.0.lock().unwrap() = true;
    release.1.notify_all();
    assert!(blocked.join().unwrap().is_ok());
    other.join().unwrap();
    assert!(
        opened.unwrap().is_ok(),
        "a product clone must not hold the sources lock"
    );
    service.shutdown();
}
