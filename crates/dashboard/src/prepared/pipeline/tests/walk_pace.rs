use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::pace::{self, Lifecycle, WalkPace};

/// The walk's obligation this thread's last preparation started.
fn last() -> Arc<WalkPace> {
    pace::started().expect("a preparation started the walk's obligation")
}

#[test]
fn a_finished_walk_ends_its_obligation_and_a_run_with_nothing_left_never_runs() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run_checked(&source, None, &flag, |_| {}, |_| Ok(())).unwrap();
    let finished = last();
    assert_eq!(
        finished.history(),
        [Lifecycle::Pending, Lifecycle::Running, Lifecycle::Terminal]
    );
    run_checked(&source, None, &flag, |_| {}, |_| Ok(())).unwrap();
    let next = last();
    if !Arc::ptr_eq(&next, &finished) {
        assert_eq!(next.history(), [Lifecycle::Terminal]);
    }
}

#[test]
fn a_failing_cancelled_or_unwinding_walk_ends_its_obligation() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    // An error the walk returns before its first commit.
    let error = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |_| Err(io::Error::other("injected walk error")),
    )
    .unwrap_err();
    assert!(error.to_string().contains("injected walk error"));
    assert_eq!(
        last().history(),
        [Lifecycle::Pending, Lifecycle::Running, Lifecycle::Terminal]
    );
    // Cancellation the walk sees before its first commit.
    let cancelled = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |_| {
            flag.store(true, Ordering::Release);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
    assert_eq!(last().snapshot().lifecycle, Lifecycle::Terminal);
    flag.store(false, Ordering::Release);
    // A panic unwinding the walk.
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |_| panic!("injected walk panic"),
        )
    }));
    assert!(unwound.is_err(), "the walk's panic propagates");
    assert_eq!(
        last().history(),
        [Lifecycle::Pending, Lifecycle::Running, Lifecycle::Terminal]
    );
}

fn prepared(
    fixture: &mut Fixture,
    mode: impl FnOnce(&dyn Fn()),
) -> BTreeMap<(String, u64), Vec<u8>> {
    let source = fixture.source("paced");
    mode(&|| {
        run_checked(&source, None, &AtomicBool::new(false), |_| {}, |_| Ok(())).unwrap();
    });
    let payloads = payloads(&source, None);
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
    payloads
}

fn within(reads: &[independent::TestRead]) {
    assert!(!reads.is_empty(), "families read blocks");
    for (block, snapshot, _) in reads {
        assert!(
            snapshot.window().is_none_or(|(_, last)| *block <= last),
            "block {block} beyond {snapshot:?}"
        );
    }
}

fn pages_recording() -> Replay {
    let hours = |branch, span: std::ops::RangeInclusive<i32>| {
        span.map(|hour| frame(branch, hour, 2)).collect()
    };
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, hours(1, 1..=50))]).unwrap();
    let replay = first
        .append(&[continuation_records(&first, 1, hours(1, 51..=130))])
        .unwrap();
    replay.append(&[records(2, hours(2, 25..=95))]).unwrap()
}

fn measured(source: &Source) {
    run_measured_through(
        source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |_| {},
    )
    .unwrap();
}

#[test]
fn paced_families_publish_the_serial_unpaced_payloads_in_every_mode() {
    for replay in [
        pages_recording(),
        super::independent_groups::scenario(6, true),
    ] {
        let mut fixture = Fixture::new(&replay);
        let expected = prepared(&mut fixture, |run| {
            independent::with_test_one_after_another(|| {
                prepass::with_test_sequential(|| independent::shared::with_test_unshared(run))
            })
        });
        assert!(!independent::take_test_reads().unpaced.is_empty());
        assert!(
            expected
                .keys()
                .any(|(section, _)| section == "map-export-v2/endpoints/1")
        );
        let actual = prepared(&mut fixture, |run| run());
        assert_payloads_equal(&actual, &expected, "paced vs serial observation");
        let reads = independent::take_test_reads();
        assert!(reads.unpaced.is_empty());
        within(&reads.paced);
        let together = prepared(&mut fixture, |run| {
            independent::with_test_grouping(independent::Grouping::Together, run)
        });
        assert_payloads_equal(&together, &expected, "parked families in waiting groups");
        within(&independent::take_test_reads().paced);
        let serial = prepared(&mut fixture, |run| {
            independent::with_test_one_after_another(run)
        });
        assert_payloads_equal(&serial, &expected, "serial pooled extracts");
        let reads = independent::take_test_reads();
        assert!(
            reads
                .paced
                .iter()
                .all(|(_, snapshot, _)| snapshot.lifecycle == Lifecycle::Terminal)
        );

        let source = fixture.source("measured");
        independent::with_test_one_after_another(|| {
            prepass::with_test_sequential(|| {
                independent::shared::with_test_unshared(|| measured(&source))
            })
        });
        let expected_measured = payloads(&source, None);
        super::super::super::files(&source, None)
            .unwrap()
            .0
            .clear()
            .unwrap();
        measured(&source);
        assert_payloads_equal(
            &payloads(&source, None),
            &expected_measured,
            "measured serial reference",
        );
        within(&independent::take_test_reads().paced);
    }
}

#[test]
fn reads_and_claims_stay_in_the_window_with_an_unpaced_negative_control() {
    let replay = super::independent_groups::scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("window");
    let mut held = false;
    super::super::with_test_batch_blocks(1, || {
        prepass::with_test_prepass(1, None, || {
            run_checked(
                &source,
                None,
                &AtomicBool::new(false),
                |_| {},
                |_| {
                    if !held {
                        held = true;
                        independent::wait_for_test_reads(|reads| {
                            reads.waits.iter().any(|(block, snapshot, _)| {
                                snapshot.window().is_some_and(|(_, last)| *block > last)
                            })
                        });
                    }
                    Ok(())
                },
            )
        })
    })
    .unwrap();
    let reads = independent::take_test_reads();
    assert!(reads.unpaced.is_empty());
    within(&reads.paced);
    let claims = prepass::take_family_claims();
    assert!(!claims.is_empty());
    for (block, snapshot) in claims {
        assert!(snapshot.window().is_none_or(|(_, last)| block <= last));
    }
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
    independent::with_test_one_after_another(|| {
        run_checked(&source, None, &AtomicBool::new(false), |_| {}, |_| Ok(())).unwrap();
    });
    let reads = independent::take_test_reads();
    assert!(
        reads.unpaced.iter().any(|(block, snapshot, _)| {
            snapshot.lifecycle == Lifecycle::Pending
                && snapshot.window().is_some_and(|(_, last)| *block > last)
        }),
        "serial families really read ahead of the pending walk"
    );
}

#[test]
fn a_failed_cancelled_or_unwinding_walk_releases_waiting_families() {
    let replay = super::independent_groups::scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("failed-walk");
    for end in ["error", "cancel", "panic"] {
        let flag = AtomicBool::new(false);
        let mut held = false;
        let ran = super::super::with_test_batch_blocks(1, || {
            prepass::with_test_prepass(1, None, || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_checked(
                        &source,
                        None,
                        &flag,
                        |_| {},
                        |_| {
                            if !held {
                                held = true;
                                independent::wait_for_test_reads(|reads| !reads.waits.is_empty());
                            }
                            match end {
                                "error" => Err(io::Error::other("injected walk error")),
                                "cancel" => {
                                    flag.store(true, Ordering::Release);
                                    Ok(())
                                }
                                _ => panic!("injected walk panic"),
                            }
                        },
                    )
                }))
            })
        });
        match end {
            "error" => assert!(
                ran.unwrap()
                    .unwrap_err()
                    .to_string()
                    .contains("injected walk error")
            ),
            "cancel" => assert_eq!(ran.unwrap().unwrap_err().kind(), io::ErrorKind::Interrupted),
            _ => assert!(ran.is_err()),
        }
        assert_eq!(last().snapshot().lifecycle, Lifecycle::Terminal);
        let reads = independent::take_test_reads();
        assert!(!reads.waits.is_empty());
        within(&reads.paced);
        super::super::super::files(&source, None)
            .unwrap()
            .0
            .clear()
            .unwrap();
    }
}

#[test]
fn families_resumed_ahead_of_the_walk_wait_then_publish_the_complete_payloads() {
    let replay = super::independent_groups::scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let expected = prepared(&mut fixture, |run| run());
    let source = fixture.source("resumed-ahead");
    let flag = AtomicBool::new(false);
    let result = independent::with_test_one_after_another(|| {
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |_| {
                flag.store(true, Ordering::Release);
                Ok(())
            },
        )
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    assert_eq!(saved.cursor.next_block, 0);
    assert!(
        saved
            .independent
            .values()
            .any(|family| family.next_block > 0)
    );
    flag.store(false, Ordering::Release);
    run_checked(&source, None, &flag, |_| {}, |_| Ok(())).unwrap();
    assert_payloads_equal(&payloads(&source, None), &expected, "resumed ahead");
    assert_eq!(last().snapshot().lifecycle, Lifecycle::Terminal);
}

#[test]
fn refused_pool_workers_fall_back_without_unpaced_family_reads() {
    let replay = super::independent_groups::scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let expected = prepared(&mut fixture, |run| {
        independent::with_test_one_after_another(|| prepass::with_test_sequential(run))
    });
    for (workers, refused) in [(1, 1), (2, 1)] {
        let actual = prepared(&mut fixture, |run| {
            prepass::with_test_prepass(workers, None, || {
                prepass::with_test_refused_workers(refused, run)
            })
        });
        assert_payloads_equal(&actual, &expected, "worker start refusal");
        let reads = independent::take_test_reads();
        assert!(reads.unpaced.is_empty());
        within(&reads.paced);
    }
}

#[test]
fn a_five_thousand_country_recording_has_the_serial_payloads_and_named_readers() {
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(1, 100, 5_000)])]).unwrap();
    let replay = first
        .append(&[continuation_records(&first, 1, vec![frame(1, 101, 5_000)])])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let expected = prepared(&mut fixture, |run| {
        independent::with_test_one_after_another(|| prepass::with_test_sequential(run))
    });
    let source = fixture.source("five-thousand");
    run_checked(
        &source,
        None,
        &AtomicBool::new(false),
        |_| {},
        |_| {
            let census = source
                .replay_ref()
                .decoded_section_census(last().snapshot().window());
            assert!(
                census
                    .retaining
                    .iter()
                    .flatten()
                    .all(|name| *name != "reader")
            );
            Ok(())
        },
    )
    .unwrap();
    assert_payloads_equal(&payloads(&source, None), &expected, "5,000 countries");
    within(&independent::take_test_reads().paced);
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    assert_eq!(saved.cursor.next_block, 2);
    assert!(
        saved
            .independent
            .values()
            .all(|family| family.next_block == 2)
    );
}

const PENDING_UNDER_COUNTED: &[&str] = &[
    // AirMissionsObserver omits hourly refusal keys from Scan::buffered_bytes.
    "air-missions",
    // AirStatisticsObserver omits hourly refusal keys from Scan::buffered_bytes.
    "air-statistics",
    // Flush seals the uncharged hours run and publishes private group diagnostics.
    "air_superiority",
    // The catalogue advances scan.last without project records.
    "construction",
    // Flush seals unchanged identity and unresolved runs outside the batch charge.
    "diplomacy_state",
    // DoctrineMasteryObserver refusal counters are outside Scan::buffered_bytes.
    "doctrine-mastery",
    // Flush seals carried read-version runs outside pending loss rows.
    "equipment_losses",
    // FeedBranch::pending omits the private HistoryScan its flush writes.
    "event_feed",
    // Flush seals the uncharged roster and unavailable-channel runs.
    "faction_state",
    // Availability and settlement catalogue frontiers advance without fuel rows.
    "fuel",
    // GroupsBranch::pending omits Changes::flush_as diagnostic counters.
    "groups",
    // IdentitiesBranch::pending is zero while flush writes the sealed HistoryScan.
    "identities",
    // Refused summaries advance the uncharged Scan catalogue frontier.
    "intel-network-coverage",
    // Flush emits closed observation spans outside pending leader runs.
    "leaders",
    // MinefieldsObserver refusal counters are outside Scan::buffered_bytes.
    "minefields",
    // NavalSupremacyObserver refusal counters are outside Scan::buffered_bytes.
    "naval-supremacy",
    // Quiet unknown-force mixes and unread diagnostics escape the batch charge.
    "navy",
    // Refused manpower reads advance the uncharged Scan catalogue frontier.
    "occupation-reserve",
    // Flush seals the uncharged unavailable run and publishes the catalogue frontier.
    "parties",
    // Flush replaces uncharged snapshots and missing-input/gap continuation records.
    "peace",
    // Flush seals constant accepted/census runs outside the batch charge.
    "politics_ledger",
    // Flush publishes an open endpoint and its TimeEntry outside the spent/held charge.
    "production",
    // Unavailable weather advances the uncharged Scan catalogue frontier.
    "regional-weather",
    // A rename adds catalogue keys and parts without numeric repair rates.
    "repairs",
    // Choices::advance moves catalogue bounds without value or choice rows.
    "stockpile",
    // The catalogue counts incomplete hours outside pending subject events.
    "subjects",
    // The whole-scan catalogue advances without available tension values.
    "tension",
    // Territory::bytes omits the private HistoryScan its flush writes.
    "territory",
    // The observer snapshot and walk tail are outside pending change inputs.
    "wars",
];
const PUBLISHED_PER_HELD: i128 = 8;

fn under_counted(audit: &pace::PendingAudit) -> bool {
    audit.record_bytes() > i128::from(audit.held) * PUBLISHED_PER_HELD
}

#[test]
fn framing_does_not_hide_an_unaccounted_record() {
    use dashboard_base::prepared::series::tests::Fixture as SeriesFixture;
    use dashboard_base::prepared::{pages::framing, series::Accumulator, series::Run};

    let fixture = SeriesFixture::new();
    let mut writer = fixture.writer();
    let mut series = Accumulator::default();
    let cancelled = AtomicBool::new(false);
    for (hour, record) in [(100, false), (101, true)] {
        series.advance(hour).unwrap();
        series.observe(0, hour, None, false);
        if record {
            series.push(0, Run::new(hour, hour, 42, false));
            series.event(0, hour, &"unaccounted record").unwrap();
        }
        let before = (writer.payload_bytes_change(), framing::bytes());
        series.flush(&mut writer, "control", 0, &cancelled).unwrap();
        let audit = pace::PendingAudit {
            family: "control",
            branch: 1,
            held: 0,
            published: writer.payload_bytes_change() - before.0,
            framing: framing::bytes() - before.1,
        };
        assert_eq!(under_counted(&audit), record, "{audit:?}");
        if !record {
            assert_eq!(audit.record_bytes(), 0, "unknown keys contain no records");
        }
    }
}

#[test]
fn unavailable_fuel_consumers_count_the_catalogue_their_flush_publishes() {
    use dashboard_base::prepared::series::tests::Fixture as SeriesFixture;
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, pages::framing};
    use dashboard_economy::products::fuel_branches;

    let fixture = SeriesFixture::new();
    let mut prepared = fuel_branches::Prepared::default();
    let mut writer = fixture.writer();
    for (hours, expected) in [(1..=50, 12_438), (51..=130, 19_838)] {
        let derived = 256
            + hours.clone().map(|hour| {
                let reason = format!("fuel consumers unavailable: hour {hour} CountryFuelConsumers is unavailable: outside country fixture");
                2 * reason.len() + 20 + 32
            }).sum::<usize>();
        assert_eq!(
            derived, expected,
            "fixed refusal text lengths plus catalogue/edit margins"
        );
        let mut changes = FrameChangeTracker::new();
        for hour in hours {
            let frame = frame(1, hour, 2);
            let changes = changes.observe(&frame, replay_data::format::FormatVersion::CURRENT);
            prepared.observe_changes(&frame, &changes).unwrap();
        }
        let before = (writer.payload_bytes_change(), framing::bytes());
        let held = prepared.bytes() as u64;
        assert_eq!(
            held, expected as u64,
            "the catalogue margin and each changed refusal"
        );
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        let audit = pace::PendingAudit {
            family: "fuel_branches",
            branch: 1,
            held,
            published: writer.payload_bytes_change() - before.0,
            framing: framing::bytes() - before.1,
        };
        assert!(
            !under_counted(&audit),
            "pending covers the refusal catalogue: {audit:?}"
        );
        assert_eq!(
            prepared.bytes(),
            0,
            "publication releases the catalogue change charge"
        );
    }
}

#[test]
fn unavailable_nukes_exclude_empty_pages_but_not_refusal_records() {
    use crate::products::country_nukes::prepared::{IncreaseCatalogue, Prepared};
    use dashboard_base::prepared::series::tests::Fixture as SeriesFixture;
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, pages::framing, series};

    let replay = pages_recording();
    let fixture = SeriesFixture::new();
    let mut prepared = Prepared::default();
    for (batch, hours) in [1..=50, 51..=130].into_iter().enumerate() {
        let last = *hours.end();
        let mut changes = FrameChangeTracker::new();
        for hour in hours {
            let frame = frame(1, hour, 2);
            let changes = changes.observe(&frame, replay.format);
            prepared.observe(&frame, &replay, &changes).unwrap();
        }
        let pending = prepared.bytes();
        assert!(
            pending > 0,
            "unavailable stockpiles still publish refusal catalogues"
        );
        let mut writer = fixture.writer();
        let before = (writer.bytes_written(), framing::bytes());
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        let empty_pages: u64 = ["ranks", "values", "parts", "mixes", "increases"]
            .into_iter()
            .map(|kind| {
                writer
                    .read_at(&format!("nukes-{kind}-1"), batch as u64)
                    .unwrap()
                    .unwrap()
                    .len()
            })
            .sum();
        let written = writer.bytes_written() - before.0;
        let framing = framing::bytes() - before.1;
        assert!(framing >= empty_pages, "all five empty pages are framing");
        // Gross record bytes prove classification, not whether net growth crosses the oracle.
        assert!(
            written > framing,
            "the catalogues still contain record data"
        );
        assert!(
            written - framing <= 8 * pending as u64,
            "pending covers both catalogues"
        );
        assert_eq!(prepared.bytes(), 0, "no unpublished catalogue remains");
        writer.commit().unwrap();
        let (_, catalogue) = series::read_catalogue::<IncreaseCatalogue>(
            &fixture.store,
            &fixture.key,
            "nukes-increases",
            1,
        )
        .unwrap()
        .unwrap();
        assert!(catalogue.read_spans.is_empty());
        assert_eq!(catalogue.refusals.len(), 1);
        let spans = catalogue.refusals.values().next().unwrap();
        assert_eq!(spans.len(), 1, "contiguous refusals extend one span");
        assert_eq!((spans[0].first_hour, spans[0].last_hour), (1, last));
    }
}

#[test]
fn every_family_counts_as_pending_what_its_commits_publish() {
    let mut under: BTreeMap<&str, Vec<(u64, u64, i128, u64, i128)>> = BTreeMap::new();
    for replay in [
        pages_recording(),
        super::independent_groups::scenario(6, true),
    ] {
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("pending");
        measured(&source);
        let mut created = std::collections::BTreeSet::new();
        let audit = independent::take_pending_audit();
        let families: std::collections::BTreeSet<_> = audit.iter().map(|row| row.family).collect();
        assert_eq!(
            families.into_iter().collect::<Vec<_>>(),
            independent::names()
        );
        for row in audit {
            if !created.insert((row.family, row.branch)) && under_counted(&row) {
                under.entry(row.family).or_default().push((
                    row.branch,
                    row.held,
                    row.published,
                    row.framing,
                    row.record_bytes(),
                ));
            }
        }
    }
    assert_eq!(
        under.keys().copied().collect::<Vec<_>>(),
        PENDING_UNDER_COUNTED,
        "(branch, held, published, framing, records) for each under-counted commit: {under:?}"
    );
}
