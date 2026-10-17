//! Research, special projects and occupation prepared on their own, before
//! the hourly walk, each observing its blocks hour by hour on its own thread:
//! each publishes and saves exactly what hourly preparation does across
//! commits, resume, reload, append, a gap and a second branch, with one and
//! several pre-pass workers; refuses where hourly preparation refuses, at its
//! hour and with its text; answers its queries while the hourly walk has
//! accepted nothing; observes every recorded hour once; and binds its content
//! at every block as the walk does.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::research::test_fixtures::research_frame;
use crate::test_fixtures::fixture_research_inputs;
use independent::evaluated;
use replay::rows::{
    LexicalTokenRow, ProjectProgramRow, ProjectRow, ProjectState, StateGarrisonRow,
    StateResistanceRow,
};
use std::{collections::BTreeSet, sync::Mutex};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour: branch 1 starts here, branch 2 two hundred hours
/// later.
const BASE: i32 = 1000 * 24;

/// The offset of `hour` from its branch's start.
fn offset(branch: u64, hour: i32) -> i32 {
    hour - BASE - if branch == 2 { 200 } else { 0 }
}

/// One family under test: its name, the query its coverage and refusals
/// answer, the sections it publishes, its continuation in the hourly branch
/// state (when hourly preparation keeps it there) and its recorded hour.
struct Case {
    family: &'static str,
    query: &'static str,
    published: &'static [&'static str],
    /// The family's own continuation section prefix and the hourly branch
    /// state's field holding the same state.
    continuation: Option<(&'static str, &'static str)>,
    observed: &'static str,
    frame: fn(u64, i32) -> DecodedFrame,
    /// A request its gated query answers once the family covers the branch.
    request: fn(&str) -> (&'static str, serde_json::Value),
    /// Its measured run's timing rows and what each counts.
    parts: [(&'static str, Count); 2],
}

/// What a measured timing row counts.
#[derive(Clone, Copy, Debug)]
enum Count {
    Blocks,
    Hours,
    Nothing,
}

// Research: the holder GER (raw ids 11 and 9001, canonical slot 7) is read
// every four hours, each read carried into the hours after it, until +32;
// it is not read again until +62, so the read at +32 carries for a day and
// its carry ends at +56, inside a block. Its first slot researches tech 1
// until the read at +12, then tech 2 (completed at the read at +40); its
// second slot researches tech 3 from +12. Progress moves at every read. The
// holder's balances, which research does not read, change every hour.
fn research_at(branch: u64, hour: i32) -> DecodedFrame {
    let at = offset(branch, hour);
    let read_at = if (33..62).contains(&at) {
        32
    } else {
        at - at.rem_euclid(4)
    };
    let read = hour - at + read_at;
    let assignments: &[Option<u32>] = if read_at < 12 {
        &[Some(1), None]
    } else if read_at < 40 {
        &[Some(2), Some(3)]
    } else {
        &[None, Some(3)]
    };
    let completed: &[u32] = if read_at >= 40 { &[2] } else { &[] };
    let mut at_read = research_frame(branch, read, assignments, completed);
    fixture_research_inputs(&mut at_read);
    if let TableView::Complete(TableRows::ResearchInputs(rows)) =
        &mut *at_read.tables[TableId::ResearchInputs.index()]
    {
        for row in rows.iter_mut().filter(|row| row.technology_present != 0) {
            row.progress_raw = i64::from(read_at + 1) * 1_000;
        }
    }
    let mut carried = research_frame(branch, hour, assignments, completed);
    for table in [
        TableId::ResearchInputs,
        TableId::ResearchAppliedBonuses,
        TableId::SliceReads,
    ] {
        carried.tables[table.index()] = at_read.tables[table.index()].clone();
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *carried.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.political_power_raw = Some(i64::from(hour) * 7);
        }
    }
    carried
}

// Special projects: GER's program 5 runs project 1 (definition 8) from the
// start, with prototype progress moving every hour, and completes it at
// +30; project 2 (definition 9) starts at +12. The program's scientist is
// named at +44. The project table is not read at +20 to +22, so its read
// marker closes and reopens; the dictionary gains a token at +36, inside a
// block, so its view is a new allocation there. The holder's balances
// change every hour.
fn projects_at(branch: u64, hour: i32) -> DecodedFrame {
    let at = offset(branch, hour);
    let mut frame = crate::test_fixtures::source(branch, hour, Some(i64::from(hour) * 1000));
    if !(20..=22).contains(&at) {
        frame.metrics.table_ns[TableId::Projects.index()] = 1;
    }
    let running = |local_id: u32, definition_id: u32, progress: i32| ProjectRow {
        owner: 11,
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
            ..running(1, 8, 0)
        }
    } else {
        running(1, 8, at)
    }];
    if at >= 12 {
        projects.push(running(2, 9, at - 12));
    }
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(projects));
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::empty(TableId::ProjectHistory));
    *frame.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 5,
            owner: 11,
            scientist: if at >= 44 { 7 } else { 0 },
            ..Default::default()
        }]));
    let mut tokens = vec![
        LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        },
        LexicalTokenRow {
            token: 9,
            literal: b"mod_second_project".to_vec(),
        },
    ];
    if at >= 36 {
        tokens.push(LexicalTokenRow {
            token: 10,
            literal: b"mod_later_token".to_vec(),
        });
    }
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(tokens));
    frame
}

/// Occupation's daily tables are read every six hours (each read carried
/// until the next).
fn occupation_read(at: i32) -> i32 {
    at - at.rem_euclid(6)
}

// Occupation: three countries each own one state (`frame`), with balances
// changing every hour. The second country occupies the first's state from
// +5 until +40, read from the next read: its resistance and compliance move
// at each read, with a garrison row while occupied.
fn occupation_at(branch: u64, hour: i32) -> DecodedFrame {
    let at = offset(branch, hour);
    let read = occupation_read(at);
    let mut observed = frame(branch, hour, 3);
    let occupied = |at: i32| (5..40).contains(&at);
    if occupied(at)
        && let TableView::Complete(TableRows::States(rows)) =
            &mut *observed.tables[TableId::States.index()]
    {
        rows[0].controller = 100_001;
    }
    let held = occupied(read);
    *observed.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(if held {
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
        }));
    *observed.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(if held {
            vec![StateGarrisonRow {
                controller: 100_001,
                occupied: 100_000,
                state: 1,
                garrison_required: i64::from(read) * 3,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }));
    if at == read {
        observed.metrics.table_ns[TableId::StateResistance.index()] = 1;
    }
    observed
}

const RESEARCH: Case = Case {
    family: "research",
    query: "research",
    published: &["research-"],
    continuation: Some(("research-state-", "research")),
    observed: evaluated::RESEARCH_OBSERVED,
    frame: research_at,
    // The fold reads no block itself.
    parts: [
        ("research.fold", Count::Blocks),
        ("research.read", Count::Nothing),
    ],
    // Its slots' progress, read from its published series alone (the events
    // page also names technologies through the content).
    request: |id| {
        (
            "series-summary",
            serde_json::json!({
                "branchId": id,
                "tags": ["GER"],
                "from": BASE,
                "to": BASE + 300,
                "width": 10,
                "family": { "kind": "research" },
            }),
        )
    },
};

const PROJECTS: Case = Case {
    family: "projects",
    query: "special-projects",
    published: &["special-projects-"],
    continuation: Some(("special-projects-state-", "projects")),
    observed: evaluated::PROJECTS_OBSERVED,
    frame: projects_at,
    parts: [
        ("projects.read", Count::Blocks),
        ("projects.observe", Count::Hours),
    ],
    request: |id| {
        (
            "special-projects",
            serde_json::json!({ "branchId": id, "tag": "GER" }),
        )
    },
};

const OCCUPATION: Case = Case {
    family: "occupation",
    query: "occupied-states",
    published: &[
        "occupied-states-",
        "occupied-states-manifest-",
        "occupation-trend-values-",
        "occupation-",
    ],
    continuation: None,
    observed: evaluated::OCCUPATION_OBSERVED,
    frame: occupation_at,
    parts: [
        ("occupation.read", Count::Blocks),
        ("occupation.observe", Count::Hours),
    ],
    request: |id| {
        (
            "occupied-states-manifest",
            serde_json::json!({ "branchId": id }),
        )
    },
};

const CASES: [&Case; 3] = [&RESEARCH, &PROJECTS, &OCCUPATION];

/// Branch 1's first `blocks` blocks of eight hours (+24 and +25 are a gap
/// between the third and fourth), then, when `second`, branch 2's two
/// blocks.
fn scenario(case: &Case, blocks: usize, second: bool) -> Replay {
    let spans: Vec<Vec<i32>> = [0, 8, 16, 26, 34, 42, 50, 58]
        .into_iter()
        .map(|first| (first..first + 8).collect())
        .collect();
    let frames = |branch: u64, start: i32, offsets: &[i32]| -> Vec<DecodedFrame> {
        offsets
            .iter()
            .map(|&at| (case.frame)(branch, start + at))
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
fn full(case: &Case) -> Replay {
    scenario(case, 8, true)
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

/// Sections only one mode writes: the checkpoint, the family's own coverage
/// pages, and the branch state and continuation where the family keeps its
/// state in its own section rather than the branch state's.
fn mode_specific(case: &Case, name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || case.continuation.is_some_and(|(prefix, _)| {
            name.starts_with("preparation-branch-") || name.starts_with(prefix)
        })
}

fn branch_state(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical and the family
/// published its pages. Where hourly preparation keeps the family's state
/// in its branch state, the family's own continuation is that state and the
/// rest of the branch state is the same; otherwise the branch state is
/// compared byte for byte too. With `refused`, the branch's coverage pages,
/// whose refusals the family keeps in its own page, are compared apart.
fn assert_same(case: &Case, actual: &Payloads, hourly: &Payloads, label: &str, refused: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(case, name)
                    && !(refused && name.starts_with("preparation-coverage-"))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for prefix in case.published {
        assert!(
            actual.keys().any(|(name, _)| name.starts_with(prefix)),
            "{label}: {} published no {prefix} section",
            case.family
        );
    }
    let Some((prefix, field)) = case.continuation.filter(|_| !refused) else {
        return;
    };
    let mut states = 0;
    for ((name, range), bytes) in hourly {
        let Some(branch) = name.strip_prefix("preparation-branch-") else {
            continue;
        };
        states += 1;
        let mut expected = branch_state(bytes);
        let mut own = branch_state(&actual[&(name.clone(), *range)]);
        let section = actual
            .get(&(format!("{prefix}{branch}"), 0))
            .unwrap_or_else(|| panic!("{label}: no {field} continuation of branch {branch}"));
        let continuation: serde_json::Value = serde_json::from_slice(section).unwrap();
        let value = expected.as_object_mut().unwrap().remove(field).unwrap();
        assert!(
            own.as_object_mut().unwrap().remove(field).is_none(),
            "{label}: the branch state of branch {branch} still holds {field}"
        );
        assert_eq!(continuation, value, "{label}: branch {branch}'s {field}");
        assert_eq!(own, expected, "{label}: branch {branch}'s other state");
    }
    assert!(states > 0, "{label}: no branch state");
}

/// The hourly reference of `prepare`.
fn hourly(fixture: &mut Fixture, revision: &str, prepare: impl Fn(&Source)) -> Payloads {
    independent::with_test_hourly(|| {
        let source = fixture.source(revision);
        prepare(&source);
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    })
}

/// The case's family alone prepared on its own; every other family in the
/// walk.
fn own<T>(case: &Case, run: impl FnOnce() -> T) -> T {
    independent::with_test_only(&[case.family], run)
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

fn sequential(case: &Case) -> usize {
    prepass::noted(&format!("{}.sequential", case.family))
}

#[test]
fn evaluated_families_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    for case in CASES {
        let replay = full(case);
        let mut fixture = Fixture::new(&replay);
        let revision = format!("{}-parity", case.family);
        let hourly = hourly(&mut fixture, &revision, plain);
        for (workers, parallelism) in [(1, 2), (3, 4)] {
            let source = fixture.source(&revision);
            let before = sequential(case);
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || own(case, || plain(&source)))
            });
            let label = format!(
                "{}: {workers} workers, {parallelism} processors",
                case.family
            );
            assert_same(case, &payloads(&source, None), &hourly, &label, false);
            assert_eq!(
                sequential(case),
                before,
                "{label}: every block was read by a pre-pass worker"
            );
            clear(source);
        }
    }
}

#[test]
fn evaluated_families_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation()
{
    for case in CASES {
        let first = scenario(case, 3, false);
        let grown = full(case);
        // The hourly walk prepares one block per run; the family, every block
        // of the recording at the run's start, resuming after the append.
        let resumed = |source: &Source| {
            let flag = AtomicBool::new(false);
            while unprepared_blocks(source, None).unwrap() != Some(0) {
                run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
            }
        };
        let modes: [(&str, &dyn Fn(&Source), bool); 4] = [
            ("run", &plain, false),
            ("measured", &measured, false),
            ("measured, reloaded", &measured, true),
            ("resumed", &resumed, false),
        ];
        let mut fixture = Fixture::new(&first);
        let revision = format!("{}-commits", case.family);
        for (mode, prepare, reload) in modes {
            let both = |fixture: &mut Fixture| -> Payloads {
                super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
                fixture.replace(&first);
                let source = fixture.source(&revision);
                prepare(&source);
                drop(source);
                fixture.replace(&grown);
                let source = fixture.source(&revision);
                prepare(&source);
                super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
                let payloads = payloads(&source, None);
                clear(source);
                payloads
            };
            let hourly = independent::with_test_hourly(|| both(&mut fixture));
            let own = own(case, || {
                prepass::with_test_prepass(2, None, || both(&mut fixture))
            });
            assert_same(
                case,
                &own,
                &hourly,
                &format!("{}: {mode}", case.family),
                false,
            );
        }
    }
}

#[test]
fn evaluated_families_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    for case in CASES {
        let replay = full(case);
        // At a block's first hour, inside a block, and after the gap.
        for at in [8, 21, 27] {
            let hour = BASE + at;
            let refusals = [(case.family, hour)];
            let mut fixture = Fixture::new(&replay);
            let revision = format!("{}-refusal", case.family);
            let (hourly, hourly_refused) = independent::with_test_hourly(|| {
                lanes::with_test_refusals(&refusals, || {
                    let source = fixture.source(&revision);
                    measured(&source);
                    let (store, key) = super::super::super::files(&source, None).unwrap();
                    let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
                    let result = (payloads(&source, None), refused);
                    clear(source);
                    result
                })
            });
            let source = fixture.source(&revision);
            let before = sequential(case);
            own(case, || {
                lanes::with_test_refusals(&refusals, || {
                    prepass::with_test_prepass(2, None, || measured(&source))
                });
                let label = format!("{} refused at +{at}", case.family);
                assert_eq!(
                    sequential(case) - before,
                    1,
                    "{label}: the refusing block is observed hour by hour"
                );
                let own = payloads(&source, None);
                assert_same(case, &own, &hourly, &label, true);
                // The refusal is the family's own, at the hourly hour and text.
                let expected = &hourly_refused[&1].0[case.family];
                let page: CoveragePage = serde_json::from_slice(
                    &own[&(independent::coverage_section(case.family, 1), 0)],
                )
                .unwrap();
                assert_eq!(page.refused.0[case.family].hour, hour, "{label}");
                assert_eq!(page.refused.0[case.family].hour, expected.hour, "{label}");
                assert_eq!(page.refused.0[case.family].error, expected.error, "{label}");
                assert!(page.coverage.complete, "{label}");
                let flag = AtomicBool::new(false);
                let error = refusal(&source, None, 1, case.query, &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
                assert_eq!(error.to_string(), expected.error, "{label}");
                refusal(&source, None, 2, case.query, &flag).unwrap();
            });
            clear(source);
        }
    }
}

fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
) -> io::Result<serde_json::Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

#[test]
fn evaluated_families_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    for case in CASES {
        let replay = full(case);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source(&format!("{}-stalled", case.family));
        // The hourly walk is cancelled at its first commit, having accepted
        // no block, while the family committed every block.
        let flag = AtomicBool::new(false);
        let mut commits = 0;
        let error = own(case, || {
            independent::with_test_one_after_another(|| {
                run_checked(
                    &source,
                    None,
                    &flag,
                    |_| {},
                    |_| {
                        commits += 1;
                        flag.store(true, Ordering::Release);
                        Ok(())
                    },
                )
            })
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{}", case.family);
        assert_eq!(commits, 1, "{}", case.family);
        assert_eq!(
            own(case, || unprepared_blocks(&source, None)).unwrap(),
            Some(replay.index.len()),
            "{}: the hourly walk accepted no block",
            case.family
        );
        let flag = AtomicBool::new(false);
        own(case, || {
            for branch in [1, 2] {
                let label = format!("{} branch {branch}", case.family);
                let covered = product_coverage(&source, None, branch, case.query, &flag)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{label}: the family covers the branch"));
                assert!(covered.complete, "{label}");
                assert!(
                    coverage(&source, None, branch, &flag)
                        .unwrap()
                        .is_none_or(|branch| !branch.complete),
                    "{label}: the branch itself is not covered"
                );
                let id = branch.to_string();
                let (product, params) = (case.request)(&id);
                let reply = query(&source, product, params).unwrap();
                assert!(!reply.is_null(), "{label}: {product} answers");
                assert!(
                    query(&source, "subjects", serde_json::json!({ "branchId": id }))
                        .unwrap()
                        .is_null(),
                    "{label}: a family of the hourly walk still waits"
                );
            }
        });
        clear(source);
    }
}

/// No hour is quiet for these families, so each one's own thread observes
/// each recorded hour once, from blocks every one of which a pre-pass worker
/// read, and publishes hourly preparation's pages.
#[test]
fn evaluated_families_prepared_on_their_own_observe_every_recorded_hour_once() {
    for case in CASES {
        let replay = full(case);
        let total = hours(&replay);
        let mut fixture = Fixture::new(&replay);
        let revision = format!("{}-hours", case.family);
        let hourly = hourly(&mut fixture, &revision, plain);
        let source = fixture.source(&revision);
        let observed = prepass::noted(case.observed);
        let before = sequential(case);
        // One family alone prepares on this thread, where it counts.
        own(case, || {
            prepass::with_test_prepass(2, None, || plain(&source))
        });
        assert_eq!(
            prepass::noted(case.observed) - observed,
            total,
            "{}: every recorded hour observed once",
            case.family
        );
        assert_eq!(sequential(case), before, "{}", case.family);
        assert_same(case, &payloads(&source, None), &hourly, case.family, false);
        clear(source);
    }
}

/// A measured run splits each family's own thread into timing rows of its
/// own: the projects and occupation read their blocks (one read per block)
/// and observe their hours (one observation per recorded hour); research
/// folds every block and reads none itself.
#[test]
fn evaluated_families_measured_runs_split_reading_from_observing() {
    for case in CASES {
        let replay = full(case);
        let total = hours(&replay) as u64;
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source(&format!("{}-measured", case.family));
        let (_, timings) = own(case, || {
            prepass::with_test_prepass(2, None, || {
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
        .unwrap();
        let families = timings.families.expect("a measured run's timing rows");
        for (row, count) in case.parts {
            let calls = families
                .get(row)
                .unwrap_or_else(|| panic!("no {row} timing row"))
                .calls;
            let expected = match count {
                Count::Blocks => replay.index.len() as u64,
                Count::Hours => total,
                Count::Nothing => 0,
            };
            assert_eq!(calls, expected, "{row}");
        }
        clear(source);
    }
}

/// The hourly walk never observes a family prepared on its own: a hook on
/// its lane jobs sees none of the three.
#[test]
fn the_hourly_walk_observes_none_of_the_evaluated_families() {
    for case in CASES {
        let replay = scenario(case, 2, false);
        let seen = Arc::new(Mutex::new(BTreeSet::new()));
        let trace = Arc::clone(&seen);
        let hook: lanes::Hook = Arc::new(move |family, _, _| {
            trace.lock().unwrap().insert(family.to_owned());
            lanes::Fault::Continue
        });
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source(&format!("{}-walk", case.family));
        own(case, || {
            lanes::with_test_lanes(4, Some(hook), || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
        })
        .unwrap();
        let seen = seen.lock().unwrap();
        assert!(!seen.is_empty(), "{}: the walk ran its lanes", case.family);
        assert!(
            !seen.contains(case.family),
            "{}: the hourly walk observed it",
            case.family
        );
        clear(source);
    }
}

/// Special projects bind their content at every block, as the walk does:
/// with content whose project data cannot load, the family records the
/// same content error in its published metadata as hourly preparation,
/// across an appended block, which it would not without binding.
#[test]
fn special_projects_prepared_on_their_own_bind_their_content_as_hourly_preparation() {
    let offsets: Vec<i32> = (0..8).collect();
    let first: Vec<_> = offsets
        .iter()
        .map(|&at| projects_at(1, BASE + at))
        .collect();
    let mut fixture = Fixture::new(&scenario(&PROJECTS, 1, false));
    let game = fixture.root.join("content-projects-own");
    std::fs::create_dir_all(game.join("common/defines")).unwrap();
    std::fs::write(
        game.join("common/defines/synthetic.lua"),
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
    )
    .unwrap();
    let (with_content, context) = crate::test_fixtures::source_with_content(first, &game);
    let replay = with_content.replay_ref();
    let grown = replay
        .append(&[continuation_records(
            replay,
            1,
            (8..16).map(|at| projects_at(1, BASE + at)).collect(),
        )])
        .unwrap();
    fixture.replace(&grown);
    let content = context.content.as_ref();
    let prepare = |fixture: &mut Fixture| -> Payloads {
        let source = fixture.source("projects-content");
        run(&source, content, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, content);
        super::super::super::files(&source, content)
            .unwrap()
            .0
            .clear()
            .unwrap();
        payloads
    };
    let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
    let own = own(&PROJECTS, || {
        prepass::with_test_prepass(2, None, || prepare(&mut fixture))
    });
    assert_same(&PROJECTS, &own, &hourly, "projects with content", false);
    let state = &own[&(format!("{}1", "special-projects-state-"), 0)];
    let state: serde_json::Value = serde_json::from_slice(state).unwrap();
    assert!(
        state["metadata"]["content_error"].is_string(),
        "the family bound its content: {}",
        state["metadata"]
    );
}

/// Research's fold reads no block: every hour after a block's first comes
/// from the worker's index changes and kept rows, and the family still
/// publishes and saves what hourly preparation does.
#[test]
fn research_folds_hours_from_kept_index_changes_and_rows() {
    let replay = full(&RESEARCH);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "research-shipped", plain);
    let source = fixture.source("research-shipped");
    let shipped = prepass::noted(evaluated::RESEARCH_SHIPPED);
    let fallbacks = prepass::noted("research.fallback");
    own(&RESEARCH, || {
        prepass::with_test_prepass(2, None, || plain(&source))
    });
    let shipped = prepass::noted(evaluated::RESEARCH_SHIPPED) - shipped;
    let fallbacks = prepass::noted("research.fallback") - fallbacks;
    assert!(shipped > 0, "no hour came from the kept rows");
    assert!(
        fallbacks > 0 || shipped == total - replay.index.len(),
        "{shipped} of {total} hours shipped with no fallback"
    );
    assert_same(
        &RESEARCH,
        &payloads(&source, None),
        &hourly,
        "shipped",
        false,
    );
    clear(source);
}

/// Where the fold cannot take an hour from the kept rows (here forced: inside
/// a block, at a block's first hour after the gap, and in the second
/// branch), it observes the rest of the block hour by hour with its indexes
/// rebuilt from the block's first hour, and still publishes and saves what
/// hourly preparation does.
#[test]
fn research_falls_back_to_the_rest_of_the_block_as_hourly_preparation() {
    use crate::products::research::prepared::block::with_forced_fallback;
    let replay = full(&RESEARCH);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "research-fallback", plain);
    let source = fixture.source("research-fallback");
    let fallbacks = prepass::noted("research.fallback");
    own(&RESEARCH, || {
        with_forced_fallback(&[BASE + 13, BASE + 26, BASE + 45, BASE + 203], || {
            prepass::with_test_prepass(2, None, || plain(&source))
        })
    });
    assert_eq!(
        prepass::noted("research.fallback") - fallbacks,
        4,
        "every forced hour fell back"
    );
    assert_same(
        &RESEARCH,
        &payloads(&source, None),
        &hourly,
        "fallback",
        false,
    );
    clear(source);
}

/// A content root whose one project, `mod_project` (definition 8 in
/// `projects_at`), is named `name`. No other project name key, nor any
/// country-prefixed variant of one, is defined; `UNRELATED` is scoped text
/// that no project name reads.
fn project_content(root: &std::path::Path, name: &str) {
    for (path, body) in [
        (
            "common/defines/mod.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} }, NBuildings = { MAX_BUILDING_LEVELS = 15, INFRASTRUCTURE_RESOURCE_BONUS = 0, SUPPLY_ROUTE_RESOURCE_BONUS = 0 } }".to_owned(),
        ),
        (
            "common/special_projects/projects/mod.txt",
            "mod_project={specialization=mod_kind}".to_owned(),
        ),
        (
            "common/special_projects/specialization/mod.txt",
            "mod_kind={color={1 2 3}}".to_owned(),
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp".to_owned(),
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n".to_owned()),
        (
            "common/scripted_localisation/mod.txt",
            "entry={name=ProjectStage text={trigger={check_variable={project_flag=1}} localization_key=ACTIVE} text={localization_key=WAITING}}".to_owned(),
        ),
        (
            "localisation/mod_l_english.yml",
            format!(
                "\u{feff}l_english:\n mod_project:0 \"{name}\"\n mod_kind:0 \"Mod science\"\n ACTIVE:0 \"Active\"\n WAITING:0 \"Waiting\"\n UNRELATED:0 \"[ROOT.ProjectStage]\"\n ONGOING_PROJECT_BREAKTHROUGHS_NUMBER:0 \"Count $NUMBER$\"\n"
            ),
        ),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
}

/// `frame` with the core scope tables scripted text's trigger world reads:
/// GER's one state and province, and its `project_flag` variable, set from
/// +4 on.
fn with_scope_tables(mut frame: DecodedFrame) -> DecodedFrame {
    use replay::rows::{ProvinceRow, Recorded, ScriptVariableEntry, ScriptVariablesRow, StateRow};
    let at = offset(frame.branch, frame.hour);
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 9001,
            controller: 9001,
            ..Default::default()
        }]));
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            state: 1,
            owner: 9001,
            controller: 9001,
            ..Default::default()
        }]));
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![
            ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                seed: Recorded::Value(None),
                entries: Vec::new(),
            },
            ScriptVariablesRow {
                scope_kind: 1,
                scope_id: 9001,
                seed: Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"project_flag".to_vec(),
                    value_raw: if at < 4 { 0 } else { 100_000 },
                }],
            },
        ]));
    frame
}

/// Special projects read the trigger kit only where their content can
/// evaluate scoped text: with content whose project name, and every
/// country-prefixed variant of it, is plain (scoped text elsewhere does not
/// count), no block is read with it, neither by the pre-pass worker's
/// extract nor by the family's own reads; with a scoped project name, every
/// block is, and the name evaluates against the hour's trigger world. Both
/// publish and save what hourly preparation does, across an appended block.
#[test]
fn special_projects_read_the_trigger_kit_only_where_content_can_evaluate_scoped_text() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (label, name, scoped) in [
        ("plain", "Plain project", false),
        ("scoped", "[ROOT.ProjectStage]", true),
    ] {
        let frames = |hours: std::ops::Range<i32>| -> Vec<DecodedFrame> {
            hours
                .map(|at| with_scope_tables(projects_at(1, BASE + at)))
                .collect()
        };
        let mut fixture = Fixture::new(&scenario(&PROJECTS, 1, false));
        let game = fixture.root.join(format!("content-projects-{label}"));
        project_content(&game, name);
        let (with_content, context) =
            crate::test_fixtures::source_with_content(frames(0..8), &game);
        let replay = with_content.replay_ref();
        let grown = replay
            .append(&[continuation_records(replay, 1, frames(8..16))])
            .unwrap();
        fixture.replace(&grown);
        let content = context.content.as_ref();
        let revision = format!("projects-kit-{label}");
        let prepare = |fixture: &mut Fixture| -> Payloads {
            let source = fixture.source(&revision);
            run(&source, content, &AtomicBool::new(false), |_| {}).unwrap();
            let payloads = payloads(&source, content);
            super::super::super::files(&source, content)
                .unwrap()
                .0
                .clear()
                .unwrap();
            payloads
        };
        let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
        let kit = crate::products::special_projects::prepared::block::KIT;
        let before = (
            prepass::noted(evaluated::PROJECTS_WORLD),
            prepass::noted(kit),
        );
        let own = own(&PROJECTS, || {
            prepass::with_test_prepass(2, None, || prepare(&mut fixture))
        });
        assert_eq!(
            prepass::noted(evaluated::PROJECTS_WORLD) - before.0,
            if scoped { 2 } else { 0 },
            "{label}: blocks read with the trigger kit"
        );
        assert_eq!(
            prepass::noted(kit) - before.1,
            if scoped { 2 } else { 0 },
            "{label}: blocks the pre-pass worker read with the trigger kit"
        );
        assert_same(&PROJECTS, &own, &hourly, label, false);
        let state: serde_json::Value =
            serde_json::from_slice(&own[&("special-projects-state-1".to_owned(), 0)]).unwrap();
        assert!(
            state["metadata"]["content_error"].is_null(),
            "{label}: the content bound: {}",
            state["metadata"]
        );
    }
}

/// `projects_at` with GER's projects, program and dictionary held at the
/// branch's first hour: rows the observer reads change only where the
/// scenario below changes them, while the countries' balances move every
/// hour and the projects table is not read at +20 to +22.
fn quiet_projects_at(hour: i32) -> DecodedFrame {
    let mut frame = with_scope_tables(projects_at(1, hour));
    let first = projects_at(1, BASE);
    for table in [
        TableId::Projects,
        TableId::ProjectPrograms,
        TableId::LexicalTokens,
    ] {
        frame.tables[table.index()] = first.tables[table.index()].clone();
    }
    frame
}

/// Special projects fold the hours a block reader keeps as hourly
/// preparation observes them: with plain content and with a scoped name
/// (whose variable turns on at +4), over hours whose observed rows hold
/// while the countries' balances move every hour, and an appended block
/// whose dictionary is recorded again with equal rows at +30, a new
/// allocation the published dictionary revision counts. Each publishes and
/// saves what hourly preparation does with one and three workers, observes
/// every recorded hour once, and observes quiet hours on the last kept
/// frame rather than reading them.
#[test]
fn special_projects_fold_quiet_hours_and_an_equal_dictionary_read_again_as_hourly_preparation() {
    use replay::{frame::TableUpdate, spool::Record};
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (label, name) in [
        ("plain", "Plain project"),
        ("scoped", "[ROOT.ProjectStage]"),
    ] {
        let frames = |hours: std::ops::Range<i32>| -> Vec<DecodedFrame> {
            hours.map(|at| quiet_projects_at(BASE + at)).collect()
        };
        let mut fixture = Fixture::new(&scenario(&PROJECTS, 1, false));
        let game = fixture.root.join(format!("content-projects-quiet-{label}"));
        project_content(&game, name);
        let (with_content, context) =
            crate::test_fixtures::source_with_content(frames(0..24), &game);
        let replay = with_content.replay_ref();
        let mut appended = continuation_records(replay, 1, frames(24..48));
        let dictionary = quiet_projects_at(BASE).tables[TableId::LexicalTokens.index()].clone();
        let TableView::Complete(rows) = &*dictionary else {
            panic!("the fixture's dictionary is complete");
        };
        let mut reread = 0;
        for record in &mut appended {
            if let Record::Frame(frame) = record
                && frame.hour == BASE + 30
            {
                frame.tables[TableId::LexicalTokens.index()] = TableUpdate::Snapshot(rows.clone());
                reread += 1;
            }
        }
        assert_eq!(reread, 1);
        let grown = replay.append(&[appended]).unwrap();
        let total = hours(&grown);
        fixture.replace(&grown);
        let content = context.content.as_ref();
        let revision = format!("projects-quiet-{label}");
        let prepare = |fixture: &mut Fixture| -> Payloads {
            let source = fixture.source(&revision);
            run(&source, content, &AtomicBool::new(false), |_| {}).unwrap();
            let payloads = payloads(&source, content);
            super::super::super::files(&source, content)
                .unwrap()
                .0
                .clear()
                .unwrap();
            payloads
        };
        let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
        for workers in [1, 3] {
            let observed = prepass::noted(evaluated::PROJECTS_OBSERVED);
            let quiet = prepass::noted(crate::products::special_projects::prepared::block::QUIET);
            let before = sequential(&PROJECTS);
            let own = own(&PROJECTS, || {
                prepass::with_test_prepass(workers, None, || prepare(&mut fixture))
            });
            assert_same(
                &PROJECTS,
                &own,
                &hourly,
                &format!("{label}, {workers} workers"),
                false,
            );
            assert_eq!(
                prepass::noted(evaluated::PROJECTS_OBSERVED) - observed,
                total,
                "{label}: every recorded hour observed once"
            );
            assert!(
                prepass::noted(crate::products::special_projects::prepared::block::QUIET) > quiet,
                "{label}: quiet hours observed on the last kept frame"
            );
            assert_eq!(
                sequential(&PROJECTS),
                before,
                "{label}: no block read whole"
            );
        }
    }
}

/// Before occupation preparation reaches a window, the window answers from
/// its block and a proven look-back exactly as preparation writes it: every
/// window of the branch, the first (nothing to look back to), one after the
/// recorded gap (anchored by the drop there) and those whose occupation began
/// in an earlier block (anchored by the read in the block before), equal the
/// prepared window byte for byte, with the laws its states hold beside it.
/// The manifest answers the branch's windows before any is published, with
/// no coverage yet, instead of null.
#[test]
fn occupied_states_windows_answer_from_a_proven_look_back_before_preparation_reaches_them() {
    let case = &OCCUPATION;
    let replay = full(case);
    let mut fixture = Fixture::new(&replay);
    let windows = |source: &Source| -> Vec<serde_json::Value> {
        (0..replay
            .index
            .iter()
            .filter(|block| block.branch == 1)
            .count())
            .map(|window| {
                query(
                    source,
                    "occupied-states",
                    serde_json::json!({ "branchId": "1", "window": window }),
                )
                .unwrap()
            })
            .collect()
    };
    let prepared = own(case, || {
        let source = fixture.source("occupied-prepared");
        plain(&source);
        let replies = windows(&source);
        clear(source);
        replies
    });
    assert_eq!(prepared.len(), 8);
    assert!(
        prepared
            .iter()
            .all(|reply| !reply.is_null() && reply.get("laws").is_none())
    );
    // The occupation (+5 to +40) spans windows 0 to 4, across the gap.
    assert!(
        prepared[..5]
            .iter()
            .all(|reply| !reply["states"].as_array().unwrap().is_empty())
    );
    own(case, || {
        let source = fixture.source("occupied-cold");
        let flag = AtomicBool::new(false);
        assert!(
            product_coverage(&source, None, 1, case.query, &flag)
                .unwrap()
                .is_none()
        );
        let manifest = query(
            &source,
            "occupied-states-manifest",
            serde_json::json!({ "branchId": "1" }),
        )
        .unwrap();
        assert!(
            manifest["coverage"]["firstHour"]
                .as_i64()
                .is_some_and(
                    |first| manifest["coverage"]["lastHour"].as_i64() == first.checked_sub(1)
                )
                && manifest["coverage"]["complete"] == false,
            "{manifest}"
        );
        assert_eq!(
            manifest["windows"],
            serde_json::to_value(source.block_windows(1)).unwrap()
        );
        for (window, prepared) in prepared.iter().enumerate() {
            let before = prepass::noted("occupation.look-back");
            let mut cold = query(
                &source,
                "occupied-states",
                serde_json::json!({"branchId":"1", "window":window}),
            )
            .unwrap();
            let blocks = prepass::noted("occupation.look-back") - before;
            assert!(
                blocks <= 2,
                "window {window} reads only itself and its supplying-read predecessor: {blocks}"
            );
            let computations =
                dashboard_base::prepared::query::observations::usage(&source, "occupation:window")
                    .2;
            let again = query(
                &source,
                "occupied-states",
                serde_json::json!({"branchId":"1", "window":window}),
            )
            .unwrap();
            assert_eq!(again, cold);
            assert_eq!(
                dashboard_base::prepared::query::observations::usage(&source, "occupation:window")
                    .2,
                computations
            );
            assert_eq!(cold["laws"], serde_json::json!([]), "window {window}");
            cold.as_object_mut().unwrap().remove("laws");
            assert_eq!(&cold, prepared, "window {window}");
        }
        let before = prepass::noted("occupation.look-back");
        for window in (0..prepared.len()).rev() {
            query(
                &source,
                "occupied-states",
                serde_json::json!({"branchId":"1", "window":window}),
            )
            .unwrap();
        }
        assert_eq!(
            prepass::noted("occupation.look-back"),
            before,
            "neighbour and reached windows share the source cache"
        );
        clear(source);
    });
}

/// Each project run of a special-projects reply through `last`, as (episode,
/// first and last hour clipped to `last`, progress, state, project name).
fn project_runs(
    reply: &serde_json::Value,
    last: i64,
) -> Vec<(String, i64, i64, i64, String, String)> {
    let mut runs = Vec::new();
    for project in reply["projects"].as_array().unwrap() {
        for run in project["runs"].as_array().unwrap() {
            let first = run["firstHour"].as_i64().unwrap();
            if first > last {
                continue;
            }
            let presentation = run["presentation"].as_u64().unwrap() as usize;
            runs.push((
                project["id"].as_str().unwrap().to_owned(),
                first,
                run["lastHour"].as_i64().unwrap().min(last),
                run["progress"].as_i64().unwrap(),
                run["state"].to_string(),
                reply["presentations"][presentation]["name"].to_string(),
            ));
        }
    }
    runs.sort();
    runs
}

/// Before project preparation covers the branch, the special-projects reply
/// answers its prepared prefix instead of null: its projects and runs
/// through the prefix's last hour are those of the complete reply, the
/// project running there (completed only later, at +30) claims no completion,
/// and the hours after are pending. Once complete it is hourly preparation's
/// reply with nothing pending.
#[test]
fn special_projects_answer_their_prepared_prefix_with_the_hours_still_pending() {
    let case = &PROJECTS;
    let replay = full(case);
    let mut fixture = Fixture::new(&replay);
    let request = serde_json::json!({ "branchId": "1", "tag": "GER" });
    independent::with_test_hourly(|| {
        let source = fixture.source("projects-prefix");
        let flag = AtomicBool::new(false);
        let cold = query(
            &source,
            "special-projects",
            serde_json::json!({"branchId":"1", "tag":"GER", "hour": BASE + 15}),
        )
        .unwrap();
        assert!(!cold.is_null());
        assert!(!cold["pending"].as_array().unwrap().is_empty());
        assert_eq!(cold["current"]["projects"].as_array().unwrap().len(), 2);
        assert!(
            cold["current"]["projects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|project| project["runs"][0]["state"] == "running"
                    && project["runs"][0]["completion"].is_null())
        );
        // The first commit ends after the branch's first block (+0 to +7).
        run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        let accepted = coverage(&source, None, 1, &flag).unwrap().unwrap();
        assert_eq!((accepted.last_hour, accepted.complete), (BASE + 7, false));
        let prefix = query(&source, "special-projects", request.clone()).unwrap();
        assert!(!prefix.is_null());
        assert_eq!(prefix["coverage"], serde_json::to_value(&accepted).unwrap());
        assert_eq!(
            prefix["pending"],
            serde_json::to_value(crate::prepared::prefix::pending(&source, 1, BASE + 7, None))
                .unwrap()
        );
        let last = i64::from(BASE + 7);
        for project in prefix["projects"].as_array().unwrap() {
            for run in project["runs"].as_array().unwrap() {
                assert!(run["lastHour"].as_i64().unwrap() <= last, "{prefix}");
                assert!(run["completion"].is_null(), "{prefix}");
            }
        }
        plain(&source);
        let complete = query(&source, "special-projects", request).unwrap();
        assert_eq!(complete["coverage"]["complete"], true);
        assert_eq!(complete["pending"], serde_json::json!([]));
        let at = BASE + 15;
        let fields = |reply: &serde_json::Value| {
            let mut rows = Vec::new();
            for project in reply["projects"].as_array().unwrap() {
                if let Some(run) = project["runs"].as_array().unwrap().iter().find(|run| {
                    run["firstHour"].as_i64().unwrap() <= i64::from(at)
                        && i64::from(at) <= run["lastHour"].as_i64().unwrap()
                }) {
                    let info =
                        &reply["presentations"][run["presentation"].as_u64().unwrap() as usize];
                    rows.push((
                        info["name"].to_string(),
                        run["progress"].clone(),
                        run["state"].clone(),
                        run["iterations"].clone(),
                        info["scientistAsOf"].clone(),
                        reply["people"][info["scientist"].as_u64().unwrap() as usize].clone(),
                    ));
                }
            }
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            rows
        };
        assert_eq!(
            fields(&cold["current"]),
            fields(&complete),
            "current card values use preparation's builders"
        );
        let held = project_runs(&prefix, last);
        assert!(!held.is_empty());
        assert_eq!(held, project_runs(&complete, last));
        clear(source);
    });
}

#[test]
fn occupation_states_and_first_daily_samples_match_preparation_while_identity_history_is_pending() {
    let replay = full(&OCCUPATION);
    let mut fixture = Fixture::new(&replay);
    let family = serde_json::json!({"kind":"occupation-trend", "measure":"resistance", "rankMeasure":"resistance", "top":null, "collaboration":true});
    let state = |hour| serde_json::json!({"branchId":"1", "family":family, "tags":[], "reads":[{"hour":hour}]});
    let hours: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == 1)
        .flat_map(|block| [block.first_hour, block.first_hour + 1, block.last_hour])
        .collect();
    let values = |reply: &serde_json::Value| {
        let mut rows = Vec::new();
        for series in reply["series"].as_array().unwrap() {
            for run in series["runs"].as_array().unwrap() {
                rows.push(serde_json::json!([
                    series["tag"],
                    run["readHour"],
                    run["resistanceRaw"],
                    run["complianceRaw"],
                    run["states"],
                    run["heldStates"],
                    run["heldComplete"],
                    run["collaboration"]
                ]));
            }
        }
        rows.sort_by_key(serde_json::Value::to_string);
        rows
    };
    let expected = independent::with_test_hourly(|| {
        let source = fixture.source("occupation-trend-hourly");
        plain(&source);
        let expected: Vec<_> = hours
            .iter()
            .map(|&hour| query(&source, "series-state", state(hour)).unwrap())
            .collect();
        clear(source);
        expected
    });
    own(&OCCUPATION, || {
        let source = fixture.source("occupation-trend-cold");
        for (&hour, expected) in hours.iter().zip(&expected) {
            let cold = query(&source, "series-state", state(hour)).unwrap();
            assert!(!cold.is_null());
            assert_eq!(
                values(&cold),
                values(expected),
                "numeric and collaboration facts at {hour}"
            );
            assert!(
                cold["preparing"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|field| field == "identities")
            );
        }
        let last = source.block_windows(1).last().unwrap().last_hour;
        let summary = serde_json::json!({"branchId":"1", "family":family, "tags":[], "from":BASE, "to":last, "width":last-BASE+1});
        let cold = query(&source, "series-summary", summary.clone()).unwrap();
        assert!(!cold.is_null());
        assert!(cold["series"].as_array().unwrap().iter().any(|series| {
            !series["summary"]["approximate"]
                .as_array()
                .unwrap()
                .is_empty()
        }));
        let sampled =
            dashboard_base::prepared::query::observations::usage(&source, "occupation:first-read")
                .2;
        assert_eq!(query(&source, "series-summary", summary).unwrap(), cold);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(&source, "occupation:first-read")
                .2,
            sampled
        );
        for block in source
            .replay_ref()
            .index
            .iter()
            .filter(|block| block.branch == 1)
        {
            let Some(hour) = (block.first_hour..=block.last_hour)
                .find(|&hour| occupation_read(offset(1, hour)) == offset(1, hour))
            else {
                continue;
            };
            let exact = query(&source, "series-state", state(hour)).unwrap();
            for series in exact["series"].as_array().unwrap() {
                let Some(sampled) = cold["series"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|sampled| sampled["tag"] == series["tag"])
                else {
                    continue;
                };
                let Some(run) = sampled["summary"]["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|run| {
                        run["firstHour"].as_i64().unwrap() <= i64::from(hour)
                            && i64::from(hour) <= run["lastHour"].as_i64().unwrap()
                    })
                else {
                    continue;
                };
                let raw = series["runs"][0]["resistanceRaw"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                assert_eq!(run["value"].as_f64().unwrap(), raw as f64 / 100_000.0);
            }
        }
        clear(source);
    });
}

/// `quiet_projects_at` with GER's state manpower, which nothing the block
/// reader watches reads, 4321 at the branch's first hour and 8765 from +1;
/// its `project_flag` variable set from +2; and a project completion the
/// history dates +1, which is a deadline at that quiet hour.
fn deadline_projects_at(hour: i32) -> DecodedFrame {
    use replay::rows::ProjectHistoryRow;
    /// The handles the game stores in a completion's history entry.
    const COMPLETION_MARKER: u32 = 0x4c17;
    let at = offset(1, hour);
    let mut frame = quiet_projects_at(hour);
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        rows[0].manpower_available = if at < 1 { 4321 } else { 8765 };
    }
    if let TableView::Complete(TableRows::ScriptVariables(rows)) =
        &mut *frame.tables[TableId::ScriptVariables.index()]
    {
        rows[1].entries[0].value_raw = if at < 2 { 0 } else { 100_000 };
    }
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 11,
            project: 1,
            ordinal: 0,
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date: BASE + 1,
            ..Default::default()
        }]));
    frame
}

/// Content whose one project name is scoped: GER's state manpower
/// (`[ROOT.GetManpower]`) once its `project_flag` is set, plain text before.
fn manpower_content(root: &std::path::Path) {
    project_content(root, "[ROOT.ProjectStage]");
    for (path, body) in [
        (
            "common/scripted_localisation/mod.txt",
            "entry={name=ProjectStage text={trigger={check_variable={project_flag=1}} localization_key=MANPOWER} text={localization_key=WAITING}}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n mod_project:0 \"[ROOT.ProjectStage]\"\n mod_kind:0 \"Mod science\"\n MANPOWER:0 \"Manpower [ROOT.GetManpower]\"\n WAITING:0 \"Waiting\"\n ONGOING_PROJECT_BREAKTHROUGHS_NUMBER:0 \"Count $NUMBER$\"\n",
        ),
    ] {
        std::fs::write(root.join(path), body).unwrap();
    }
}

/// A block read by its changes must not mistake a frame it read for the
/// hour's whole frame while that frame carries a table built at an earlier
/// hour. At +1 a completion deadline falls in a quiet hour whose update moved
/// GER's state manpower, which nothing watches: the pre-pass builds +1's
/// frame on +0's, its states holding +0's manpower, and the scoped name,
/// which reads no manpower yet, advances. At +2 the name's variable changes
/// and the states do not: the frame read there carries the states built at
/// +1, still +0's manpower, while the name now reads it. Hourly preparation
/// names the project with +1's manpower from +2; so must the family on its
/// own, with one and three workers, falling back to the rest of the block
/// where the name cannot be told without the hour's whole frame, observing
/// every recorded hour once, across an appended block.
#[test]
fn special_projects_never_read_a_read_hour_carrying_a_built_table_as_whole() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frames = |hours: std::ops::Range<i32>| -> Vec<DecodedFrame> {
        hours.map(|at| deadline_projects_at(BASE + at)).collect()
    };
    let mut fixture = Fixture::new(&scenario(&PROJECTS, 1, false));
    let game = fixture.root.join("content-projects-deadline");
    manpower_content(&game);
    let (with_content, context) = crate::test_fixtures::source_with_content(frames(0..12), &game);
    let replay = with_content.replay_ref();
    let grown = replay
        .append(&[continuation_records(replay, 1, frames(12..24))])
        .unwrap();
    let total = hours(&grown);
    fixture.replace(&grown);
    let content = context.content.as_ref();
    let revision = "projects-deadline";
    let prepare = |fixture: &mut Fixture| -> (Payloads, String) {
        let source = fixture.source(revision);
        run(&source, content, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, content);
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "special-projects".into(),
            params: serde_json::json!({ "branchId": "1", "tag": "GER" }),
        };
        let reply =
            super::super::super::query::query(&source, &request, content, &AtomicBool::new(false))
                .unwrap();
        super::super::super::files(&source, content)
            .unwrap()
            .0
            .clear()
            .unwrap();
        (payloads, reply)
    };
    let (hourly, reply) = independent::with_test_hourly(|| prepare(&mut fixture));
    assert!(
        reply.contains("Manpower 8765") && !reply.contains("Manpower 4321"),
        "hourly preparation names the project with the manpower of +1 on: {reply}"
    );
    for workers in [1, 3] {
        let label = format!("{workers} workers");
        let observed = prepass::noted(evaluated::PROJECTS_OBSERVED);
        let fallback = prepass::noted("projects.fallback");
        let before = sequential(&PROJECTS);
        let (own, answered) = own(&PROJECTS, || {
            prepass::with_test_prepass(workers, None, || prepare(&mut fixture))
        });
        assert_same(&PROJECTS, &own, &hourly, &label, false);
        assert_eq!(answered, reply, "{label}: the projects reply");
        assert_eq!(
            prepass::noted(evaluated::PROJECTS_OBSERVED) - observed,
            total,
            "{label}: every recorded hour observed once"
        );
        assert!(
            prepass::noted("projects.fallback") > fallback,
            "{label}: the read hour carrying the built states was observed on its whole frame"
        );
        assert_eq!(
            sequential(&PROJECTS),
            before,
            "{label}: no block read whole"
        );
    }
}
