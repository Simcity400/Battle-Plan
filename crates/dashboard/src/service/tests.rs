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

#[test]
fn panicking_recording_requests_report_errors_and_leave_loaders_running() {
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    for stage in ["open", "metadata", "product"] {
        let id = format!("panic-{stage}");
        let request = service.inner.begin(id.clone()).unwrap();
        service.inner.submit(
            Arc::clone(&request),
            LoadJob::Fixture {
                request,
                interactive: false,
                run: Box::new(move || panic!("synthetic {stage} panic")),
            },
        );
        assert!(matches!(
            receive.recv_timeout(HANG_GUARD).unwrap(),
            DataEvent::Error { id: failed, message } if failed == id && message.contains("Dashboard request failed")
        ));
        assert!(!service.inner.closed.load(Ordering::Acquire));
    }
    // Use the real open/metadata path and then a real product after those panics.
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("after-panic.battleplan");
    let valid = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![crate::test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, valid.bytes()).unwrap();
    service.dispatch(DataCommand::Open {
        id: "valid-open".into(),
        path: path.to_string_lossy().into_owned(),
        revision: crate::protocol::replay_revision(&path).unwrap(),
    });
    let mut json = String::new();
    loop {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Chunk {
                id, fragment, last, ..
            } => {
                assert_eq!(id, "valid-open");
                json.push_str(&fragment);
                if last {
                    break;
                }
            }
            event => panic!("unexpected open event: {event:?}"),
        }
    }
    let opened: SourceReply = serde_json::from_str(&json).unwrap();
    assert!(service.source(&opened.id).is_ok());
    service.dispatch(DataCommand::Product {
        id: "valid-product".into(),
        request: ProductRequest {
            source_id: opened.id,
            revision: opened.revision,
            product: "ships-sunk".into(),
            params: serde_json::json!({"branchId": "1"}),
        },
    });
    loop {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Chunk { id, last, .. } => {
                assert_eq!(id, "valid-product");
                if last {
                    break;
                }
            }
            event => panic!("unexpected product event: {event:?}"),
        }
    }
    assert!(!service.inner.closed.load(Ordering::Acquire));
    service.shutdown();
}

/// A valid in-memory source needs a session identity, but no map assets,
/// recorded tables or file I/O for these ownership tests.
fn source() -> Arc<Source> {
    source_with_id("source-test")
}

fn source_with_id(id: &str) -> Arc<Source> {
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
        source_id: id.into(),
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
fn preparation_commands_preserve_lease_identity_and_exact_progress_counts() {
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let source = source();
    let open_request = Request::new("prepared-open".into());
    service.inner.pending.lock().unwrap().leases.insert(
        "prepared-lease".into(),
        Lease {
            request: open_request,
            source: Arc::clone(&source),
        },
    );
    let exact = "9007199254740993";
    {
        let progress = source.state::<crate::prepared::Progress>();
        let mut progress = progress.lock().unwrap();
        progress.state = crate::prepared::State::Ready;
        progress.bytes = exact.parse().unwrap();
        progress.sections.push(crate::prepared::Section {
            id: "focus".into(),
            label: "National focus".into(),
            state: crate::prepared::State::Ready,
            done: exact.into(),
            total: Some(exact.into()),
            error: None,
        });
    }
    service.dispatch(DataCommand::Preparation {
        id: "prepared-status".into(),
        source_id: "prepared-lease".into(),
        revision: source.key().revision.clone(),
        action: crate::prepared::Action::Status,
    });
    let mut json = String::new();
    loop {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Chunk {
                id, fragment, last, ..
            } => {
                assert_eq!(id, "prepared-status");
                json.push_str(&fragment);
                if last {
                    break;
                }
            }
            event => panic!("unexpected preparation event: {event:?}"),
        }
    }
    let reply: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(reply["sourceId"], "prepared-lease");
    assert_eq!(reply["revision"], source.key().revision);
    assert_eq!(reply["requestId"], "prepared-status");
    assert_eq!(reply["state"], "ready");
    assert_eq!(reply["bytes"], exact);
    assert_eq!(reply["sections"][0]["done"], exact);
    assert_eq!(reply["sections"][0]["total"], exact);
    service.dispatch(DataCommand::Preparation {
        id: "stale-cancel".into(),
        source_id: "prepared-lease".into(),
        revision: "previous-revision".into(),
        action: crate::prepared::Action::Cancel,
    });
    assert!(matches!(receive.recv_timeout(HANG_GUARD).unwrap(),
        DataEvent::Error { id, message } if id == "stale-cancel" && message.contains("unavailable or changed")));
    assert!(
        !source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    service.dispatch(DataCommand::PreparedQuery {
        id: "stale-page".into(),
        request: ProductRequest {
            source_id: "prepared-lease".into(),
            revision: "previous-revision".into(),
            product: "focus-hour".into(),
            params: serde_json::json!({"branchId":"1", "tag":"synthetic", "hour":0}),
        },
    });
    assert!(matches!(receive.recv_timeout(HANG_GUARD).unwrap(),
        DataEvent::Error { id, message } if id == "stale-page" && message.contains("unavailable or changed")));
    service.shutdown();
}

#[test]
fn disabling_background_work_preserves_explicit_preparation_and_clear() {
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let automatic = source();
    let manual = source();
    for (key, source, is_automatic) in [("automatic", &automatic, true), ("manual", &manual, false)]
    {
        let progress = source.state::<crate::prepared::Progress>();
        let mut progress = progress.lock().unwrap();
        progress.running = true;
        progress.state = crate::prepared::State::Preparing;
        progress.automatic = is_automatic;
        service
            .inner
            .sources
            .lock()
            .unwrap()
            .entries
            .insert((PathBuf::from(key), key.into()), Arc::downgrade(source));
    }
    assert!(service.preparation_preferences().background_enabled);
    service.set_background_preparation(false);
    assert!(!service.preparation_preferences().background_enabled);
    assert!(
        automatic
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    assert!(
        !manual
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    // An explicit Start remains valid while the global background default is off.
    service
        .inner
        .start_preparation(Arc::clone(&manual), false, &AtomicBool::new(false))
        .unwrap();
    assert!(
        !manual
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .automatic
    );
    // Automatic admission never opens storage after this source was cleared.
    let cleared = source();
    cleared
        .state::<crate::prepared::Progress>()
        .lock()
        .unwrap()
        .cleared = true;
    service.set_background_preparation(true);
    service
        .inner
        .start_preparation(Arc::clone(&cleared), true, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        cleared
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .state,
        crate::prepared::State::Idle
    );
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn damaged_opens_use_sidebar_sentences_and_valid_opens_keep_their_metadata() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let valid = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![crate::test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    for (name, bytes) in [
        ("magic", {
            let mut bytes = valid.bytes().to_vec();
            bytes[0] ^= 255;
            bytes
        }),
        ("truncated", valid.bytes()[..5].to_vec()),
        ("declared-size", {
            let mut bytes = valid.bytes().to_vec();
            let footer = bytes.len() - 24;
            bytes[footer + 12..footer + 20].copy_from_slice(&u64::MAX.to_le_bytes());
            bytes
        }),
    ] {
        let path = folder.0.join(format!("{name}.battleplan"));
        std::fs::write(&path, bytes).unwrap();
        let error = service.open_map(&path, None).err().unwrap();
        assert!(
            error.starts_with(
                "This recording is damaged or isn't a Battle Plan recording. Details: "
            ),
            "{name}: {error}"
        );
    }
    let path = folder.0.join("valid.battleplan");
    std::fs::write(&path, valid.bytes()).unwrap();
    let source = service.open_map(&path, None).unwrap();
    assert!(source.replay_ref().is_file_backed());
    let expected = crate::metadata::read(
        &Arc::new(valid),
        &path.canonicalize().unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(source.metadata()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    let path = folder.0.join("modified-crc.battleplan");
    std::fs::write(&path, bytes).unwrap();
    let modified = service.open_map(&path, None).unwrap();
    assert!(modified.replay_ref().is_file_backed());
    let metadata = modified.metadata();
    assert!(metadata.damaged.is_some());
    assert!(
        metadata
            .warnings
            .iter()
            .any(|reason| reason.contains("trailer CRC mismatch"))
    );
    assert_eq!(metadata.countries, source.metadata().countries);
    assert_eq!(metadata.branches, source.metadata().branches);
    service.shutdown();
}

struct LibraryFixture {
    _folder: crate::catalogue::tests::Folder,
    paths: Vec<PathBuf>,
    replay: Replay,
}
impl LibraryFixture {
    fn new(names: &[&str]) -> Self {
        let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
        let first = Replay::from_records(
            crate::test_fixtures::CAMPAIGN.into(),
            &[crate::test_fixtures::records(
                1,
                vec![crate::test_fixtures::source(1, 0, Some(1))],
            )],
        )
        .unwrap();
        let replay = first
            .append(&[crate::test_fixtures::records(
                2,
                vec![crate::test_fixtures::source(2, 10, Some(2))],
            )])
            .unwrap();
        let paths = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let path = folder.0.join(format!("{name}.battleplan"));
                std::fs::write(&path, replay.bytes()).unwrap();
                let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                file.set_times(std::fs::FileTimes::new().set_modified(
                    std::time::UNIX_EPOCH + Duration::from_secs(1_000_000 + index as u64),
                ))
                .unwrap();
                path.canonicalize().unwrap()
            })
            .collect();
        Self {
            _folder: folder,
            paths,
            replay,
        }
    }
    fn append(&self, path: &Path) {
        self.replay
            .append(&[crate::test_fixtures::records(
                3,
                vec![crate::test_fixtures::source(3, 20, Some(3))],
            )])
            .unwrap()
            .write_atomic(path)
            .unwrap();
    }
}
impl Drop for LibraryFixture {
    fn drop(&mut self) {
        for path in &self.paths {
            if let Ok(store) = crate::prepared::recording_store(path) {
                let _ = store.clear();
            }
        }
    }
}

fn wait_library_source(source: &Source) {
    let progress = source.state::<crate::prepared::Progress>();
    let finished = Arc::clone(&progress.lock().unwrap().finished);
    let done = finished.0.lock().unwrap();
    let (done, _) = finished
        .1
        .wait_timeout_while(done, HANG_GUARD, |done| !*done)
        .unwrap();
    assert!(*done, "preparation did not finish");
    assert_eq!(
        progress.lock().unwrap().state,
        crate::prepared::State::Ready
    );
}

/// Occupy the real workers so queue order and cancellation are deterministic.
struct PreparationHold(Vec<Arc<AtomicBool>>);
impl PreparationHold {
    fn new(service: &Service) -> Self {
        let (sent, seen) = mpsc::channel();
        let flags: Vec<_> = (0..2).map(|_| Arc::new(AtomicBool::new(false))).collect();
        for flag in &flags {
            let sent = sent.clone();
            service
                .inner
                .preparation
                .submit(Arc::clone(flag), move |cancelled| {
                    sent.send(()).unwrap();
                    while !cancelled.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                })
                .unwrap();
        }
        for _ in &flags {
            seen.recv_timeout(HANG_GUARD).unwrap();
        }
        Self(flags)
    }
}
impl Drop for PreparationHold {
    fn drop(&mut self) {
        for flag in &self.0 {
            flag.store(true, Ordering::Release);
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn library_preparation_waits_for_game_exit_and_prepares_unopened_published_sessions() {
    let fixture = LibraryFixture::new(&["unopened"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    service.set_preparation_suspended(true);
    service.prepare_library(&fixture.paths).unwrap();
    assert!(service.inner.sources.lock().unwrap().entries.is_empty());
    service.set_preparation_suspended(false);
    service.prepare_library(&fixture.paths).unwrap();
    let source = service.open_map(&fixture.paths[0], None).unwrap();
    wait_library_source(&source);
    assert!(service.inner.pending.lock().unwrap().leases.is_empty());
    assert_eq!(
        crate::prepared::unprepared_blocks(&source, None).unwrap(),
        Some(0)
    );
    service.prepare_library(&fixture.paths).unwrap();
    assert!(
        service.inner.sources.lock().unwrap().library.is_empty(),
        "ready library sources are released"
    );
    fixture.append(&fixture.paths[0]);
    service.set_preparation_suspended(true);
    service.prepare_library(&fixture.paths).unwrap();
    assert_eq!(
        service
            .inner
            .sources
            .lock()
            .unwrap()
            .completed
            .get(&fixture.paths[0]),
        Some(&source.key().revision)
    );
    service.set_preparation_suspended(false);
    service.prepare_library(&fixture.paths).unwrap();
    let grown = service.open_map(&fixture.paths[0], None).unwrap();
    assert_ne!(grown.key().revision, source.key().revision);
    wait_library_source(&grown);
    assert_eq!(
        crate::prepared::unprepared_blocks(&grown, None).unwrap(),
        Some(0)
    );
    service.shutdown();
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn library_preparation_admits_newest_first_and_retries_a_full_runner() {
    let fixture = LibraryFixture::new(&["oldest", "middle", "newest"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    assert_eq!(
        service.prepare_library(&fixture.paths).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    let admitted: Vec<_> = {
        let sources = service.inner.sources.lock().unwrap();
        assert!(sources.library.contains_key(&fixture.paths[2]));
        assert!(sources.library.contains_key(&fixture.paths[1]));
        let waiting = sources.library.get(&fixture.paths[0]).unwrap();
        assert_eq!(
            waiting
                .state::<crate::prepared::Progress>()
                .lock()
                .unwrap()
                .state,
            crate::prepared::State::Idle
        );
        sources
            .library
            .values()
            .filter(|source| {
                source
                    .state::<crate::prepared::Progress>()
                    .lock()
                    .unwrap()
                    .running
            })
            .cloned()
            .collect()
    };
    drop(hold);
    for source in admitted {
        wait_library_source(&source);
    }
    service.prepare_library(&fixture.paths).unwrap();
    let oldest = service.open_map(&fixture.paths[0], None).unwrap();
    wait_library_source(&oldest);
    assert_eq!(
        crate::prepared::unprepared_blocks(&oldest, None).unwrap(),
        Some(0)
    );
    service.shutdown();
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn library_preparation_suspends_queued_automatic_work_and_resumes_outside_library() {
    let fixture = LibraryFixture::new(&["paused"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = service.open_map(&fixture.paths[0], None).unwrap();
    service.set_preparation_suspended(true);
    assert!(
        source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    drop(hold);
    crate::prepared::wait_cancelled(&source, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        crate::prepared::unprepared_blocks(&source, None).unwrap(),
        None
    );
    service.prepare_library(&fixture.paths).unwrap();
    assert!(
        !source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .running
    );
    service.set_preparation_suspended(false);
    // Even after the configured library changes, interrupted opened paths resume.
    service.prepare_library(&[]).unwrap();
    wait_library_source(&source);
    assert_eq!(
        crate::prepared::unprepared_blocks(&source, None).unwrap(),
        Some(0)
    );
    service.shutdown();
}

#[test]
fn game_presence_cancels_automatic_work_without_cancelling_manual_prepare() {
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let automatic = source();
    let manual = source();
    for (key, source, automatic) in [("automatic", &automatic, true), ("manual", &manual, false)] {
        let progress = source.state::<crate::prepared::Progress>();
        let mut progress = progress.lock().unwrap();
        progress.running = true;
        progress.automatic = automatic;
        service
            .inner
            .sources
            .lock()
            .unwrap()
            .entries
            .insert((PathBuf::from(key), key.into()), Arc::downgrade(source));
    }
    service.set_preparation_suspended(true);
    assert!(
        automatic
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    assert!(
        !manual
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    let unopened = source();
    service
        .inner
        .start_preparation(Arc::clone(&unopened), true, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        unopened
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .state,
        crate::prepared::State::Idle
    );
    service.shutdown();
}

fn library_action(
    service: &Service,
    source: Arc<Source>,
    action: crate::prepared::Action,
    id: &str,
    receive: &mpsc::Receiver<DataEvent>,
) {
    let request = service.inner.begin(id.into()).unwrap();
    service.inner.submit(
        Arc::clone(&request),
        LoadJob::Preparation {
            request,
            source_id: "library-test".into(),
            source,
            action,
        },
    );
    loop {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Chunk {
                id: received, last, ..
            } => {
                assert_eq!(received, id);
                if last {
                    break;
                }
            }
            event => panic!("unexpected preparation action reply: {event:?}"),
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn library_preparation_preserves_disabled_clear_and_cancel_until_manual_start() {
    use crate::prepared::{Action, Progress};
    let fixture = LibraryFixture::new(&["preferences"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.set_background_preparation(false);
    service.prepare_library(&fixture.paths).unwrap();
    assert!(service.inner.sources.lock().unwrap().entries.is_empty());
    let source = service.open_map(&fixture.paths[0], None).unwrap();
    // Both the game guard and the saved automatic preference permit manual work.
    service.set_preparation_suspended(true);
    library_action(
        &service,
        Arc::clone(&source),
        Action::Start,
        "manual-disabled",
        &receive,
    );
    wait_library_source(&source);
    assert!(!source.state::<Progress>().lock().unwrap().automatic);
    library_action(
        &service,
        Arc::clone(&source),
        Action::Clear,
        "clear-library",
        &receive,
    );
    service.set_background_preparation(true);
    service.set_preparation_suspended(false);
    for _ in 0..2 {
        service.prepare_library(&fixture.paths).unwrap();
    }
    assert!(
        crate::prepared::files(&source, None)
            .unwrap()
            .0
            .saved_key()
            .unwrap()
            .is_none()
    );
    fixture.append(&fixture.paths[0]);
    let grown = service.open_map(&fixture.paths[0], None).unwrap();
    service.prepare_library(&fixture.paths).unwrap();
    assert!(!grown.state::<Progress>().lock().unwrap().running);
    assert!(
        crate::prepared::files(&grown, None)
            .unwrap()
            .0
            .saved_key()
            .unwrap()
            .is_none()
    );
    library_action(
        &service,
        Arc::clone(&grown),
        Action::Cancel,
        "cancel-library",
        &receive,
    );
    service.prepare_library(&fixture.paths).unwrap();
    assert!(!grown.state::<Progress>().lock().unwrap().running);
    library_action(
        &service,
        Arc::clone(&grown),
        Action::Start,
        "manual-cleared",
        &receive,
    );
    wait_library_source(&grown);
    assert_eq!(
        crate::prepared::unprepared_blocks(&grown, None).unwrap(),
        Some(0)
    );

    service.shutdown();
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn library_preparation_failed_opens_stop_at_one_attempt_for_5000_inputs() {
    let fixture = LibraryFixture::new(&["failed-open"]);
    let path = &fixture.paths[0];
    std::fs::write(path, b"synthetic unreadable recording").unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let paths = vec![path.clone(); 5_000];
    for _ in 0..5 {
        service.prepare_library(&paths).unwrap();
    }
    assert_eq!(
        service.inner.sources.lock().unwrap().open_attempts(),
        1,
        "unchanged failed inputs must not reopen on no-progress replies"
    );
    std::fs::write(path, b"a changed synthetic unreadable recording").unwrap();
    for _ in 0..5 {
        service.prepare_library(&paths).unwrap();
    }
    assert_eq!(
        service.inner.sources.lock().unwrap().open_attempts(),
        2,
        "only a changed recording permits another open attempt"
    );
    service.set_preparation_suspended(true);
    service.set_preparation_suspended(false);
    for _ in 0..5 {
        service.prepare_library(&paths).unwrap();
    }
    assert_eq!(
        service.inner.sources.lock().unwrap().open_attempts(),
        2,
        "queue/game changes must not reopen the same failed input"
    );
    service.shutdown();
}

/// Only the loaders are stalled. All repeated product bodies and cancellation
/// calls below go through the real service admission and physical lane queue.
struct LoaderHold(Arc<(Mutex<bool>, std::sync::Condvar)>);
impl LoaderHold {
    fn new(service: &Service, interactive: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (sent, seen) = mpsc::channel();
        let count = if interactive {
            INTERACTIVE_THREADS
        } else {
            LOADER_THREADS
        };
        for index in 0..count {
            let id = format!("stalled-{sequence}-{index}");
            let request = service.inner.begin(id.clone()).unwrap();
            let released = Arc::clone(&release);
            let sent = sent.clone();
            service.inner.submit(
                Arc::clone(&request),
                LoadJob::Fixture {
                    request,
                    interactive,
                    run: Box::new(move || {
                        sent.send(()).unwrap();
                        let ready = released.0.lock().unwrap();
                        let (ready, _) = released
                            .1
                            .wait_timeout_while(ready, HANG_GUARD, |ready| !*ready)
                            .unwrap();
                        assert!(*ready, "stalled loader was not released");
                    }),
                },
            );
            seen.recv_timeout(HANG_GUARD).unwrap();
            drop(service.inner.pending.lock().unwrap().cancel(&id));
        }
        Self(release)
    }
}
impl Drop for LoaderHold {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}

/// Requests still alive once the response pump lets go of them. A refusal
/// from a full lane is delivered on the pump thread, which holds that one
/// request until delivery ends, so a count taken at once can see the full lane
/// plus it. Lane-retained requests never settle, so the old unbounded channel
/// still fails at `MAX_PENDING + 1`.
fn settled_live_requests(retained: &[std::sync::Weak<Request>]) -> usize {
    let live = || {
        retained
            .iter()
            .filter(|request| request.strong_count() != 0)
            .count()
    };
    let deadline = std::time::Instant::now() + HANG_GUARD;
    loop {
        let count = live();
        if count <= MAX_PENDING || std::time::Instant::now() >= deadline {
            return count;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn cancelled_resubmitted_jobs_keep_both_loader_lanes_bounded() {
    // Hard bound: the old unbounded channel fails at MAX_PENDING + 1, rather
    // than letting the reproducer itself run away.
    const CYCLES: usize = MAX_PENDING * 4;
    for interactive in [false, true] {
        let (sent, receive) = mpsc::channel();
        let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
        let hold = LoaderHold::new(&service, interactive);
        let source = source();
        let countries: Vec<_> = (0..5_000)
            .map(|index| format!("synthetic-{index}"))
            .collect();
        let params = serde_json::json!({"countries": countries, "hour": 1});
        let mut retained = Vec::new();
        for index in 0..CYCLES {
            let id = format!("cancel-resubmit-{index}");
            let request = service.inner.begin(id.clone()).unwrap();
            retained.push(Arc::downgrade(&request));
            let body = ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: "country-groups".into(),
                params: params.clone(),
            };
            assert_eq!(body.params["countries"].as_array().unwrap().len(), 5_000);
            let job = if interactive {
                LoadJob::PreparedQuery {
                    request: Arc::clone(&request),
                    source: Arc::clone(&source),
                    body,
                }
            } else {
                LoadJob::Product {
                    request: Arc::clone(&request),
                    lease_id: "test-lease".into(),
                    source: Arc::clone(&source),
                    body,
                }
            };
            service.inner.submit(Arc::clone(&request), job);
            drop(service.inner.pending.lock().unwrap().cancel(&id));
            drop(request);
            assert!(
                settled_live_requests(&retained) <= MAX_PENDING,
                "cancelled queued jobs exceeded the existing lane budget at cycle {index}"
            );
        }
        assert_eq!(settled_live_requests(&retained), MAX_PENDING);
        // A full lane reports the existing retryable capacity refusal.
        let request = service.inner.begin("lane-full".into()).unwrap();
        service.inner.submit(
            Arc::clone(&request),
            if interactive {
                LoadJob::PreparedQuery {
                    request,
                    source: Arc::clone(&source),
                    body: ProductRequest {
                        source_id: "source-test".into(),
                        revision: "revision-test".into(),
                        product: "country-groups".into(),
                        params: params.clone(),
                    },
                }
            } else {
                LoadJob::Product {
                    request,
                    lease_id: "test-lease".into(),
                    source: Arc::clone(&source),
                    body: ProductRequest {
                        source_id: "source-test".into(),
                        revision: "revision-test".into(),
                        product: "country-groups".into(),
                        params: params.clone(),
                    },
                }
            },
        );
        let mut refused = false;
        for _ in 0..=CYCLES {
            if let DataEvent::Error { id, message } = receive.recv_timeout(HANG_GUARD).unwrap() {
                assert_eq!(message, crate::transport::CAPACITY_REACHED);
                if id == "lane-full" {
                    refused = true;
                    break;
                }
            }
        }
        assert!(refused);
        drop(hold);
        service.shutdown();
        assert!(retained.iter().all(|request| request.strong_count() == 0));
    }
}

#[test]
fn preparation_refreshes_coalesce_per_source_and_share_the_history_bound() {
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = LoaderHold::new(&service, false);
    let source = source();
    for _ in 0..MAX_PENDING * 4 {
        service
            .inner
            .refresh_preparation(Arc::clone(&source), false)
            .unwrap();
    }
    service
        .inner
        .refresh_preparation(Arc::clone(&source), true)
        .unwrap();
    service
        .inner
        .refresh_preparation(Arc::clone(&source), false)
        .unwrap();
    {
        let loader = service.inner.loader.lock().unwrap();
        let refreshes = &loader.as_ref().unwrap().refreshes;
        assert_eq!(
            refreshes.len(),
            1,
            "repeat refreshes must retain one queued job"
        );
        assert!(
            refreshes.get(source.key()).unwrap().load(Ordering::Acquire),
            "restart upgrades cannot be lost"
        );
    }
    assert_eq!(
        Arc::strong_count(&source),
        2,
        "one queued refresh owns one source reference"
    );
    // The refresh consumes an ordinary history slot, so only budget - 1
    // cancelled product jobs can still be physically retained.
    let mut retained = Vec::new();
    for index in 0..MAX_PENDING * 2 {
        let id = format!("refresh-shared-{index}");
        let request = service.inner.begin(id.clone()).unwrap();
        retained.push(Arc::downgrade(&request));
        service.inner.submit(
            Arc::clone(&request),
            LoadJob::Product {
                request: Arc::clone(&request),
                lease_id: "test-lease".into(),
                source: Arc::clone(&source),
                body: ProductRequest {
                    source_id: "source-test".into(),
                    revision: "revision-test".into(),
                    product: "country-groups".into(),
                    params: serde_json::json!({}),
                },
            },
        );
        drop(service.inner.pending.lock().unwrap().cancel(&id));
        drop(request);
    }
    assert_eq!(
        retained
            .iter()
            .filter(|request| request.strong_count() != 0)
            .count(),
        MAX_PENDING - 1
    );
    let other = source_with_id("other-refresh");
    assert_eq!(
        service
            .inner
            .refresh_preparation(other, true)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        service
            .inner
            .loader
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .refreshes
            .len(),
        1
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn preparation_refresh_coalescing_retires_when_loader_receives_the_job() {
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = LoaderHold::new(&service, false);
    let source = source();
    service
        .inner
        .refresh_preparation(Arc::clone(&source), false)
        .unwrap();
    assert_eq!(
        service
            .inner
            .loader
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .refreshes
            .len(),
        1
    );
    drop(hold);
    // Occupying every worker behind that refresh proves physical retirement,
    // without an unbounded status/request poll.
    let behind_refresh = LoaderHold::new(&service, false);
    assert!(
        service
            .inner
            .loader
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .refreshes
            .is_empty()
    );
    service.inner.refresh_preparation(source, true).unwrap();
    assert_eq!(
        service
            .inner
            .loader
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .refreshes
            .len(),
        1
    );
    drop(behind_refresh);
    service.shutdown();
}
