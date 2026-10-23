use super::*;
use dashboard_base::snow_export::{Limits, Reader, Restore, Session};

#[test]
fn ordinary_walk_commits_snow_status_endpoints_and_preparation_rebuilds_a_corrupt_binding() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("snow-export-walk");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let session = super::super::snow_session(&source, None, &flag).unwrap();
    assert!(Session::ready(&store, &key, &session.key).unwrap());
    let reader = Reader::new(
        Arc::clone(&store),
        session.key.clone(),
        Limits {
            dimensions: [1, 1],
            provinces: 0,
            factors: 0,
        },
    );
    for (branch, first, last) in [(1, 10, 11), (2, 20, 21)] {
        let coverage = reader.coverage(branch).unwrap().unwrap();
        assert_eq!(
            (coverage.first, coverage.last, coverage.endpoints),
            (first, last, 2)
        );
        for hour in [first, last] {
            match reader.restore(branch, hour) {
                Restore::Miss(reason) => assert!(reason.contains("game content not configured")),
                Restore::Hit(_) => panic!("unconfigured content is not zero snow"),
            }
        }
    }
    assert!(restore(&source, None, &flag).unwrap().is_some());
    let mut stale = session.key.clone();
    stale.version += 1;
    let mut writer = store.extend(&key).unwrap();
    writer
        .write_at(&stale.section("binding"), 0, stale.bytes().as_slice())
        .unwrap();
    writer.commit().unwrap();
    assert!(restore(&source, None, &flag).unwrap().is_some());
    let rebuilt = run(&source, None, &flag, |_| {}).unwrap();
    assert_eq!(rebuilt.new_blocks, replay.index.len());
    assert!(Session::ready(&store, &key, &session.key).unwrap());
    assert!(restore(&source, None, &flag).unwrap().is_some());
}

fn snowy_frame(hour: i32) -> DecodedFrame {
    use replay_data::rows::{Recorded, WeatherDisplay, WeatherProvinceRow};
    let mut frame = super::frame(1, hour, 2);
    let mut rows = vec![WeatherProvinceRow {
        id: 0,
        display: Recorded::Value(Some(WeatherDisplay {
            width: 3,
            height: 2,
            refresh_interval: 25_000,
            refresh_identity: hour as u64 + 7,
            edges: vec![11, 12, 13, 14, 21, 22, 23, 24],
        })),
        ..Default::default()
    }];
    rows.extend((1..=3).map(|id| WeatherProvinceRow {
        id,
        snow: 30_000 + i64::from(hour),
        display_snow: 60 + id as u8,
        display_mud: id as u8,
        ..Default::default()
    }));
    frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(rows)).into();
    frame
}

fn content_fixture(frames: Vec<DecodedFrame>) -> (Fixture, Replay, ContentOptions) {
    let placeholder = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let fixture = Fixture::new(&placeholder);
    let game = fixture.root.join("snow-content");
    dashboard_base::test_fixtures::snow_content::write(&game);
    let (bound, context) = crate::test_fixtures::source_with_content(frames, &game);
    let replay = Replay::open(bound.replay_ref().bytes().to_vec()).unwrap();
    fixture.replace(&replay);
    (fixture, replay, context.content.unwrap())
}

fn viewer_reader(source: &Source, content: &ContentOptions) -> Reader {
    use dashboard_base::snow_export::{Inputs, Key, PRODUCER_VERSION};
    let resolved = source.content(&content.options).unwrap();
    let inputs = Inputs::load(&resolved.layers).unwrap();
    let key = Key::for_inputs(Ok((&inputs.cells, &inputs.winter)), PRODUCER_VERSION)
        .unwrap()
        .bind_recording(source.replay_ref(), || Ok(()))
        .unwrap();
    Reader::new(
        dashboard_base::prepared::recording_store(source.path()).unwrap(),
        key,
        inputs.limits(),
    )
}

fn corrupt_snow_range(source: &Source, content: &ContentOptions, suffix: &str, body: bool) {
    use std::io::Read;
    let (store, key) = super::super::super::files(source, Some(content)).unwrap();
    let section = format!("map-export-snow-v1/{suffix}");
    let mut range = store.read(&key, &section, 0).unwrap().unwrap();
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes).unwrap();
    let at = if body { bytes.len() - 1 } else { 0 };
    bytes[at] ^= 1;
    let mut writer = store.extend(&key).unwrap();
    writer.write_at(&section, 0, bytes.as_slice()).unwrap();
    writer.commit().unwrap();
}

#[test]
fn ordinary_walk_available_endpoints_match_live_runtime_inputs_and_the_viewer_binding() {
    use dashboard_base::snow_export::Inputs;
    let frames = vec![snowy_frame(0), snowy_frame(1)];
    let (mut fixture, _, content) = content_fixture(frames.clone());
    let source = fixture.source("snow-available");
    assert!(source.replay_ref().is_file_backed());
    let resolved = source.content(&content.options).unwrap();
    let inputs = Inputs::load(&resolved.layers).unwrap();
    let live: Vec<_> = frames
        .iter()
        .map(|frame| inputs.build(frame).unwrap())
        .collect();
    assert!(live[0].texels.iter().any(|&v| v != 0));
    assert!(
        live[0]
            .post_factors
            .as_ref()
            .unwrap()
            .iter()
            .any(|factor| factor.is_some_and(|factor| factor > 0.0))
    );
    assert_eq!(live[0].summary.provinces, 4);
    run(&source, Some(&content), &AtomicBool::new(false), |_| {}).unwrap();
    let reader = viewer_reader(&source, &content);
    assert_eq!(reader.coverage(1).unwrap().unwrap().endpoints, 2);
    for (frame, expected) in frames.iter().zip(live) {
        match reader.restore(1, frame.hour) {
            Restore::Hit(snow) => assert_eq!(&*snow, &expected),
            miss => panic!("available pipeline endpoint missed: {miss:?}"),
        }
    }
}

#[test]
fn corrupt_snow_payload_before_resume_rejects_the_checkpoint_and_rewalks_cleanly() {
    for body in [false, true] {
        let (mut fixture, first, content) = content_fixture(vec![snowy_frame(0), snowy_frame(1)]);
        let replay = first
            .append(&[continuation_records(
                &first,
                1,
                vec![snowy_frame(40), snowy_frame(41)],
            )])
            .unwrap();
        fixture.replace(&replay);
        let source = fixture.source("snow-corrupt-resume");
        let flag = AtomicBool::new(false);
        let (prefix, _) =
            run_measured(&source, Some(&content), &flag, true, Duration::ZERO, |_| {}).unwrap();
        assert_eq!(prefix.next_block, 1);
        corrupt_snow_range(&source, &content, "payloads", body);
        let (prefix, _) =
            run_measured(&source, Some(&content), &flag, true, Duration::ZERO, |_| {}).unwrap();
        assert_eq!(
            prefix.next_block, 1,
            "cache damage must restart, not resume the second block"
        );
        assert_eq!(prefix.new_blocks, 1);
        assert!(matches!(
            viewer_reader(&source, &content).restore(1, 0),
            Restore::Hit(_)
        ));
        run(&source, Some(&content), &flag, |_| {}).unwrap();
        assert!(restore(&source, Some(&content), &flag).unwrap().is_some());
    }
}

#[test]
fn corrupt_snow_payload_or_status_before_append_rebuilds_in_a_clean_transaction() {
    for suffix in ["payloads", "endpoints/1"] {
        let mut first_frames = vec![snowy_frame(0), snowy_frame(1)];
        if suffix == "endpoints/1" {
            first_frames[0].tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
                reason: "weather temporarily unreadable".into(),
            }
            .into();
        }
        let (mut fixture, first, content) = content_fixture(first_frames);
        let source = fixture.source("snow-corrupt-append-before");
        let flag = AtomicBool::new(false);
        run(&source, Some(&content), &flag, |_| {}).unwrap();
        corrupt_snow_range(&source, &content, suffix, false);
        let replay = first
            .append(&[continuation_records(
                &first,
                1,
                vec![snowy_frame(40), snowy_frame(41)],
            )])
            .unwrap();
        fixture.replace(&replay);
        let appended = fixture.source("snow-corrupt-append-after");
        let accepted = run(&appended, Some(&content), &flag, |_| {}).unwrap();
        assert_eq!(
            accepted.new_blocks, 2,
            "a damaged reachable cache must rewalk old source too"
        );
        assert!(matches!(
            viewer_reader(&appended, &content).restore(1, 41),
            Restore::Hit(_)
        ));
        assert!(restore(&appended, Some(&content), &flag).unwrap().is_some());
    }
}

#[test]
fn over_geometry_diagnostics_are_named_refusals_that_remain_inheritable_on_append() {
    use dashboard_base::snow_export::Inputs;
    use std::io::Read;
    let mut frames = vec![snowy_frame(0), snowy_frame(1)];
    let reason = "weather unavailable with a synthetic diagnostic ".repeat(256);
    frames[0].tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
        reason: reason.clone(),
    }
    .into();
    let (mut fixture, first, content) = content_fixture(frames);
    let source = fixture.source("snow-long-status-before");
    let inputs = Inputs::load(&source.content(&content.options).unwrap().layers).unwrap();
    assert!(reason.len() + 8 > inputs.limits().maximum().unwrap());
    let flag = AtomicBool::new(false);
    run(&source, Some(&content), &flag, |_| {}).unwrap();
    match viewer_reader(&source, &content).restore(1, 0) {
        Restore::Miss(reason) => assert!(
            reason.contains(
                "unsupported: snow export: diagnostic exceeds runtime geometry admission"
            ),
            "{reason}"
        ),
        _ => panic!("long diagnostic must not become an available empty product"),
    }
    let (store, previous) = super::super::super::files(&source, Some(&content)).unwrap();
    let section = "map-export-snow-v1/endpoints/1";
    let mut before = Vec::new();
    store
        .read(&previous, section, 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut before)
        .unwrap();
    let replay = first
        .append(&[continuation_records(
            &first,
            1,
            vec![snowy_frame(40), snowy_frame(41)],
        )])
        .unwrap();
    fixture.replace(&replay);
    let appended = fixture.source("snow-long-status-after");
    let accepted = run(&appended, Some(&content), &flag, |_| {}).unwrap();
    assert_eq!(
        accepted.new_blocks, 1,
        "valid named refusal preserves accepted source prefix"
    );
    let (store, current) = super::super::super::files(&appended, Some(&content)).unwrap();
    let mut after = Vec::new();
    store
        .read(&current, section, 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut after)
        .unwrap();
    assert_eq!(before, after);
    assert!(matches!(
        viewer_reader(&appended, &content).restore(1, 41),
        Restore::Hit(_)
    ));
}

fn with_snow_check<T>(hook: impl Fn(&'static str) + 'static, run: impl FnOnce() -> T) -> T {
    type Hook = Option<std::rc::Rc<dyn Fn(&'static str)>>;
    struct Reset(Hook);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_SNOW_CHECK.with(|hook| *hook.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_SNOW_CHECK.with(|value| value.replace(Some(std::rc::Rc::new(hook)))));
    run()
}

#[test]
fn completed_restore_answers_the_ordinary_checkpoint_without_binding_or_validating_snow() {
    use std::{cell::Cell, rc::Rc};
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("snow-free-restore");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let calls = Rc::new(Cell::new([0, 0]));
    let counted = Rc::clone(&calls);
    with_snow_check(
        move |stage| {
            let mut counts = counted.get();
            counts[usize::from(stage == "validate")] += 1;
            counted.set(counts);
        },
        || {
            let unchanged = run(&source, None, &flag, |_| {}).unwrap();
            assert_eq!(unchanged.new_blocks, 0);
        },
    );
    assert!(calls.get()[0] > 0, "the control must actually bind");
    assert!(calls.get()[1] > 4, "the control must scan real endpoints");
    let expected = restore(&source, None, &flag).unwrap().unwrap();
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let mut writer = store.extend(&key).unwrap();
    for section in [
        "map-export-snow-v1/binding",
        "map-export-snow-v1/endpoints/1",
    ] {
        writer
            .write_at(section, 0, b"invalid snow".as_slice())
            .unwrap();
    }
    writer.commit().unwrap();
    calls.set([0, 0]);
    let counted = Rc::clone(&calls);
    let restored = with_snow_check(
        move |stage| {
            let mut counts = counted.get();
            counts[usize::from(stage == "validate")] += 1;
            counted.set(counts);
            panic!("Open must not enter snow {stage}");
        },
        || restore(&source, None, &flag).unwrap().unwrap(),
    );
    assert_eq!(calls.get(), [0, 0]);
    assert_eq!(restored.next_block, source.replay_ref().index.len());
    assert_eq!(restored.new_blocks, 0);
    assert_eq!(restored.bytes, expected.bytes);
    assert_eq!(restored.coverage, expected.coverage);
    assert_eq!(restored.families, expected.families);
    assert_eq!(
        serde_json::to_value(&restored.sections).unwrap(),
        serde_json::to_value(&expected.sections).unwrap()
    );
}

#[test]
fn cancelling_during_the_preparation_recording_bind_interrupts_before_publication() {
    use std::{cell::Cell, rc::Rc};
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("cancel-snow-bind");
    assert!(source.replay_ref().is_file_backed());
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    let checks = Rc::new(Cell::new(0));
    let counted = Rc::clone(&checks);
    let mut accepted = 0;
    let error = with_snow_check(
        move |stage| {
            assert_eq!(stage, "bind");
            counted.set(counted.get() + 1);
            // The fourth check is inside the witness's first held-byte callback.
            if counted.get() == 4 {
                signal.store(true, Ordering::Release);
            }
        },
        || run(&source, None, &cancelled, |_| accepted += 1),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(checks.get(), 4, "cancellation stops the current bind check");
    assert_eq!(accepted, 0);
    let (store, _) = super::super::super::files(&source, None).unwrap();
    assert!(store.saved_key().unwrap().is_none());
    cancelled.store(false, Ordering::Release);
    let finished = run(&source, None, &cancelled, |_| {}).unwrap();
    assert_eq!(finished.new_blocks, source.replay_ref().index.len());
    assert!(restore(&source, None, &cancelled).unwrap().is_some());
}

#[test]
fn cancelling_preparation_snow_validation_preserves_the_accepted_checkpoint() {
    use std::{cell::Cell, rc::Rc};
    let (mut fixture, _, content) = content_fixture(vec![snowy_frame(0), snowy_frame(1)]);
    let source = fixture.source("cancel-snow-validation");
    let cancelled = Arc::new(AtomicBool::new(false));
    let expected = run(&source, Some(&content), &cancelled, |_| {}).unwrap();
    let before = family_lanes::payloads(&source, Some(&content));
    assert!(
        before
            .keys()
            .any(|(section, _)| section == "map-export-snow-v1/payloads")
    );
    let signal = Arc::clone(&cancelled);
    let checks = Rc::new(Cell::new(0));
    let counted = Rc::clone(&checks);
    let mut accepted = 0;
    let error = with_snow_check(
        move |stage| {
            if stage == "validate" {
                counted.set(counted.get() + 1);
                // The sixth check follows the first available product's validation.
                if counted.get() == 6 {
                    signal.store(true, Ordering::Release);
                }
            }
        },
        || run(&source, Some(&content), &cancelled, |_| accepted += 1),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(checks.get(), 6);
    assert_eq!(accepted, 0);
    assert_eq!(family_lanes::payloads(&source, Some(&content)), before);
    cancelled.store(false, Ordering::Release);
    let restored = restore(&source, Some(&content), &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(restored.next_block, expected.next_block);
    assert!(matches!(
        viewer_reader(&source, &content).restore(1, 0),
        Restore::Hit(_)
    ));
}

#[test]
fn zero_walk_checkpoint_without_snow_preserves_completed_independent_air_work() {
    use crate::prepared::prepass;
    use replay_data::rows::AirLossEventRow;
    use std::sync::atomic::AtomicUsize;
    let frames = (0..2)
        .map(|hour| {
            let mut frame = super::frame(1, hour, 2);
            frame.tables[TableId::AirLossEvents.index()] =
                TableView::Complete(TableRows::AirLossEvents(vec![AirLossEventRow {
                    id: hour as u64 + 1,
                    game_hour: hour,
                    country: 100_000,
                    region: 40_001,
                    native_cause: 1,
                    amount_raw: 5_000,
                    opponent: Some(100_001),
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::BuildingBombingDamage.index()] =
                TableView::Complete(TableRows::BuildingBombingDamage(Vec::new())).into();
            frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(Vec::new())).into();
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("zero-walk-independent-air");
    let extracted = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&extracted);
    let hook: prepass::Hook = Arc::new(move |family, _| {
        if family == "air_ledger" {
            counted.fetch_add(1, Ordering::Relaxed);
        }
        prepass::Fault::Continue
    });
    independent::with_test_only(&["air_ledger"], || {
        independent::with_test_one_after_another(|| {
            prepass::with_test_prepass(1, Some(hook), || {
                let cancelled = AtomicBool::new(false);
                let mut attempted = 0;
                let error = run_checked(
                    &source,
                    None,
                    &cancelled,
                    |_| {},
                    |_| {
                        attempted += 1;
                        cancelled.store(true, Ordering::Release);
                        Ok(())
                    },
                )
                .unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::Interrupted);
                assert_eq!(attempted, 1);
                let (store, key) = super::super::super::files(&source, None).unwrap();
                let (zero, _) = checkpoint(&store, &key, None).unwrap().unwrap();
                assert_eq!(zero.cursor.next_block, 0);
                assert!(!zero.accepted_walk());
                assert!(
                    !zero.map.ranges.is_empty(),
                    "timeline bounds are not accepted walk work"
                );
                assert_eq!(
                    zero.independent["air_ledger"].next_block,
                    replay.index.len()
                );
                assert!(zero.independent["air_ledger"].refused.is_empty());
                assert!(
                    extracted.load(Ordering::Relaxed) > 0,
                    "air actually extracted source"
                );
                let payloads = family_lanes::payloads(&source, None);
                let air = |payloads: &BTreeMap<(String, u64), Vec<u8>>| -> BTreeMap<_, _> {
                    payloads
                        .iter()
                        .filter(|((name, _), _)| {
                            name.starts_with("military-air-")
                                || name.starts_with("military-bombing-")
                                || name == "preparation-family-air_ledger-1"
                        })
                        .map(|(key, bytes)| (key.clone(), bytes.clone()))
                        .collect()
                };
                let accepted_air = air(&payloads);
                assert!(
                    accepted_air
                        .keys()
                        .any(|(name, _)| name.starts_with("military-air-losses-1-series"))
                );
                assert!(accepted_air.values().all(|bytes| !bytes.is_empty()));
                let mut writer = store.replace(key.clone()).unwrap();
                let mut zero = zero;
                zero.bytes = payloads
                    .iter()
                    .filter(|((name, _), _)| {
                        name != CHECKPOINT && !name.starts_with("map-export-snow-v1/")
                    })
                    .map(|(_, bytes)| bytes.len() as u64)
                    .sum();
                for ((name, ordinal), bytes) in &payloads {
                    if !name.starts_with("map-export-snow-v1/") && name != CHECKPOINT {
                        writer.write_at(name, *ordinal, bytes.as_slice()).unwrap();
                    }
                }
                writer
                    .write_at(CHECKPOINT, 0, serde_json::to_vec(&zero).unwrap().as_slice())
                    .unwrap();
                writer.commit().unwrap();
                assert!(metadata_prefix(&zero, &replay, None).unwrap());
                assert!(
                    store
                        .read(&key, "map-export-snow-v1/binding", 0)
                        .unwrap()
                        .is_none()
                );
                assert_eq!(air(&family_lanes::payloads(&source, None)), accepted_air);
                let extracted_before = extracted.load(Ordering::Relaxed);
                let result = run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
                assert_eq!(result.new_blocks, replay.index.len());
                assert_eq!(
                    extracted.load(Ordering::Relaxed),
                    extracted_before,
                    "completed air work must not be extracted again"
                );
                assert_eq!(air(&family_lanes::payloads(&source, None)), accepted_air);
                let session =
                    super::super::snow_session(&source, None, &AtomicBool::new(false)).unwrap();
                assert!(Session::ready(&store, &key, &session.key).unwrap());
                assert_eq!(
                    Reader::new(
                        Arc::clone(&store),
                        session.key,
                        Limits {
                            dimensions: [1, 1],
                            provinces: 0,
                            factors: 0,
                        }
                    )
                    .coverage(1)
                    .unwrap()
                    .unwrap()
                    .endpoints,
                    2
                );
                assert!(
                    restore(&source, None, &AtomicBool::new(false))
                        .unwrap()
                        .is_some()
                );
                let changes: [fn(&mut Checkpoint); 7] = [
                    |state| state.cursor.next_block = 1,
                    |state| state.cursor.previous = Some(state.index[0].clone()),
                    |state| state.map.resume_range = Some(0),
                    |state| state.map.branches.entry(1).or_default().checkpoints = 1,
                    |state| state.map.branches.entry(1).or_default().since_checkpoint = 1,
                    |state| state.map.branches.entry(1).or_default().last_hour = Some(0),
                    |state| state.map.branches.entry(1).or_default().last_block = Some(0),
                ];
                for change in changes {
                    let mut accepted = zero.clone();
                    change(&mut accepted);
                    assert!(
                        accepted.accepted_walk(),
                        "any accepted walk/map progress needs snow validation"
                    );
                }
            })
        })
    });
}

fn snow_air_frames(first: i32) -> Vec<DecodedFrame> {
    use replay_data::rows::AirStatisticsRow;
    [first, first + 1]
        .into_iter()
        .map(|hour| {
            let mut frame = snowy_frame(hour);
            frame.tables[TableId::AirStatistics.index()] =
                TableView::Complete(TableRows::AirStatistics(vec![AirStatisticsRow {
                    country: 100_000,
                    region: 3,
                    channels: [i64::from(hour) + 5; 24],
                    ..Default::default()
                }]))
                .into();
            frame.metrics.table_ns[TableId::AirStatistics.index()] = 1;
            frame
        })
        .collect()
}

fn publish_external_air(
    source: &Source,
    content: &ContentOptions,
    block: usize,
    frames: &[DecodedFrame],
) {
    use super::super::external::{BoundSource, ExternalFamily, Family, Publication};
    let mut family = ExternalFamily::begin(
        BoundSource::open(source, Some(content)).unwrap(),
        Family::AirStatistics,
        1,
    )
    .unwrap();
    let mut changes = FrameChangeTracker::new();
    for frame in frames {
        let changes = changes.observe(frame, source.replay_ref().format);
        family.observe(frame, block, frame.hour, &changes).unwrap();
    }
    family.seal_block(block).unwrap();
    assert_eq!(family.publish_saved().unwrap(), Publication::Published);
}

fn external_air_sections(
    payloads: &BTreeMap<(String, u64), Vec<u8>>,
) -> BTreeMap<(String, u64), Vec<u8>> {
    payloads
        .iter()
        .filter(|((section, _), _)| {
            section.starts_with("air-statistics-")
                || section.starts_with("preparation-family-air-statistics-")
        })
        .map(|(key, bytes)| (key.clone(), bytes.clone()))
        .collect()
}

#[test]
fn external_first_zero_walk_resumes_on_the_ordinary_snow_writer_without_reextracting_air() {
    use std::sync::atomic::AtomicUsize;
    let frames = snow_air_frames(0);
    let (mut fixture, replay, content) = content_fixture(frames.clone());
    let recorder = fixture.source("snow-external-recorder");
    let app = fixture.source("snow-external-app");
    let flag = AtomicBool::new(false);
    let extracted = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&extracted);
    let hook: prepass::Hook = Arc::new(move |name, _| {
        if name == "air-statistics" {
            counted.fetch_add(1, Ordering::Relaxed);
        }
        prepass::Fault::Continue
    });
    independent::with_test_one_after_another(|| {
        prepass::with_test_prepass(1, Some(hook), || {
            let (store, stale) = super::super::super::files(&app, Some(&content)).unwrap();
            store.replace(stale.clone()).unwrap().commit().unwrap();
            store.clear().unwrap();
            with_snow_check(
                |stage| panic!("external publication must not enter snow {stage}"),
                || publish_external_air(&recorder, &content, 0, &frames),
            );
            assert!(!store.is_reusable(&stale).unwrap());
            let (store, key) = super::super::super::files(&recorder, Some(&content)).unwrap();
            let (zero, _) = checkpoint(&store, &key, None).unwrap().unwrap();
            assert!(!zero.accepted_walk());
            assert_eq!(zero.cursor.next_block, 0);
            assert_eq!(
                zero.independent["air-statistics"].next_block,
                replay.index.len()
            );
            assert!(zero.independent["air-statistics"].refused.is_empty());
            let before = family_lanes::payloads(&recorder, Some(&content));
            assert!(
                !before
                    .keys()
                    .any(|(name, _)| name.starts_with("map-export-snow-v1/"))
            );
            let air = external_air_sections(&before);
            assert!(
                air.keys()
                    .any(|(name, _)| name.starts_with("air-statistics-series-"))
            );
            assert!(air.values().all(|bytes| !bytes.is_empty()));
            let session = super::super::snow_session(&recorder, Some(&content), &flag).unwrap();
            assert!(!session.has_history(&store, &key, [1], || Ok(())).unwrap());
            {
                let mut writer = store.extend(&key).unwrap();
                session.initialise(&mut writer).unwrap();
                for frame in &frames {
                    session.observe(&mut writer, frame, 1).unwrap();
                }
                assert!(
                    writer
                        .read_at(&session.key.records(1), 1)
                        .unwrap()
                        .is_some()
                );
                assert!(
                    writer
                        .read_at(&session.key.section("cursor/1"), 0)
                        .unwrap()
                        .is_some()
                );
                assert!(
                    writer_checkpoint_matches(&mut writer, Some(&before[&(CHECKPOINT.into(), 0)]))
                        .unwrap()
                );
                assert_eq!(family_lanes::payloads(&recorder, Some(&content)), before);
            }
            assert_eq!(family_lanes::payloads(&recorder, Some(&content)), before);
            assert_eq!(extracted.load(Ordering::Relaxed), 0);
            let result = crate::prepared::with_test_cell(&TEST_REPLANS, Some(0), || {
                let result = run(&app, Some(&content), &flag, |_| {}).unwrap();
                assert_eq!(TEST_REPLANS.get(), Some(0));
                result
            });
            assert_eq!(result.new_blocks, replay.index.len());
            assert_eq!(
                extracted.load(Ordering::Relaxed),
                0,
                "external air must not be re-extracted"
            );
            assert!(
                store.is_reusable(&stale).unwrap(),
                "refresh must repair the saved-manifest identity"
            );
            let finished = family_lanes::payloads(&app, Some(&content));
            assert_eq!(external_air_sections(&finished), air);
            assert!(Session::ready(&store, &key, &session.key).unwrap());
            let reader = viewer_reader(&app, &content);
            let coverage = reader.coverage(1).unwrap().unwrap();
            assert_eq!(
                (coverage.first, coverage.last, coverage.endpoints),
                (0, 1, 2)
            );
            let inputs = dashboard_base::snow_export::Inputs::load(
                &app.content(&content.options).unwrap().layers,
            )
            .unwrap();
            for frame in &frames {
                let expected = inputs.build(frame).unwrap();
                assert!(expected.texels.iter().any(|&value| value != 0));
                match reader.restore(1, frame.hour) {
                    Restore::Hit(actual) => assert_eq!(&*actual, &expected),
                    miss => panic!("external-first snow endpoint missed: {miss:?}"),
                }
            }
            assert!(restore(&app, Some(&content), &flag).unwrap().is_some());
            store.clear().unwrap();
            let fresh = fixture.source("snow-external-fresh-reference");
            run(&fresh, Some(&content), &flag, |_| {}).unwrap();
            assert!(
                extracted.load(Ordering::Relaxed) > 0,
                "the fresh control must extract real air rows"
            );
            let expected = family_lanes::payloads(&fresh, Some(&content));
            assert_eq!(
                external_air_sections(&finished),
                external_air_sections(&expected)
            );
            let snow = |payloads: &BTreeMap<(String, u64), Vec<u8>>| -> BTreeMap<_, _> {
                payloads
                    .iter()
                    .filter(|((name, _), _)| name.starts_with("map-export-snow-v1/"))
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect()
            };
            assert_eq!(snow(&finished), snow(&expected));
        })
    });
}

#[test]
fn snow_rebinds_after_external_writer_replanning_and_cancellation_publishes_nothing() {
    use std::{cell::Cell, rc::Rc, sync::atomic::AtomicUsize};
    for (existing, cancel) in [(false, false), (true, false), (false, true), (true, true)] {
        let frames = snow_air_frames(0);
        let (mut fixture, first, content) = content_fixture(frames.clone());
        let tail = snow_air_frames(40);
        let replay = first
            .append(&[continuation_records(&first, 1, tail.clone())])
            .unwrap();
        fixture.replace(&replay);
        let app = fixture.source("snow-replan-app");
        let recorder = fixture.source("snow-replan-recorder");
        let (store, key) = super::super::super::files(&app, Some(&content)).unwrap();
        if existing {
            publish_external_air(&recorder, &content, 0, &frames);
        }
        let planned_saved = store.saved_key().unwrap();
        let planned_checkpoint = checkpoint_bytes(&store, &key, planned_saved.as_ref()).unwrap();
        let mut holder = Some(if existing {
            store.extend(&key).unwrap()
        } else {
            store.replace(key.clone()).unwrap()
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let published = Rc::new(Cell::new(false));
        let after_publication = Rc::clone(&published);
        let rebind_checks = Rc::new(Cell::new(0));
        let counted = Rc::clone(&rebind_checks);
        let extracted = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&extracted);
        let hook: prepass::Hook = Arc::new(move |name, _| {
            if name == "air-statistics" {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            prepass::Fault::Continue
        });
        let mut external = None;
        let result = with_snow_check(
            move |stage| {
                if stage == "bind" && after_publication.get() {
                    counted.set(counted.get() + 1);
                    if cancel {
                        signal.store(true, Ordering::Release);
                    }
                }
            },
            || {
                independent::with_test_one_after_another(|| {
                    prepass::with_test_prepass(1, Some(hook), || {
                        run(&app, Some(&content), &cancelled, |accepted| {
                            if accepted.sections.iter().any(|row| row.id == "writer")
                                && !published.get()
                            {
                                drop(holder.take());
                                if !existing {
                                    assert!(store.saved_key().unwrap().is_none());
                                    publish_external_air(&recorder, &content, 0, &frames);
                                }
                                publish_external_air(&recorder, &content, 1, &tail);
                                let current = store.saved_key().unwrap();
                                assert_eq!(current == planned_saved, existing);
                                assert_ne!(
                                    checkpoint_bytes(&store, &key, current.as_ref()).unwrap(),
                                    planned_checkpoint
                                );
                                let payloads = family_lanes::payloads(&recorder, Some(&content));
                                assert!(
                                    !payloads
                                        .keys()
                                        .any(|(name, _)| name.starts_with("map-export-snow-v1/"))
                                );
                                external = Some(payloads);
                                published.set(true);
                            }
                        })
                    })
                })
            },
        );
        assert!(
            published.get(),
            "the real writer wait must publish externally"
        );
        assert!(
            rebind_checks.get() > 0,
            "a changed checkpoint must construct a fresh snow session"
        );
        assert_eq!(extracted.load(Ordering::Relaxed), 0);
        if cancel {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
            assert_eq!(rebind_checks.get(), 1);
            assert_eq!(
                family_lanes::payloads(&recorder, Some(&content)),
                external.unwrap()
            );
            let (store, key) = super::super::super::files(&recorder, Some(&content)).unwrap();
            assert!(
                !checkpoint(&store, &key, None)
                    .unwrap()
                    .unwrap()
                    .0
                    .accepted_walk()
            );
        } else {
            assert_eq!(result.unwrap().new_blocks, replay.index.len());
            let finished = family_lanes::payloads(&app, Some(&content));
            assert_eq!(
                external_air_sections(&finished),
                external_air_sections(&external.unwrap())
            );
            let reader = viewer_reader(&app, &content);
            assert_eq!(reader.coverage(1).unwrap().unwrap().endpoints, 3);
            for hour in [0, 40, 41] {
                assert!(matches!(reader.restore(1, hour), Restore::Hit(_)));
            }
            let (store, key) = super::super::super::files(&app, Some(&content)).unwrap();
            for section in ["binding", "cursor/1"] {
                assert_eq!(
                    store
                        .range_count(&key, &format!("map-export-snow-v1/{section}"))
                        .unwrap(),
                    Some(1)
                );
            }
            assert!(restore(&app, Some(&content), &cancelled).unwrap().is_some());
        }
    }
}
