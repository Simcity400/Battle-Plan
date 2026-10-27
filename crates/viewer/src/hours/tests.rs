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
        grid: None,
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

fn tagged_frame(hour: Hour, rows: usize) -> Current {
    use replay_data::{rows::CountryRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let Current::Frame(frame) = sized_frame(hour, rows) else {
        unreachable!()
    };
    let mut frame = (*frame).clone();
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..rows)
            .map(|slot| CountryRow {
                canonical_slot: slot as u32,
                tag: vec![b'A'; 16],
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    Current::Frame(Arc::new(frame))
}

fn reference_cache_bytes(frames: &[&Current]) -> u64 {
    use replay_data::rows::{CountryRow, TableRows};
    use replay_frame::frame::TableView;
    let mut seen = std::collections::HashSet::new();
    let mut total = 0usize;
    for current in frames {
        if let Current::Frame(frame) = current {
            for table in &frame.tables {
                if !seen.insert(table.as_ptr() as usize) {
                    continue;
                }
                total += std::mem::size_of::<TableView>() + 2 * std::mem::size_of::<usize>();
                total += match &**table {
                    TableView::Complete(TableRows::Countries(rows)) => {
                        rows.capacity() * std::mem::size_of::<CountryRow>()
                            + rows.iter().map(|row| row.tag.capacity()).sum::<usize>()
                    }
                    TableView::Unavailable { reason } => reason.capacity(),
                    _ => panic!("unexpected synthetic table"),
                };
            }
        }
    }
    total as u64
}

#[test]
fn the_frame_cache_counts_true_bytes_once_and_releases_only_the_last_holder() {
    let first = tagged_frame(0, 1_500);
    let Current::Frame(frame) = &first else {
        unreachable!()
    };
    let mut next = (**frame).clone();
    next.hour = 1;
    let second = Current::Frame(Arc::new(next));
    let third = tagged_frame(2, 500);
    let replacement = tagged_frame(2, 250);
    let mut cache = FrameCache::default();
    cache.insert((1, 0), first.clone());
    assert_eq!(cache.added_bytes, reference_cache_bytes(&[&first]));
    cache.insert((1, 1), second.clone());
    assert_eq!(cache.added_bytes, 0, "every allocation was already held");
    cache.insert((1, 2), third.clone());
    assert_eq!(
        cache.bytes,
        reference_cache_bytes(&[&first, &second, &third])
    );
    cache.insert((1, 2), replacement.clone());
    assert_eq!(cache.frames.len(), 3);
    assert_eq!(
        cache.bytes,
        reference_cache_bytes(&[&first, &second, &replacement])
    );
    cache.retain_only(&[(1, 1)].into());
    assert_eq!(cache.bytes, reference_cache_bytes(&[&second]));
    assert!(
        matches!(cache.get(&(1, 1)), Some(Current::Frame(held)) if Arc::ptr_eq(&held, match &second { Current::Frame(frame) => frame, _ => unreachable!() }))
    );
    let mut plain = FrameCache::default();
    plain.insert((1, 0), sized_frame(0, 1_500));
    assert_eq!(cache.bytes - plain.bytes, 1_500 * 16);
    cache.retain_only(&BTreeSet::new());
    assert_eq!(
        (cache.frames.len(), cache.tables.len(), cache.bytes),
        (0, 0, 0)
    );
}

#[test]
fn a_required_gap_set_has_no_content_count_cap() {
    let required: BTreeSet<_> = (0..1_500).map(|hour| (1, hour)).collect();
    let mut cache = FrameCache::default();
    for &(branch, hour) in &required {
        cache.insert(
            (branch, hour),
            Current::Gap {
                branch,
                hour,
                reason: "recorded gap".into(),
            },
        );
    }
    cache.retain_only(&required);
    assert_eq!(
        cache.frames.keys().copied().collect::<BTreeSet<_>>(),
        required
    );
    assert_eq!(cache.bytes, 0, "gaps own no table allocations");
    cache.retain_only(&[(1, 1_499)].into());
    assert_eq!(
        cache.frames.keys().copied().collect::<Vec<_>>(),
        [(1, 1_499)]
    );
}

#[test]
fn the_window_follows_the_measured_throughput_and_the_live_memory_gate() {
    assert_eq!(window_len(36.0, 0.025, 8, true), 2);
    assert_eq!(window_len(36.0, 0.045, 8, true), 3);
    assert_eq!(window_len(24.0, 0.003, 8, true), 2);
    assert_eq!(window_len(0.0, 0.0, 8, true), 1, "paused");
    assert_eq!(window_len(1_000.0, 1.0, 8, true), 8, "never past the pool");
    assert_eq!(window_len(36.0, 0.045, 8, false), 1, "the gate closed");
    assert_eq!(window_len(36.0, 0.045, 0, true), 1);
    let newest = 100_000_000;
    let plenty = Some((8_000_000_000, 8_000_000_000));
    assert_eq!(memory_gate(None, newest, false), None, "unreadable memory");
    assert_eq!(memory_gate(plenty, newest, false), None);
    assert_eq!(memory_gate(plenty, newest, true), Some("os"));
    let commit = Some((newest - 1, 8_000_000_000));
    assert_eq!(memory_gate(commit, newest, false), Some("commit"));
    let physical = Some((8_000_000_000, newest - 1));
    assert_eq!(memory_gate(physical, newest, false), Some("physical"));
    assert_eq!(memory_gate(Some((newest, newest)), newest, false), None);
    let start = Instant::now();
    let mut throughput = Throughput::default();
    for read in 0..9u64 {
        throughput.took(start + Duration::from_millis(25 * read), 8);
    }
    let rate = throughput.take_rate(start + Duration::from_millis(200));
    assert!((rate - 40.0).abs() < 1e-6, "{rate}");
    assert!(throughput.take_rate(start + Duration::from_secs(10)) < 1.0);
    for service in [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.09, 0.10] {
        throughput.served(service, 8);
    }
    assert!((throughput.service_p90() - 0.10).abs() < 1e-12);
}

#[test]
fn the_frame_cache_keeps_only_the_required_hours_and_a_pause_releases_the_grid() {
    let grid: Vec<(BranchId, Hour)> = (0..20).map(|step| (1, 400 + step * 40)).collect();
    for ahead in [1, 2, 8] {
        let mut state = LaneState {
            frames: FrameCache::default(),
            ..LaneState::default()
        };
        state.set_want(grid.clone(), 0, 8);
        state.ahead = ahead;
        state.served = Some(grid[2]);
        state.urgent = Some((1, 5_000));
        for key in grid
            .iter()
            .copied()
            .chain([(1, 5_000), (1, 5_040), (1, 5_080)])
        {
            state.frames.insert(key, sized_frame(key.1, 1));
        }
        state.evict(Some((1, 5_040)));
        assert_eq!(state.window().len(), 3 + ahead, "ahead {ahead}");
        let mut expected: BTreeSet<_> = match ahead {
            1 => vec![400, 440, 480, 520],
            2 => vec![400, 440, 480, 520, 560],
            8 => vec![400, 440, 480, 520, 560, 600, 640, 680, 720, 760, 800],
            _ => unreachable!(),
        }
        .into_iter()
        .map(|hour| (1, hour))
        .collect();
        expected.extend([(1, 5_000), (1, 5_040)]);
        let held: BTreeSet<_> = state.frames.frames.keys().copied().collect();
        assert_eq!(held, expected, "ahead {ahead}");
        state.set_want(vec![(1, 500), (1, 501), (1, 499), (1, 502), (1, 498)], 2, 8);
        state.served = None;
        state.urgent = None;
        for hour in [500, 501] {
            state.frames.insert((1, hour), sized_frame(hour, 1));
        }
        state.evict(None);
        let held: Vec<_> = state.frames.frames.keys().copied().collect();
        assert_eq!(held, state.window().to_vec(), "ahead {ahead}");
        assert_eq!(held, [(1, 500)]);
    }
    let mut state = LaneState {
        frames: FrameCache::default(),
        ..LaneState::default()
    };
    state.set_want(grid.clone(), 0, 8);
    state.served = Some(grid[2]);
    state.gate = Some("commit");
    state.resize(Instant::now());
    assert_eq!(state.window(), &grid[2..4]);
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
        grid: None,
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
            .fog
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
    match result
        .recv_timeout(TEST_HANG_GUARD)
        .expect("source replies")
    {
        source_mailbox::Reply::Ready { value, .. } => value,
        source_mailbox::Reply::Stopped => Current::Gap {
            branch,
            hour,
            reason: LANE_STOPPED.to_owned(),
        },
        source_mailbox::Reply::Cancelled => panic!("ordinary request is not cancelled"),
        source_mailbox::Reply::Pressure(pressure) => panic!("{}", pressure.reason),
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
fn an_already_obsolete_walk_starts_no_native_work() {
    let hours: Vec<Hour> = (10..=300).collect();
    let replay = long_replay(&hours);
    let mut playback = Playback::unloaded(Arc::clone(&replay)).unwrap();
    let shared = LaneShared {
        mailbox: OnceLock::new(),
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    };
    // Nothing wants 290, so no source block is admitted.
    assert!(!DecodeLane::walk(&shared, &mut playback, (1, 290)));
    assert_eq!(playback.walk_start(1, 290), Some(9));
    assert!(!playback.holds_source());
    assert!(playback.source_paths().is_empty());
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
fn the_lane_window_retains_exactly_its_adaptive_required_hours() {
    let mut state = LaneState::default();
    state.set_want((0..100).map(|hour| (1, hour)).collect(), 0, 8);
    state.ahead = 3;
    assert_eq!(state.window(), &[(1, 0), (1, 1), (1, 2)]);
    state.served = Some((1, 10));
    assert_eq!(
        state.window(),
        &(0..=13).map(|hour| (1, hour)).collect::<Vec<_>>()
    );
    for hour in 0..100 {
        state.frames.insert((1, hour), sized_frame(hour, 1));
    }
    state.evict(Some((2, 90)));
    assert_eq!(
        state.frames.frames.keys().copied().collect::<Vec<_>>(),
        (0..=13).map(|hour| (1, hour)).collect::<Vec<_>>()
    );
    state.gate = Some("physical");
    state.resize(Instant::now());
    state.evict(None);
    assert_eq!(
        state.frames.frames.keys().copied().collect::<Vec<_>>(),
        [(1, 10), (1, 11)]
    );
}

#[test]
fn shrinking_decode_admission_keeps_already_decoded_wanted_hours() {
    let mut state = LaneState::default();
    state.set_want((100..108).map(|hour| (1, hour)).collect(), 0, 8);
    state.ahead = 8;
    state.served = Some((1, 100));
    for hour in 100..108 {
        state.frames.insert((1, hour), sized_frame(hour, 1));
    }
    state.decoded = Some((1, 107));
    let bytes = state.frames.bytes;
    state.gate = Some("physical");
    state.resize(Instant::now());
    assert_eq!(state.window(), &[(1, 100), (1, 101)]);
    state.evict(None);
    assert_eq!(state.frames.bytes, bytes);
    assert_eq!(state.frames.frames.len(), 8);
    for hour in 101..108 {
        state.set_want((hour..hour + 8).map(|hour| (1, hour)).collect(), 0, 8);
        state.evict(None);
        assert!(state.frames.contains_key(&(1, hour)), "no source rewind at {hour}");
        state.served = Some((1, hour));
        state.evict(None);
        assert_eq!(state.frames.frames.len(), (108 - hour) as usize);
    }
    assert_eq!(state.next(), Some((1, 108)), "only new work is admitted");
    state.set_want(vec![(2, 200)], 0, 8);
    state.served = None;
    state.evict(None);
    assert!(state.frames.frames.is_empty(), "a branch switch releases old prefetch");
    assert_eq!(state.frames.bytes, 0);
    state.set_want(vec![(1, 107)], 0, 8);
    state.frames.insert((1, 108), sized_frame(108, 1));
    state.decoded = Some((1, 108));
    state.evict(None);
    assert!(state.frames.frames.is_empty(), "a pause drops no-longer-wanted prefetch");
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
        grid: None,
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
        grid: None,
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
            Current::Gap {
                branch: 1,
                hour,
                reason: "recorded gap".into(),
            },
        );
        state
            .decode_profile
            .get_or_insert_with(BTreeMap::new)
            .insert((1, hour), sample);
    }
    state.set_want(vec![(1, 40), (1, 41)], 0, 2);
    state.served = Some((1, 20));
    state.urgent = Some((1, 30));
    state.evict(Some((1, 99)));
    let metadata = state.decode_profile.as_ref().unwrap();
    assert_eq!(
        metadata.keys().copied().collect::<Vec<_>>(),
        [(1, 20), (1, 30), (1, 40), (1, 99)]
    );
    assert_eq!(
        metadata.keys().copied().collect::<Vec<_>>(),
        state.frames.frames.keys().copied().collect::<Vec<_>>(),
    );
}

pub(super) fn paged_map(
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

#[test]
fn rewant_retains_the_mailboxs_pending_hour_until_the_lane_takes_it() {
    let shared = LaneShared {
        mailbox: OnceLock::new(),
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    };
    let mailbox = SourceMailbox::new(|_| true);
    shared.mailbox.set(mailbox.clone()).ok().unwrap();
    let _handle = mailbox.post(1, (1, 80), |_, _| {});
    let mut state = DecodeLane::lock(&shared);
    for hour in [40, 80, 120] {
        state.frames.insert((1, hour), sized_frame(hour, 1));
    }
    state.set_want(vec![(1, 40)], 0, 1);
    state.evict(DecodeLane::pending(&shared));
    assert_eq!(
        state.frames.frames.keys().copied().collect::<Vec<_>>(),
        [(1, 40), (1, 80)]
    );
    let read = mailbox.take().unwrap();
    assert_eq!(read.key, (1, 80));
    state.urgent = Some(read.key);
    state.evict(DecodeLane::pending(&shared));
    assert!(state.frames.contains_key(&(1, 80)));
    state.served = Some(read.key);
    state.urgent = None;
    state.evict(None);
    assert!(state.frames.contains_key(&(1, 80)));
    drop(state);
    read.ready(sized_frame(80, 1), None);
}

#[test]
fn evicted_backward_and_branch_requests_match_independent_source_seeks() {
    let replay = Arc::new(
        Replay::from_records(
            b"campaign".to_vec(),
            &[
                crate::playback::tests::records(1, &(10..=310).collect::<Vec<_>>()),
                crate::playback::tests::records(2, &(20..=80).collect::<Vec<_>>()),
            ],
        )
        .unwrap(),
    );
    let lane = lane_over(&replay);
    for (branch, hour) in [(1, 290), (1, 100), (2, 70), (1, 300), (2, 25), (1, 10)] {
        lane.want(vec![(branch, hour)], 0);
        let actual = source_frame(&lane.client(), branch, hour);
        let replay::container::SeekResult::Frame(expected) = replay.seek(branch, hour).unwrap()
        else {
            panic!("recorded hour")
        };
        assert_eq!(actual, Current::Frame(Arc::from(expected)));
    }
}

#[test]
fn resource_pressure_is_pending_not_a_terminal_part_failure() {
    assert!(Part::Names(Err(io::Error::from(io::ErrorKind::WouldBlock))).resource_pending());
    assert!(!Part::Names(Err(io::Error::other("named content refusal"))).resource_pending());
    assert!(!Part::Names(Ok(Arc::new(Placements::default()))).resource_pending());
}

#[test]
fn a_colour_credit_retry_does_not_advance_or_reapply_its_predecessor() {
    let (events, inbox) = mpsc::channel();
    let initial = Arc::new(viewer_world::map::map_mode::ColourState {
        mode: "before".into(),
        layers: Default::default(),
    });
    let mut calls = 0;
    let work = dispatch_plan::colour_job(
        11,
        (1, 100),
        sized_frame(100, 1),
        events,
        None,
        Some(Ok(Some(initial))),
        move |_, previous| {
            calls += 1;
            assert_eq!(previous.unwrap().mode, "before");
            if calls == 1 {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            Ok(Some(Arc::new(viewer_world::map::map_mode::ColourState {
                mode: "after".into(),
                layers: Default::default(),
            })))
        },
    );
    work();
    let retry = match inbox.recv().unwrap() {
        Event::RetryColour {
            ticket: 11,
            key: (1, 100),
            work,
            ..
        } => work,
        _ => panic!("pressure is not a colour refusal"),
    };
    retry();
    match inbox.recv().unwrap() {
        Event::Colour {
            ticket: 11,
            key: (1, 100),
            folded,
        } => {
            assert_eq!(folded.colour.unwrap().unwrap().mode, "after");
        }
        _ => panic!("one terminal result after retry"),
    }
    assert!(inbox.try_recv().is_err());
}

#[test]
fn quiet_political_hours_never_rebuild_border_geometry() {
    let zone = map_display_data::retained::Retained::build(0, Zone::default).unwrap();
    let built = Cell::new(0);
    let build = |_: &Zone| {
        built.set(built.get() + 1);
        (ClassPlan::default(), ZonePlan::default())
    };
    let first = Borders::reuse_or_build(7, zone.clone(), &[], build);
    assert_eq!(built.get(), 1);
    for _ in 0..1500 {
        let same = Borders::reuse_or_build(7, zone.clone(), &[Arc::clone(&first)], |_| {
            panic!("unchanged border operands must not visit the meshes")
        });
        assert!(Arc::ptr_eq(&same, &first));
    }
    let changed = Borders::reuse_or_build(8, zone.clone(), &[Arc::clone(&first)], build);
    assert_eq!(built.get(), 2);
    assert!(!Arc::ptr_eq(&first, &changed));
    let same = Borders::reuse_or_build(8, zone.clone(), &[Arc::clone(&changed)], build);
    assert!(Arc::ptr_eq(&same, &changed));
    assert_eq!(built.get(), 2);
}

thread_local! {
    static FINAL_SEEK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn during_final_seek() {
    FINAL_SEEK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[test]
fn cancellation_in_the_final_seek_clears_urgent_and_evicts_before_acknowledging() {
    let replay = long_replay(&(10..=40).collect::<Vec<_>>());
    let mut playback = Playback::unloaded(replay).unwrap();
    playback.seek(1, 10).unwrap();
    let shared = Arc::new(LaneShared {
        mailbox: OnceLock::new(),
        state: Mutex::new(LaneState::default()),
        changed: Condvar::new(),
    });
    let mailbox = SourceMailbox::new(|_| true);
    shared.mailbox.set(mailbox.clone()).ok().unwrap();
    {
        let mut state = DecodeLane::lock(&shared);
        state.frames.insert((1, 10), playback.current().clone());
        state.set_want(vec![(1, 10)], 0, 1);
    }
    let (send, reply) = mpsc::channel();
    let handle = mailbox.post(7, (1, 20), move |id, result| {
        send.send((id, result)).unwrap();
    });
    std::thread::scope(|scope| {
        let run = Arc::clone(&shared);
        let thread = scope.spawn(move || {
            FINAL_SEEK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new({
                    let shared = Arc::clone(&run);
                    move || {
                        handle.cancel();
                        let mut state = DecodeLane::lock(&shared);
                        state.set_want(vec![(1, 10)], 0, 1);
                        state.frames.insert((1, 20), sized_frame(20, 1));
                    }
                }))
            });
            DecodeLane::run(&run, playback);
        });
        let reply = reply.recv_timeout(TEST_HANG_GUARD);
        let (urgent, next, frames) = {
            let mut state = DecodeLane::lock(&shared);
            let snapshot = (
                state.urgent,
                state.next(),
                state.frames.frames.keys().copied().collect::<Vec<_>>(),
            );
            state.stop = true;
            snapshot
        };
        shared.changed.notify_all();
        thread.join().unwrap();
        let (id, result) = reply.unwrap();
        assert_eq!(id, 7);
        assert!(matches!(result, source_mailbox::Reply::Cancelled));
        assert_eq!(urgent, None);
        assert_eq!(next, None);
        assert_eq!(frames, [(1, 10)]);
    });
}
