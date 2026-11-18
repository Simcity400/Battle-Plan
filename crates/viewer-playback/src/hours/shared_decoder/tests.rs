use super::*;
use viewer_foundation::map::test_support::TEST_HANG_GUARD;

fn actor(replay: Arc<Replay>) -> SharedDecoder {
    SharedDecoder::new(replay).unwrap()
}
fn get(client: &Arc<SourceMailbox>, id: u64, key: (BranchId, Hour)) -> Current {
    let (send, done) = mpsc::channel();
    let _handle = client.post(id, key, move |_, reply| {
        send.send(reply).unwrap();
    });
    match done
        .recv_timeout(TEST_HANG_GUARD)
        .expect("source actor replies")
    {
        source_mailbox::Reply::Ready { value, .. } => value,
        _ => panic!("ordinary request is not cancelled or stopped"),
    }
}
fn assert_current(actual: &Current, expected: &Current) {
    match (actual, expected) {
        (Current::Frame(a), Current::Frame(b)) => assert_eq!(a.as_ref(), b.as_ref()),
        (
            Current::Gap {
                branch: ab,
                hour: ah,
                reason: ar,
            },
            Current::Gap {
                branch: bb,
                hour: bh,
                reason: br,
            },
        ) => assert_eq!((ab, ah, ar), (bb, bh, br)),
        _ => panic!("frame availability differs"),
    }
}

#[test]
fn actor_two_views_and_campaign_mailbox_match_every_selected_source_row_and_gap() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let decoder = actor(replay.clone());
    let first = decoder.client();
    let second = decoder.client();
    let campaign = super::super::tests::lane_over(&replay);
    let mut oracle = Playback::unloaded(replay).unwrap();
    // Hour101 contains unavailable/truncated tables;102 a recorded refusal.
    // Sparse/backward/duplicate endpoint requests stress the shared cache.
    for (at, hour) in [100, 101, 105, 102, 820, 101, 100].into_iter().enumerate() {
        oracle.seek(1, hour).unwrap();
        let expected = oracle.current();
        let a = get(&first, at as u64, (1, hour));
        let b = get(&second, at as u64, (1, hour));
        assert_current(&a, expected);
        assert_current(&b, expected);
        if let (Current::Frame(a), Current::Frame(b)) = (&a, &b) {
            assert!(
                Arc::ptr_eq(a, b),
                "live views share the immutable cached frame"
            );
        }
        let c = get(&campaign.client(), at as u64, (1, hour));
        assert_current(&c, expected);
    }
}

#[test]
fn actor_seed_is_ordered_and_exit_terminalizes_pending_and_future_clients() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let decoder = actor(replay.clone());
    let client = decoder.client();
    let mut seed = Playback::unloaded(replay).unwrap();
    seed.seek(1, 105).unwrap();
    let expected = seed.current().clone();
    decoder.seed(seed).unwrap();
    assert_current(&get(&client, 1, (1, 105)), &expected);
    // A source-exit snapshot stops all registered clients outside registry and
    // mailbox locks. Registering afterwards sees dead and cannot lose a reply.
    let registry = decoder.actor.clients.clone();
    // Drive the same exit guard explicitly; no thread timing assumption.
    drop(Exit(registry));
    let late = decoder.client();
    let (send, done) = mpsc::channel();
    let _handle = late.post(2, (1, 106), move |_, reply| {
        send.send(reply).unwrap();
    });
    assert!(matches!(
        done.recv_timeout(TEST_HANG_GUARD).unwrap(),
        source_mailbox::Reply::Stopped
    ));
}

#[test]
fn campaign_cancelled_mailbox_read_never_requires_another_want_to_wake() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let lane = super::super::tests::lane_over(&replay);
    let client = lane.client();
    let (send, done) = mpsc::channel();
    let handle = client.post(1, (1, 820), move |_, reply| {
        send.send(reply).unwrap();
    });
    handle.cancel();
    // Either service completed before cancellation, or the cancellation wins.
    // Both are terminal and credit-safe; neither needs an unrelated Want.
    assert!(matches!(
        done.recv_timeout(TEST_HANG_GUARD).unwrap(),
        source_mailbox::Reply::Cancelled | source_mailbox::Reply::Ready { .. }
    ));
    assert!(done.try_recv().is_err(), "one terminal reply");
    let actual = get(&client, 2, (1, 101));
    let mut oracle = Playback::unloaded(replay).unwrap();
    oracle.seek(1, 101).unwrap();
    assert_current(&actual, oracle.current());
}

#[test]
fn scripted_colour_mailbox_traversal_visits_every_native_hour_and_resets_at_gaps() {
    use viewer_world::map::map_mode::ColourState;
    let replay = Arc::new(
        Replay::from_records(
            b"campaign".to_vec(),
            &[crate::playback::tests::records(
                1,
                &[10, 11, 12, 14, 15, 16, 17],
            )],
        )
        .unwrap(),
    );
    let decoder = actor(replay.clone());
    let client = decoder.client();
    let lane = super::super::tests::lane_over(&replay);
    let mut direct = Playback::unloaded(replay).unwrap();
    let mut old_colour = None;
    let mut actor_colour = None;
    let mut lane_colour = None;
    let mut from = 10;
    for endpoint in [12, 17] {
        let mut old_frames = Vec::new();
        let mut actor_frames = Vec::new();
        let mut lane_frames = Vec::new();
        for hour in from..=endpoint {
            direct.seek(1, hour).unwrap();
            old_frames.push(direct.current().clone());
            actor_frames.push(get(&client, hour as u64, (1, hour)));
            lane_frames.push(get(&lane.client(), hour as u64, (1, hour)));
        }
        let mut traversals = Vec::new();
        for (colour, frames) in [
            (&mut old_colour, old_frames),
            (&mut actor_colour, actor_frames),
            (&mut lane_colour, lane_frames),
        ] {
            let mut visits = Vec::new();
            *colour = carry_colour(
                colour.take(),
                frames,
                |frame, previous: Option<&ColourState>| {
                    visits.push(frame.hour);
                    let mode = format!(
                        "{}:{}",
                        previous.map_or("cold", |p| p.mode.as_str()),
                        frame.hour
                    );
                    Ok(Some(Arc::new(ColourState {
                        mode,
                        layers: Default::default(),
                    })))
                },
            )
            .unwrap();
            traversals.push(visits);
        }
        assert_eq!(traversals[0], traversals[1]);
        assert_eq!(traversals[0], traversals[2]);
        assert_eq!(
            old_colour.as_ref().unwrap().mode.as_bytes(),
            actor_colour.as_ref().unwrap().mode.as_bytes()
        );
        assert_eq!(
            old_colour.as_ref().unwrap().mode.as_bytes(),
            lane_colour.as_ref().unwrap().mode.as_bytes()
        );
        from = endpoint + 1;
    }
    assert_eq!(old_colour.unwrap().mode, "cold:14:15:16:17");
}

#[test]
fn stalled_actor_keeps_one_wake_and_does_not_retain_cancelled_clients() {
    // Hold the actual actor command queue without servicing it: this is the
    // precise stalled-source boundary, independent of OS thread scheduling.
    let (events, inbox) = mpsc::channel();
    let actor = Actor {
        events,
        clients: Default::default(),
        closing: Arc::new(AtomicBool::new(false)),
    };
    let live = actor.client();
    let (tx, rx) = mpsc::channel();
    let _live = live.post(1, (1, 10), move |_, reply| {
        tx.send(reply).unwrap();
    });
    for id in 2..1002 {
        let client = actor.client();
        let weak = Arc::downgrade(&client);
        let (tx, rx) = mpsc::channel();
        let handle = client.post(id, (1, id as i32), move |_, reply| {
            tx.send(reply).unwrap();
        });
        handle.cancel();
        assert!(matches!(
            rx.try_recv().unwrap(),
            source_mailbox::Reply::Cancelled
        ));
        drop(client);
        assert!(
            weak.upgrade().is_none(),
            "queued wake must not own a discarded map"
        );
        let clients = actor.clients.lock().unwrap();
        assert!(clients.ready.len() <= 2);
        assert!(clients.mailboxes.len() <= 2);
    }
    assert!(matches!(inbox.try_recv().unwrap(), Command::ReadReady));
    assert!(inbox.try_recv().is_err(), "one coalesced ready-client wake");
    let mailbox = Actor::take_ready(&actor.clients, &actor.events).unwrap();
    assert!(Arc::ptr_eq(&mailbox, &live));
    mailbox.take().unwrap().ready(
        Current::Gap {
            branch: 1,
            hour: 10,
            reason: "fixture".into(),
        },
        None,
    );
    assert!(matches!(
        rx.try_recv().unwrap(),
        source_mailbox::Reply::Ready { .. }
    ));
    // A dead tail may have one already posted wake, but cannot strand a new
    // request when it is drained: registration/wake and take share the lock.
    while inbox.try_recv().is_ok() {
        let _ = Actor::take_ready(&actor.clients, &actor.events);
    }
    let (tx, rx) = mpsc::channel();
    let _next = live.post(1003, (1, 11), move |_, reply| {
        tx.send(reply).unwrap();
    });
    assert!(matches!(inbox.try_recv().unwrap(), Command::ReadReady));
    Actor::take_ready(&actor.clients, &actor.events)
        .unwrap()
        .take()
        .unwrap()
        .ready(
            Current::Gap {
                branch: 1,
                hour: 11,
                reason: "next".into(),
            },
            None,
        );
    assert!(matches!(
        rx.try_recv().unwrap(),
        source_mailbox::Reply::Ready { .. }
    ));
}

#[test]
fn the_shared_registry_keeps_every_live_frame_and_prunes_dead_entries_on_decode() {
    let replay = Arc::new(
        Replay::from_records(
            b"campaign".to_vec(),
            &[crate::playback::tests::records(
                1,
                &(100..=229).collect::<Vec<_>>(),
            )],
        )
        .unwrap(),
    );
    let mut state = DecoderState {
        playback: Playback::unloaded(replay.clone()).unwrap(),
        frames: BTreeMap::new(),
        workers: 1,
        dropper: None,
    };
    let mut held = Vec::new();
    for hour in 100..=227 {
        held.push(state.frame(1, hour).unwrap());
    }
    assert_eq!(state.frames.len(), 128);
    let Current::Frame(first) = &held[0] else {
        panic!("recorded hour")
    };
    assert!(
        matches!(state.frame(1, 100).unwrap(), Current::Frame(reused) if Arc::ptr_eq(first, &reused))
    );
    let weak = Arc::downgrade(first);
    held.clear();
    assert!(weak.upgrade().is_none(), "the registry owns no frame");
    let current = state.frame(1, 228).unwrap();
    let replay::campaign::SeekResult::Frame(expected) = replay.seek(1, 228).unwrap() else {
        panic!("recorded hour")
    };
    assert_eq!(current, Current::Frame(Arc::from(expected)));
    assert_eq!(state.frames.keys().copied().collect::<Vec<_>>(), [(1, 228)]);
}

#[test]
fn the_shared_host_hands_forward_edits_and_explicit_seek_and_gap_boundaries() {
    use replay_data::rows::TableId;
    use replay_frame::frame::{HourInput, TableChanges};
    let replay = Arc::new(
        Replay::from_records(
            b"campaign".to_vec(),
            &[crate::playback::tests::records(1, &[100, 103, 104])],
        )
        .unwrap(),
    );
    let mut state = DecoderState {
        playback: Playback::unloaded(replay).unwrap().with_display_changes(),
        frames: BTreeMap::new(),
        workers: 1,
        dropper: None,
    };
    let Current::Frame(first) = state.frame(1, 100).unwrap() else {
        panic!("first hour")
    };
    let identity = first.tables[TableId::Countries.index()].identity();
    let Current::Frame(after_gap) = state.frame(1, 103).unwrap() else {
        panic!("after gap")
    };
    assert!(matches!(
        HourInput::new(&after_gap)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX),
        TableChanges::Discontinuity(_)
    ));
    let identity = after_gap.tables[TableId::Countries.index()].identity();
    let Current::Frame(next) = state.frame(1, 104).unwrap() else {
        panic!("next hour")
    };
    let TableChanges::Rows(changes) =
        HourInput::new(&next)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX)
    else {
        panic!("forward changed rows")
    };
    assert_eq!(changes.changed().len(), 1);
    assert_eq!(changes.changed()[0].key, 1);
    assert!(changes.deleted().is_empty());
    let identity = next.tables[TableId::Countries.index()].identity();
    drop(first);
    let Current::Frame(sought) = state.frame(1, 100).unwrap() else {
        panic!("seek hour")
    };
    assert!(matches!(
        HourInput::new(&sought)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX),
        TableChanges::Discontinuity(_)
    ));
}
