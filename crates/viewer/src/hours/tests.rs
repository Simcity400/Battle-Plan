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
    // Seeks and branch changes keep what was built for a step back; only
    // a perspective change starts a new epoch.
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
const ROW: u64 = std::mem::size_of::<replay::rows::CountryRow>() as u64;

/// A decoded hour whose `Countries` table holds `rows` rows.
fn sized_frame(hour: Hour, rows: usize) -> Current {
    use replay::{
        frame::{DecodedFrame, Metrics, TableView},
        rows::{CountryRow, TableId, TableRows},
    };
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

fn test_view() -> View {
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

fn pending(ticket: u64) -> Pending {
    Pending {
        ticket,
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
            let done = pending.take(7, name, 0.0, Part::failed(name));
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
    assert!(!replacement.take(7, "snow", 0.0, Part::failed("snow")));
    assert_eq!(replacement.received, 0);
    assert!(replacement.error.is_none());
    assert!(replacement.laps.is_empty());
    assert!(!replacement.take(8, "snow", 0.0, Part::failed("snow")));
    assert_eq!(replacement.received, 1);
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
        show_allied_plans: false,
        counters: Default::default(),
        view: test_view(),
        have: vec![10, 11],
        received: 2,
    };
    let mut next = old.clone();
    next.perspective.fog = !next.perspective.fog;
    assert!(next.discard_stale_have(Some(&old)));
    assert!(next.have.is_empty());
    let mut next = old.clone();
    next.view.camera.height += 1.0;
    assert!(!next.discard_stale_have(Some(&old)));
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
                Ok::<_, ()>(Arc::new(IntelMap {
                    texels: fog.map_or_else(Vec::new, |fog| fog.into_values().collect()),
                    fog: true,
                }))
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

fn lane_over(replay: &Arc<Replay>) -> DecodeLane {
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

/// `lane.frame` on another thread, failing the test rather than hanging it.
fn frame_within(lane: &Arc<DecodeLane>, branch: BranchId, hour: Hour) -> Current {
    let (done, result) = mpsc::channel();
    let asker = Arc::clone(lane);
    thread::spawn(move || {
        let _ = done.send(asker.frame(branch, hour));
    });
    result
        .recv_timeout(TEST_HANG_GUARD)
        .expect("the lane answers")
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
    let hours: Vec<Hour> = (10..=700).collect();
    let replay = long_replay(&hours);
    let mut playback = Playback::unloaded(Arc::clone(&replay)).unwrap();
    let shared = LaneShared {
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    };
    // Nothing wants 690: the walk stops after its first step.
    assert!(!DecodeLane::walk(&shared, &mut playback, (1, 690)));
    assert_eq!(playback.walk_start(1, 690), Some(9 + DECODE_STEP));
    // Wanted, the walk goes all the way to the last step before it.
    Mutex::lock(&shared.state).unwrap().urgent = Some((1, 690));
    assert!(DecodeLane::walk(&shared, &mut playback, (1, 690)));
    assert!(
        playback
            .walk_start(1, 690)
            .is_some_and(|at| 690 - at <= DECODE_STEP)
    );
}

#[test]
fn a_new_want_mid_walk_is_served() {
    let hours: Vec<Hour> = (10..=700).collect();
    let replay = long_replay(&hours);
    let lane = Arc::new(lane_over(&replay));
    lane.want(vec![(1, 690)], 2);
    // Seek away before or while the lane walks to 690.
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
    let dead = DecodeLane {
        shared: Arc::new(LaneShared {
            state: Mutex::new(LaneState {
                dead: true,
                ..LaneState::default()
            }),
            changed: Condvar::new(),
        }),
    };
    assert!(matches!(
        dead.frame(1, 10),
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
    assert_eq!(decoded(lane.frame(1, 105)), Some(105));
    // Decoded ahead, in want order: already there.
    assert_eq!(decoded(lane.frame(1, 100)), Some(100));
    // An hour the want does not name is decoded on request; a hole is a gap.
    assert_eq!(decoded(lane.frame(1, 820)), Some(820));
    assert_eq!(decoded(lane.frame(1, 103)), None);
    let expected = match replay.seek(1, 101).unwrap() {
        replay::container::SeekResult::Frame(frame) => *frame,
        other => panic!("{other:?}"),
    };
    match lane.frame(1, 101) {
        Current::Frame(frame) => assert_eq!(*frame, expected),
        other => panic!("{other:?}"),
    }
}

#[test]
fn source_worker_seed_attaches_committed_checkpoints_and_clear_keeps_exact_hour_ownership() {
    use dashboard_base::prepared_map::{Builder, Position, Progress, selected_tables};
    use replay::{frame::FrameDecoder, rows::TableId};
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
    let current = worker.frame(1, 105).unwrap();
    assert!(matches!(current, Current::Frame(ref frame) if frame.hour == 105 && frame.branch == 1));
    let expected = replay
        .seek_selected(1, 105, Some(selected_tables()))
        .unwrap();
    // The map's nuclear impacts survive checkpoint restoration.
    let strikes = |current: &Current| match current {
        Current::Frame(frame) => frame
            .complete::<replay::rows::tables::NuclearStrikes>()
            .map(|rows| rows.iter().map(|row| row.id).collect::<Vec<_>>())
            .map_err(|error| error.to_string()),
        Current::Gap { reason, .. } => Err(reason.clone()),
    };
    assert_eq!(strikes(&current), Ok(vec![7]));
    assert!(
        matches!((current, expected), (Current::Frame(actual), replay::container::SeekResult::Frame(expected)) if actual.as_ref() == expected.as_ref())
    );
    // Stepping on from the restored hour replaces the hourly impacts.
    assert_eq!(strikes(&worker.frame(1, 820).unwrap()), Ok(Vec::new()));
    let metrics = worker
        .0
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
        matches!(worker.frame(1, 101).unwrap(), Current::Frame(ref frame) if frame.hour == 101)
    );
    assert!(
        matches!(worker.frame(1, 102).unwrap(), Current::Gap { hour: 102, branch: 1, ref reason } if reason == "flush failed")
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
