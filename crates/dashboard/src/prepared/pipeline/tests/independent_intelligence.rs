//! The intelligence family prepared on its own, before the hourly walk,
//! publishes and saves exactly what hourly preparation does: its blocks read
//! by their changes while unwatched fields move every hour, the observers'
//! own clocks firing inside quiet hours, the espionage log reading its own
//! committed series and finishing its day pages at the hourly walk's commits,
//! its refusals, and its queries answered while the hourly walk has accepted
//! nothing.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::intelligence_prepared::{HISTORY_READS, KEPT_HOURS, QUIET_HOURS};
use replay::rows::{
    CountryAppearanceRow, CryptologyRow, IntelligenceAgencyRow, LexicalTokenRow,
    OperationAssignmentRow, OperationCompletionRow, OperationRow, OperativeMembershipRow,
    PoliticsRow, RaidIntelInputRow, Recorded, ReservedCountryNamingRow, SimulationFeatureRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Countries of `frame(_, _, 4)`: A runs the agency, the cipher race and the
/// operation against B; C is retargeted and renamed.
const A: u32 = 100_001;
const B: u32 = 100_002;
const C: u32 = 100_003;

/// Each branch's first recorded hour.
fn base(branch: u64) -> i32 {
    if branch == 1 { 1000 } else { 2000 }
}

/// The relative hour of branch 1's cipher break: the daily step learned at
/// 29 expects progress at its next phase, 53, inside a quiet span of the
/// block 44-67, where nothing watched changes.
const BREAK: i32 = 53;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

/// The intelligence tables at `hour`, `at` hours after the branch's first:
/// - A decrypts B: progress at 5 and 29 learns a daily step, then stops, so
///   the step's next phase ([`BREAK`]) ends the run with no table changing.
/// - Memberships are read at 16, 40, 64, 88 and 112: A's first operative on
///   a mission against B, a second assigned to operation 500 from the read
///   at 40, the first retargeted at C from the read at 64; experience moves
///   at each read.
/// - A's agency defense rises at 33, inside a block.
/// - Operations are read at 23, 47, 71 and 95: operation 500 by A against B
///   (A's first operative assigned), gone from the read at 71; its
///   completion, at 70, is drained at 105, in a later block.
/// - C is renamed at 75.
/// - Every hour the countries' balances, the simulation features' debug
///   byte, the agencies' upgrade progress, B's raid defense and the
///   countries' appearance colours move, none of them a field the family
///   watches.
fn intel_frame(branch: u64, hour: i32) -> DecodedFrame {
    let at = hour - base(branch);
    let mut observed = frame(branch, hour, 4);
    if at >= 75
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *observed.tables[TableId::Countries.index()]
    {
        rows[3].tag = b"ZZZ".to_vec();
    }
    let amount = if at < 5 {
        0
    } else if at < 29 {
        100
    } else {
        200
    };
    set(
        &mut observed,
        TableId::Cryptology,
        TableRows::Cryptology(vec![CryptologyRow {
            owner_country: A,
            target_country: B,
            amount_raw: amount,
            bonus_days: 0,
            bonus_active: 0,
            decrypting: 1,
        }]),
    );
    set(
        &mut observed,
        TableId::RaidIntelInputs,
        TableRows::RaidIntelInputs(vec![RaidIntelInputRow {
            country: B,
            strategic_present: 1,
            country_defense: i64::from(hour),
            crypto_strength: Some(5_000),
            operatives: Vec::new(),
        }]),
    );
    set(
        &mut observed,
        TableId::SimulationFeatures,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 5,
            human_ai: u8::from(hour % 2 == 1),
            ..Default::default()
        }]),
    );
    // The country naming operands, recorded complete as a campaign records
    // them: a table unavailable every hour is decoded again every hour, and
    // the pre-pass gives the extract a frame at each rebuild of an incomplete
    // watched table. Each appearance's colour moves every hour, a field the
    // family does not watch.
    set(
        &mut observed,
        TableId::CountryAppearance,
        TableRows::CountryAppearance(
            (0..4)
                .map(|index| CountryAppearanceRow {
                    country: 100_000 + index,
                    red: u32::try_from(hour).unwrap(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow::default()]),
    );
    set(
        &mut observed,
        TableId::Politics,
        TableRows::Politics(
            (0..4)
                .map(|index| PoliticsRow {
                    country: 100_000 + index,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableId::PoliticalParties,
        TableRows::PoliticalParties(Vec::new()),
    );
    set(
        &mut observed,
        TableId::Autonomy,
        TableRows::Autonomy(Vec::new()),
    );
    // The last membership read at or before this hour; -1 before the first.
    let read = (at - 16).div_euclid(24);
    let operative =
        |id: u64, assignment: u32, target: u32, operation: Option<u64>| OperativeMembershipRow {
            id,
            owner_country: A,
            roster_country: A,
            roster_kind: 0,
            legacy_description_length: 0,
            assignment,
            experience_raw: 1_000 + 10 * i64::from(read.max(0)),
            skill: Some(1),
            capturer_country: 0,
            captured_hour: 0,
            action_variant: (assignment == 3).then_some(1),
            ordinal: Recorded::Value(u32::try_from(id - 1).unwrap()),
            target_country: Recorded::Value(Some(target)),
            operation: Recorded::Value(operation),
            name: Recorded::Value(format!("AGENT_{id}").into_bytes()),
            cooldown_kind: Recorded::Value(None),
            cooldown_start_hour: Recorded::Value(None),
            cooldown_end_hour: Recorded::Value(None),
        };
    let mut members = vec![operative(1, 3, if read >= 2 { C } else { B }, None)];
    if read >= 1 {
        members.push(operative(2, 4, B, Some(500)));
    }
    set(
        &mut observed,
        TableId::OperativeMemberships,
        TableRows::OperativeMemberships(members),
    );
    set(
        &mut observed,
        TableId::CharacterOperatives,
        TableRows::CharacterOperatives(Vec::new()),
    );
    set(
        &mut observed,
        TableId::CharacterPortraits,
        TableRows::CharacterPortraits(Vec::new()),
    );
    set(
        &mut observed,
        TableId::IntelligenceAgencies,
        TableRows::IntelligenceAgencies(vec![
            IntelligenceAgencyRow {
                country: A,
                upgrade_progress_raw: i64::from(hour),
                defense_raw: if at >= 33 { 2_000 } else { 1_500 },
                active_count: if read >= 1 { 2 } else { 1 },
                admission_ceiling: 0,
                unknown_f0: 3,
                name: b"AGENCY_A".to_vec(),
                created: 1,
            },
            IntelligenceAgencyRow {
                country: B,
                upgrade_progress_raw: 2 * i64::from(hour),
                defense_raw: 900,
                active_count: 0,
                admission_ceiling: 0,
                unknown_f0: 2,
                name: Vec::new(),
                created: 0,
            },
        ]),
    );
    set(
        &mut observed,
        TableId::AgencyUpgradeLevels,
        TableRows::AgencyUpgradeLevels(Vec::new()),
    );
    set(
        &mut observed,
        TableId::LexicalTokens,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"OPERATION_STEAL".to_vec(),
        }]),
    );
    // The last operations read at or before this hour; -1 before the first.
    let operations_read = (at - 23).div_euclid(24);
    let present = matches!(operations_read, 0 | 1);
    set(
        &mut observed,
        TableId::Operations,
        TableRows::Operations(if present {
            vec![OperationRow {
                id: 500,
                owner: A,
                target: B,
                target_province: None,
                definition_token: 77,
                preparation_current: 1_000 * i64::from(operations_read + 1),
                preparation_required: 10_000,
                flag_40: 0,
                auto_repeat: 0,
                terminal: 0,
                start_hour: base(branch) + 23,
                duration_days: 10,
                phase_count: 2,
                assigned_slots: 1,
                bonus: Recorded::Value(0),
                detected: Recorded::Value(0),
                effect_description: Recorded::Value(Vec::new()),
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        &mut observed,
        TableId::OperationAssignments,
        TableRows::OperationAssignments(if present {
            vec![OperationAssignmentRow {
                operation: 500,
                ordinal: 0,
                operative: 1,
                selection: 0,
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        &mut observed,
        TableId::OperationCompletions,
        TableRows::OperationCompletions(if at == 105 {
            vec![OperationCompletionRow {
                owner: A,
                sequence: 1,
                id: 500,
                target: B,
                definition_token: 77,
                completion_hour: base(branch) + 70,
                terminal: 1,
                bonus: 0,
                detected: 1,
                effect_description: b"done".to_vec(),
                description_error: 0,
                description_required: 4,
            }]
        } else {
            Vec::new()
        }),
    );
    if at % 24 == 16 {
        observed.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
    }
    if at % 24 == 23 {
        observed.metrics.table_ns[TableId::Operations.index()] = 1;
    }
    observed
}

/// Branch 1's blocks, as hours after its first: a gap of two hours precedes
/// the fourth.
const SPANS: [(i32, i32); 5] = [(0, 19), (20, 43), (44, 67), (70, 99), (100, 119)];

/// Branch 1's first `blocks` blocks, then branch 2's one block (hours 0-19
/// after its first) when `second`.
fn scenario(blocks: usize, second: bool) -> Replay {
    let frames = |branch: u64, (first, last): (i32, i32)| -> Vec<DecodedFrame> {
        (first..=last)
            .map(|at| intel_frame(branch, base(branch) + at))
            .collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(1, SPANS[0]))]).unwrap();
    for &span in &SPANS[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(1, span))])
            .unwrap();
    }
    if second {
        replay = replay.append(&[records(2, frames(2, (0, 19)))]).unwrap();
    }
    replay
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

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// The family prepared on its own, alone.
fn own<T>(run: impl FnOnce() -> T) -> T {
    let families: [&'static dyn independent::Own; 1] = [&independent::intelligence::INTELLIGENCE];
    independent::with_test_families(&families, run)
}

/// Blocks observed hour by hour, folds that fell back, hours folded on kept
/// frames, hours folded between them, and reads of the committed series.
fn counts() -> [usize; 5] {
    [
        "intelligence.sequential",
        "intelligence.fallback",
        KEPT_HOURS,
        QUIET_HOURS,
        HISTORY_READS,
    ]
    .map(prepass::noted)
}

fn since(before: [usize; 5]) -> [usize; 5] {
    let after = counts();
    std::array::from_fn(|index| after[index] - before[index])
}

/// The family's own sections, as both modes publish them.
fn intelligence_section(name: &str) -> bool {
    [
        "decryption-",
        "operative-placements-",
        "agency-comparison-",
        "operatives-roster-",
        "espionage-log-",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

/// Every section both modes publish is byte-identical, the family's own
/// first. Only the checkpoint and the family's coverage pages are the
/// family's own; with `refused`, so is the branch's coverage page, whose
/// refusals the hourly walk records only for families it observes.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, refused: bool) {
    let shared = |payloads: &Payloads, own: bool| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                name != "preparation-checkpoint-v1"
                    && !name.starts_with("preparation-family-")
                    && !(refused && name.starts_with("preparation-coverage-"))
                    && (!own || intelligence_section(name))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    let own = shared(hourly, true);
    for prefix in [
        "decryption-1",
        "operative-placements-1",
        "agency-comparison-1",
        "operatives-roster-1",
        "espionage-log-1-windows",
    ] {
        assert!(
            own.keys().any(|(name, _)| name.starts_with(prefix)),
            "{label}: hourly preparation published no {prefix}"
        );
    }
    assert_payloads_equal(
        &shared(actual, true),
        &own,
        &format!("{label}: intelligence"),
    );
    assert_payloads_equal(&shared(actual, false), &shared(hourly, false), label);
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

/// The gated queries a widget's default view makes on `branch`.
fn requests(branch: &str) -> Vec<(&'static str, serde_json::Value)> {
    let mut requests = vec![
        ("decryption", serde_json::json!({ "branchId": branch })),
        (
            "operative-placements",
            serde_json::json!({ "branchId": branch }),
        ),
        (
            "agency-comparison",
            serde_json::json!({
                "selection": { "branchId": branch, "tags": [], "top": null }
            }),
        ),
        (
            "espionage-log",
            serde_json::json!({ "branchId": branch, "window": null }),
        ),
    ];
    if branch == "1" {
        // The day of the operation's disappearance, repaired by its late
        // completion.
        requests.push((
            "espionage-log",
            serde_json::json!({ "branchId": branch, "window": 2 }),
        ));
    }
    requests
}

#[test]
fn intelligence_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = scenario(SPANS.len(), true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("intelligence-parity");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source("intelligence-parity");
        let before = counts();
        own(|| {
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    run(&source, None, &AtomicBool::new(false), |_| {})
                })
            })
        })
        .unwrap();
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&payloads(&source, None), &hourly, &label, false);
        let [sequential, fallback, kept, quiet, _] = since(before);
        assert_eq!(
            (sequential, fallback),
            (0, 0),
            "{label}: every block folded from its extract"
        );
        // Every hour is observed once; frames only where something watched
        // changed, a table watched when read was read, or a block began.
        assert_eq!(kept + quiet, total, "{label}: every hour observed");
        assert!(
            quiet > 0 && kept >= replay.index.len() && kept < total,
            "{label}: {kept} kept of {total} hours"
        );
        // The cipher's step broke at its next phase inside the quiet hours.
        let decryption = query(
            &source,
            "decryption",
            serde_json::json!({ "branchId": "1" }),
        )
        .unwrap();
        let first_hours: Vec<i64> = decryption["pairs"][0]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["firstHour"].as_i64().unwrap())
            .collect();
        assert!(
            first_hours.contains(&i64::from(base(1) + BREAK)),
            "{label}: cipher runs from {first_hours:?}"
        );
        clear(source);
    }
}

/// Read whole, the family keeps a frame every hour and publishes the same
/// pages as read by its changes, which keeps fewer.
#[test]
fn intelligence_read_by_its_changes_builds_no_frame_for_quiet_hours() {
    let replay = scenario(SPANS.len(), true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let mut published = Vec::new();
    for whole in [true, false] {
        let label = if whole {
            "read whole"
        } else {
            "read by changes"
        };
        let source = fixture.source("intelligence-frames");
        let before = counts();
        let read = || {
            own(|| {
                prepass::with_test_prepass(2, None, || {
                    run(&source, None, &AtomicBool::new(false), |_| {})
                })
            })
        };
        if whole {
            prepass::with_test_alone(read)
        } else {
            read()
        }
        .unwrap();
        let [sequential, fallback, kept, quiet, _] = since(before);
        assert_eq!((sequential, fallback), (0, 0), "{label}");
        if whole {
            assert_eq!((kept, quiet), (total, 0), "{label}: a frame every hour");
        } else {
            assert_eq!(kept + quiet, total, "{label}");
            assert!(kept < total / 2, "{label}: {kept} frames of {total} hours");
        }
        published.push(payloads(&source, None));
        clear(source);
    }
    assert_same(&published[1], &published[0], "both reads", false);
}

/// With every family prepared on its own at once, on threads of their own,
/// the family's sections are hourly preparation's.
#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn intelligence_prepared_with_every_family_on_its_own_publishes_hourly_sections() {
    let replay = scenario(SPANS.len(), true);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("intelligence-registry");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    let source = fixture.source("intelligence-registry");
    prepass::with_test_prepass(3, None, || {
        lanes::with_test_lanes(4, None, || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
    })
    .unwrap();
    let own = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| intelligence_section(name))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    let actual = payloads(&source, None);
    assert!(!own(&hourly).is_empty());
    assert_payloads_equal(&own(&actual), &own(&hourly), "every family at once");
    clear(source);
}

/// Prepared on part of the recording, then on all of it, by every commit
/// rule: the espionage log reads back its committed series (and repairs
/// committed day pages) at the hourly walk's commit boundaries.
#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn intelligence_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    let first = scenario(3, false);
    let grown = scenario(SPANS.len(), true);
    let plain = |source: &Source| {
        run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
    };
    // Every block committed on its own.
    let measured = |source: &Source| {
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
    };
    // The hourly walk prepares one block per run.
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
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("intelligence-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("intelligence-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let before = counts();
        let actual = own(|| prepass::with_test_prepass(2, None, || both(&mut fixture)));
        assert_same(&actual, &hourly, mode, false);
        if mode.starts_with("measured") {
            // The completion drained at 105 follows a commit that let its
            // operation go: the log reads its committed instance back.
            assert!(since(before)[4] > 0, "{mode}: no committed series read");
        }
    }
}

fn refused(source: &Source) -> BTreeMap<u64, Refused> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    checkpoint(&store, &key, None).unwrap().unwrap().0.refused
}

#[test]
fn intelligence_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    let replay = scenario(SPANS.len(), true);
    // At a block's first hour, and at the cipher break inside quiet hours.
    for at in [SPANS[2].0, BREAK] {
        let hour = base(1) + at;
        let refusals = [("intelligence", hour)];
        let measured = |source: &Source| {
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
        };
        let mut fixture = Fixture::new(&replay);
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("intelligence-refusal");
                measured(&source);
                let result = (payloads(&source, None), refused(&source));
                clear(source);
                result
            })
        });
        let source = fixture.source("intelligence-refusal");
        let before = counts();
        own(|| {
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || measured(&source))
            })
        });
        let label = format!("refusal at {hour}");
        assert_eq!(
            since(before)[0],
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, true);
        // The family refuses on its own page from the hour hourly
        // preparation records, and its products refuse from it.
        let text = format!("Injected refusal of intelligence at hour {hour}");
        let recorded = &hourly_refused[&1].0["intelligence"];
        assert_eq!(
            (recorded.hour, recorded.error.as_str()),
            (hour, text.as_str())
        );
        assert!(
            refused(&source)
                .get(&1)
                .is_none_or(|refused| refused.open("intelligence")),
            "{label}: the hourly walk does not observe the family"
        );
        for (product, params) in requests("1") {
            let error = query(&source, product, params).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidData,
                "{label}: {product}"
            );
            assert_eq!(error.to_string(), text, "{label}: {product}");
        }
        for (product, params) in requests("2") {
            assert!(
                !query(&source, product, params).unwrap().is_null(),
                "{label}: {product} on branch 2"
            );
        }
        clear(source);
    }
}

#[test]
fn intelligence_answers_from_its_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(SPANS.len(), true);
    let mut fixture = Fixture::new(&replay);
    let expected: Vec<serde_json::Value> = independent::with_test_hourly(|| {
        let source = fixture.source("intelligence-stalled");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let replies = ["1", "2"]
            .into_iter()
            .flat_map(requests)
            .map(|(product, params)| query(&source, product, params).unwrap())
            .collect();
        clear(source);
        replies
    });
    let source = fixture.source("intelligence-stalled");
    let flag = AtomicBool::new(false);
    own(|| {
        // The hourly walk stalls: it is cancelled at its first commit.
        let error = run_checked(
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
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        flag.store(false, Ordering::Release);
        assert_eq!(
            unprepared_blocks(&source, None).unwrap(),
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
        // The roster's default view also needs the country identities,
        // prepared by another family; the four other products answer.
        let actual: Vec<serde_json::Value> = ["1", "2"]
            .into_iter()
            .flat_map(requests)
            .map(|(product, params)| query(&source, product, params).unwrap())
            .collect();
        assert!(actual.iter().all(|reply| !reply.is_null()));
        assert_eq!(actual, expected, "the family answers from its own coverage");
        assert!(
            query(&source, "subjects", serde_json::json!({ "branchId": "1" }))
                .unwrap()
                .is_null(),
            "a family of the hourly walk still waits"
        );
    });
    clear(source);
}
