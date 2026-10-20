use super::*;
use dashboard_base::prepared::frame_changes::{FrameChangeTracker, FrameChanges};
use dashboard_keyed::query::{KeyedRun, KeyedWindow};
use replay::container::BlockChange;

fn read(hour: i32, slot: u32, tag: &[u8]) -> DecodedFrame {
    let mut frame = statistics(hour, vec![stat(3, [-17; 24])]);
    identity(&mut frame, slot, tag);
    frame
}

fn identity(frame: &mut DecodedFrame, slot: u32, tag: &[u8]) {
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 11,
            canonical_slot: slot,
            tag: tag.to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 21,
            canonical_slot: 8,
            tag: b"SPARE".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: slot,
            },
            CountryLinkRow {
                id: 21,
                canonical_slot: 8,
            },
        ]));
}

fn carried(frame: &DecodedFrame, hour: i32) -> DecodedFrame {
    let mut frame = frame.clone();
    frame.hour = hour;
    frame.metrics.table_ns[TableId::AirStatistics.index()] = 0;
    frame
}

fn observe(
    scan: &mut Scan<AirStatisticsObserver>,
    tracker: &mut FrameChangeTracker,
    frame: &DecodedFrame,
) {
    let changes = tracker.observe(frame, FormatVersion::CURRENT);
    scan.prepare_changed(FormatVersion::CURRENT, frame, &changes)
        .unwrap();
}

fn save(scan: &mut Scan<AirStatisticsObserver>, fixture: &Fixture) {
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
}

fn restore(fixture: &Fixture) -> Scan<AirStatisticsObserver> {
    series::read_catalogue(&fixture.store, &fixture.key, AirStatisticsObserver::NAME, 1)
        .unwrap()
        .unwrap()
        .1
}

fn window(fixture: &Fixture, first: i32, last: i32) -> KeyedWindow {
    query::open::<AirStatisticsObserver>(&fixture.store, &fixture.key, 1, &AtomicBool::new(false))
        .unwrap()
        .unwrap()
        .window(first, last, None, |_| true)
        .unwrap()
}

fn known(window: &KeyedWindow) -> Vec<(i32, i32)> {
    window
        .known
        .iter()
        .map(|span| (span.first_hour, span.last_hour))
        .collect()
}

fn recorded<'a>(window: &'a KeyedWindow, name: &str) -> &'a [KeyedRun] {
    &window
        .series
        .iter()
        .find(|series| series.name == name)
        .unwrap()
        .runs
}

fn reuse_frames() -> Vec<DecodedFrame> {
    let first = read(102, 7, b"SYN");
    let stable = carried(&first, 103);
    let mut reused = carried(&stable, 104);
    identity(&mut reused, 9, b"SYN");
    let mut returned = carried(&reused, 105);
    identity(&mut returned, 7, b"SYN");
    let fresh = read(106, 9, b"SYN");
    let last = carried(&fresh, 107);
    vec![first, stable, reused, returned, fresh, last]
}

#[test]
fn midday_carry_keeps_read_time_binding_and_actual_read_date() {
    let first = read(102, 7, b"SYN");
    let frames = vec![first.clone(), carried(&first, 103), carried(&first, 104)];
    let fixture = Fixture::new();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        observe(&mut scan, &mut tracker, frame);
    }
    save(&mut scan, &fixture);
    let reply = answer(
        &fixture,
        frames,
        json!({"branchId":"1","tag":"SYN@7","channel":3}),
        true,
        None,
    );
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":102,"lastHour":104}])
    );
    assert_eq!(reply["window"]["reads"], json!([[102, 104, 102]]));
    assert_eq!(
        runs(&reply, "SYN@7|3|c3"),
        &json!([[102, 104, "-17", true]])
    );
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 4);
    assert!(scan.observer.refusals.is_empty());
}

#[test]
fn midday_slot_reuse_is_unknown_while_identity_differs_and_known_again_when_it_returns() {
    let fixture = Fixture::new();
    let frames = reuse_frames();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        observe(&mut scan, &mut tracker, frame);
    }
    save(&mut scan, &fixture);
    let window = window(&fixture, 102, 107);
    // Hour 104 links the raw reference to another slot: unknown. At 105 it links
    // to the read-time slot again, so the read's attribution holds again.
    assert_eq!(known(&window), [(102, 103), (105, 107)]);
    assert_eq!(window.reads, [(102, 103, 102), (105, 105, 102), (106, 107, 106)]);
    for series in &window.series {
        let expected: &[(i32, i32)] = if series.name.starts_with("SYN@7|") {
            &[(102, 103), (105, 105)]
        } else {
            assert!(series.name.starts_with("SYN@9|"));
            &[(106, 107)]
        };
        let spans: Vec<_> = series.runs.iter().map(|run| (run.0, run.1)).collect();
        assert_eq!(spans, expected, "{}", series.name);
    }
    assert_eq!(scan.observer.refusals.values().sum::<u64>(), 1);
    assert!(
        scan.observer
            .refusals
            .keys()
            .all(|reason| reason.contains("supplying-read binding"))
    );
}

#[test]
fn changed_tag_refuses_carry_but_unrelated_country_changes_do_not() {
    let first = read(102, 7, b"SYN");
    let mut unrelated = carried(&first, 103);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *unrelated.tables[TableId::Countries.index()]
    {
        rows[1].tag = b"OTHER".to_vec();
    }
    let mut renamed = carried(&unrelated, 104);
    identity(&mut renamed, 7, b"ALT");
    let returned = carried(&first, 105);
    let fresh = read(106, 7, b"ALT");
    let frames = [first, unrelated, renamed, returned, fresh];
    let fixture = Fixture::new();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        observe(&mut scan, &mut tracker, frame);
    }
    save(&mut scan, &fixture);
    let window = window(&fixture, 102, 106);
    // The rename at 104 is unknown; the read-time identity holds again at 105.
    assert_eq!(known(&window), [(102, 103), (105, 106)]);
    assert_eq!(
        recorded(&window, "SYN@7|3|c0"),
        [(102, 103, "-17".into(), true), (105, 105, "-17".into(), true)]
    );
    assert_eq!(
        recorded(&window, "ALT@7|3|c0"),
        [(106, 106, "-17".into(), true)]
    );
}

#[test]
fn carried_start_and_refused_fresh_read_cannot_borrow_later_country_identity() {
    let good = read(102, 7, b"SYN");
    let mut failed = read(104, 7, b"SYN");
    *failed.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "Synthetic unavailable country links".into(),
    };
    let frames = [
        carried(&good, 102),
        read(103, 7, b"SYN"),
        failed,
        carried(&good, 105),
        read(106, 7, b"SYN"),
    ];
    let fixture = Fixture::new();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        observe(&mut scan, &mut tracker, frame);
    }
    save(&mut scan, &fixture);
    let window = window(&fixture, 102, 106);
    assert_eq!(known(&window), [(103, 103), (106, 106)]);
    assert_eq!(window.reads, [(103, 103, 103), (106, 106, 106)]);
    assert_eq!(
        recorded(&window, "SYN@7|3|c0"),
        [
            (103, 103, "-17".into(), true),
            (106, 106, "-17".into(), true),
        ]
    );
    assert_eq!(scan.observer.refusals.values().sum::<u64>(), 3);
}

#[test]
fn continuation_preserves_frozen_and_invalidated_bindings() {
    let frames = reuse_frames();
    let fixture = Fixture::new();
    let mut uninterrupted = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        observe(&mut uninterrupted, &mut tracker, frame);
    }
    save(&mut uninterrupted, &fixture);
    let expected = serde_json::to_value(window(&fixture, 102, 107)).unwrap();
    for split in [1, 2, 3, 4] {
        let fixture = Fixture::new();
        let mut scan = Scan::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[..split] {
            observe(&mut scan, &mut tracker, frame);
        }
        save(&mut scan, &fixture);
        scan = restore(&fixture);
        tracker.invalidate();
        for frame in &frames[split..] {
            observe(&mut scan, &mut tracker, frame);
        }
        save(&mut scan, &fixture);
        assert_eq!(
            serde_json::to_value(window(&fixture, 102, 107)).unwrap(),
            expected
        );
        assert_eq!(scan.observer.refusals, uninterrupted.observer.refusals);
    }
}

#[test]
fn block_prepass_rebinds_at_an_equal_fresh_read_after_carried_reuse() {
    let first = read(102, 7, b"SYN");
    let mut reused = carried(&first, 103);
    identity(&mut reused, 9, b"SYN");
    let stable = carried(&reused, 104);
    let mut fresh = carried(&reused, 105);
    fresh.metrics.table_ns[TableId::AirStatistics.index()] = 1;
    let last = carried(&fresh, 106);
    let replay = test_fixtures::replay(vec![first, reused, stable, fresh, last]);
    let mut scan = Scan::default();
    let fields = AirStatisticsObserver::fields().unwrap();
    let mut observed_hours = Vec::new();
    for index in &replay.index {
        let mut extract = (AirStatisticsObserver::family().start)(replay.format);
        let mut previous: Option<i32> = None;
        replay
            .read_block_changes(
                index,
                &fields,
                |change| {
                    match change {
                        BlockChange::Changed { frame, rebuilt, .. } => {
                            let continues =
                                previous.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
                            let changes =
                                FrameChanges::rebuilt(frame, replay.format, continues, |table| {
                                    rebuilt.contains(&table)
                                });
                            observed_hours.push(frame.hour);
                            previous = Some(frame.hour);
                            extract.observe(&replay, frame, &changes)?;
                        }
                        BlockChange::Quiet(hours) => {
                            previous = Some(hours.last());
                            extract.quiet(&replay, hours, 0..hours.hours().len())?;
                        }
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        dashboard_keyed::block::fold(
            &mut scan,
            extract.finish().unwrap(),
            &replay,
            index,
            &mut |_, result| result.map(|()| true),
            &mut || Ok(()),
        )
        .unwrap();
    }
    assert!(
        observed_hours.contains(&105),
        "an equal fresh read must not be a quiet carry"
    );
    let fixture = Fixture::new();
    save(&mut scan, &fixture);
    let window = window(&fixture, 102, 106);
    assert_eq!(known(&window), [(102, 102), (105, 106)]);
    assert_eq!(window.reads, [(102, 102, 102), (105, 106, 105)]);
    assert_eq!(
        recorded(&window, "SYN@7|3|c0"),
        [(102, 102, "-17".into(), true)]
    );
    assert_eq!(
        recorded(&window, "SYN@9|3|c0"),
        [(105, 106, "-17".into(), true)]
    );
}

#[test]
fn identity_change_during_unavailable_statistics_does_not_outlast_the_change() {
    let first = read(102, 7, b"SYN");
    let mut unavailable = carried(&first, 103);
    identity(&mut unavailable, 7, b"ALT");
    *unavailable.tables[TableId::AirStatistics.index()] = TableView::Unavailable {
        reason: "Synthetic unavailable carried statistics".into(),
    };
    let frames = [
        first.clone(),
        unavailable,
        carried(&first, 104),
        read(105, 7, b"SYN"),
    ];
    let fixture = Fixture::new();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        observe(&mut scan, &mut tracker, frame);
    }
    save(&mut scan, &fixture);
    let window = window(&fixture, 102, 105);
    // Hour 103 is unknown (statistics unavailable, identity changed); at 104 the
    // carried read and its identity agree again.
    assert_eq!(known(&window), [(102, 102), (104, 105)]);
    assert_eq!(
        recorded(&window, "SYN@7|3|c0"),
        [
            (102, 102, "-17".into(), true),
            (104, 105, "-17".into(), true),
        ]
    );
    assert!(
        !window
            .series
            .iter()
            .any(|series| series.name.starts_with("ALT@"))
    );
}
