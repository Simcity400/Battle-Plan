//! The families observing every block hour by hour on their own thread
//! (occupation and the naval family) read
//! each block once, together, and each reconstructs its own tables from that
//! read: each publishes, saves and refuses exactly what reading each block
//! alone made it publish, save and refuse (the reference, with the shared
//! reader off), prepared at once or one after another, with one and several
//! pre-pass workers, across commits, resume from different blocks, reload,
//! append, a gap and a second branch, and with no read-ahead budget; every
//! block is read once for all of them, each family observing every hour once.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use independent::{evaluated, shared, windowed};
use replay_data::{
    rows::LexicalTokenRow, rows::ProjectProgramRow, rows::ProjectRow, rows::ProjectState,
    rows::StateGarrisonRow, rows::StateResistanceRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour: branch 1 starts here, branch 2 two hundred hours
/// later.
const BASE: i32 = 1000 * 24;

/// The families reading through the shared reader, each with the count of
/// the hours its own thread observes.
const FAMILIES: [(&str, &str); 2] = [
    ("occupation", evaluated::OCCUPATION_OBSERVED),
    ("naval", windowed::NAVAL_OBSERVED),
];

/// The offset of `hour` from its branch's start.
fn offset(branch: u64, hour: i32) -> i32 {
    hour - BASE - if branch == 2 { 200 } else { 0 }
}

fn set(observed: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *observed.tables[index] = TableView::Complete(rows);
}

// Three countries (`frame`) whose balances change every hour. The first runs project
// 1 from the start, prototype progress moving every hour, completing it at
// +30; project 2 starts at +12, and the program's scientist is named at +44.
// The project table is not read at +20 to +22. The dictionary gains a token
// at +36, inside a block, so its view is a new allocation there. The second
// country occupies the first's state from +5 until +40, its resistance and
// garrison read every six hours.
fn shared_frame(branch: u64, hour: i32) -> DecodedFrame {
    let at = offset(branch, hour);
    let mut observed = frame(branch, hour, 3);
    if !(20..=22).contains(&at) {
        observed.metrics.table_ns[TableId::Projects.index()] = 1;
    }
    let running = |local_id: u32, definition_id: u32, progress: i32| ProjectRow {
        owner: 100_000,
        local_id,
        definition_id,
        program_reference: 5,
        started: 1,
        state: ProjectState {
            current_offset: 0x18,
            not_completed: 1,
            prototype_progress: progress,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut projects = vec![if at >= 30 {
        ProjectRow {
            state: ProjectState {
                current_offset: 0xe8,
                not_completed: 0,
                ..Default::default()
            },
            ..running(1, 9_001, 0)
        }
    } else {
        running(1, 9_001, at)
    }];
    if at >= 12 {
        projects.push(running(2, 9_002, at - 12));
    }
    set(&mut observed, TableRows::Projects(projects));
    set(&mut observed, TableRows::empty(TableId::ProjectHistory));
    set(
        &mut observed,
        TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 5,
            owner: 100_000,
            scientist: if at >= 44 { 7 } else { 0 },
            ..Default::default()
        }]),
    );
    let mut tokens = match &*observed.tables[TableId::LexicalTokens.index()] {
        TableView::Complete(TableRows::LexicalTokens(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    tokens.push(LexicalTokenRow {
        token: 9_001,
        literal: b"mod_project".to_vec(),
    });
    tokens.push(LexicalTokenRow {
        token: 9_002,
        literal: b"mod_second_project".to_vec(),
    });
    if at >= 36 {
        tokens.push(LexicalTokenRow {
            token: 9_003,
            literal: b"mod_later_token".to_vec(),
        });
    }
    tokens.sort_by_key(|row| row.token);
    set(&mut observed, TableRows::LexicalTokens(tokens));
    let occupied = |at: i32| (5..40).contains(&at);
    if occupied(at)
        && let TableView::Complete(TableRows::States(rows)) =
            &mut *observed.tables[TableId::States.index()]
    {
        rows[0].controller = 100_001;
    }
    let read = at - at.rem_euclid(6);
    let held = occupied(read);
    set(
        &mut observed,
        TableRows::StateResistance(if held {
            vec![StateResistanceRow {
                state: 1,
                country: 100_000,
                active: 1,
                level: i64::from(read) * 100,
                compliance: i64::from(read) * 10,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        &mut observed,
        TableRows::StateGarrisons(if held {
            vec![StateGarrisonRow {
                controller: 100_001,
                occupied: 100_000,
                state: 1,
                garrison_required: i64::from(read) * 3,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    if at == read {
        observed.metrics.table_ns[TableId::StateResistance.index()] = 1;
    }
    observed
}

/// Branch 1's first `blocks` blocks of eight hours (+24 and +25 are a gap
/// between the third and fourth), then, when `second`, branch 2's two
/// blocks.
fn scenario(blocks: usize, second: bool) -> Replay {
    let spans: Vec<Vec<i32>> = [0, 8, 16, 26, 34, 42, 50, 58]
        .into_iter()
        .map(|first| (first..first + 8).collect())
        .collect();
    let frames = |branch: u64, start: i32, offsets: &[i32]| -> Vec<DecodedFrame> {
        offsets
            .iter()
            .map(|&at| shared_frame(branch, start + at))
            .collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(1, BASE, &spans[0]))]).unwrap();
    for offsets in &spans[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(1, BASE, offsets))])
            .unwrap();
    }
    if second {
        let start = BASE + 200;
        replay = replay
            .append(&[records(2, frames(2, start, &spans[0]))])
            .unwrap();
        replay = replay
            .append(&[continuation_records(
                &replay,
                2,
                frames(2, start, &spans[1]),
            )])
            .unwrap();
    }
    replay
}

/// Every block of branch 1 and both of branch 2.
fn full() -> Replay {
    scenario(8, true)
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Hours recorded in `replay`.
fn hours(replay: &Replay) -> usize {
    let mut hours = 0;
    for index in &replay.index {
        replay
            .read_block_alone(
                index,
                &[TableId::Campaign],
                |_, _| {
                    hours += 1;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    hours
}

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
}

/// Every block committed on its own.
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

/// The three families prepared on their own; every other family in the walk.
fn families<T>(run: impl FnOnce() -> T) -> T {
    independent::with_test_only(&FAMILIES.map(|(name, _)| name), run)
}

/// The reference: each of the families reading each block alone.
fn alone<T>(run: impl FnOnce() -> T) -> T {
    shared::with_test_unshared(run)
}

/// What `prepare` publishes and saves with the three families prepared on
/// their own.
fn prepared(fixture: &mut Fixture, revision: &str, prepare: impl Fn(&Source)) -> Payloads {
    let source = fixture.source(revision);
    families(|| prepare(&source));
    let payloads = payloads(&source, None);
    clear(source);
    payloads
}

#[test]
fn families_reading_together_publish_and_save_what_each_reading_alone_does() {
    let replay = full();
    let mut fixture = Fixture::new(&replay);
    let revision = "shared-parity";
    let reference =
        alone(|| prepass::with_test_prepass(2, None, || prepared(&mut fixture, revision, plain)));
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        for one_after_another in [false, true] {
            let label = format!(
                "{workers} workers, {parallelism} processors, {}",
                if one_after_another {
                    "one after another"
                } else {
                    "at once"
                }
            );
            let together = prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    if one_after_another {
                        independent::with_test_one_after_another(|| {
                            prepared(&mut fixture, revision, plain)
                        })
                    } else {
                        prepared(&mut fixture, revision, plain)
                    }
                })
            });
            assert_payloads_equal(&together, &reference, &label);
            for (family, _) in FAMILIES {
                for branch in [1, 2] {
                    assert!(
                        together.contains_key(&(independent::coverage_section(family, branch), 0)),
                        "{label}: {family} covers branch {branch}"
                    );
                }
            }
        }
    }
}

#[test]
fn families_reading_together_commit_resume_reload_and_append_as_each_reading_alone() {
    let first = scenario(3, false);
    let grown = full();
    // The hourly walk prepares one block per run; the families, every block
    // of the recording at the run's start, resuming after the append.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    let modes: [PrepareReloadMode; 4] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("shared-commits");
            families(|| prepare(&source));
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("shared-commits");
            families(|| prepare(&source));
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let reference = alone(|| prepass::with_test_prepass(2, None, || both(&mut fixture)));
        for workers in [1, 3] {
            let together = prepass::with_test_prepass(workers, None, || both(&mut fixture));
            assert_payloads_equal(&together, &reference, &format!("{mode}, {workers} workers"));
        }
    }
}

/// `payloads` without the checkpoint, whose accounting follows the runs that
/// wrote it.
fn published(payloads: Payloads) -> Payloads {
    payloads
        .into_iter()
        .filter(|((name, _), _)| name != "preparation-checkpoint-v1")
        .collect()
}

/// Cancelled after a number of commits, each family stops at a block of its
/// own; resumed, they read from blocks apart (apart or together again) and
/// publish what an uninterrupted run reading each block alone publishes.
/// Every block commits on its own, so where the cancel landed changes no
/// published range.
#[test]
fn families_resumed_from_different_blocks_publish_what_each_reading_alone_does() {
    let replay = full();
    let mut fixture = Fixture::new(&replay);
    let revision = "shared-resumed";
    let reference = published(alone(|| {
        prepass::with_test_prepass(2, None, || prepared(&mut fixture, revision, measured))
    }));
    for stop_after in [1, 5, 13, 23] {
        let interrupted = |source: &Source| {
            let flag = AtomicBool::new(false);
            let mut commits = 0;
            let _ = run_measured_through(
                source,
                None,
                &flag,
                true,
                Duration::ZERO,
                Some(u64::MAX),
                |_| Ok(()),
                |_| {
                    commits += 1;
                    if commits == stop_after {
                        flag.store(true, Ordering::Release);
                    }
                },
            );
            measured(source);
        };
        for workers in [1, 3] {
            let together = published(prepass::with_test_prepass(workers, None, || {
                prepared(&mut fixture, revision, interrupted)
            }));
            assert_payloads_equal(
                &together,
                &reference,
                &format!("cancelled after {stop_after} commits, {workers} workers"),
            );
        }
    }
}

#[test]
fn families_reading_together_refuse_where_each_reading_alone_refuses() {
    let replay = full();
    let mut fixture = Fixture::new(&replay);
    let revision = "shared-refusal";
    for (family, _) in FAMILIES {
        // At a block's first hour, inside a block, and after the gap.
        for at in [8, 21, 27] {
            let refusals = [(family, BASE + at)];
            let label = format!("{family} refused at +{at}");
            let reference = alone(|| {
                lanes::with_test_refusals(&refusals, || {
                    prepass::with_test_prepass(2, None, || {
                        prepared(&mut fixture, revision, measured)
                    })
                })
            });
            for workers in [1, 3] {
                let together = lanes::with_test_refusals(&refusals, || {
                    prepass::with_test_prepass(workers, None, || {
                        prepared(&mut fixture, revision, measured)
                    })
                });
                let label = format!("{label}, {workers} workers");
                assert_payloads_equal(&together, &reference, &label);
                let page: CoveragePage = serde_json::from_slice(
                    &together[&(independent::coverage_section(family, 1), 0)],
                )
                .unwrap();
                assert_eq!(page.refused[family].hour, BASE + at, "{label}");
            }
        }
    }
}

/// Prepared together, every block is read once for all three families, by a
/// dedicated reader or a pre-pass task depositing its read. None reads a block
/// alone, and each observes every recorded hour once; reading alone, as before,
/// no block is read for them together.
#[test]
fn each_block_is_read_once_for_every_family_observing_it_hour_by_hour() {
    let replay = full();
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let revision = "shared-once";
    for workers in [1, 3] {
        let label = format!("{workers} workers");
        let source = fixture.source(revision);
        let observed: Vec<usize> = FAMILIES
            .iter()
            .map(|(_, observed)| prepass::noted(observed))
            .collect();
        let reads = prepass::noted(shared::SHARED_READ);
        let deposited = prepass::noted(shared::SHARED_DEPOSITED);
        let apart = prepass::noted(shared::SHARED_ALONE);
        prepass::with_test_prepass(workers, None, || {
            prepass::with_test_sequential(|| families(|| plain(&source)))
        });
        let own = payloads(&source, None);
        for (family, _) in FAMILIES {
            for branch in [1, 2] {
                let page: CoveragePage = serde_json::from_slice(
                    &own[&(independent::coverage_section(family, branch), 0)],
                )
                .unwrap();
                assert!(
                    page.refused.is_empty(),
                    "{label}: {family} refused branch {branch}: {:?}",
                    page.refused
                );
                assert!(page.coverage.complete, "{label}: {family} branch {branch}");
            }
        }
        assert_eq!(
            prepass::noted(shared::SHARED_READ) - reads + prepass::noted(shared::SHARED_DEPOSITED)
                - deposited,
            replay.index.len(),
            "{label}: every block read once for all three families, by a reader or deposited task"
        );
        assert_eq!(
            prepass::noted(shared::SHARED_ALONE) - apart,
            0,
            "{label}: no family read a block alone"
        );
        for ((family, counted), before) in FAMILIES.iter().zip(observed) {
            assert_eq!(
                prepass::noted(counted) - before,
                total,
                "{label}: {family} observed every recorded hour once"
            );
        }
        clear(source);
    }
    let source = fixture.source(revision);
    let reads = prepass::noted(shared::SHARED_READ);
    let deposits = prepass::noted(shared::SHARED_DEPOSITED);
    alone(|| prepass::with_test_prepass(2, None, || families(|| plain(&source))));
    assert_eq!(
        (prepass::noted(shared::SHARED_READ) - reads)
            + (prepass::noted(shared::SHARED_DEPOSITED) - deposits),
        0,
        "read alone, no block is read or deposited for them together"
    );
    clear(source);
}

/// With no read-ahead budget only each family's next block is read ahead,
/// and the families publish the same. A measured run times dedicated shared
/// reads apart from task deposits (together one per block), and each evaluated
/// family's own read row still counts one read per block.
#[test]
fn shared_reads_hold_to_their_budget_and_are_timed_apart() {
    let replay = full();
    let mut fixture = Fixture::new(&replay);
    let revision = "shared-budget";
    let reference =
        alone(|| prepass::with_test_prepass(2, None, || prepared(&mut fixture, revision, plain)));
    let together = shared::with_test_budget(0, || {
        prepass::with_test_prepass(3, None, || prepared(&mut fixture, revision, plain))
    });
    assert_payloads_equal(&together, &reference, "no read-ahead budget");
    let source = fixture.source("shared-timed");
    let (_, timings) = prepass::with_test_prepass(2, None, || {
        prepass::with_test_sequential(|| {
            families(|| {
                run_measured(
                    &source,
                    None,
                    &AtomicBool::new(false),
                    false,
                    Duration::ZERO,
                    |_| {},
                )
            })
        })
    })
    .unwrap();
    let rows = timings.families.expect("a measured run's timing rows");
    let blocks = replay.index.len() as u64;
    assert_eq!(
        rows[shared::SHARED_READ].calls + rows[shared::SHARED_DEPOSITED].calls,
        blocks,
        "shared reads, including those deposited by the pre-pass"
    );
    assert_eq!(rows["occupation.read"].calls, blocks, "occupation.read");
    clear(source);
}
