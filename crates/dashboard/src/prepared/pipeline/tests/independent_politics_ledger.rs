//! The politics ledger prepared on its own, before the hourly walk, publishes
//! and saves exactly what hourly preparation does, across commits, resume,
//! reload, append, a gap and a second branch, with one and several workers;
//! refuses where hourly preparation refuses; answers its query while the
//! hourly walk has accepted nothing; observes every recorded hour once on its
//! own thread; and takes the observer's advance path, without the whole
//! accounting, at most hours, where its captures only extend.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use independent::ledger;
use replay::rows::{
    CountryCommandPowerRow, CountryPowerChangeRow, CountryPowerDailyRow,
    PoliticsCaptureCoverageRow, PowerCause, PowerCurrency, PowerIdentity, PowerObjectKind,
    PowerOperation, PowerSite,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour.
const BASE: i32 = 1000 * 24;
/// The first country of `frame`: raw id, canonical slot 0, tag "000".
const FIRST: u32 = 100_000;
/// The second country: raw id, canonical slot 1, tag "001".
const SECOND: u32 = 100_001;

fn set(observed: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *observed.tables[index] = TableView::Complete(rows);
}

fn capture(country: u32, instance: u32, since: i32, through: i32) -> PoliticsCaptureCoverageRow {
    PoliticsCaptureCoverageRow {
        country,
        instance,
        epoch: 1,
        since_hour: since,
        through_hour: through,
        streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
    }
}

/// The ledger's tables at `hour` of `branch`. The countries' balances (in
/// `frame`) change every hour. Every capture extends to the hour, as a live
/// recording's do, except the second country's, which stops at +21 and
/// starts again (another instance) at +27. The first country spends
/// political power at +3 and gains its daily political power at day 1001's
/// first hour (+24), when its daily application is published; the second
/// spends command power on a leader at +14, reserving it. The published
/// window drops the first spend at +29. Branch 2 spends once, at +102.
fn ledger_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    let start = BASE + if branch == 2 { 100 } else { 0 };
    let second = if branch == 2 || hour <= BASE + 21 {
        capture(SECOND, 1, start, hour)
    } else if hour < BASE + 27 {
        capture(SECOND, 1, start, BASE + 21)
    } else {
        capture(SECOND, 2, BASE + 27, hour)
    };
    set(
        &mut observed,
        TableRows::PoliticsCaptureCoverage(vec![capture(FIRST, 1, start, hour), second]),
    );
    let mut power = Vec::new();
    let spend = |country: u32, sequence: u64, at: i32| CountryPowerChangeRow {
        country,
        instance: 1,
        epoch: 1,
        sequence,
        hour: at,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Subtract,
        balance_before_raw: 5_000_000,
        balance_after_raw: 4_975_000,
        requested_delta_raw: Some(-25_000),
        cause: PowerCause::Decision,
        ..Default::default()
    };
    if branch == 1 {
        if (BASE + 3..BASE + 29).contains(&hour) {
            power.push(spend(FIRST, 1, BASE + 3));
        }
        if hour >= BASE + 24 {
            power.push(CountryPowerChangeRow {
                country: FIRST,
                instance: 1,
                epoch: 1,
                sequence: 3,
                hour: BASE + 24,
                currency: PowerCurrency::Political,
                operation: PowerOperation::Add,
                balance_before_raw: 4_975_000,
                balance_after_raw: 4_975_500,
                requested_delta_raw: Some(500),
                cause: PowerCause::DailyGain,
                ..Default::default()
            });
        }
        if hour >= BASE + 14 {
            power.push(CountryPowerChangeRow {
                country: SECOND,
                instance: 1,
                epoch: 1,
                sequence: 2,
                hour: BASE + 14,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Subtract,
                balance_before_raw: 1_000_000,
                balance_after_raw: 700_000,
                requested_delta_raw: Some(-300_000),
                cause: PowerCause::LeaderAssignment,
                site: PowerSite::AllocateCommandPower,
                identity: PowerIdentity {
                    object_kind: PowerObjectKind::Leader,
                    object_reference: Some((7_u64 << 32) | 2),
                    ..Default::default()
                },
            });
        }
    } else if hour >= BASE + 102 {
        power.push(spend(FIRST, 10, BASE + 102));
    }
    power.sort_by_key(|row| (row.country, row.sequence));
    set(&mut observed, TableRows::CountryPowerChanges(power));
    set(
        &mut observed,
        TableRows::CountryPowerDaily(if branch == 1 && hour >= BASE + 24 {
            vec![CountryPowerDailyRow {
                country: FIRST,
                instance: 1,
                epoch: 1,
                day_hour: BASE + 24,
                pp_gross_gain_raw: Some(500),
                pp_cost_raw: Some(0),
                pp_net_gain_raw: Some(500),
                pp_realized_delta_raw: Some(500),
                pp_sequence: Some(3),
                cp_gain_raw: Some(30),
                cp_realized_delta_raw: Some(30),
                cp_sequence: Some(4),
            }]
        } else {
            Vec::new()
        }),
    );
    set(&mut observed, TableRows::CountryPowerDailyCosts(Vec::new()));
    set(&mut observed, TableRows::GovernmentChanges(Vec::new()));
    set(
        &mut observed,
        TableRows::CountryCommandPower(
            (0..2)
                .map(|canonical_slot| CountryCommandPowerRow {
                    canonical_slot,
                    allocated_command_power_raw: if branch == 1
                        && canonical_slot == 1
                        && hour >= BASE + 14
                    {
                        300_000
                    } else {
                        0
                    },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    observed
}

/// Branch 1's first `blocks` blocks of six hours (+18 and +19 are a gap
/// between the third and fourth), then branch 2's one block when `second`.
fn scenario(blocks: usize, second: bool) -> Replay {
    let hours: [Vec<i32>; 5] = [
        (0..=5).collect(),
        (6..=11).collect(),
        (12..=17).collect(),
        (20..=25).collect(),
        (26..=31).collect(),
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&offset| ledger_frame(1, BASE + offset))
            .collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(&hours[0]))]).unwrap();
    for hours in &hours[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (100..=105)
                    .map(|offset| ledger_frame(2, BASE + offset))
                    .collect(),
            )])
            .unwrap();
    }
    replay
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

/// Sections only one mode writes: the checkpoint and the ledger's own
/// coverage pages.
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1" || name.starts_with("preparation-family-")
}

/// Every section both modes publish is byte-identical, the hourly branch
/// state included (the ledger's state is its own section in both), and the
/// ledger published its pages and its state. With `refused`, the branch's
/// coverage pages, whose refusals are the hourly walk's own, are compared
/// apart.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, refused: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(name) && !(refused && name.starts_with("preparation-coverage-"))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for prefix in ["politics-ledger-state-", "politics-ledger-"] {
        assert!(
            actual.keys().any(|(name, _)| name.starts_with(prefix)),
            "{label}: the ledger published no {prefix} section"
        );
    }
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

/// The ledger alone prepared on its own; every other family in the walk.
fn own<T>(run: impl FnOnce() -> T) -> T {
    independent::with_test_only(&["politics_ledger"], run)
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

#[test]
fn politics_ledger_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "politics-ledger-parity", plain);
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source("politics-ledger-parity");
        let sequential = prepass::noted("politics_ledger.sequential");
        prepass::with_test_prepass(workers, None, || {
            lanes::with_test_lanes(parallelism, None, || own(|| plain(&source)))
        });
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&payloads(&source, None), &hourly, &label, false);
        assert_eq!(
            prepass::noted("politics_ledger.sequential"),
            sequential,
            "{label}: every block was read by a pre-pass worker"
        );
        clear(source);
    }
}

#[test]
fn politics_ledger_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    let first = scenario(3, false);
    let grown = scenario(5, true);
    // The hourly walk prepares one block per run; the ledger, every block of
    // the recording at the run's start, resuming after the append.
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
            let source = fixture.source("politics-ledger-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("politics-ledger-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = own(|| prepass::with_test_prepass(2, None, || both(&mut fixture)));
        assert_same(&own, &hourly, mode, false);
    }
}

#[test]
fn politics_ledger_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    let replay = scenario(5, true);
    // At a block's first hour, inside a block, and at an hour that only
    // advances captures (the observer's advance path).
    for hour in [BASE + 6, BASE + 21, BASE + 16] {
        let refusals = [("politics_ledger", hour)];
        let mut fixture = Fixture::new(&replay);
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("politics-ledger-refusal");
                measured(&source);
                let (store, key) = super::super::super::files(&source, None).unwrap();
                let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
                let result = (payloads(&source, None), refused);
                clear(source);
                result
            })
        });
        let source = fixture.source("politics-ledger-refusal");
        let sequential = prepass::noted("politics_ledger.sequential");
        own(|| {
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || measured(&source))
            });
            let label = format!("politics ledger refused at {hour}");
            assert_eq!(
                prepass::noted("politics_ledger.sequential") - sequential,
                1,
                "{label}: the refusing block is observed hour by hour"
            );
            let own = payloads(&source, None);
            assert_same(&own, &hourly, &label, true);
            // The refusal is the ledger's own, at the hourly hour and text.
            let expected = &hourly_refused[&1].0["politics_ledger"];
            let page: CoveragePage = serde_json::from_slice(
                &own[&(independent::coverage_section("politics_ledger", 1), 0)],
            )
            .unwrap();
            assert_eq!(page.refused.0["politics_ledger"].hour, hour, "{label}");
            assert_eq!(
                page.refused.0["politics_ledger"].hour, expected.hour,
                "{label}"
            );
            assert_eq!(
                page.refused.0["politics_ledger"].error, expected.error,
                "{label}"
            );
            assert!(page.coverage.complete, "{label}");
            let flag = AtomicBool::new(false);
            let error = refusal(&source, None, 1, "politics-ledger", &flag).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(error.to_string(), expected.error, "{label}");
            refusal(&source, None, 2, "politics-ledger", &flag).unwrap();
        });
        clear(source);
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
fn politics_ledger_answers_from_its_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("politics-ledger-stalled");
    // The hourly walk is cancelled at its first commit, having accepted no
    // block, while the ledger committed every block.
    let flag = AtomicBool::new(false);
    let mut commits = 0;
    let error = own(|| {
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
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(commits, 1);
    assert_eq!(
        own(|| unprepared_blocks(&source, None)).unwrap(),
        Some(replay.index.len()),
        "the hourly walk accepted no block"
    );
    let flag = AtomicBool::new(false);
    own(|| {
        for branch in [1, 2] {
            let covered = product_coverage(&source, None, branch, "politics-ledger", &flag)
                .unwrap()
                .unwrap_or_else(|| panic!("the ledger covers branch {branch}"));
            assert!(covered.complete, "branch {branch}");
            assert!(
                coverage(&source, None, branch, &flag)
                    .unwrap()
                    .is_none_or(|branch| !branch.complete),
                "branch {branch} itself is not covered"
            );
            let id = branch.to_string();
            let ledger = query(
                &source,
                "politics-ledger",
                serde_json::json!({ "branchId": id, "tag": "000", "view": "raw" }),
            )
            .unwrap();
            assert!(!ledger.is_null(), "branch {branch}: the ledger answers");
            assert!(
                query(&source, "subjects", serde_json::json!({ "branchId": id }))
                    .unwrap()
                    .is_null(),
                "branch {branch}: a family of the hourly walk still waits"
            );
        }
    });
    clear(source);
}

/// No hour is quiet for the ledger (every capture extends to the hour), so
/// its own thread observes each recorded hour once, from blocks every one of
/// which a pre-pass worker read, and publishes hourly preparation's pages.
#[test]
fn politics_ledger_prepared_on_its_own_observes_every_recorded_hour_once() {
    let replay = scenario(5, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "politics-ledger-hours", plain);
    let source = fixture.source("politics-ledger-hours");
    let observed = prepass::noted(ledger::OBSERVED);
    let sequential = prepass::noted("politics_ledger.sequential");
    // One family alone prepares on this thread, where it counts.
    own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    assert_eq!(
        prepass::noted(ledger::OBSERVED) - observed,
        total,
        "every recorded hour observed once"
    );
    assert_eq!(prepass::noted("politics_ledger.sequential"), sequential);
    let pages = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| name.starts_with("politics-ledger"))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    let own_pages = pages(&payloads(&source, None));
    assert!(!own_pages.is_empty());
    assert_payloads_equal(&own_pages, &pages(&hourly), "the ledger's pages");
    clear(source);
}

/// Hours at which every capture only extends to the hour and nothing else
/// the ledger joins changes take the observer's advance path on the
/// family's own thread, without the whole accounting, and the ledger still
/// publishes and saves byte for byte what hourly preparation does, through
/// the first country's spend, its daily gain at day 1001's first hour, the
/// second's command power hold, its capture's stop and restart under
/// another instance, the gap, every block boundary and the second branch.
/// Each block's first hour (no change facts of the block before), each
/// hour that publishes a row, a capture's expiry and a capture starting
/// again take the whole accounting; most hours do not.
#[test]
fn politics_ledger_prepared_on_its_own_skips_the_whole_accounting_where_captures_only_advance() {
    let replay = scenario(5, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "politics-ledger-advance", plain);
    let source = fixture.source("politics-ledger-advance");
    let observed = prepass::noted(ledger::OBSERVED);
    let advanced = prepass::noted(ledger::ADVANCED);
    // One family alone prepares on this thread, where it counts.
    own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    let observed = prepass::noted(ledger::OBSERVED) - observed;
    let advanced = prepass::noted(ledger::ADVANCED) - advanced;
    assert_eq!(observed, total, "every recorded hour observed once");
    assert!(advanced > 0, "no hour took the advance path");
    assert!(
        advanced * 2 >= observed,
        "most hours only advance captures: {advanced} of {observed} took the advance path"
    );
    assert!(
        advanced + replay.index.len() <= observed,
        "every block's first hour takes the whole accounting"
    );
    assert_same(&payloads(&source, None), &hourly, "advance path", false);
    clear(source);
}

/// Countries of the dense campaign: raw ids from [`FIRST`], slots and tags
/// from 0.
const DENSE: u32 = 5;
/// Hours a published power row stays in the dense campaign's window.
const WINDOW: i32 = 30;

/// One hour of a dense campaign of `DENSE` countries at `hour` of `branch`,
/// whose captures extend to every hour they are read. Every country gains
/// its daily political power at each day's first hour, when its daily
/// application is published, and each spends political power every 23
/// hours, four hours after the country before; each power row leaves the
/// published window `WINDOW` hours after its hour. The second country allocates
/// command power to a leader at +14, held from then on. The third country's
/// capture stops at +40 and starts again (another instance) at +47. Branch 2
/// starts at +200.
fn dense_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, DENSE);
    let start = BASE + if branch == 2 { 200 } else { 0 };
    let captures = (0..DENSE)
        .map(|index| {
            let country = FIRST + index;
            if branch == 1 && index == 2 && hour > BASE + 40 {
                if hour < BASE + 47 {
                    capture(country, 1, start, BASE + 40)
                } else {
                    capture(country, 2, BASE + 47, hour)
                }
            } else {
                capture(country, 1, start, hour)
            }
        })
        .collect();
    set(&mut observed, TableRows::PoliticsCaptureCoverage(captures));
    let instance = |index: u32, at: i32| {
        if branch == 1 && index == 2 && at >= BASE + 47 {
            2
        } else {
            1
        }
    };
    let held = |at: i32| at <= hour && hour < at + WINDOW && at >= start;
    let mut power = Vec::new();
    let mut daily = Vec::new();
    let today = hour.div_euclid(24) * 24;
    for index in 0..DENSE {
        let country = FIRST + index;
        // The daily gains of the days in the window, and today's application.
        for day in (start.div_euclid(24) * 24..=today).step_by(24) {
            let sequence = u64::try_from(day - BASE).unwrap() * 10 + 1;
            if held(day) {
                power.push(CountryPowerChangeRow {
                    country,
                    instance: instance(index, day),
                    epoch: 1,
                    sequence,
                    hour: day,
                    currency: PowerCurrency::Political,
                    operation: PowerOperation::Add,
                    balance_before_raw: 4_975_000,
                    balance_after_raw: 4_975_500,
                    requested_delta_raw: Some(500),
                    cause: PowerCause::DailyGain,
                    ..Default::default()
                });
            }
            if day == today && day >= start {
                daily.push(CountryPowerDailyRow {
                    country,
                    instance: instance(index, day),
                    epoch: 1,
                    day_hour: day,
                    pp_gross_gain_raw: Some(500),
                    pp_cost_raw: Some(0),
                    pp_net_gain_raw: Some(500),
                    pp_realized_delta_raw: Some(500),
                    pp_sequence: Some(sequence),
                    cp_gain_raw: Some(30),
                    cp_realized_delta_raw: Some(30),
                    cp_sequence: Some(sequence + 2),
                });
            }
        }
        // Its spends in the window.
        let offset = 3 + 4 * i32::try_from(index).unwrap();
        for at in (start..=hour).filter(|at| (at - BASE) % 23 == offset) {
            if held(at) && !(branch == 1 && index == 2 && (BASE + 41..BASE + 47).contains(&at)) {
                power.push(CountryPowerChangeRow {
                    country,
                    instance: instance(index, at),
                    epoch: 1,
                    sequence: u64::try_from(at - BASE).unwrap() * 10 + 2,
                    hour: at,
                    currency: PowerCurrency::Political,
                    operation: PowerOperation::Subtract,
                    balance_before_raw: 5_000_000,
                    balance_after_raw: 4_975_000,
                    requested_delta_raw: Some(-25_000),
                    cause: PowerCause::Decision,
                    ..Default::default()
                });
            }
        }
    }
    if branch == 1 && hour >= BASE + 14 {
        power.push(CountryPowerChangeRow {
            country: SECOND,
            instance: 1,
            epoch: 1,
            // After its spend at +7, in sequence.
            sequence: 145,
            hour: BASE + 14,
            currency: PowerCurrency::Command,
            operation: PowerOperation::Subtract,
            balance_before_raw: 1_000_000,
            balance_after_raw: 700_000,
            requested_delta_raw: Some(-300_000),
            cause: PowerCause::LeaderAssignment,
            site: PowerSite::AllocateCommandPower,
            identity: PowerIdentity {
                object_kind: PowerObjectKind::Leader,
                object_reference: Some((7_u64 << 32) | 2),
                ..Default::default()
            },
        });
    }
    power.sort_by_key(|row| (row.country, row.sequence));
    set(&mut observed, TableRows::CountryPowerChanges(power));
    daily.sort_by_key(|row| row.country);
    set(&mut observed, TableRows::CountryPowerDaily(daily));
    set(&mut observed, TableRows::CountryPowerDailyCosts(Vec::new()));
    set(&mut observed, TableRows::GovernmentChanges(Vec::new()));
    set(
        &mut observed,
        TableRows::CountryCommandPower(
            (0..DENSE)
                .map(|canonical_slot| CountryCommandPowerRow {
                    canonical_slot,
                    allocated_command_power_raw: if branch == 1
                        && canonical_slot == 1
                        && hour >= BASE + 14
                    {
                        300_000
                    } else {
                        0
                    },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    observed
}

/// Branch 1's first `blocks` blocks of the dense campaign (+58 and +59 are
/// a gap inside the fourth block), then branch 2's two blocks when
/// `second`. Every block but the last of a branch holds sixteen recorded
/// hours, and every one but the first a day's first hour.
fn dense_scenario(blocks: usize, second: bool) -> Replay {
    let hours: [Vec<i32>; 6] = [
        (0..=15).collect(),
        (16..=31).collect(),
        (32..=47).collect(),
        (48..=65).filter(|hour| !(58..=59).contains(hour)).collect(),
        (66..=81).collect(),
        (82..=97).collect(),
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&offset| dense_frame(1, BASE + offset))
            .collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(&hours[0]))]).unwrap();
    for hours in &hours[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        let frames = |hours: std::ops::RangeInclusive<i32>| -> Vec<DecodedFrame> {
            hours.map(|offset| dense_frame(2, BASE + offset)).collect()
        };
        replay = replay.append(&[records(2, frames(200..=215))]).unwrap();
        replay = replay
            .append(&[continuation_records(&replay, 2, frames(216..=229))])
            .unwrap();
    }
    replay
}

/// What the ledger's own preparation counted on this thread while `run`
/// ran: hours observed, those advanced from a worker's certificate without
/// joining their captures, and those presenting their countries on several
/// threads.
fn folded<T>(run: impl FnOnce() -> T) -> (T, [usize; 3]) {
    let names = [ledger::OBSERVED, ledger::CERTIFIED, ledger::PARALLEL];
    let before = names.map(prepass::noted);
    let result = run();
    let after = names.map(prepass::noted);
    (result, std::array::from_fn(|at| after[at] - before[at]))
}

/// The ledger's own preparation presenting its countries on three threads
/// wherever an hour presents two or more.
fn presented<T>(run: impl FnOnce() -> T) -> T {
    crate::products::politics_ledger::prepared::with_test_presentation(2, 3, run)
}

/// The dense campaign prepared on its own, with one and three workers,
/// publishes and saves exactly what hourly preparation does, through every
/// day boundary, spend, window drop, hold, a capture stopping and starting
/// again, a gap inside a block and the second branch, with each hour's
/// countries presented on several threads. The fold observes every
/// recorded hour, joins the captures of only some of them in full (each
/// block's first hour, each day boundary, spend and window drop, and the
/// capture's stop and restart), and advances the others from the workers'
/// certificates.
#[test]
fn politics_ledger_folds_dense_blocks_from_worker_joins_as_hourly_preparation_does() {
    let replay = dense_scenario(6, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "politics-ledger-fold", plain);
    for workers in [1, 3] {
        let source = fixture.source("politics-ledger-fold");
        let sequential = prepass::noted("politics_ledger.sequential");
        let ((), [observed, certified, parallel]) = folded(|| {
            prepass::with_test_prepass(workers, None, || own(|| presented(|| plain(&source))))
        });
        let label = format!("{workers} workers");
        assert_same(&payloads(&source, None), &hourly, &label, false);
        assert_eq!(
            prepass::noted("politics_ledger.sequential"),
            sequential,
            "{label}: every block was joined by a pre-pass worker"
        );
        assert_eq!(
            observed, total,
            "{label}: every recorded hour observed once"
        );
        // The hours the fold joins in full: each block's first hour, a day's
        // first hours, the spends, window drops and capture changes.
        let joined = observed - certified;
        assert!(
            certified > 0 && certified * 4 >= total,
            "{label}: a quarter of the hours advance from the workers' certificates; {certified} of {total} did"
        );
        assert!(
            joined >= replay.index.len(),
            "{label}: every block's first hour is joined in full"
        );
        assert!(
            parallel > 0,
            "{label}: the hours presenting several countries present them on several threads"
        );
        clear(source);
    }
}

/// The dense campaign committed every block, resumed per run, reloaded and
/// appended to publishes what hourly preparation does, and its folds still
/// advance hours from the workers' certificates after each resume.
#[test]
fn politics_ledger_folds_dense_blocks_across_commits_resume_reload_and_append() {
    let first = dense_scenario(3, false);
    let grown = dense_scenario(6, true);
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
            let source = fixture.source("politics-ledger-fold-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("politics-ledger-fold-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let (own, [_, certified, _]) = folded(|| {
            own(|| prepass::with_test_prepass(3, None, || presented(|| both(&mut fixture))))
        });
        assert_same(&own, &hourly, mode, false);
        assert!(
            certified > 0,
            "{mode}: hours advance from the workers' certificates"
        );
    }
}

/// The dense campaign refuses where hourly preparation does, at the hour and
/// with the text it refuses with: at an hour a worker certifies (+20, which
/// neither spends nor drops a spend), at a day's first hour and at a block's
/// first hour. The refusing block is observed
/// hour by hour; the blocks before it are folded from their certificates.
#[test]
fn politics_ledger_folding_dense_blocks_refuses_where_hourly_preparation_does() {
    let replay = dense_scenario(6, true);
    for hour in [BASE + 20, BASE + 48, BASE + 66] {
        let refusals = [("politics_ledger", hour)];
        let mut fixture = Fixture::new(&replay);
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("politics-ledger-fold-refusal");
                measured(&source);
                let (store, key) = super::super::super::files(&source, None).unwrap();
                let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
                let result = (payloads(&source, None), refused);
                clear(source);
                result
            })
        });
        let source = fixture.source("politics-ledger-fold-refusal");
        let sequential = prepass::noted("politics_ledger.sequential");
        let label = format!("politics ledger refused at {hour}");
        let ((), [_, certified, _]) = folded(|| {
            own(|| {
                lanes::with_test_refusals(&refusals, || {
                    prepass::with_test_prepass(3, None, || presented(|| measured(&source)))
                })
            })
        });
        assert_eq!(
            prepass::noted("politics_ledger.sequential") - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert!(certified > 0, "{label}: the blocks before it are folded");
        let own = payloads(&source, None);
        assert_same(&own, &hourly, &label, true);
        let expected = &hourly_refused[&1].0["politics_ledger"];
        let page: CoveragePage =
            serde_json::from_slice(&own[&(independent::coverage_section("politics_ledger", 1), 0)])
                .unwrap();
        assert_eq!(page.refused.0["politics_ledger"].hour, hour, "{label}");
        assert_eq!(
            page.refused.0["politics_ledger"].hour, expected.hour,
            "{label}"
        );
        assert_eq!(
            page.refused.0["politics_ledger"].error, expected.error,
            "{label}"
        );
        let flag = AtomicBool::new(false);
        let error = refusal(&source, None, 1, "politics-ledger", &flag).unwrap_err();
        assert_eq!(error.to_string(), expected.error, "{label}");
        refusal(&source, None, 2, "politics-ledger", &flag).unwrap();
        clear(source);
    }
}
