use super::*;
#[cfg(test)]
use crate::map::test_support::TEST_HANG_GUARD;

#[test]
fn playing_finishes_in_flight_hours_at_or_before_the_playhead() {
    let old = Want {
        branch: 1,
        hour: 10,
        ahead: 0,
        behind: 0,
        playing: true,
        perspective: Default::default(),
        map_mode: Default::default(),
        show_allied_plans: false,
        counters: Default::default(),
        view: test_view(),
        have: vec![],
        received: 0,
    };
    let mut next = Want {
        hour: 100,
        ..old.clone()
    };
    let keep = BTreeSet::from([(1, 100)]);
    assert!(next.keeps_pending((1, 10), &keep));
    assert!(!next.keeps_pending((2, 10), &keep));
    assert!(!next.keeps_pending((1, 101), &keep));
    // Paused, only the wanted hours are finished.
    next.playing = false;
    assert!(!next.keeps_pending((1, 10), &keep));
    assert!(next.keeps_pending((1, 100), &keep));
    next.map_mode = viewer_world::map::map_mode::MapMode::Scripted("held colours".into());
    assert!(
        next.keeps_pending((1, 10), &keep),
        "paused scripted catch-up finishes its colour predecessors"
    );
    assert!(!next.keeps_pending((1, 101), &keep));
    assert!(!next.keeps_pending((2, 10), &keep));
    // Moving the clock alone is not an explicit seek. The host seek counter
    // separately invalidates the epoch, including for a same-hour seek.
    for moved in [
        Want {
            hour: 9,
            ..old.clone()
        },
        Want {
            branch: 2,
            ..old.clone()
        },
    ] {
        assert!(!old.invalidates(&moved));
    }
    next = old.clone();
    next.counters.selected = Arc::new(BTreeSet::from([
        crate::map::unit_selection::Unit::Division(7),
    ]));
    next.counters.hovered = Some(crate::map::unit_selection::Unit::Division(7));
    assert!(!old.invalidates(&next));
    next.counters.settings.nato = !next.counters.settings.nato;
    assert!(old.invalidates(&next));
    next = old.clone();
    next.perspective.fog = !next.perspective.fog;
    assert!(old.invalidates(&next));
    next = old.clone();
    next.show_allied_plans = true;
    assert!(old.invalidates(&next));
}

/// The row size `sized_frame` fills its `Countries` table with.
const ROW: u64 = std::mem::size_of::<replay_data::rows::CountryRow>() as u64;

/// A decoded hour whose `Countries` table holds `rows` rows.
pub(super) fn sized_frame(hour: Hour, rows: usize) -> Current {
    use replay_data::{rows::CountryRow, rows::TableId, rows::TableRows};
    use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed".into(),
    });
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..rows)
            .map(|slot| CountryRow {
                canonical_slot: slot as u32,
                ..Default::default()
            })
            .collect(),
    ));
    Current::Frame(Arc::new(DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }))
}

#[test]
fn the_frame_cache_is_bounded_by_bytes_but_keeps_the_wanted_hours() {
    let mut cache = FrameCache::default();
    // Each hour holds 1,000 rows.
    let each = 1_000 * ROW;
    for hour in 0..10 {
        cache.insert((1, hour), sized_frame(hour, 1_000));
    }
    assert_eq!(cache.bytes, 10 * each);
    // A budget of four hours keeps the newest four.
    cache.evict_except(4 * each, &BTreeSet::new());
    assert_eq!(cache.frames.len(), 4);
    assert!(cache.contains_key(&(1, 9)) && !cache.contains_key(&(1, 5)));
    assert_eq!(cache.bytes, 4 * each);
    // Spared hours stay even over budget; the oldest others go first.
    cache.evict_except(0, &BTreeSet::from([(1, 6), (1, 8)]));
    assert_eq!(cache.frames.len(), 2);
    assert!(cache.contains_key(&(1, 6)) && cache.contains_key(&(1, 8)));
    // A hit makes an hour the most recently used: hour 6 outlives 8.
    assert!(cache.get(&(1, 6)).is_some());
    cache.evict_except(each, &BTreeSet::new());
    assert!(cache.contains_key(&(1, 6)) && !cache.contains_key(&(1, 8)));
    cache.insert((1, 7), sized_frame(7, 1_000));
    cache.insert((1, 8), sized_frame(8, 1_000));
    // Replacing an hour does not count it twice.
    cache.insert((1, 8), sized_frame(8, 500));
    assert_eq!(cache.frames.len(), 3);
    assert_eq!(cache.bytes, 2 * each + 500 * ROW);
    // The count bound still applies to hours that cost nothing.
    let mut gaps = FrameCache::default();
    for hour in 0..(FRAME_CACHE as i32 + 10) {
        gaps.insert(
            (1, hour),
            Current::Gap {
                branch: 1,
                hour,
                reason: String::new(),
            },
        );
    }
    gaps.evict_except(FRAME_CACHE_BYTES, &BTreeSet::new());
    assert_eq!(gaps.frames.len(), FRAME_CACHE);
}

/// Hours share their unchanged tables, and the cache counts each shared
/// table once, releasing it only with the last hour that holds it.
#[test]
fn the_frame_cache_counts_a_shared_table_once() {
    let each = 1_000 * ROW;
    let Current::Frame(first) = sized_frame(0, 1_000) else {
        unreachable!()
    };
    let mut second = (*first).clone();
    second.hour = 1;
    let mut cache = FrameCache::default();
    cache.insert((1, 0), Current::Frame(first));
    cache.insert((1, 1), Current::Frame(Arc::new(second)));
    assert_eq!(cache.bytes, each);
    cache.evict_except(0, &BTreeSet::from([(1, 1)]));
    assert_eq!(cache.frames.len(), 1);
    assert_eq!(cache.bytes, each, "the kept hour still holds the table");
    let mut third = FrameCache::default();
    third.insert((1, 0), sized_frame(0, 1_000));
    third.insert((1, 1), sized_frame(1, 1_000));
    assert_eq!(third.bytes, 2 * each, "equal rows in two allocations");
}

pub(super) fn test_view() -> View {
    View {
        camera: Camera::fit(
            crate::map::camera::MapSize {
                width: 100,
                height: 100,
            },
            crate::map::camera::MapDefines::test(),
            [100, 100],
        ),
        viewport: [100, 100],
        ui_scale: 1.0,
    }
}

pub(super) fn pending(ticket: u64) -> Pending {
    Pending {
        ticket,
        trace: Default::default(),
        current: Current::Gap {
            branch: 1,
            hour: 10,
            reason: String::new(),
        },
        alive: Arc::new(AtomicBool::new(true)),
        started: Instant::now(),
        political: None,
        names: None,
        overlay: None,
        rails: None,
        buildings: None,
        movers: None,
        fog: None,
        snow: None,
        received: 0,
        error: None,
        error_rank: None,
        laps: Vec::new(),
        child_laps: Vec::new(),
        view: test_view(),
    }
}

#[test]
fn concurrent_errors_wait_for_all_parts_and_choose_fixed_priority() {
    let names = [
        "political",
        "names",
        "overlay",
        "rails",
        "buildings",
        "movers",
        "fog",
        "snow",
    ];
    for shift in 0..PARTS {
        let mut pending = pending(7);
        for index in 0..PARTS {
            let name = names[(index + shift) % PARTS];
            let done = pending.take(7, name, 0.0, Vec::new(), Part::failed(name));
            assert_eq!(
                done,
                index + 1 == PARTS,
                "arrival {index}, rotation {shift}"
            );
        }
        assert_eq!(pending.error_rank, Some(0));
        assert!(pending.layers().err().unwrap().starts_with("political:"));
    }
}

#[test]
fn a_cancelled_dispatch_cannot_fill_a_rerequested_hour() {
    let mut replacement = pending(8);
    let child = || {
        vec![BuilderSpan {
            name: "snow.grid",
            ms: 0.0,
            cpu_ms: Some(0.0),
        }]
    };
    assert!(!replacement.take(7, "snow", 0.0, child(), Part::failed("snow")));
    assert_eq!(replacement.received, 0);
    assert!(replacement.error.is_none());
    assert!(replacement.laps.is_empty());
    assert!(replacement.child_laps.is_empty());
    assert!(!replacement.take(8, "snow", 0.0, child(), Part::failed("snow")));
    assert_eq!(replacement.received, 1);
    assert_eq!(replacement.child_laps.len(), 1);
    let child = &replacement.child_laps[0].1[0];
    assert_eq!(child.name, "snow.grid");
    assert_eq!(child.ms, 0.0);
    assert_eq!(child.cpu_ms, Some(0.0));
}

#[test]
fn builder_children_keep_fixed_part_order_and_nested_completion_order() {
    let span = |name| BuilderSpan {
        name,
        ms: 0.0,
        cpu_ms: None,
    };
    let parts = vec![
        (
            "fog",
            vec![span("fog.intel_build"), span("fog.intel_lookup")],
        ),
        ("snow", vec![span("snow.grid")]),
        ("overlay", vec![span("overlay.tables")]),
    ];
    assert_eq!(
        ordered_child_laps(parts)
            .iter()
            .map(|span| span.name)
            .collect::<Vec<_>>(),
        [
            "overlay.tables",
            "fog.intel_build",
            "fog.intel_lookup",
            "snow.grid"
        ]
    );
}

#[test]
fn dropping_requests_stops_a_coordinator_that_keeps_an_internal_sender() {
    let (events, inbox) = mpsc::channel();
    let internal = events.clone();
    drop(Requests(events));
    assert!(matches!(inbox.try_recv(), Ok(Event::Shutdown)));
    drop(internal);
}

#[test]
fn perspective_changes_discard_old_hour_acknowledgements() {
    let old = Want {
        branch: 1,
        hour: 10,
        ahead: 2,
        behind: 2,
        playing: false,
        perspective: Default::default(),
        map_mode: Default::default(),
        show_allied_plans: false,
        counters: Default::default(),
        view: test_view(),
        have: vec![10, 11],
        received: 2,
    };
    let mut next = old.clone();
    next.perspective.fog = !next.perspective.fog;
    assert!(next.discard_stale_have(Some(&old), false));
    assert!(next.have.is_empty());
    let mut next = old.clone();
    next.view.camera.height += 1.0;
    assert!(!next.discard_stale_have(Some(&old), false));
    assert_eq!(next.have, old.have);
}

#[test]
fn a_filled_slot_answers_every_waiter() {
    let slot = Slot::<u32>::new();
    let fill = Fill(Arc::clone(&slot));
    let waiter = {
        let slot = Arc::clone(&slot);
        thread::spawn(move || slot.wait().map(|value| *value))
    };
    fill.set(Some(Arc::new(7)));
    assert_eq!(waiter.join().unwrap(), Some(7));
    assert_eq!(slot.wait().map(|value| *value), Some(7));
}

#[test]
fn a_part_that_never_runs_leaves_its_slot_empty_not_pending() {
    let slot = Slot::<u32>::new();
    drop(Fill(Arc::clone(&slot)));
    assert_eq!(slot.wait(), None);
}

#[test]
fn the_intel_map_follows_the_viewing_countrys_province_bytes() {
    let memos = Memos::new();
    let built = std::cell::Cell::new(0);
    let intel = |fog: Option<BTreeMap<u32, u8>>| {
        memos
            .intel
            .get_or_insert_with(fog.clone(), || {
                built.set(built.get() + 1);
                Ok::<_, ()>(Arc::new(IntelMap::new(
                    fog.map_or_else(Vec::new, |fog| fog.into_values().collect()),
                    true,
                )))
            })
            .unwrap()
    };
    let german = intel(Some(BTreeMap::from([(1, 150), (2, 0)])));
    // Another country, or the same one after its sight changed: rebuilt.
    let soviet = intel(Some(BTreeMap::from([(1, 0), (2, 150)])));
    // Fog off draws no fog: a key of its own.
    let clear = intel(None);
    assert_eq!(built.get(), 3);
    assert!(!Arc::ptr_eq(&german, &soviet) && !Arc::ptr_eq(&german, &clear));
    // Back to the first view: the same grid, nothing uploaded again.
    assert!(Arc::ptr_eq(
        &german,
        &intel(Some(BTreeMap::from([(1, 150), (2, 0)])))
    ));
    assert_eq!(built.get(), 3);
}

pub(super) fn lane_over(replay: &Arc<Replay>) -> DecodeLane {
    DecodeLane::start(Playback::unloaded(Arc::clone(replay)).unwrap().with_reader(
        BlockReader::new(Arc::clone(replay), Arc::new(Dropper::new().unwrap())),
    ))
    .unwrap()
}

fn long_replay(hours: &[Hour]) -> Arc<Replay> {
    Arc::new(
        Replay::from_records(
            b"campaign".to_vec(),
            &[crate::playback::tests::records(1, hours)],
        )
        .unwrap(),
    )
}

/// Wait only in the test driver; production coordinators consume reply events.
pub(super) fn source_frame(client: &Arc<SourceMailbox>, branch: BranchId, hour: Hour) -> Current {
    let (send, result) = mpsc::channel();
    let _handle = client.post(next_id(), (branch, hour), move |_, reply| {
        let _ = send.send(reply);
    });
    match result.recv_timeout(TEST_HANG_GUARD).expect("source replies") {
        source_mailbox::Reply::Ready { value, .. } => value,
        source_mailbox::Reply::Stopped => Current::Gap {
            branch,
            hour,
            reason: LANE_STOPPED.to_owned(),
        },
        source_mailbox::Reply::Cancelled => panic!("ordinary request is not cancelled"),
    }
}

fn frame_within(lane: &DecodeLane, branch: BranchId, hour: Hour) -> Current {
    source_frame(&lane.client(), branch, hour)
}

#[test]
fn the_lane_serves_a_wanted_hour_past_its_window() {
    let hours: Vec<Hour> = (10..=110).collect();
    let replay = long_replay(&hours);
    let lane = Arc::new(lane_over(&replay));
    // A want of 101 hours, as playing at 720 h/s asks for.
    lane.want(hours.iter().map(|hour| (1, *hour)).collect(), 2);
    assert!(matches!(frame_within(&lane, 1, 10), Current::Frame(_)));
    // The window already holds 11..=59 built, so the next hour asked for
    // is 60: wanted, but past the lane's window.
    match frame_within(&lane, 1, 60) {
        Current::Frame(frame) => assert_eq!(frame.hour, 60),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_walk_whose_target_is_no_longer_wanted_stops_after_one_step() {
    let hours: Vec<Hour> = (10..=300).collect();
    let replay = long_replay(&hours);
    let mut playback = Playback::unloaded(Arc::clone(&replay)).unwrap();
    let shared = LaneShared {
        mailbox: OnceLock::new(),
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    };
    // Nothing wants 290: the walk stops after its first step.
    assert!(!DecodeLane::walk(&shared, &mut playback, (1, 290)));
    assert_eq!(playback.walk_start(1, 290), Some(i64::from(9 + DECODE_STEP)));
    // Wanted, the walk goes all the way to the last step before it.
    Mutex::lock(&shared.state).unwrap().urgent = Some((1, 290));
    assert!(DecodeLane::walk(&shared, &mut playback, (1, 290)));
    assert!(
        playback
            .walk_start(1, 290)
            .is_some_and(|at| 290 - at <= i64::from(DECODE_STEP))
    );
}

#[test]
fn a_new_want_mid_walk_is_served() {
    let hours: Vec<Hour> = (10..=300).collect();
    let replay = long_replay(&hours);
    let lane = Arc::new(lane_over(&replay));
    lane.want(vec![(1, 290)], 2);
    // Seek away before or while the lane walks to 290.
    lane.want(vec![(1, 12), (1, 13)], 2);
    match frame_within(&lane, 1, 12) {
        Current::Frame(frame) => assert_eq!(frame.hour, 12),
        other => panic!("{other:?}"),
    }
    match frame_within(&lane, 1, 13) {
        Current::Frame(frame) => assert_eq!(frame.hour, 13),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_dropped_lane_ends_its_thread_and_a_dead_lane_answers_with_a_gap() {
    let replay = long_replay(&[10, 11, 12]);
    let lane = lane_over(&replay);
    let shared = Arc::downgrade(&lane.shared);
    drop(lane);
    let deadline = std::time::Instant::now() + TEST_HANG_GUARD;
    while shared.strong_count() > 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the lane thread ended"
        );
        thread::sleep(std::time::Duration::from_millis(5));
    }
    // A lane whose thread is gone answers at once.
    let mailbox = SourceMailbox::new(|_| false);
    mailbox.stop();
    let dead = DecodeLane {
        shared: Arc::new(LaneShared {
            mailbox: OnceLock::from(mailbox),
            state: Mutex::new(LaneState {
                dead: true,
                ..LaneState::default()
            }),
            changed: Condvar::new(),
        }),
    };
    assert!(matches!(
        frame_within(&dead, 1, 10),
        Current::Gap { reason, .. } if reason == LANE_STOPPED
    ));
}

#[test]
fn the_decode_lane_decodes_wanted_hours_ahead_and_serves_any_other() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let lane = lane_over(&replay);
    lane.want(vec![(1, 100), (1, 101), (1, 105)], 2);
    let decoded = |current: Current| match current {
        Current::Frame(frame) => Some(frame.hour),
        Current::Gap { .. } => None,
    };
    assert_eq!(decoded(frame_within(&lane, 1, 105)), Some(105));
    // Decoded ahead, in want order: already there.
    assert_eq!(decoded(frame_within(&lane, 1, 100)), Some(100));
    // An hour the want does not name is decoded on request; a hole is a gap.
    assert_eq!(decoded(frame_within(&lane, 1, 820)), Some(820));
    assert_eq!(decoded(frame_within(&lane, 1, 103)), None);
    let expected = match replay.seek(1, 101).unwrap() {
        replay::container::SeekResult::Frame(frame) => *frame,
        other => panic!("{other:?}"),
    };
    match frame_within(&lane, 1, 101) {
        Current::Frame(frame) => assert_eq!(*frame, expected),
        other => panic!("{other:?}"),
    }
}

#[test]
fn source_worker_seed_attaches_committed_checkpoints_and_clear_keeps_exact_hour_ownership() {
    use dashboard_base::prepared_map::{Builder, Position, Progress, selected_tables};
    use replay_data::rows::TableId;
    use replay_frame::frame::FrameDecoder;
    let path = std::env::temp_dir().join(format!(
        "native-checkpoint-worker-{}.battleplan",
        uuid::Uuid::new_v4()
    ));
    let original = crate::playback::tests::fixture();
    std::fs::write(&path, original.bytes()).unwrap();
    let replay = Arc::new(Replay::open_path(&path).unwrap());
    let source = Arc::new(
        dashboard::Source::from_replay(
            dashboard::SourceKey {
                source_id: "synthetic-host-source".into(),
                revision: dashboard::protocol::replay_revision(&path).unwrap(),
            },
            path.clone(),
            Arc::clone(&replay),
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
    let (store, key) = dashboard::prepared::files(&source, None).unwrap();
    let mut writer = store.replace(key).unwrap();
    let mut builder = Builder::new(
        std::num::NonZeroU32::new(2).unwrap(),
        selected_tables(),
        Progress::default(),
    )
    .unwrap();
    builder.set_ranges(&replay);
    let mut decoder = FrameDecoder::new();
    let mut last = None;
    for (block, index) in replay.index.iter().enumerate() {
        for (offset, frame) in replay.read_block(index).unwrap().iter().enumerate() {
            let decoded = decoder.apply(frame).unwrap();
            let position = Position {
                block,
                next_frame: offset + 1,
            };
            builder
                .accept(&mut writer, &frame, &decoded, position)
                .unwrap();
            last = Some((decoded, position));
        }
    }
    let (last, position) = last.unwrap();
    builder
        .save_resume(&mut writer, &last, position, &TableId::ALL)
        .unwrap();
    builder.save_gaps(&mut writer, &replay.gaps).unwrap();
    dashboard::prepared::pipeline::write_map_progress(&mut writer, builder.progress()).unwrap();
    writer.commit().unwrap();
    let resources = Resources::new().unwrap();
    let worker = resources.decoder(Arc::clone(&replay)).unwrap();
    let context = crate::map_checkpoints::CheckpointSource::new(source, None);
    worker
        .seed(
            Playback::unloaded(Arc::clone(&replay))
                .unwrap()
                .with_checkpoint_source(context),
        )
        .unwrap();
    let client = worker.client();
    let current = source_frame(&client, 1, 105);
    assert!(matches!(current, Current::Frame(ref frame) if frame.hour == 105 && frame.branch == 1));
    let expected = replay
        .seek_selected(1, 105, Some(selected_tables()))
        .unwrap();
    // The map's nuclear impacts survive checkpoint restoration.
    let strikes = |current: &Current| match current {
        Current::Frame(frame) => frame
            .complete::<replay_data::rows::tables::NuclearStrikes>()
            .map(|rows| rows.iter().map(|row| row.id).collect::<Vec<_>>())
            .map_err(|error| error.to_string()),
        Current::Gap { reason, .. } => Err(reason.clone()),
    };
    assert_eq!(strikes(&current), Ok(vec![7]));
    // The resources map mode's and the trade routes' tables, including exchange ordinals, survive checkpoint restoration
    // as ordinary playback decodes them.
    let replay::container::SeekResult::Frame(ordinary) = replay.seek(1, 105).unwrap() else {
        panic!("hour 105 is recorded");
    };
    let Current::Frame(restored) = &current else {
        panic!("hour 105 is recorded");
    };
    for rows in crate::playback::tests::resource_map_rows(105) {
        let table = rows.table();
        assert_eq!(
            *ordinary.tables[table.index()],
            replay_frame::frame::TableView::Complete(rows),
            "{table:?}"
        );
        assert_eq!(
            *restored.tables[table.index()],
            *ordinary.tables[table.index()],
            "{table:?}"
        );
    }
    assert!(
        matches!((current, expected), (Current::Frame(actual), replay::container::SeekResult::Frame(expected)) if actual.as_ref() == expected.as_ref())
    );
    // Stepping on from the restored hour replaces the hourly impacts.
    assert_eq!(strikes(&source_frame(&client, 1, 820)), Ok(Vec::new()));
    let metrics = worker
        .state
        .lock()
        .unwrap()
        .playback
        .checkpoint_metrics()
        .unwrap();
    assert!(metrics.0 < 2);
    assert!(
        metrics.1 > 0,
        "the actual shared worker used an indexed checkpoint"
    );
    store.clear().unwrap();
    assert!(
        matches!(source_frame(&client, 1, 101), Current::Frame(ref frame) if frame.hour == 101)
    );
    assert!(
        matches!(source_frame(&client, 1, 102), Current::Gap { hour: 102, branch: 1, ref reason } if reason == "flush failed")
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "native playback does not resurrect cleared preparation"
    );
    drop(worker);
    drop(resources);
    drop(replay);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn the_lane_window_runs_from_the_last_hour_served_and_is_never_evicted() {
    let mut state = LaneState {
        wanted: (0..100).map(|hour| (1, hour)).collect(),
        ..LaneState::default()
    };
    assert_eq!(state.window().len(), DECODE_AHEAD);
    state.served = Some((1, 10));
    assert_eq!(state.window().last(), Some(&(1, 10 + DECODE_AHEAD as i32)));
    state.served = Some((1, 99));
    assert_eq!(state.window().len(), 100);
    // A full cache frees hours outside the window first.
    state.served = Some((1, 0));
    for hour in 0..(FRAME_CACHE as i32 + 20) {
        state.frames.insert((1, hour), sized_frame(hour, 1));
    }
    state.evict();
    assert_eq!(state.frames.frames.len(), FRAME_CACHE);
    for key in state.window().to_vec() {
        assert!(state.frames.contains_key(&key), "{key:?}");
    }
}

#[test]
fn an_equal_value_keeps_the_previous_id() {
    let first = Shared::share(Vec::new(), vec![1, 2]);
    let other = Shared::share(Vec::new(), vec![5]);
    let same = Shared::share(vec![Arc::clone(&other), Arc::clone(&first)], vec![1, 2]);
    let changed = Shared::share(vec![Arc::clone(&first)], vec![1, 3]);
    assert_eq!(first.id, same.id);
    assert!(Arc::ptr_eq(&first, &same));
    assert_ne!(first.id, changed.id);
}

#[test]
fn a_failed_part_of_every_name_is_an_error_not_a_value() {
    for name in [
        "political",
        "names",
        "overlay",
        "rails",
        "buildings",
        "movers",
        "fog",
        "snow",
    ] {
        let failed = match Part::failed(name) {
            Part::Political(result) => result.is_err(),
            Part::Names(result) => result.is_err(),
            Part::Overlay(result) => result.is_err(),
            Part::Rails(result) => result.is_err(),
            Part::Buildings(result) => result.is_err(),
            Part::Movers(result) => result.is_err(),
            Part::Fog(result) => result.is_err(),
            Part::Snow(result) => result.is_err(),
        };
        assert!(failed, "{name} must fail its hour");
    }
}

#[test]
fn playing_targets_consecutive_hours_from_the_playhead_and_stops_at_the_branch_end() {
    let want = Want {
        branch: 1,
        hour: 100,
        ahead: playback_ahead(8),
        behind: 0,
        playing: true,
        perspective: Default::default(),
        map_mode: Default::default(),
        show_allied_plans: false,
        counters: Default::default(),
        view: test_view(),
        have: vec![],
        received: 0,
    };
    assert_eq!(want.hours((0, 1000)), (100..=107).collect::<Vec<_>>());
    assert_eq!(want.hours((0, 105)), (100..=105).collect::<Vec<_>>());
    let moved = Want {
        hour: 200,
        ..want.clone()
    };
    assert_eq!(moved.hours((0, 1000)), (200..=207).collect::<Vec<_>>());
    let paused = Want {
        playing: false,
        ahead: 2,
        behind: 2,
        ..want.clone()
    };
    assert_eq!(paused.hours((0, 1000)), [100, 101, 102, 99, 98]);
    let keep = want
        .hours((0, 1000))
        .into_iter()
        .map(|hour| (1, hour))
        .collect();
    assert!(want.keeps_pending((1, 107), &keep));
    assert!(want.keeps_pending((1, 99), &keep));
    assert!(!want.keeps_pending((1, 108), &keep));
    assert!(!want.keeps_pending((2, 100), &keep));
    let end = Want {
        hour: i32::MAX,
        ..want
    };
    assert_eq!(end.hours((i32::MAX - 5, i32::MAX)), [i32::MAX]);
}

#[test]
fn lookahead_requests_one_live_pool_including_the_playhead_without_a_chosen_hour_cap() {
    for parallel in [1, 8, 32, 128, 5_000] {
        assert_eq!(playback_ahead(parallel) as usize + 1, parallel);
    }
    assert_eq!(playback_ahead(usize::MAX), i32::MAX);
}

#[test]
fn sharing_uses_nearest_built_hours_across_sparse_samples_and_keeps_branch_identity() {
    let previous = Shared::share(Vec::new(), vec![1, 2]);
    let later = Shared::share(Vec::new(), vec![3]);
    let slots = BTreeMap::from([
        ((1, 100), Arc::clone(&previous)),
        ((1, 112), Arc::clone(&later)),
        ((2, 105), Shared::share(Vec::new(), vec![1, 2])),
    ]);
    let neighbours = neighbour_slots(&slots, 1, 106);
    assert_eq!(
        neighbours.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        [(1, 100), (1, 112)]
    );
    let same = Shared::share(
        neighbours
            .into_iter()
            .map(|(_, slot)| Arc::clone(slot))
            .collect(),
        vec![1, 2],
    );
    assert!(Arc::ptr_eq(&same, &previous));
    let neighbours = || {
        neighbour_slots(&slots, 1, 106)
            .into_iter()
            .map(|(_, slot)| Arc::clone(slot))
            .collect()
    };
    let same_later = Shared::share(neighbours(), vec![3]);
    assert!(Arc::ptr_eq(&same_later, &later));
    let changed = Shared::share(neighbours(), vec![4]);
    assert_ne!(changed.id, previous.id);
    assert_ne!(changed.id, later.id);
    assert!(neighbour_slots(&slots, 3, 106).is_empty());
    assert_eq!(
        neighbour_slots(&slots, 1, 120)
            .iter()
            .map(|(key, _)| *key)
            .collect::<Vec<_>>(),
        [(1, 112)]
    );
}

#[test]
fn sparse_playback_decode_is_identical_to_an_independent_paused_seek_at_every_sample() {
    let hours: Vec<_> = (100..=300).collect();
    let replay = long_replay(&hours);
    let lane = lane_over(&replay);
    // Sparse upcoming frames and a final off-grid branch end decode exactly,
    // regardless of the order an asynchronous request reaches them.
    let samples: Vec<_> = (160..=298)
        .step_by(6)
        .chain((100..160).step_by(6))
        .chain([300])
        .collect();
    lane.want(samples.iter().map(|hour| (1, *hour)).collect(), 2);
    for hour in samples {
        let Current::Frame(actual) = frame_within(&lane, 1, hour) else {
            panic!("sample {hour} is not a frame")
        };
        let replay::container::SeekResult::Frame(expected) = replay.seek(1, hour).unwrap() else {
            panic!("paused {hour} is not a frame")
        };
        assert_eq!(actual.as_ref(), expected.as_ref(), "hour {hour}");
    }
}

#[test]
fn same_hour_seeks_discard_ready_acknowledgements_just_like_other_history_resets() {
    let old = Want {
        branch: 1,
        hour: 10,
        ahead: 2,
        behind: 2,
        playing: false,
        perspective: Default::default(),
        map_mode: Default::default(),
        show_allied_plans: false,
        counters: Default::default(),
        view: test_view(),
        have: vec![10, 11],
        received: 2,
    };
    // A pause or camera update keeps the epoch; a seek to precisely the same
    // position must force it to change and prevent reusing the ready result.
    let mut next = old.clone();
    assert!(!next.discard_stale_have(Some(&old), false));
    assert_eq!(next.have, [10, 11]);
    assert!(next.discard_stale_have(Some(&old), true));
    assert!(next.have.is_empty());
    assert_eq!((next.branch, next.hour), (old.branch, old.hour));
}

#[test]
fn skipped_colour_refreshes_carry_in_recorded_order_and_gaps_start_cold() {
    use viewer_world::map::map_mode::ColourState;
    let colour = |name: &str| {
        Arc::new(ColourState {
            mode: name.into(),
            layers: Default::default(),
        })
    };
    let old = colour("before midnight");
    let daily = colour("midnight refresh");
    let forced = colour("effect refresh");
    let cold = colour("after gap");
    // Model the colour evaluator's first/daily/event refresh decisions; this
    // tests the builder's traversal, independently of the evaluator's own tests.
    let mut visited = vec![];
    let mut evaluate = |frame: &replay_frame::frame::DecodedFrame,
                        previous: Option<&ColourState>| {
        visited.push((frame.hour, previous.map(|state| state.mode.clone())));
        Ok(Some(match previous {
            None => Arc::clone(&cold),
            Some(_) if frame.hour == 24 => Arc::clone(&daily),
            Some(_) if frame.hour == 26 => Arc::clone(&forced),
            Some(previous) => colour(&previous.mode),
        }))
    };
    let carried = carry_colour(
        Some(old),
        (24..=27).map(|hour| sized_frame(hour, 0)),
        &mut evaluate,
    )
    .unwrap()
    .unwrap();
    assert_eq!(carried.mode, forced.mode);
    assert_eq!(
        visited,
        [
            (24, Some("before midnight".into())),
            (25, Some("midnight refresh".into())),
            (26, Some("midnight refresh".into())),
            (27, Some("effect refresh".into())),
        ]
    );
    let frames = vec![
        Current::Gap {
            branch: 1,
            hour: 28,
            reason: "not recorded".into(),
        },
        sized_frame(29, 0),
    ];
    let after_gap = carry_colour(Some(carried), frames, |_, previous| {
        assert!(previous.is_none());
        Ok(Some(Arc::clone(&cold)))
    })
    .unwrap()
    .unwrap();
    assert!(Arc::ptr_eq(&after_gap, &cold));
    // Evaluation failure is not hidden by the sampling layer.
    assert!(
        carry_colour(None, vec![sized_frame(30, 0)], |_, _| Err(
            io::Error::other("refused colour")
        ))
        .is_err()
    );
}

#[test]
fn forward_playback_retains_only_targets_not_cancellation_endpoints_or_behind_hours() {
    let replay = long_replay(&(10..=310).collect::<Vec<_>>());
    let lane = Arc::new(lane_over(&replay));
    lane.want(vec![(1, 290)], 0);
    let Current::Frame(actual) = frame_within(&lane, 1, 290) else {
        panic!("target frame")
    };
    let replay::container::SeekResult::Frame(expected) = replay.seek(1, 290).unwrap() else {
        panic!("expected frame")
    };
    assert_eq!(actual.as_ref(), expected.as_ref());
    let state = DecodeLane::lock(&lane.shared);
    assert_eq!(
        state.frames.frames.keys().copied().collect::<Vec<_>>(),
        [(1, 290)]
    );
}

#[test]
fn target_trace_decode_metadata_follows_frame_cache_eviction() {
    let mut state = LaneState::default();
    assert!(state.decode_profile.is_none());
    let sample = trace::DecodeClock::start(true).finish(1, 100).unwrap();
    for hour in 0..100 {
        state.frames.insert(
            (1, hour),
            Current::Gap { branch: 1, hour, reason: "recorded gap".into() },
        );
        state.decode_profile
            .get_or_insert_with(BTreeMap::new)
            .insert((1, hour), sample);
    }
    state.evict();
    let metadata = state.decode_profile.as_ref().unwrap();
    assert_eq!(metadata.len(), FRAME_CACHE);
    assert_eq!(
        metadata.keys().copied().collect::<Vec<_>>(),
        state.frames.frames.keys().copied().collect::<Vec<_>>(),
    );
}

#[test]
fn railway_neighbour_wait_precedes_single_flight_ownership_after_reversed_worker_entry() {
    let memo = Arc::new(viewer_foundation::map::memo::Recent::<u8, u32>::new(2));
    let predecessor = Slot::new();
    let (entered, waiting) = mpsc::channel();
    let (done, result) = mpsc::channel();
    let later_memo = memo.clone();
    let near = predecessor.clone();
    let later = thread::spawn(move || {
        let value = memo_after_neighbours(
            &later_memo,
            1,
            || {
                entered.send(()).unwrap();
                wait_all(vec![near])
            },
            |_| -> io::Result<_> { Ok(Arc::new(999)) },
        ).unwrap();
        done.send(value).unwrap();
    });
    waiting.recv_timeout(TEST_HANG_GUARD).unwrap();
    // B reached the dependency first. A has not entered the memo until now.
    // If B claimed it before waiting, A and B would form the reported cycle.
    let (built, first) = mpsc::channel();
    let earlier = thread::spawn(move || {
        let value = memo_after_neighbours(
            &memo,
            1,
            Vec::<Arc<u32>>::new,
            |_| -> io::Result<_> { Ok(Arc::new(42)) },
        ).unwrap();
        Fill(predecessor).set(Some(value.clone()));
        built.send(value).unwrap();
    });
    let first = first.recv_timeout(TEST_HANG_GUARD).unwrap();
    let second = result.recv_timeout(TEST_HANG_GUARD).unwrap();
    assert_eq!(*second, 42);
    assert!(Arc::ptr_eq(&first, &second));
    earlier.join().unwrap();
    later.join().unwrap();
}

#[test]
fn shared_rail_keys_follow_expected_networks_hour_by_hour() {
    use replay_data::rows::{NavalBaseRow, ProvinceSupplyRow, RailEdgeRow, TableId, TableRows};
    use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
    let edge = |from, to, level| RailEdgeRow { from, to, level };
    let frame = |hour, edges: Vec<RailEdgeRow>, complete: bool| {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        });
        tables[TableId::RailEdges.index()] = TableView::Complete(TableRows::RailEdges(edges));
        tables[TableId::ProvinceSupply.index()] =
            TableView::Complete(TableRows::ProvinceSupply(vec![ProvinceSupplyRow {
                province: 2,
                supply_node_level: 1,
                ..Default::default()
            }]));
        let bases = TableRows::NavalBases(vec![NavalBaseRow {
            province: 3,
            ..Default::default()
        }]);
        tables[TableId::NavalBases.index()] = if complete {
            TableView::Complete(bases)
        } else {
            TableView::Truncated {
                rows: bases,
                missing: 1,
            }
        };
        DecodedFrame {
            branch: 1,
            hour,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    };
    let line = vec![edge(1, 2, 3), edge(2, 1, 3), edge(2, 3, 2)];
    let hours = [
        frame(1, line.clone(), true),
        frame(2, line.clone(), true),
        frame(3, vec![edge(1, 2, 3), edge(2, 1, 3), edge(2, 3, 1)], true),
        frame(
            4,
            vec![edge(1, 2, 3), edge(2, 1, 3), edge(2, 3, 5), edge(2, 3, 1)],
            true,
        ),
        frame(
            5,
            [line.clone(), vec![edge(3, 4, 0), edge(4, 4, 2)]].concat(),
            true,
        ),
        frame(6, line.clone(), false),
        frame(7, vec![edge(5, 6, 1)], false),
        frame(8, line.clone(), true),
    ];
    let run = || {
        let memo = Recent::<Arc<RailNetwork>, usize>::new(MEMO_ENTRIES);
        let mut built = Vec::new();
        let answers: Vec<usize> = hours
            .iter()
            .map(|frame| {
                let network = Arc::new(RailNetwork::read(frame).unwrap());
                *memo_after_neighbours(
                    &memo,
                    Arc::clone(&network),
                    Vec::<Arc<()>>::new,
                    |_| -> io::Result<_> {
                        built.push(RailNetwork::clone(&network));
                        Ok(Arc::new(built.len()))
                    },
                )
                .unwrap()
            })
            .collect();
        (answers, built)
    };
    let new = run();
    assert_eq!(new.0, [1, 1, 2, 2, 1, 3, 3, 1]);
    assert_eq!(new.1[0], RailNetwork::read(&hours[0]).unwrap());
    assert_eq!(new.1[2], RailNetwork::new(&[], &[], &[]));
}

#[test]
fn a_reused_hour_leaves_its_successor_no_buildings_state_to_wait_for() {
    let slots = Slots::new();
    let (done, waited) = mpsc::channel();
    let successor = {
        let state = Arc::clone(&slots.buildings_state);
        thread::spawn(move || done.send(state.wait().is_none()).unwrap())
    };
    slots.without_buildings_state();
    assert!(waited.recv_timeout(TEST_HANG_GUARD).unwrap());
    successor.join().unwrap();
}

fn paged_map(
    replay: &Arc<Replay>,
) -> (
    crate::map_checkpoints::PreparedMap,
    std::path::PathBuf,
    uuid::Uuid,
) {
    use dashboard_base::prepared_map::{
        Builder, Position, Progress,
        pages::{self, BlockPages, PageFold},
        selected_tables,
    };
    use replay_frame::frame::FrameDecoder;
    let id = uuid::Uuid::new_v4();
    let path = std::env::temp_dir().join(format!("paged-lane-{id}.battleplan"));
    std::fs::write(&path, replay.bytes()).unwrap();
    let key = store::SourceKey::capture(&path, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let selected = selected_tables();
    let mut builder = Builder::new(
        std::num::NonZeroU32::new(720).unwrap(),
        selected,
        Progress::default(),
    )
    .unwrap();
    builder.set_ranges(replay);
    let mut writer = store.replace(key.clone()).unwrap();
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut folds: BTreeMap<BranchId, PageFold> = BTreeMap::new();
    for (block, index) in replay.index.iter().enumerate() {
        for (offset, frame) in replay
            .read_block_selected(index, selected)
            .unwrap()
            .iter()
            .enumerate()
        {
            let decoded = decoder.advance(frame).unwrap();
            let position = Position {
                block,
                next_frame: offset + 1,
            };
            builder
                .accept(&mut writer, frame, decoded, position)
                .unwrap();
        }
        let fold = folds
            .entry(index.branch)
            .or_insert_with(|| PageFold::new(index.branch, selected, &[20, 40], None).unwrap());
        let produced = BlockPages::produce(replay, block, selected, &[20, 40], || Ok(())).unwrap();
        assert_eq!(fold.fold(block, produced).unwrap(), None);
    }
    for (&branch, fold) in &mut folds {
        fold.publish(&mut writer).unwrap();
        let chains = serde_json::to_vec(fold.pages()).unwrap();
        writer
            .write_at(&pages::progress_section(branch), 0, chains.as_slice())
            .unwrap();
        fold.save_states(&mut writer).unwrap();
    }
    builder.save_gaps(&mut writer, &replay.gaps).unwrap();
    let progress = builder.progress().clone();
    writer.commit().unwrap();
    let map = crate::map_checkpoints::PreparedMap::new(
        store::Store::local(id).unwrap(),
        key,
        progress,
        Arc::clone(replay),
        selected,
    )
    .unwrap()
    .with_committed_pages();
    (map, path, id)
}

/// The native route's answer at `hour`, as the selected replay reader gives it.
fn assert_native(actual: &Current, replay: &Replay, hour: Hour) {
    use replay::container::SeekResult;
    let expected = replay
        .seek_selected(
            1,
            hour,
            Some(dashboard_base::prepared_map::selected_tables()),
        )
        .unwrap();
    match (actual, expected) {
        (Current::Frame(actual), SeekResult::Frame(expected)) => {
            assert_eq!(actual.as_ref(), expected.as_ref(), "hour {hour}");
            for (actual, expected) in actual.tables.iter().zip(&expected.tables) {
                assert_eq!(actual.sampled_at(), expected.sampled_at(), "hour {hour}");
            }
        }
        (
            Current::Gap { reason: actual, .. },
            SeekResult::Gap {
                reason: expected, ..
            },
        ) => {
            assert_eq!(actual, &expected, "hour {hour}")
        }
        pair => panic!("hour {hour}: {pair:?}"),
    }
}

fn walk_and_export(
    shared: &LaneShared,
    playback: &mut Playback,
    replay: &Replay,
    hour: Hour,
    behind: i32,
) {
    {
        let mut state = Mutex::lock(&shared.state).unwrap();
        state.behind = behind;
        state.urgent = Some((1, hour));
    }
    assert!(DecodeLane::walk(shared, playback, (1, hour)));
    let mut passed = Vec::new();
    playback
        .seek_with(1, hour, |decoded| {
            if hour - decoded.hour <= behind {
                passed.push(Current::Frame(Arc::new(decoded.clone())));
            }
        })
        .unwrap();
    assert_native(playback.current(), replay, hour);
    assert_eq!(
        passed
            .iter()
            .map(|frame| match frame {
                Current::Frame(frame) => frame.hour,
                Current::Gap { .. } => panic!("only recorded visitors"),
            })
            .collect::<Vec<_>>(),
        (hour - behind..hour).collect::<Vec<_>>(),
        "visitor hours at {hour}",
    );
    for frame in &passed {
        if let Current::Frame(decoded) = frame {
            assert_native(frame, replay, decoded.hour);
        }
    }
}

#[test]
fn a_lane_walk_reaches_grid_targets_by_prepared_pages_and_exports_the_native_frames() {
    // Three blocks of recorded hours, with a hole holding grid hour 760.
    let hours: Vec<Hour> = (10..=1500).filter(|hour| *hour != 760).collect();
    let replay = long_replay(&hours);
    assert!(replay.index.len() > 2, "the targets cross blocks");
    let (map, path, id) = paged_map(&replay);
    let reference = Replay::open(replay.bytes().to_vec()).unwrap();
    reference.cache_decoded_sections(false);
    let mut playback = Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map)
        .with_page_workers(4);
    let shared = LaneShared {
        mailbox: OnceLock::new(),
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    };
    let mut paged = Vec::new();
    for hour in (20..=1500).step_by(20) {
        walk_and_export(&shared, &mut playback, &reference, hour, 0);
        if playback.checkpoint_metrics() == Some((0, 0, 0)) {
            paged.push(hour);
        }
    }
    let expected: Vec<Hour> = (40..=1500)
        .step_by(20)
        .filter(|hour| ![760, 780].contains(hour))
        .collect();
    assert_eq!(paged, expected);
    playback.seek(1, 10).unwrap();
    for hour in (40..=400).step_by(40) {
        walk_and_export(&shared, &mut playback, &reference, hour, 2);
        if hour > 40 {
            assert_eq!(
                playback.checkpoint_metrics().map(|metrics| metrics.0),
                Some(20),
                "hour {hour}"
            );
        }
    }
    let _ = store::Store::local(id).unwrap().clear();
    let _ = std::fs::remove_file(path);
}

thread_local! {
    static WALK_WANTED: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn before_walk_wanted() {
    WALK_WANTED.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook();
        }
    });
}

#[test]
fn a_page_walk_cancellation_does_not_enter_native_fallback_or_publish_the_endpoint() {
    let replay = long_replay(&(10..=400).collect::<Vec<_>>());
    let (map, path, id) = paged_map(&replay);
    let reference = Replay::open(replay.bytes().to_vec()).unwrap();
    reference.cache_decoded_sections(false);
    let mut playback = Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    playback.seek(1, 120).unwrap();
    let before = playback.current().clone();
    let shared = Arc::new(LaneShared {
        mailbox: OnceLock::new(),
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    });
    Mutex::lock(&shared.state).unwrap().urgent = Some((1, 360));
    let checks = std::rc::Rc::new(std::cell::Cell::new(0));
    WALK_WANTED.with(|hook| {
        *hook.borrow_mut() = Some(Box::new({
            let checks = std::rc::Rc::clone(&checks);
            let shared = Arc::clone(&shared);
            move || {
                checks.set(checks.get() + 1);
                // Three search-header checks precede the post-first-payload cancellation.
                if checks.get() == 9 {
                    Mutex::lock(&shared.state).unwrap().stop = true;
                }
            }
        }))
    });
    let walked = DecodeLane::walk(&shared, &mut playback, (1, 360));
    WALK_WANTED.with(|hook| *hook.borrow_mut() = None);
    assert!(!walked);
    assert_eq!(
        checks.get(),
        10,
        "the page cancellation returns before any native-walk check"
    );
    assert_eq!(playback.hour(), 120);
    assert_eq!(playback.current(), &before);
    assert_eq!(
        playback.checkpoint_metrics(),
        Some((0, 0, 0)),
        "no native fallback was loaded or applied"
    );
    assert_eq!(playback.page_fallbacks(), 0);
    playback
        .seek_with(1, 129, |frame| {
            assert_native(
                &Current::Frame(Arc::new(frame.clone())),
                &reference,
                frame.hour,
            );
        })
        .unwrap();
    assert_native(playback.current(), &reference, 129);
    let _ = store::Store::local(id).unwrap().clear();
    let _ = std::fs::remove_file(path);
}
