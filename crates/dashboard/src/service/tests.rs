use super::pump::{Delivery, deliver, emit_error};
use super::*;
use crate::transport::ReplyChunks;
use crate::{
    SourceKey,
    transport::{ProductRequest, SourceReply, bounded_json},
};
use replay::{container::Replay, spool::Record};
use replay_data::{
    identity::CaptureMode, identity::Identity, identity::Roster, identity::SampleBoundary,
    identity::SessionStart, identity::Verification,
};
use std::{sync::mpsc, time::Duration};

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
                lane: Lane::History,
                run: Arc::new(move || panic!("synthetic {stage} panic")),
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
        capture_mode: CaptureMode::Full,
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
        refresh: false,
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
        .start_preparation(
            Arc::clone(&manual),
            jobs::Admission::Manual,
            jobs::Checked::unread(true),
            &AtomicBool::new(false),
        )
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
        .start_preparation(
            Arc::clone(&cleared),
            jobs::Admission::Opened,
            jobs::Checked::unread(true),
            &AtomicBool::new(false),
        )
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
        metadata: source.metadata().unwrap(),
        presentation_generation: 0,
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

#[test]
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
        serde_json::to_value(source.metadata().unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    let path = folder.0.join("modified-crc.battleplan");
    std::fs::write(&path, bytes).unwrap();
    let modified = service.open_map(&path, None).unwrap();
    assert!(modified.replay_ref().is_file_backed());
    let metadata = modified.metadata().unwrap();
    assert!(metadata.damaged.is_some());
    assert!(
        metadata
            .warnings
            .iter()
            .any(|reason| reason.contains("trailer CRC mismatch"))
    );
    assert_eq!(metadata.countries, source.metadata().unwrap().countries);
    assert_eq!(metadata.branches, source.metadata().unwrap().branches);
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
        Self::holding(service, 2)
    }
    /// Occupy the workers and fill their queue too.
    fn filled(service: &Service) -> Self {
        Self::holding(service, 4)
    }
    fn holding(service: &Service, jobs: usize) -> Self {
        let (sent, seen) = mpsc::channel();
        let flags: Vec<_> = (0..jobs)
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect();
        for (index, flag) in flags.iter().enumerate() {
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
            // Occupy both workers before filling their bounded queue.
            if index < 2 {
                seen.recv_timeout(HANG_GUARD).unwrap();
            }
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
fn library_preparation_prepares_unopened_published_sessions_and_their_growth() {
    let fixture = LibraryFixture::new(&["unopened"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
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
fn library_preparation_admits_newest_first_and_retries_a_full_runner() {
    let fixture = LibraryFixture::new(&["oldest", "middle", "newest"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::filled(&service);
    assert_eq!(
        service.prepare_library(&fixture.paths).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    {
        let sources = service.inner.sources.lock().unwrap();
        let waiting = sources.library.get(&fixture.paths[2]).unwrap();
        assert_eq!(
            waiting
                .state::<crate::prepared::Progress>()
                .lock()
                .unwrap()
                .state,
            crate::prepared::State::Idle
        );
        assert!(
            !sources.library.contains_key(&fixture.paths[1])
                && !sources.library.contains_key(&fixture.paths[0]),
            "older recordings wait for the newest"
        );
    }
    drop(hold);
    // One recording at a time, newest first.
    for (index, path) in fixture.paths.iter().enumerate().rev() {
        admit_library(&service, &fixture.paths);
        let source = library_source(&service, path).expect("the newest unprepared is admitted");
        for older in &fixture.paths[..index] {
            assert!(library_source(&service, older).is_none());
        }
        wait_library_source(&source);
        assert_eq!(
            crate::prepared::unprepared_blocks(&source, None).unwrap(),
            Some(0)
        );
    }
    service.shutdown();
}

#[test]
fn library_preparation_pauses_queued_work_and_resumes_outside_library() {
    let fixture = LibraryFixture::new(&["paused"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = service.open_map(&fixture.paths[0], None).unwrap();
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    assert!(
        source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    drop(hold);
    crate::prepared::wait_finished(&source, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        crate::prepared::status(&source, "paused", "status").pause_reason,
        Some(crate::prepared::PauseReason::DiskSpace)
    );
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
    TEST_ROOM.with(|room| room.set(None));
    // Even after the configured library changes, paused retained paths resume.
    service.prepare_library(&[]).unwrap();
    wait_library_source(&source);
    assert_eq!(
        crate::prepared::unprepared_blocks(&source, None).unwrap(),
        Some(0)
    );
    service.shutdown();
}

/// Nothing outside the service holds preparation back: a new service admits
/// library work, then an open's preparation, each straight away, and both finish.
#[test]
fn preparation_and_library_admission_proceed_without_a_gate() {
    let fixture = LibraryFixture::new(&["library", "opened"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.prepare_library(&fixture.paths[..1]).unwrap();
    let library = library_source(&service, &fixture.paths[0])
        .expect("the published recording is admitted at once");
    wait_library_source(&library);
    let opened = open_lease(&service, &receive, &fixture.paths[1], "ungated-open");
    wait_library_source(&opened);
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
fn library_preparation_preserves_disabled_clear_and_cancel_until_manual_start() {
    use crate::prepared::{Action, Progress};
    let fixture = LibraryFixture::new(&["preferences"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.set_background_preparation(false);
    service.prepare_library(&fixture.paths).unwrap();
    assert!(service.inner.sources.lock().unwrap().entries.is_empty());
    let source = service.open_map(&fixture.paths[0], None).unwrap();
    // The saved automatic preference permits manual work.
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
fn library_preparation_failed_opens_stop_at_one_attempt_for_repeated_inputs() {
    let fixture = LibraryFixture::new(&["failed-open"]);
    let path = &fixture.paths[0];
    std::fs::write(path, b"synthetic unreadable recording").unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let paths = vec![path.clone(); 24];
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
    service.set_background_preparation(false);
    service.set_background_preparation(true);
    for _ in 0..5 {
        service.prepare_library(&paths).unwrap();
    }
    assert_eq!(
        service.inner.sources.lock().unwrap().open_attempts(),
        2,
        "queue/preference changes must not reopen the same failed input"
    );
    service.shutdown();
}

/// Only the loaders are stalled. All repeated product bodies and cancellation
/// calls below go through the real service admission and physical lane queue.
struct LoaderHold(Arc<(Mutex<bool>, std::sync::Condvar)>);
impl LoaderHold {
    fn new(service: &Service, lane: Lane) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (sent, seen) = mpsc::channel();
        for index in 0..lane.threads(parallelism()) {
            let id = format!("stalled-{sequence}-{index}");
            let request = service.inner.begin(id.clone()).unwrap();
            let released = Arc::clone(&release);
            let sent = sent.clone();
            service.inner.submit(
                Arc::clone(&request),
                LoadJob::Fixture {
                    request,
                    lane,
                    run: Arc::new(move || {
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
/// plus it. A cancelled refusal is discarded without a reply, so the count
/// waits for a reply queued behind them instead: the pump takes replies in
/// order and lets go of each before taking the next. `events` receives
/// every event the service emits. Lane-retained requests never settle, so the old
/// unbounded channel still fails at `MAX_PENDING + 1`.
fn settled_live_requests(
    service: &Service,
    events: &mpsc::Receiver<DataEvent>,
    retained: &[std::sync::Weak<Request>],
) -> usize {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = format!("pump-settled-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let request = service.inner.begin(id.clone()).unwrap();
    service.inner.error(request, "settled".into());
    while !matches!(
        events.recv_timeout(HANG_GUARD).unwrap(),
        DataEvent::Error { id: delivered, .. } if delivered == id
    ) {}
    retained
        .iter()
        .filter(|request| request.strong_count() != 0)
        .count()
}

#[test]
fn cancelled_resubmitted_jobs_keep_the_history_and_prepared_lanes_bounded() {
    // Hard bound: the old unbounded channel fails at MAX_PENDING + 1, rather
    // than letting the reproducer itself run away.
    const CYCLES: usize = MAX_PENDING * 4;
    for (lane, prepared) in [(Lane::History, false), (Lane::Prepared, true)] {
        let (sent, receive) = mpsc::channel();
        let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
        let hold = LoaderHold::new(&service, lane);
        let source = source();
        let countries: Vec<_> = (0..24).map(|index| format!("synthetic-{index}")).collect();
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
            assert_eq!(body.params["countries"].as_array().unwrap().len(), 24);
            let job = if prepared {
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
            assert_eq!(
                settled_live_requests(&service, &receive, &retained),
                (index + 1).min(MAX_PENDING),
                "cancelled queued jobs must keep exactly the lane budget at cycle {index}"
            );
        }
        // A full lane reports the existing retryable capacity refusal.
        let request = service.inner.begin("lane-full".into()).unwrap();
        service.inner.submit(
            Arc::clone(&request),
            if prepared {
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
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = LoaderHold::new(&service, Lane::History);
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
        settled_live_requests(&service, &receive, &retained),
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
    let hold = LoaderHold::new(&service, Lane::History);
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
    let behind_refresh = LoaderHold::new(&service, Lane::History);
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

#[test]
fn lanes_are_sized_from_the_machine_with_at_least_three_loaders() {
    for (parallelism, history, sampled, prepared) in [
        (1, 3, 3, 2),
        (4, 3, 3, 2),
        (8, 3, 4, 4),
        (16, 4, 8, 8),
        (64, 16, 32, 32),
    ] {
        assert_eq!(Lane::History.threads(parallelism), history);
        assert_eq!(Lane::Sampled.threads(parallelism), sampled);
        assert_eq!(Lane::Interactive.threads(parallelism), INTERACTIVE_THREADS);
        assert_eq!(Lane::Prepared.threads(parallelism), prepared);
    }
    // Every lane's loaders and the response pump.
    let service = Service::new(Arc::new(|_| true)).unwrap();
    assert_eq!(
        service.thread_ids.len(),
        Lane::ALL
            .iter()
            .map(|lane| lane.threads(parallelism()))
            .sum::<usize>()
            + 1
    );
    service.shutdown();
}

/// A prepared answer that needs no block decode is given while every sampled
/// loader is busy; one that samples a block leaves the prepared lane at its
/// first block decode and is answered on a sampled loader.
#[test]
fn prepared_answers_do_not_wait_behind_sampled_reads() {
    use crate::test_fixtures::{CAMPAIGN, records, source as frame};
    use replay_data::rows::TableId;
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let recording = Arc::new(
        Source::from_replay(
            SourceKey {
                source_id: "sampled".into(),
                revision: "1".into(),
            },
            PathBuf::from("memory.battleplan"),
            Arc::new(
                Replay::from_records(
                    CAMPAIGN.into(),
                    &[records(1, vec![frame(1, 0, Some(1)), frame(1, 1, Some(2))])],
                )
                .unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
    let hold = LoaderHold::new(&service, Lane::Sampled);
    let (sent, answers) = mpsc::channel::<(&str, Option<i32>, String)>();
    let lane_of = || thread::current().name().unwrap_or_default().to_owned();
    let submit = |id: &str, run: Arc<dyn Fn() + Send + Sync>| {
        let request = service.inner.begin(id.into()).unwrap();
        service.inner.submit(
            Arc::clone(&request),
            LoadJob::Fixture {
                request,
                lane: Lane::Prepared,
                run,
            },
        );
    };
    let sampled = sent.clone();
    submit(
        "sampled",
        Arc::new(move || {
            let read = crate::prepared::query::progressive::frame_at(
                &recording,
                1,
                1,
                &[TableId::Countries],
                &AtomicBool::new(false),
            );
            // A refused decode answers nothing; the job runs again.
            if let Ok(found) = read {
                let hour = found.map(|found| found.hour);
                sampled.send(("sampled", hour, lane_of())).unwrap();
            }
        }),
    );
    submit(
        "prepared",
        Arc::new(move || sent.send(("prepared", None, lane_of())).unwrap()),
    );
    let (job, _, lane) = answers.recv_timeout(HANG_GUARD).unwrap();
    assert_eq!(job, "prepared");
    assert!(lane.starts_with("dashboard prepared"), "{lane}");
    assert!(
        answers.try_recv().is_err(),
        "the sampled read waits for a sampled loader"
    );
    drop(hold);
    let (job, hour, lane) = answers.recv_timeout(HANG_GUARD).unwrap();
    assert_eq!((job, hour), ("sampled", Some(1)));
    assert!(lane.starts_with("dashboard sampled"), "{lane}");
    service.shutdown();
}

/// Reads `id`'s reply chunks to their end.
fn reply_of(receive: &mpsc::Receiver<DataEvent>, wanted: &str) -> String {
    let mut json = String::new();
    loop {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Chunk {
                id, fragment, last, ..
            } if id == wanted => {
                json.push_str(&fragment);
                if last {
                    return json;
                }
            }
            DataEvent::Chunk { .. } => {}
            event => panic!("unexpected event for {wanted}: {event:?}"),
        }
    }
}

/// Waits until `done` holds, failing instead of hanging.
fn wait_until(done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + HANG_GUARD;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "the condition never held"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A recording of eight one-hour blocks, each a read of Germany's two lines,
/// written to a file of its own, opened through the service and prepared
/// whole: the service, its events, the opened source's reply and source.
fn prepared_production_source() -> (
    Arc<Service>,
    mpsc::Receiver<DataEvent>,
    SourceReply,
    Arc<Source>,
    PathBuf,
) {
    use crate::test_fixtures::{
        CAMPAIGN, PRODUCTION_HOUR as FIRST, continuation_records, production_frame, records,
    };
    let frame = |hour: i32| {
        let units = i64::from(hour - FIRST);
        production_frame(
            hour,
            true,
            true,
            9001,
            &[(5, 70, 8, 0), (6, 71, 2, 0)],
            Some(&[(5, 70, units, true, false), (6, 71, units, true, false)]),
        )
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(FIRST)])]).unwrap();
    for hour in FIRST + 1..FIRST + 8 {
        replay = replay
            .append(&[continuation_records(&replay, 1, vec![frame(hour)])])
            .unwrap();
    }
    let (service, receive, opened, source, path) = production_source(&replay);
    let before = service.preparation_revision();
    crate::prepared::start(
        &service.inner.preparation,
        Arc::clone(&source),
        None,
        crate::prepared::pipeline::restore(&source, None, &AtomicBool::new(false)),
        crate::prepared::store_epoch(&source),
        &AtomicBool::new(false),
    )
    .unwrap();
    // Ready, and the runner's job has returned: its revision moved past it.
    wait_until(|| {
        progress_of(&source) == (false, crate::prepared::State::Ready)
            && service.preparation_revision() > before
    });
    (service, receive, opened, source, path)
}

fn production_source(
    replay: &Replay,
) -> (
    Arc<Service>,
    mpsc::Receiver<DataEvent>,
    SourceReply,
    Arc<Source>,
    PathBuf,
) {
    let folder = std::env::temp_dir().join(format!("production-kept-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("campaign.battleplan");
    std::fs::write(&path, replay.bytes()).unwrap();
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    // Tests publish only the preparation they explicitly request.
    service.set_background_preparation(false);
    service.dispatch(DataCommand::Open {
        id: "open".into(),
        path: path.to_string_lossy().into_owned(),
        revision: crate::protocol::replay_revision(&path).unwrap(),
    });
    let opened: SourceReply = serde_json::from_str(&reply_of(&receive, "open")).unwrap();
    let source = service.source(&opened.id).unwrap();
    (service, receive, opened, source, path)
}

/// Whether the source's preparation runs, and its state.
fn progress_of(source: &Source) -> (bool, crate::prepared::State) {
    let progress = source.state::<crate::prepared::Progress>();
    let progress = progress.lock().unwrap();
    (progress.running, progress.state)
}

/// The source's preparation sections as its status reports them.
fn sections_of(source: &Source) -> serde_json::Value {
    let progress = source.state::<crate::prepared::Progress>();
    serde_json::to_value(&progress.lock().unwrap().sections).unwrap()
}

/// Asks for Germany's exact production lines through the service.
fn request_exact(
    service: &Service,
    receive: &mpsc::Receiver<DataEvent>,
    opened: &SourceReply,
) -> String {
    service.dispatch(DataCommand::Product {
        id: "exact".into(),
        request: ProductRequest {
            source_id: opened.id.clone(),
            revision: opened.revision.clone(),
            product: "production-lines".into(),
            params: serde_json::json!({"branchId": "1", "tag": "GER", "exact": true}),
        },
    });
    reply_of(receive, "exact")
}

fn germany_kept(source: &Source) -> bool {
    let (store, key) = crate::prepared::files(source, None).unwrap();
    crate::products::production_lines::prepared::current(
        &store,
        &key,
        source,
        &crate::products::LoadContext { content: None },
        1,
    )
    .unwrap()
}

/// A production request reads completed pages without registering or writing work.
#[test]
fn a_production_request_after_preparation_queues_no_preparation_and_writes_nothing() {
    let (service, receive, opened, source, _path) = prepared_production_source();
    let sections = sections_of(&source);
    let revision = service.preparation_revision();
    // A production line request has nothing to run after its reply.
    request_exact(&service, &receive, &opened);
    assert!(
        service
            .inner
            .loader
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|lanes| lanes.refreshes.is_empty()),
        "no preparation refresh is queued"
    );
    assert!(
        germany_kept(&source),
        "preparation already published the country"
    );
    assert_eq!(progress_of(&source), (false, crate::prepared::State::Ready));
    assert_eq!(sections_of(&source), sections);
    assert_eq!(service.preparation_revision(), revision);
    service.shutdown();
}

#[test]
fn production_detail_and_breakdown_routes_keep_unprepared_history_pending() {
    use crate::products::{
        LoadContext, Product,
        production_lines::{
            self, ProductionBreakdownParams, ProductionDetailParams, ProductionLineBreakdown,
            ProductionLineDetail,
        },
    };
    use crate::test_fixtures::{
        CAMPAIGN, PRODUCTION_HOUR as FIRST, continuation_records, production_frame, records,
    };
    use serde_json::{Value, json};
    let frame = |hour| {
        production_frame(
            hour,
            true,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, i64::from(hour - FIRST), true, false)]),
        )
    };
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(FIRST), frame(719)])],
    )
    .unwrap();
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(720), frame(1439)],
        )])
        .unwrap();
    let replay = second
        .append(&[continuation_records(
            &second,
            1,
            vec![frame(1440), frame(1441)],
        )])
        .unwrap();
    assert_eq!(replay.index.len(), 3);
    let (service, receive, opened, source, _path) = production_source(&replay);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let mut prepared =
        production_lines::prepared::Prepared::new(source.replay_ref(), 1, ["GER".into()], None);
    let detail = |window| json!({"branchId":"1", "tag":"GER", "window":window});
    let breakdown = json!({"branchId":"1", "tag":"GER", "key":"5#1", "group":"output"});
    let request = |product: &str, params: Value| ProductRequest {
        source_id: opened.id.clone(),
        revision: opened.revision.clone(),
        product: product.into(),
        params,
    };
    let query = |id: &str, product: &str, params: Value| -> Value {
        service.dispatch(DataCommand::PreparedQuery {
            id: id.into(),
            refresh: false,
            request: request(product, params),
        });
        serde_json::from_str(&reply_of(&receive, id)).unwrap()
    };
    let pending = |reply: Value| {
        assert!(reply["data"].is_null(), "{reply}");
        assert_eq!(reply["gate"], json!(["production-lines"]));
    };
    for (id, product, params) in [
        ("direct-detail", "production-line-detail", detail(1)),
        (
            "direct-breakdown",
            "production-line-breakdown",
            breakdown.clone(),
        ),
    ] {
        service.dispatch(DataCommand::Product {
            id: id.into(),
            request: request(product, params),
        });
        assert!(matches!(receive.recv_timeout(HANG_GUARD).unwrap(),
            DataEvent::Error { id: received, message }
            if received == id && message == format!("{product} is served by prepared queries")));
    }
    pending(query("cold-detail", "production-line-detail", detail(0)));
    pending(query(
        "cold-breakdown",
        "production-line-breakdown",
        breakdown.clone(),
    ));
    for (position, block) in source.replay_ref().index.iter().enumerate() {
        let mut writer = if position == 0 {
            store.replace(key.clone()).unwrap()
        } else {
            store.extend(&key).unwrap()
        };
        prepared
            .advance_block(&mut writer, source.replay_ref(), position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut writer, source.replay_ref(), block, &flag)
            .unwrap();
        writer.commit().unwrap();
        if position < 2 {
            let covered = query("covered-detail", "production-line-detail", detail(position));
            assert!(covered["data"].is_object(), "{covered}");
            assert!(!covered["data"]["lines"].as_array().unwrap().is_empty());
            pending(query(
                "later-detail",
                "production-line-detail",
                detail(position + 1),
            ));
            pending(query(
                "pending-breakdown",
                "production-line-breakdown",
                breakdown.clone(),
            ));
        }
    }
    for window in 0..=2 {
        let actual = query("completed-detail", "production-line-detail", detail(window));
        let params: ProductionDetailParams = serde_json::from_value(detail(window)).unwrap();
        let mut expected = ProductionLineDetail::load(&source, &params, &context, &flag).unwrap();
        source.append_stack_warnings(&mut expected.warnings);
        source.append_presentation_warnings(&mut expected.warnings, None);
        assert_eq!(actual["data"], serde_json::to_value(expected).unwrap());
        assert!(actual.get("gate").is_none());
    }
    let actual = query(
        "completed-breakdown",
        "production-line-breakdown",
        breakdown.clone(),
    );
    let params: ProductionBreakdownParams = serde_json::from_value(breakdown).unwrap();
    let mut expected = ProductionLineBreakdown::load(&source, &params, &context, &flag).unwrap();
    source.append_stack_warnings(&mut expected.warnings);
    source.append_presentation_warnings(&mut expected.warnings, None);
    assert_eq!(actual["data"], serde_json::to_value(expected).unwrap());
    assert!(actual.get("gate").is_none());
    assert!(production_lines::prepared::current(&store, &key, &source, &context, 1).unwrap());
    service.shutdown();
}

/// Open `path` through the frontend's open command and return its lease's source.
fn open_lease(
    service: &Service,
    receive: &mpsc::Receiver<DataEvent>,
    path: &Path,
    id: &str,
) -> Arc<Source> {
    open_view(service, receive, path, id).1
}

/// Open `path` as the frontend does, returning the view's lease and source.
fn open_view(
    service: &Service,
    receive: &mpsc::Receiver<DataEvent>,
    path: &Path,
    id: &str,
) -> (String, Arc<Source>) {
    service.dispatch(DataCommand::Open {
        id: id.into(),
        path: path.to_string_lossy().into_owned(),
        revision: crate::protocol::replay_revision(path).unwrap(),
    });
    let mut json = String::new();
    loop {
        match receive.recv_timeout(HANG_GUARD).unwrap() {
            DataEvent::Chunk {
                id: received,
                fragment,
                last,
                ..
            } => {
                assert_eq!(received, id);
                json.push_str(&fragment);
                if last {
                    break;
                }
            }
            event => panic!("unexpected open event: {event:?}"),
        }
    }
    let opened: SourceReply = serde_json::from_str(&json).unwrap();
    let source = service.source(&opened.id).unwrap();
    (opened.id, source)
}

fn library_source(service: &Service, path: &Path) -> Option<Arc<Source>> {
    service
        .inner
        .sources
        .lock()
        .unwrap()
        .library
        .get(path)
        .cloned()
}

/// Admit library work, retrying while the workers' queue is still full of
/// jobs that are about to leave it.
fn admit_library(service: &Service, paths: &[PathBuf]) {
    let started = std::time::Instant::now();
    while let Err(error) = service.prepare_library(paths) {
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(started.elapsed() < HANG_GUARD, "the queue never drained");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether the source's job is queued or running, was admitted ahead of the
/// user, and has been asked to stop.
fn job(source: &Source) -> (bool, bool, bool) {
    let progress = source.state::<crate::prepared::Progress>();
    let progress = progress.lock().unwrap();
    (
        progress.running,
        progress.background.load(Ordering::Acquire),
        progress.cancelled.load(Ordering::Acquire),
    )
}

/// Why the scheduler, not the user, stopped the source's job.
fn paused(source: &Source) -> Option<crate::prepared::PauseReason> {
    source
        .state::<crate::prepared::Progress>()
        .lock()
        .unwrap()
        .paused
}

fn state(source: &Source) -> crate::prepared::State {
    source
        .state::<crate::prepared::Progress>()
        .lock()
        .unwrap()
        .state
}

/// The cancellation flag of the source's latest admission: a new admission
/// replaces it, so an unchanged pointer means no second job was admitted.
fn admission_of(source: &Source) -> *const AtomicBool {
    Arc::as_ptr(
        &source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .cancelled,
    )
}

#[test]
fn ending_a_session_schedules_its_preparation_ahead_of_the_user() {
    let fixture = LibraryFixture::new(&["earlier", "session"]);
    let session = fixture.paths[1].clone();
    // While the session records, its recording is not published yet.
    std::fs::remove_file(&session).unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service
        .prepare_library(std::slice::from_ref(&session))
        .unwrap();
    assert!(library_source(&service, &session).is_none());
    // Ending the session publishes the recording whole, as finalization does.
    fixture.append(&session);
    service
        .prepare_library(std::slice::from_ref(&session))
        .unwrap();
    let source = library_source(&service, &session).expect("the ended session is admitted");
    assert_eq!(
        source.key().revision,
        crate::protocol::replay_revision(&session).unwrap()
    );
    assert_eq!(job(&source), (true, true, false));
    assert!(
        source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .automatic
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn app_start_prepares_every_recording_behind_its_store_newest_first() {
    // Oldest first.
    let fixture = LibraryFixture::new(&["r0", "r1", "r2", "r3", "r4"]);
    let paths = &fixture.paths;
    // An earlier app session prepared r3 fully and r4 before its last session.
    let earlier = Service::new(Arc::new(|_| true)).unwrap();
    for path in [&paths[3], &paths[4]] {
        let source = earlier.open_map(path, None).unwrap();
        crate::prepared::pipeline::run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    }
    earlier.shutdown();
    fixture.append(&paths[4]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    // One recording at a time, newest first; r3's store is up to date.
    for expected in [4, 2, 1, 0] {
        admit_library(&service, paths);
        let source = library_source(&service, &paths[expected])
            .expect("the newest recording behind its store is admitted");
        assert!(job(&source).1, "prepared ahead of the user");
        let sources = service.inner.sources.lock().unwrap();
        for older in &paths[..expected] {
            assert!(
                !sources.entries.keys().any(|(opened, _)| opened == older),
                "{} waits for newer work",
                older.display()
            );
        }
        drop(sources);
        wait_library_source(&source);
        assert_eq!(
            crate::prepared::unprepared_blocks(&source, None).unwrap(),
            Some(0)
        );
        assert!(library_source(&service, &paths[3]).is_none());
    }
    service.prepare_library(paths).unwrap();
    assert!(
        service.inner.sources.lock().unwrap().library.is_empty(),
        "every recording is prepared"
    );
    service.shutdown();
}

#[test]
fn newer_library_work_goes_first() {
    let fixture = LibraryFixture::new(&["a", "b", "c"]);
    let newer = fixture.paths[2].clone();
    std::fs::remove_file(&newer).unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let b = library_source(&service, &fixture.paths[1]).expect("the newest is admitted");
    assert_eq!(job(&b), (true, true, false));
    assert!(
        library_source(&service, &fixture.paths[0]).is_none(),
        "older work waits for newer work"
    );
    // A later session publishes a newer recording.
    fixture.append(&newer);
    service.prepare_library(&fixture.paths).unwrap();
    let c = library_source(&service, &newer).expect("newer work goes first");
    assert_eq!(job(&c), (true, true, false));
    assert!(job(&b).2, "older library work pauses");
    assert_eq!(
        paused(&b),
        Some(crate::prepared::PauseReason::NewerRecording)
    );
    assert!(
        library_source(&service, &fixture.paths[1]).is_some(),
        "and is kept to resume"
    );
    assert!(library_source(&service, &fixture.paths[0]).is_none());
    drop(hold);
    service.shutdown();
}

#[test]
fn library_work_pauses_below_the_free_space_floor_and_resumes_above_it() {
    assert!(room_for_background(100, 1_000));
    assert!(!room_for_background(99, 1_000));
    assert!(!room_for_background(0, 1));
    assert!(room_for_background(u64::MAX, u64::MAX));
    let fixture = LibraryFixture::new(&["floor"]);
    let path = &fixture.paths[0];
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    assert!(
        library_source(&service, path).is_none(),
        "nothing starts below the floor"
    );
    TEST_ROOM.with(|room| room.set(Some(true)));
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).unwrap();
    let admitted = admission_of(&source);
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    assert!(job(&source).2, "running library work pauses at the floor");
    assert_eq!(
        paused(&source),
        Some(crate::prepared::PauseReason::DiskSpace)
    );
    assert_eq!(admission_of(&source), admitted);
    TEST_ROOM.with(|room| room.set(None));
    drop(hold);
    crate::prepared::wait_finished(&source, &AtomicBool::new(false)).unwrap();
    let reply = crate::prepared::status(&source, "floor", "status");
    assert_eq!(reply.state, crate::prepared::State::Paused);
    assert_eq!(
        reply.pause_reason,
        Some(crate::prepared::PauseReason::DiskSpace)
    );
    // Above the floor the library scan resumes it, queued behind held workers.
    let hold = PreparationHold::new(&service);
    admit_library(&service, &fixture.paths);
    let resumed = admission_of(&source);
    assert_ne!(resumed, admitted);
    assert_eq!(job(&source), (true, true, false));
    assert_eq!(
        crate::prepared::status(&source, "floor", "status").pause_reason,
        None
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn opening_a_recording_joins_its_library_preparation() {
    let fixture = LibraryFixture::new(&["older", "newest"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let library = library_source(&service, &fixture.paths[1]).unwrap();
    assert_eq!(job(&library), (true, true, false));
    let admitted = admission_of(&library);
    let opened = open_lease(&service, &receive, &fixture.paths[1], "join-open");
    assert!(
        Arc::ptr_eq(&opened, &library),
        "the open shares the library's source"
    );
    assert_eq!(
        admission_of(&opened),
        admitted,
        "the open joins the queued job instead of admitting a second one"
    );
    assert_eq!(
        job(&opened),
        (true, false, false),
        "the joined job keeps running as the user's"
    );
    // The older recording waits while the user's preparation runs.
    service.prepare_library(&fixture.paths).unwrap();
    assert!(library_source(&service, &fixture.paths[0]).is_none());
    drop(hold);
    service.shutdown();
}

#[test]
fn library_work_pauses_for_another_opened_recording_and_waits_for_it() {
    let fixture = LibraryFixture::new(&["opened", "library"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let library = library_source(&service, &fixture.paths[1]).unwrap();
    assert_eq!(job(&library), (true, true, false));
    let opened = open_lease(&service, &receive, &fixture.paths[0], "yield-open");
    assert_eq!(
        job(&opened),
        (true, false, false),
        "the open admits the user's preparation"
    );
    assert!(job(&library).2, "library work pauses for the open");
    assert_eq!(
        paused(&library),
        Some(crate::prepared::PauseReason::Requested)
    );
    let paused_at = admission_of(&library);
    // The library scan keeps the paused job but does not resume it while the
    // user's preparation is queued or running.
    service.prepare_library(&fixture.paths).unwrap();
    assert_eq!(admission_of(&library), paused_at);
    assert!(library_source(&service, &fixture.paths[1]).is_some());
    drop(hold);
    crate::prepared::wait_finished(&library, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        state(&library),
        crate::prepared::State::Paused,
        "the status reply says Paused, not Cancelled"
    );
    let reply = crate::prepared::status(&library, "library", "status");
    assert_eq!(reply.state, crate::prepared::State::Paused);
    assert_eq!(
        reply.pause_reason,
        Some(crate::prepared::PauseReason::Requested)
    );
    service.shutdown();
}

#[test]
fn a_cancel_overrides_a_pause() {
    let paused_source = source();
    crate::prepared::pause(&paused_source, crate::prepared::PauseReason::NewerRecording);
    assert_eq!(state(&paused_source), crate::prepared::State::Paused);
    crate::prepared::cancel(&paused_source);
    assert_eq!(state(&paused_source), crate::prepared::State::Cancelled);
    let cancelled = source();
    crate::prepared::cancel(&cancelled);
    crate::prepared::pause(&cancelled, crate::prepared::PauseReason::NewerRecording);
    assert_eq!(
        state(&cancelled),
        crate::prepared::State::Cancelled,
        "the scheduler never turns a Cancel into a pause"
    );
}

#[test]
fn opening_a_newer_revision_takes_over_its_library_preparation() {
    let fixture = LibraryFixture::new(&["continued"]);
    let path = fixture.paths[0].clone();
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let earlier = library_source(&service, &path).unwrap();
    // The next session continues the recording before the library job ran.
    fixture.append(&path);
    // The open waits for the earlier revision's writer, which needs a worker.
    let release = std::thread::spawn(move || drop(hold));
    let grown = open_lease(&service, &receive, &path, "take-over-open");
    release.join().unwrap();
    assert_ne!(grown.key().revision, earlier.key().revision);
    assert!(
        !job(&earlier).0,
        "the earlier revision stopped before the open admitted its own"
    );
    // One writer at a time: the grown revision does not fail on the store.
    wait_library_source(&grown);
    assert_eq!(
        crate::prepared::unprepared_blocks(&grown, None).unwrap(),
        Some(0)
    );
    service.shutdown();
}

#[test]
fn background_preparation_off_schedules_nothing() {
    let fixture = LibraryFixture::new(&["off-earlier", "off-session"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.set_background_preparation(false);
    // Neither the app start scan nor a session ending admits anything.
    service.prepare_library(&fixture.paths).unwrap();
    fixture.append(&fixture.paths[1]);
    service.prepare_library(&fixture.paths).unwrap();
    {
        let sources = service.inner.sources.lock().unwrap();
        assert!(sources.library.is_empty());
        assert!(sources.entries.is_empty(), "no recording was even opened");
    }
    // Nor does opening a recording.
    let opened = open_lease(&service, &receive, &fixture.paths[1], "off-open");
    assert_eq!(job(&opened), (false, false, false));
    assert_eq!(state(&opened), crate::prepared::State::Idle);
    service.shutdown();
}

#[test]
fn library_admission_rechecks_the_users_preparation_under_the_admission_lock() {
    let fixture = LibraryFixture::new(&["opened", "library"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    // The scan found no preparation of the user's, then the user opened a recording.
    let opened = open_lease(&service, &receive, &fixture.paths[0], "race-open");
    assert_eq!(job(&opened), (true, false, false));
    // The scan goes on to admit its candidate.
    service
        .inner
        .prepare_recording(&fixture.paths[1], true)
        .unwrap();
    assert!(
        library_source(&service, &fixture.paths[1]).is_none_or(|source| !job(&source).0),
        "library work is not admitted beside the user's"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn library_work_that_slipped_past_an_open_pauses_at_the_next_scan() {
    let fixture = LibraryFixture::new(&["opened", "library"]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths[1..]).unwrap();
    let library = library_source(&service, &fixture.paths[1]).unwrap();
    assert_eq!(job(&library), (true, true, false));
    // The user's preparation started without making way, as a race leaves it.
    let opened = service.open_map(&fixture.paths[0], None).unwrap();
    service
        .inner
        .start_preparation(
            Arc::clone(&opened),
            jobs::Admission::Opened,
            service.inner.check_store(&opened, true),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(job(&opened), (true, false, false));
    service.prepare_library(&fixture.paths).unwrap();
    assert_eq!(
        paused(&library),
        Some(crate::prepared::PauseReason::Requested),
        "the next scan pauses library work running beside the user's"
    );
    assert_eq!(job(&opened), (true, false, false));
    drop(hold);
    service.shutdown();
}

#[test]
fn a_stale_scan_never_pauses_preparation_the_user_asked_for() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["manual"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, &fixture.paths[0]).unwrap();
    // The scan saw library work...
    let stale = [Arc::clone(&source)];
    assert_eq!(job(&source), (true, true, false));
    // ...then the user pressed Prepare, making it theirs.
    library_action(
        &service,
        Arc::clone(&source),
        Action::Start,
        "manual-start",
        &receive,
    );
    assert_eq!(job(&source), (true, false, false));
    // The stale scan's low-space result does not pause it.
    service
        .inner
        .pause_library(&stale, crate::prepared::PauseReason::DiskSpace);
    assert_eq!(job(&source), (true, false, false));
    assert_eq!(paused(&source), None);
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    TEST_ROOM.with(|room| room.set(None));
    assert_eq!(
        job(&source),
        (true, false, false),
        "the floor never bounds the user's work"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn opening_a_recording_whose_library_work_is_pausing_makes_it_the_users() {
    let fixture = LibraryFixture::new(&["stopping"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).unwrap();
    assert_eq!(job(&source), (true, true, false));
    // The library job is asked to stop but is still queued...
    crate::prepared::pause(&source, crate::prepared::PauseReason::NewerRecording);
    assert_eq!(job(&source), (true, true, true));
    // ...when the user opens its recording.
    let opened = open_lease(&service, &receive, path, "stopping-open");
    assert!(Arc::ptr_eq(&opened, &source));
    assert_eq!(
        job(&source),
        (true, false, true),
        "the open makes the stopping job the user's"
    );
    drop(hold);
    crate::prepared::wait_finished(&source, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        paused(&source),
        Some(crate::prepared::PauseReason::NewerRecording)
    );
    let hold = PreparationHold::new(&service);
    // Below the floor, which bounds only library work, it still resumes.
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    TEST_ROOM.with(|room| room.set(None));
    assert_eq!(
        job(&source),
        (true, false, false),
        "it resumes as the user's, not as library work"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn a_stale_library_admission_never_takes_back_the_users_job() {
    let fixture = LibraryFixture::new(&["claimed"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).unwrap();
    // A pause is stopping the library job, which a scan sees as library work...
    crate::prepared::pause(&source, crate::prepared::PauseReason::NewerRecording);
    assert_eq!(job(&source), (true, true, true));
    // ...when the user opens its recording and claims it.
    let opened = open_lease(&service, &receive, path, "claim-open");
    assert!(Arc::ptr_eq(&opened, &source));
    assert_eq!(job(&source), (true, false, true));
    drop(hold);
    crate::prepared::wait_finished(&source, &AtomicBool::new(false)).unwrap();
    assert!(!service.inner.interactive_preparation());
    // The stale scan's library admission now reaches it.
    let hold = PreparationHold::new(&service);
    service.inner.prepare_recording(path, true).unwrap();
    assert_eq!(
        job(&source),
        (true, false, false),
        "it resumes as the user's, not as library work"
    );
    drop(hold);
    service.shutdown();
}

/// Whether the user's claim on the source's job is still open.
fn claimed(source: &Source) -> bool {
    source
        .state::<crate::prepared::Progress>()
        .lock()
        .unwrap()
        .claimed
}

#[test]
fn a_finished_claim_does_not_carry_into_a_later_revision() {
    let fixture = LibraryFixture::new(&["continued"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).unwrap();
    // The user opens the recording, claiming its job, which then finishes.
    let (lease, opened) = open_view(&service, &receive, path, "claim-view");
    assert!(Arc::ptr_eq(&opened, &source));
    wait_library_source(&source);
    assert!(!claimed(&source), "a finished job is no longer claimed");
    // The view closes before the next scan retires the source.
    service.dispatch(DataCommand::Release { source_id: lease });
    drop(opened);
    // Another session appends to the recording.
    fixture.append(path);
    let hold = PreparationHold::new(&service);
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    TEST_ROOM.with(|room| room.set(None));
    assert!(
        library_source(&service, path).is_none_or(|held| !job(&held).0),
        "below the floor nothing the user does not hold is admitted"
    );
    service.prepare_library(&fixture.paths).unwrap();
    let grown = library_source(&service, path).unwrap();
    assert_ne!(grown.key().revision, source.key().revision);
    assert_eq!(
        job(&grown),
        (true, true, false),
        "the new revision is library work"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn a_recording_whose_store_cannot_be_checked_is_prepared_as_library_work() {
    let fixture = LibraryFixture::new(&["unreadable-store"]);
    let path = &fixture.paths[0];
    // A manifest too short to hold its header cannot be read.
    let store = crate::prepared::recording_store(path).unwrap();
    std::fs::create_dir_all(store.directory()).unwrap();
    std::fs::write(store.directory().join("manifest"), b"bad").unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    // It is library work: the floor bounds it.
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    TEST_ROOM.with(|room| room.set(None));
    assert!(library_source(&service, path).is_none());
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).expect("an unknown store is admitted");
    crate::prepared::wait_finished(&source, &AtomicBool::new(false)).unwrap();
    {
        let progress = source.state::<crate::prepared::Progress>();
        let progress = progress.lock().unwrap();
        assert!(progress.automatic && progress.background.load(Ordering::Acquire));
        assert!(!progress.claimed, "nobody holds it");
    }
    let reply = crate::prepared::status(&source, "unreadable", "status");
    assert_eq!(reply.state, crate::prepared::State::Failed);
    assert!(
        reply
            .error
            .is_some_and(|error| !error.contains("could not be checked")),
        "the preparation's own failure is shown"
    );
    // The failure is terminal and clearable: the next scan retires it.
    service.prepare_library(&fixture.paths).unwrap();
    assert!(library_source(&service, path).is_none());
    assert!(
        service
            .inner
            .sources
            .lock()
            .unwrap()
            .completed
            .contains_key(path)
    );
    service.shutdown();
}

#[test]
fn paused_library_work_completed_by_another_writer_ends_ready() {
    let fixture = LibraryFixture::new(&["paused"]);
    let path = &fixture.paths[0];
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).unwrap();
    TEST_ROOM.with(|room| room.set(Some(false)));
    service.prepare_library(&fixture.paths).unwrap();
    drop(hold);
    crate::prepared::wait_finished(&source, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        paused(&source),
        Some(crate::prepared::PauseReason::DiskSpace)
    );
    // Another writer completes the store while the job is paused.
    prepare_elsewhere(path);
    TEST_ROOM.with(|room| room.set(None));
    service.prepare_library(&[]).unwrap();
    let reply = crate::prepared::status(&source, "paused", "status");
    assert_eq!(reply.state, crate::prepared::State::Ready);
    assert_eq!(reply.pause_reason, None);
    assert_eq!(reply.error, None);
    assert!(!job(&source).0, "nothing prepares it again");
    service.shutdown();
}

#[test]
fn a_damaged_store_is_cleared_and_prepared_again() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["damaged"]);
    let path = &fixture.paths[0];
    // A manifest too short to hold its header cannot be read or bound.
    let store = crate::prepared::recording_store(path).unwrap();
    std::fs::create_dir_all(store.directory()).unwrap();
    let manifest = store.directory().join("manifest");
    std::fs::write(&manifest, b"bad").unwrap();
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let source = service.open_map(path, None).unwrap();
    library_action(
        &service,
        Arc::clone(&source),
        Action::Clear,
        "clear-damaged",
        &receive,
    );
    assert!(!manifest.exists(), "Clear removes the damaged store");
    library_action(
        &service,
        Arc::clone(&source),
        Action::Start,
        "prepare-cleared",
        &receive,
    );
    wait_library_source(&source);
    assert_eq!(
        crate::prepared::unprepared_blocks(&source, None).unwrap(),
        Some(0)
    );
    service.shutdown();
}

#[test]
fn a_complete_store_that_cannot_be_restored_is_prepared_again() {
    let fixture = LibraryFixture::new(&["damaged-branch"]);
    let path = &fixture.paths[0];
    // An earlier app session prepared it completely...
    let earlier = Service::new(Arc::new(|_| true)).unwrap();
    let prepared = earlier.open_map(path, None).unwrap();
    crate::prepared::pipeline::run(&prepared, None, &AtomicBool::new(false), |_| {}).unwrap();
    // ...then its branch continuations were damaged; the checkpoint stays complete.
    let (store, key) = crate::prepared::files(&prepared, None).unwrap();
    let branches: std::collections::BTreeSet<_> = prepared
        .replay_ref()
        .index
        .iter()
        .map(|block| block.branch)
        .collect();
    let mut writer = store.extend(&key).unwrap();
    for branch in branches {
        writer
            .write_at(&format!("preparation-branch-{branch}"), 0, &b"damaged"[..])
            .unwrap();
    }
    writer.commit().unwrap();
    assert_eq!(
        crate::prepared::unprepared_blocks(&prepared, None).unwrap(),
        Some(0)
    );
    assert!(!matches!(
        crate::prepared::pipeline::restore(&prepared, None, &AtomicBool::new(false)),
        Ok(Some(_))
    ));
    drop(prepared);
    earlier.shutdown();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).expect("it is admitted");
    assert_ne!(state(&source), crate::prepared::State::Ready);
    assert_eq!(
        job(&source),
        (true, true, false),
        "it is prepared again as library work"
    );
    assert!(
        !service
            .inner
            .sources
            .lock()
            .unwrap()
            .completed
            .contains_key(path)
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn a_prepare_that_could_not_be_queued_leaves_no_claim() {
    let fixture = LibraryFixture::new(&["queue-full"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let library = library_source(&service, path).unwrap();
    // Opened while a pause stops its library job: it ends paused and claimed.
    crate::prepared::pause(&library, crate::prepared::PauseReason::NewerRecording);
    drop(library);
    let (lease, opened) = open_view(&service, &receive, path, "paused-view");
    drop(hold);
    crate::prepared::wait_finished(&opened, &AtomicBool::new(false)).unwrap();
    assert_eq!(state(&opened), crate::prepared::State::Paused);
    assert!(claimed(&opened));
    // An explicit Prepare meets a full queue.
    let hold = PreparationHold::filled(&service);
    let error = service
        .inner
        .admit_interactive(
            &opened,
            jobs::Admission::Manual,
            service.inner.check_store(&opened, true),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(state(&opened), crate::prepared::State::Failed);
    assert!(!claimed(&opened), "a failed submission ends the claim");
    // The view closes and a session appends.
    service.dispatch(DataCommand::Release { source_id: lease });
    drop(opened);
    fixture.append(path);
    drop(hold);
    admit_library(&service, &fixture.paths);
    let grown = library_source(&service, path).unwrap();
    assert_eq!(
        grown.key().revision,
        crate::protocol::replay_revision(path).unwrap()
    );
    assert!(job(&grown).1, "the new revision is library work");
    assert!(!claimed(&grown));
    service.shutdown();
}

#[test]
fn a_view_closed_while_the_scan_orders_its_work_leaves_library_work() {
    let fixture = LibraryFixture::new(&["released"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.prepare_library(&fixture.paths).unwrap();
    let earlier = library_source(&service, path).unwrap();
    // The user views it; the job finishes, ending the claim, while the view stays.
    let (lease, opened) = open_view(&service, &receive, path, "released-view");
    assert!(Arc::ptr_eq(&opened, &earlier));
    wait_library_source(&earlier);
    assert!(!claimed(&earlier));
    drop(opened);
    // Another session appends.
    fixture.append(path);
    // The view closes right after the scan ordered the recording as the user's.
    TEST_AFTER_PARTITION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |service: &Service| {
            service.dispatch(DataCommand::Release { source_id: lease });
        }));
    });
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let grown = library_source(&service, path).unwrap();
    assert_ne!(grown.key().revision, earlier.key().revision);
    assert_eq!(
        job(&grown),
        (true, true, false),
        "admitted as library work, decided at admission"
    );
    assert!(!claimed(&grown));
    drop(hold);
    service.shutdown();
}

/// Prepare `path` completely, as an earlier app session or another writer would.
fn prepare_elsewhere(path: &Path) {
    let other = Service::new(Arc::new(|_| true)).unwrap();
    let writer = other.open_map(path, None).unwrap();
    crate::prepared::pipeline::run(&writer, None, &AtomicBool::new(false), |_| {}).unwrap();
    drop(writer);
    other.shutdown();
}

#[test]
fn a_restore_read_for_replaced_content_is_never_published() {
    let fixture = LibraryFixture::new(&["rebound"]);
    let path = &fixture.paths[0];
    prepare_elsewhere(path);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    // The content binding changes after the scan restored the store, before
    // it publishes: the rebind invalidates every check under the admission lock.
    jobs::TEST_AFTER_CHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|inner: &Inner| {
            let _admission = inner.preparation_actions.lock().unwrap();
            inner.invalidate_checks();
        }));
    });
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, path).expect("it is admitted as usual");
    assert_ne!(
        state(&source),
        crate::prepared::State::Ready,
        "the restore for the old binding is dropped"
    );
    assert_eq!(job(&source), (true, true, false));
    assert!(
        !service
            .inner
            .sources
            .lock()
            .unwrap()
            .completed
            .contains_key(path)
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn admission_queues_a_store_the_unlocked_check_found_incomplete_without_restoring_again() {
    let fixture = LibraryFixture::new(&["checked-once"]);
    let path = fixture.paths[0].clone();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    // Another writer completes the store after the scan's unlocked check found
    // it incomplete. Admission uses that check rather than restoring again
    // under the admission lock, so it queues the preparation.
    let writer = path.clone();
    jobs::TEST_AFTER_CHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_: &Inner| prepare_elsewhere(&writer)));
    });
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, &path).unwrap();
    assert_eq!(
        job(&source),
        (true, true, false),
        "queued, not restored under the lock"
    );
    assert_eq!(state(&source), crate::prepared::State::Queued);
    drop(hold);
    service.shutdown();
}

#[test]
fn admission_never_measures_the_volume_under_the_admission_lock() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["measured-a", "measured-b"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    // The scan measures once, before it admits anything.
    service.prepare_library(&fixture.paths).unwrap();
    // Opens and Prepare are the user's and need no measurement.
    let opened = open_lease(&service, &receive, &fixture.paths[0], "measured-open");
    library_action(
        &service,
        Arc::clone(&opened),
        Action::Start,
        "measured-prepare",
        &receive,
    );
    // A refresh whose measurement is stale measures again, outside the lock.
    service.inner.room.lock().unwrap().measured = None;
    let before = service.inner.test_measures.load(Ordering::Relaxed);
    service
        .inner
        .refresh_preparation(Arc::clone(&opened), false)
        .unwrap();
    let started = std::time::Instant::now();
    while service.inner.test_measures.load(Ordering::Relaxed) == before {
        assert!(started.elapsed() < HANG_GUARD, "the refresh never measured");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(service.inner.test_measures.load(Ordering::Relaxed) >= 2);
    assert_eq!(
        service.inner.test_measures_locked.load(Ordering::Relaxed),
        0,
        "no admission measures the volume under the admission lock"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn work_the_user_stops_holding_mid_scan_yields_to_newer_work() {
    let fixture = LibraryFixture::new(&["older", "newer"]);
    let (older, newer) = (fixture.paths[0].clone(), fixture.paths[1].clone());
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    // The older recording is prepared while the user views it, ending the claim.
    service
        .prepare_library(std::slice::from_ref(&older))
        .unwrap();
    let earlier = library_source(&service, &older).unwrap();
    let (lease, opened) = open_view(&service, &receive, &older, "older-view");
    wait_library_source(&earlier);
    drop(opened);
    // Sessions append to both; the newer recording stays the newest.
    fixture.append(&older);
    fixture.append(&newer);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&newer)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() + Duration::from_secs(60)),
        )
        .unwrap();
    // The view closes right after the scan ordered the older one as held.
    TEST_AFTER_PARTITION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |service: &Service| {
            service.dispatch(DataCommand::Release { source_id: lease });
        }));
    });
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let older_job = library_source(&service, &older).unwrap();
    let newer_job = library_source(&service, &newer).unwrap();
    assert_ne!(older_job.key().revision, earlier.key().revision);
    assert_eq!(job(&newer_job), (true, true, false));
    assert_eq!(
        paused(&older_job),
        Some(crate::prepared::PauseReason::NewerRecording),
        "the older library work yields to the newer"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn a_clear_between_prepares_check_and_admission_makes_it_rebuild() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["cleared-between"]);
    let path = &fixture.paths[0];
    prepare_elsewhere(path);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let source = service.open_map(path, None).unwrap();
    // Prepare checks the complete store outside the admission lock...
    let checked = service.inner.check_store(&source, true);
    assert!(matches!(checked.restored, Ok(Some(_))));
    // ...and a Clear runs before Prepare is admitted.
    library_action(
        &service,
        Arc::clone(&source),
        Action::Clear,
        "clear-between",
        &receive,
    );
    let hold = PreparationHold::new(&service);
    {
        let _admission = service.inner.preparation_actions.lock().unwrap();
        service
            .inner
            .admit_interactive(
                &source,
                jobs::Admission::Manual,
                checked,
                &AtomicBool::new(false),
            )
            .unwrap();
    }
    assert_ne!(
        state(&source),
        crate::prepared::State::Ready,
        "the cleared store is not reported Ready"
    );
    assert_eq!(job(&source), (true, false, false), "it is rebuilt");
    drop(hold);
    service.shutdown();
}

#[test]
fn a_content_restart_of_the_users_prepare_restarts_it_as_theirs() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["restarted"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    service.set_background_preparation(false);
    let hold = PreparationHold::new(&service);
    let source = service.open_map(path, None).unwrap();
    library_action(
        &service,
        Arc::clone(&source),
        Action::Start,
        "prepare-before-restart",
        &receive,
    );
    assert!(job(&source).0);
    let admitted = admission_of(&source);
    // A content change restarts the running Prepare.
    service
        .inner
        .refresh_preparation(Arc::clone(&source), true)
        .unwrap();
    // The cancelled job leaves the queue once the workers are free.
    drop(hold);
    let started = std::time::Instant::now();
    while admission_of(&source) == admitted {
        assert!(
            started.elapsed() < HANG_GUARD,
            "the restart was refused as library work"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap()
            .automatic,
        "it restarts as the user's Prepare"
    );
    service.shutdown();
}

#[test]
fn work_that_loses_its_view_yields_to_newer_work_already_running() {
    let fixture = LibraryFixture::new(&["older", "newer"]);
    let (older, newer) = (fixture.paths[0].clone(), fixture.paths[1].clone());
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    // The older recording is prepared while the user views it, ending the claim.
    service
        .prepare_library(std::slice::from_ref(&older))
        .unwrap();
    let earlier = library_source(&service, &older).unwrap();
    let (lease, opened) = open_view(&service, &receive, &older, "older-view");
    wait_library_source(&earlier);
    drop(opened);
    // Sessions append to both; the newer recording stays the newest.
    fixture.append(&older);
    fixture.append(&newer);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&newer)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() + Duration::from_secs(60)),
        )
        .unwrap();
    // The newer recording's library work is already running.
    let hold = PreparationHold::new(&service);
    service.inner.prepare_recording(&newer, true).unwrap();
    let newer_job = library_source(&service, &newer).unwrap();
    assert_eq!(job(&newer_job), (true, true, false));
    // The older one's view closes right after the scan ordered it as held.
    TEST_AFTER_PARTITION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |service: &Service| {
            service.dispatch(DataCommand::Release { source_id: lease });
        }));
    });
    service.prepare_library(&fixture.paths).unwrap();
    let older_job = library_source(&service, &older).unwrap();
    assert_ne!(older_job.key().revision, earlier.key().revision);
    assert_eq!(
        paused(&older_job),
        Some(crate::prepared::PauseReason::NewerRecording),
        "the older library work yields to the newer one already running"
    );
    assert_eq!(job(&newer_job), (true, true, false));
    drop(hold);
    service.shutdown();
}

/// Wait until a refresh submitted after `before` reached its admission decision.
fn wait_refresh(service: &Service, before: u64) {
    let started = std::time::Instant::now();
    while service.inner.test_refreshes.load(Ordering::Acquire) == before {
        assert!(started.elapsed() < HANG_GUARD, "the refresh never decided");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_cancel_during_a_restarts_check_leaves_it_cancelled() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["cancel-restart"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    let source = service.open_map(path, None).unwrap();
    library_action(
        &service,
        Arc::clone(&source),
        Action::Start,
        "prepare-to-restart",
        &receive,
    );
    assert_eq!(job(&source), (true, false, false));
    // The user Cancels while a content restart of their Prepare checks the store.
    let hooked = Arc::clone(&source);
    *service.inner.test_refresh_checked.lock().unwrap() = Some(Box::new(move |inner: &Inner| {
        let _admission = inner.preparation_actions.lock().unwrap();
        inner.cancel_by_user(&hooked);
    }));
    let before = service.inner.test_refreshes.load(Ordering::Acquire);
    service
        .inner
        .refresh_preparation(Arc::clone(&source), true)
        .unwrap();
    // The restart's own cancellation needs a worker to finish.
    drop(hold);
    wait_refresh(&service, before);
    assert!(!job(&source).0, "the restart did not override the Cancel");
    assert_eq!(state(&source), crate::prepared::State::Cancelled);
    assert!(
        service
            .inner
            .sources
            .lock()
            .unwrap()
            .suppressed
            .contains(path.as_path())
    );
    service.shutdown();
}

#[test]
fn a_refresh_admitted_after_newer_library_work_yields_to_it() {
    let fixture = LibraryFixture::new(&["older", "newer"]);
    let (older, newer) = (&fixture.paths[0], &fixture.paths[1]);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let hold = PreparationHold::new(&service);
    // A scan admitted the newer recording.
    service.inner.prepare_recording(newer, true).unwrap();
    let newer_job = library_source(&service, newer).unwrap();
    assert_eq!(job(&newer_job), (true, true, false));
    // Then a refresh admits the older one as library work.
    let older_source = service.open_map(older, None).unwrap();
    let before = service.inner.test_refreshes.load(Ordering::Acquire);
    service
        .inner
        .refresh_preparation(Arc::clone(&older_source), false)
        .unwrap();
    wait_refresh(&service, before);
    assert_eq!(job(&older_source), (true, true, true));
    assert_eq!(
        paused(&older_source),
        Some(crate::prepared::PauseReason::NewerRecording),
        "the refresh's library admission keeps newest-first"
    );
    assert_eq!(job(&newer_job), (true, true, false));
    drop(hold);
    service.shutdown();
}

#[test]
fn a_store_cleared_elsewhere_after_its_check_is_not_left_ready() {
    let fixture = LibraryFixture::new(&["cleared-elsewhere"]);
    let path = fixture.paths[0].clone();
    prepare_elsewhere(&path);
    let service = Service::new(Arc::new(|_| true)).unwrap();
    // Another service or process clears the store after the scan restored it.
    let cleared = path.clone();
    jobs::TEST_AFTER_CHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_: &Inner| {
            crate::prepared::recording_store(&cleared)
                .unwrap()
                .clear()
                .unwrap();
        }));
    });
    service.prepare_library(&fixture.paths).unwrap();
    let source = library_source(&service, &path).expect("kept for the next scan");
    assert_ne!(
        state(&source),
        crate::prepared::State::Ready,
        "the publication over a cleared store is withdrawn"
    );
    assert!(
        !service
            .inner
            .sources
            .lock()
            .unwrap()
            .completed
            .contains_key(&path)
    );
    service.shutdown();
}

#[test]
fn no_recording_revision_is_read_under_the_admission_lock() {
    let fixture = LibraryFixture::new(&["revision-a", "revision-b"]);
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    service.prepare_library(&fixture.paths).unwrap();
    let _opened = open_lease(&service, &receive, &fixture.paths[0], "revision-open");
    // Turning background preparation back on reads every live source's revision.
    service.set_background_preparation(false);
    service.set_background_preparation(true);
    assert!(service.inner.test_revisions.load(Ordering::Relaxed) > 0);
    assert_eq!(
        service.inner.test_revisions_locked.load(Ordering::Relaxed),
        0,
        "no recording revision is read under the admission lock"
    );
    drop(hold);
    service.shutdown();
}

#[test]
fn first_ask_priority_is_bounded_on_prepared_and_sampled_lanes() {
    for lane in [Lane::Prepared, Lane::Sampled] {
        let service = Service::new(Arc::new(|_| true)).unwrap();
        let hold = LoaderHold::new(&service, lane);
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (first_sent, first_seen) = mpsc::channel();
        let (refresh_sent, refresh_seen) = mpsc::channel();
        let refresh = service.inner.begin("refresh".into()).unwrap();
        refresh.refresh.store(true, Ordering::Release);
        service.inner.submit(
            Arc::clone(&refresh),
            LoadJob::Fixture {
                request: refresh,
                lane,
                run: Arc::new(move || {
                    refresh_sent.send(()).unwrap();
                }),
            },
        );
        for index in 0..lane.threads(parallelism()) {
            let request = service.inner.begin(format!("first-{index}")).unwrap();
            let release = Arc::clone(&release);
            let sent = first_sent.clone();
            service.inner.submit(
                Arc::clone(&request),
                LoadJob::Fixture {
                    request,
                    lane,
                    run: Arc::new(move || {
                        sent.send(()).unwrap();
                        let (lock, changed) = &*release;
                        let mut ready = lock.lock().unwrap();
                        while !*ready {
                            ready = changed.wait(ready).unwrap();
                        }
                    }),
                },
            );
        }
        drop(hold);
        for _ in 0..lane.threads(parallelism()) {
            first_seen.recv_timeout(HANG_GUARD).unwrap();
        }
        let early_refresh = refresh_seen.try_recv().is_ok();
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        service.shutdown();
        assert_eq!(
            early_refresh,
            lane.threads(parallelism()) > queue::FIRST_ASK_BURST,
            "{lane:?} did not apply the first-ask burst bound"
        );
    }
}

/// A source opening while the folders change is registered before its first binding, so the
/// folder change's writer rebinds it too, and the opening's own stale rebind retries: it opens
/// bound to the newest folders, never to the ones it started with or to none.
#[test]
fn a_source_opening_while_the_folders_change_is_bound_to_the_newest() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("opening.battleplan");
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![crate::test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, replay.bytes()).unwrap();
    let root = folder.0.clone();
    let folders = move |user: &str| ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: root.clone(),
            user_dir: root.join(user),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let old = folders("old");
    service.inner.content.update(|content| *content = Some(old));
    let newer = folders("new");
    let seen = Arc::new(AtomicBool::new(false));
    *service.inner.test_rebind_derived.lock().unwrap() = Some(Box::new({
        let seen = Arc::clone(&seen);
        move |inner: &Inner| {
            // The folders change while the opening source's binding is derived; the change's
            // writer rebinds every source it sees, the opening one included.
            inner.content.update(|content| *content = Some(newer));
            let sources = inner.bound_sources();
            seen.store(!sources.is_empty(), Ordering::Release);
            inner.present(&sources, &AtomicBool::new(false)).unwrap();
        }
    }));
    let source = service
        .inner
        .open(&path, None, &AtomicBool::new(false))
        .unwrap();
    assert!(
        seen.load(Ordering::Acquire),
        "the opening source is registered before it is bound"
    );
    let binding = source.binding().expect("bound");
    assert_eq!(binding.options.user_dir, folder.0.join("new"));
    service.shutdown();
}

/// A committed settings change catches every source up even when the request that made it is
/// cancelled during the rebind: the catch-up is the service's, so a newer change landing
/// meanwhile is caught up too, and no source stays on a selection no longer saved.
#[test]
fn a_cancelled_settings_request_still_catches_every_source_up() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("catch_up.battleplan");
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![crate::test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, replay.bytes()).unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let options = installed_content_core::ResolveOptions {
        game_dir: folder.0.clone(),
        user_dir: folder.0.join("user"),
        cache_dir: folder.0.join("cache"),
    };
    service.inner.content.update(|content| {
        *content = Some(ContentOptions {
            options: options.clone(),
            allow_unverified: true,
            presentation: Default::default(),
        })
    });
    let never = AtomicBool::new(false);
    let source = service.inner.open(&path, None, &never).unwrap();
    // The source shows a selection the saved settings no longer hold.
    let stale = crate::presentation::Presentation {
        visual_mods: vec![installed_content_core::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }],
        revision: "stale".into(),
    };
    source.bind(Some((&options, true)), stale.clone());
    let request = Request::new("settings".into());
    *service.inner.test_rebind_derived.lock().unwrap() = Some(Box::new({
        let request = Arc::clone(&request);
        move |inner: &Inner| {
            // The panel closes while the rebind derives, and another change lands meanwhile.
            request.cancel();
            inner.content.update(|_| ());
        }
    }));
    let _ = service.inner.map_settings_job(super::jobs::MapSettingsJob {
        target: Default::default(),
        request: Arc::clone(&request),
        source_id: String::new(),
        source: Some(Arc::clone(&source)),
        settings: None,
        flip: None,
        options,
    });
    assert!(request.cancelled());
    let binding = source.binding().expect("bound");
    assert_ne!(binding.presentation, stale, "the source caught up");
    assert!(binding.presentation.visual_mods.is_empty());
    service.shutdown();
}

/// A rebind in flight when the service shuts down (here a map settings save's) installs
/// nothing: shutdown closes the configuration before unbinding, so the source it unbound stays
/// unbound, and no rebind starts afterwards.
#[test]
fn a_rebind_paused_across_shutdown_leaves_the_source_unbound() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let path = folder.0.join("closing.battleplan");
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![crate::test_fixtures::source(1, 0, Some(1))],
        )],
    )
    .unwrap();
    std::fs::write(&path, replay.bytes()).unwrap();
    let service = Service::new(Arc::new(|_| true)).unwrap();
    service.inner.content.update(|content| {
        *content = Some(ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.join("user"),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        })
    });
    let never = AtomicBool::new(false);
    let source = service.inner.open(&path, None, &never).unwrap();
    assert!(source.binding().is_some());
    let closed = Arc::new(AtomicBool::new(false));
    *service.inner.test_rebind_derived.lock().unwrap() = Some(Box::new({
        let closed = Arc::clone(&closed);
        move |inner: &Inner| {
            inner.stop();
            closed.store(true, Ordering::Release);
        }
    }));
    assert!(service.inner.rebind(&source, &never).is_err());
    assert!(
        closed.load(Ordering::Acquire),
        "shutdown ran during the rebind"
    );
    assert!(
        source.binding().is_none(),
        "the stale binding is not installed"
    );
    assert!(service.inner.rebind(&source, &never).is_err());
    assert!(source.binding().is_none());
}

/// A map settings request admitted under earlier folders reads the game's settings of the folders
/// configured when it runs: it never restores the earlier folders' defaults.
#[test]
fn a_stale_map_settings_request_keeps_the_configured_folders_game_settings() {
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[(
        "new/settings.txt",
        "graphics={} mapRenderingOptions={}",
    )]);
    let root = folder.0.clone();
    let folders = |user: &str| installed_content_core::ResolveOptions {
        game_dir: root.clone(),
        user_dir: root.join(user),
        cache_dir: root.join("cache"),
    };
    let service = Service::new(Arc::new(|_| true)).unwrap();
    service.inner.content.update(|content| {
        *content = Some(ContentOptions {
            options: folders("new"),
            allow_unverified: true,
            presentation: Default::default(),
        })
    });
    // Admitted under a user folder without game settings, run after the change to "new".
    let _ = service.inner.map_settings_job(super::jobs::MapSettingsJob {
        target: Default::default(),
        request: Request::new("settings".into()),
        source_id: String::new(),
        source: None,
        settings: None,
        flip: None,
        options: folders("old"),
    });
    let store = service.inner.map_settings.lock().unwrap();
    assert!(store.game().is_some(), "{:?}", store.game_error());
    drop(store);
    service.shutdown();
}

/// A restart a rebind could not queue behind a full history queue is kept, and queued as soon
/// as a job leaves the queue, so it restarts as a queued refresh would.
#[test]
fn a_restart_deferred_by_a_full_history_queue_is_queued_once_it_has_room() {
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = LoaderHold::new(&service, Lane::History);
    let source = source();
    service
        .inner
        .refresh_preparation(Arc::clone(&source), false)
        .unwrap();
    let mut retained = Vec::new();
    for index in 0..MAX_PENDING * 2 {
        let id = format!("refresh-deferred-{index}");
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
        settled_live_requests(&service, &receive, &retained),
        MAX_PENDING - 1
    );
    let other = source_with_id("deferred-refresh");
    let deferred = |service: &Service| {
        service
            .inner
            .loader
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .deferred
            .contains_key(other.key())
    };
    service.inner.refresh_or_defer(Arc::clone(&other));
    assert!(deferred(&service), "a full queue keeps the restart");
    drop(hold);
    let started = std::time::Instant::now();
    while deferred(&service) {
        assert!(started.elapsed() < HANG_GUARD, "the deferred restart was never queued");
        std::thread::sleep(Duration::from_millis(10));
    }
    service.shutdown();
}

/// A restart that runs after the user's Cancel, while the job it stopped is still ending,
/// leaves it cancelled rather than restarting it as the user's.
#[test]
fn a_restart_after_a_cancel_still_ending_leaves_it_cancelled() {
    use crate::prepared::Action;
    let fixture = LibraryFixture::new(&["cancel-then-restart"]);
    let path = &fixture.paths[0];
    let (sent, receive) = mpsc::channel();
    let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
    let hold = PreparationHold::new(&service);
    let source = service.open_map(path, None).unwrap();
    library_action(
        &service,
        Arc::clone(&source),
        Action::Start,
        "prepare-then-cancel",
        &receive,
    );
    assert_eq!(job(&source), (true, false, false));
    {
        let _admission = service.inner.preparation_actions.lock().unwrap();
        service.inner.cancel_by_user(&source);
    }
    // The cancelled job is still running when the restart is decided.
    assert!(job(&source).0);
    service
        .inner
        .refresh_preparation(Arc::clone(&source), true)
        .unwrap();
    // Every history worker taken behind the restart proves it was decided.
    drop(LoaderHold::new(&service, Lane::History));
    drop(hold);
    let started = std::time::Instant::now();
    while job(&source).0 {
        assert!(started.elapsed() < HANG_GUARD, "the cancelled job never ended");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state(&source), crate::prepared::State::Cancelled);
    service.shutdown();
}
