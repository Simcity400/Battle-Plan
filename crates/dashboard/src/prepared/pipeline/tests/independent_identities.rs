//! The country identity history prepared on its own, before the hourly walk,
//! publishes and saves exactly what hourly preparation does (territory and
//! the event feed, which read it in the same hour, included), is given frames
//! only at the hours what it reads changes, and answers while the hourly walk
//! has accepted nothing.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::country_history::{
    self as history,
    prepared::{built_frames, observed_hours},
};
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryExperienceGainRow, rows::CountryLinkRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;
/// B's native country once replaced.
const REPLACED: u32 = 200_001;
/// A second raw link to A's slot.
const ALIAS: u32 = 300_000;
/// A counter row whose country no link resolves.
const UNRESOLVED: u32 = 999_999;

/// Branch 1's blocks (hours 112 and 113 are a gap between the third and
/// fourth); branch 2 is one block, hours 20 to 23.
const BLOCKS: [&[i32]; 6] = [
    &[100, 101, 102, 103],
    &[104, 105, 106, 107],
    &[108, 109, 110, 111],
    &[114, 115, 116, 117],
    &[118, 119, 120, 121],
    &[122, 123, 124, 125],
];

/// Hours read by their changes at which nothing the history reads changes
/// and no deadline falls: 101 and 103; 105 and 107; 109 and 111; 116 and
/// 117; 123; and branch 2's 21 to 23.
const QUIET_HOURS: usize = 12;

/// Three countries' identity tables at `hour`. Every hour the countries'
/// balances, the counters' owned-state counts and the gains' amounts change,
/// which the history does not read. A counter row no link resolves appears
/// at 102 and a second row for A's slot at 121 (each inside a block); C is
/// renamed at 106 (inside a block); B's native country is replaced at 108
/// (a block's first hour); C's native instance changes at 110; A gains a
/// second raw link at 115; A is renamed at 119, an hour that does not read
/// the countries, so the native names held then are stale at 120, where
/// nothing the history reads changes; the links are unavailable at 124. The
/// countries are not read at 101, nor the gains at 105 and 116, hours inside
/// quiet spans.
fn identity_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 3);
    let b = if hour >= 108 { REPLACED } else { B };
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *observed.tables[TableId::Countries.index()]
    {
        if hour >= 106 {
            rows[2].tag = b"mod/renamed".to_vec();
        }
        if hour >= 119 {
            rows[0].tag = b"mod/second".to_vec();
        }
        rows[1].id = b;
        rows.sort_by_key(|row| row.id);
    }
    *observed.tables[TableId::CountryLinks.index()] = if hour == 124 {
        TableView::Unavailable {
            reason: "fixture links not read".into(),
        }
    } else {
        let mut links = vec![
            CountryLinkRow {
                id: A,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: b,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: C,
                canonical_slot: 2,
            },
        ];
        if hour >= 115 {
            links.push(CountryLinkRow {
                id: ALIAS,
                canonical_slot: 0,
            });
        }
        links.sort_by_key(|row| row.id);
        TableView::Complete(TableRows::CountryLinks(links))
    };
    let mut counters: Vec<_> = [A, b, C]
        .into_iter()
        .enumerate()
        .map(|(index, country)| CountryCounterInputRow {
            country,
            owned_state_count: (hour + index as i32).rem_euclid(3),
            ..Default::default()
        })
        .collect();
    if hour >= 102 {
        counters.push(CountryCounterInputRow {
            country: UNRESOLVED,
            ..Default::default()
        });
    }
    if hour >= 121 {
        counters.push(CountryCounterInputRow {
            country: ALIAS,
            ..Default::default()
        });
    }
    counters.sort_by_key(|row| row.country);
    *observed.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(counters));
    let mut gains: Vec<_> = [A, b, C]
        .into_iter()
        .enumerate()
        .map(|(index, country)| CountryExperienceGainRow {
            country,
            instance: 7 + index as u32 + u32::from(country == C && hour >= 110),
            complete: 1,
            army_raw: i64::from(hour),
            ..Default::default()
        })
        .collect();
    gains.sort_by_key(|row| (row.country, row.instance));
    *observed.tables[TableId::CountryExperienceGains.index()] =
        TableView::Complete(TableRows::CountryExperienceGains(gains));
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperienceGains,
    ] {
        observed.metrics.table_ns[table.index()] = 1;
    }
    if matches!(hour, 101 | 119) {
        observed.metrics.table_ns[TableId::Countries.index()] = 0;
    }
    if matches!(hour, 105 | 116) {
        observed.metrics.table_ns[TableId::CountryExperienceGains.index()] = 0;
    }
    observed
}

/// Branch 1's first `blocks` blocks, then branch 2's one block when `second`.
fn scenario(blocks: usize, second: bool) -> Replay {
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| identity_frame(1, hour)).collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(BLOCKS[0]))]).unwrap();
    for &hours in &BLOCKS[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (20..=23).map(|hour| identity_frame(2, hour)).collect(),
            )])
            .unwrap();
    }
    replay
}

/// Run `run` with the identities as the only family prepared on its own, so
/// it prepares on this thread and its counts are this thread's.
fn own<T>(run: impl FnOnce() -> T) -> T {
    let families: [&'static dyn independent::Own; 1] = [&independent::IDENTITIES];
    independent::with_test_families(&families, run)
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

fn refused(source: &Source) -> BTreeMap<u64, Refused> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    checkpoint(&store, &key, None).unwrap().unwrap().0.refused
}

fn query(source: &Source, product: &str, branch: &str) -> io::Result<serde_json::Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params: serde_json::json!({ "branchId": branch }),
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
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

/// Sections only one mode writes: the checkpoint (which holds the
/// identities' progress), the families' own coverage pages, and territory's
/// own read of the identities.
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("territory-identities-")
}

/// Every section both modes publish is byte-identical, the identity pages
/// and territory's pages first. With `complete`, territory's own read of the
/// identities, kept by the hourly walk, equals the history the identities
/// published on their own.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, complete: bool) {
    let shared = |payloads: &Payloads, prefix: Option<&str>| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(name) && prefix.is_none_or(|prefix| name.starts_with(prefix))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    for prefix in ["country-identities-", "territory"] {
        assert!(
            !shared(hourly, Some(prefix)).is_empty(),
            "{label}: hourly preparation published no {prefix} page"
        );
        assert_payloads_equal(
            &shared(actual, Some(prefix)),
            &shared(hourly, Some(prefix)),
            &format!("{label}: {prefix}"),
        );
    }
    assert_payloads_equal(&shared(actual, None), &shared(hourly, None), label);
    if !complete {
        return;
    }
    let mut kept = 0;
    for ((name, range), bytes) in actual {
        let Some(branch) = name.strip_prefix("territory-identities-") else {
            continue;
        };
        kept += 1;
        assert_eq!(
            Some(bytes),
            actual.get(&(format!("country-identities-{branch}"), *range)),
            "{label}: territory's read of branch {branch}'s identities"
        );
    }
    assert!(
        kept > 0,
        "{label}: the hourly walk kept no read of the identities"
    );
}

#[test]
fn country_identities_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("identities-parity");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source("identities-parity");
        let before = (
            prepass::noted("identities.sequential"),
            prepass::noted("identities.fallback"),
            observed_hours(),
        );
        own(|| {
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    run(&source, None, &AtomicBool::new(false), |_| {})
                })
            })
        })
        .unwrap();
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&payloads(&source, None), &hourly, &label, true);
        assert_eq!(
            (
                prepass::noted("identities.sequential") - before.0,
                prepass::noted("identities.fallback") - before.1
            ),
            (0, 0),
            "{label}: every block folded from its extract"
        );
        // Quiet hours are stepped, not observed, on the coordinator.
        let observed = observed_hours() - before.2;
        assert_eq!(
            observed,
            hours(&replay) - QUIET_HOURS,
            "{label}: hours observed"
        );
        clear(source);
    }
}

/// Read by their changes, the identities are given a frame only at the hours
/// what they read changes, at the hour their stale native names need rows,
/// and at each block's first hour, while the fields they do not read change
/// every hour; read whole, at every hour. Both publish what hourly
/// preparation does.
#[test]
fn country_identities_read_by_their_changes_build_frames_only_where_the_history_reads_change() {
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("identities-changes");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    for whole in [true, false] {
        let label = if whole {
            "read whole"
        } else {
            "read by changes"
        };
        let source = fixture.source("identities-changes");
        let before = (
            observed_hours(),
            built_frames(),
            prepass::noted("identities.sequential"),
            prepass::noted("identities.fallback"),
        );
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
        assert_same(&payloads(&source, None), &hourly, label, true);
        assert_eq!(
            (
                prepass::noted("identities.sequential") - before.2,
                prepass::noted("identities.fallback") - before.3
            ),
            (0, 0),
            "{label}: every block folded from its extract"
        );
        let observed = observed_hours() - before.0;
        let frames = built_frames() - before.1;
        if whole {
            assert_eq!(frames, hours(&replay), "{label}: a frame every hour");
            assert_eq!(observed, hours(&replay), "{label}: every hour observed");
        } else {
            assert_eq!(frames, observed, "{label}: frames only at observed hours");
            assert_eq!(
                frames,
                hours(&replay) - QUIET_HOURS,
                "{label}: no frame at a quiet hour"
            );
        }
        clear(source);
    }
}

#[test]
fn country_identities_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation()
{
    let first = scenario(3, false);
    let grown = scenario(6, true);
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
            let source = fixture.source("identities-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("identities-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let apart = own(|| both(&mut fixture));
        assert_same(&apart, &hourly, mode, true);
    }
}

#[test]
fn country_identities_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    let replay = scenario(6, true);
    // At a block's first hour, and at a quiet hour inside a block.
    for hour in [108, 116] {
        let refusals = [("identities", hour)];
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
                let source = fixture.source("identities-refusal");
                measured(&source);
                let result = (payloads(&source, None), refused(&source));
                clear(source);
                result
            })
        });
        let source = fixture.source("identities-refusal");
        let sequential = prepass::noted("identities.sequential");
        own(|| {
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || measured(&source))
            })
        });
        let label = format!("refusal at {hour}");
        assert_eq!(
            prepass::noted("identities.sequential") - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, false);
        // Territory and the event feed refuse with the identities at the
        // same hour, through the hourly walk's own read.
        assert_eq!(refused(&source), hourly_refused, "{label}");
        let branch = &hourly_refused[&1].0;
        assert_eq!(branch["identities"].hour, hour);
        for reader in ["territory", "event_feed"] {
            assert_eq!(branch[reader].hour, hour, "{label}: {reader}");
            assert_eq!(branch[reader].from.as_deref(), Some("identities"));
        }
        let text = format!("Injected refusal of identities at hour {hour}");
        let error = query(&source, "country-identities", "1").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), text);
        // Every reader of the history refuses with it, never with its
        // earlier pages.
        let error = history::prepared_branch_histories(&source, 1, None, &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(error.to_string(), text);
        assert!(!query(&source, "country-identities", "2").unwrap().is_null());
        clear(source);
    }
}

#[test]
fn country_identities_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    // What hourly preparation answers once the branches are prepared.
    let expected: Vec<serde_json::Value> = independent::with_test_hourly(|| {
        let source = fixture.source("identities-stalled");
        run(&source, None, &flag, |_| {}).unwrap();
        let expected = [1, 2]
            .map(|branch| {
                let history = history::prepared_branch_histories(&source, branch, None, &flag)
                    .unwrap()
                    .expect("prepared identities");
                serde_json::to_value(&*history).unwrap()
            })
            .to_vec();
        clear(source);
        expected
    });
    let source = fixture.source("identities-stalled");
    // The hourly walk stalls: it is cancelled at its first commit.
    let mut commits = 0;
    let error = own(|| {
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
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(commits, 1);
    flag.store(false, Ordering::Release);
    for (branch, expected) in [1_u64, 2].into_iter().zip(&expected) {
        assert!(
            coverage(&source, None, branch, &flag).unwrap().is_none(),
            "branch {branch}: the hourly walk covers nothing"
        );
        let history = history::prepared_branch_histories(&source, branch, None, &flag)
            .unwrap()
            .expect("identities answer from their own coverage");
        assert_eq!(
            &serde_json::to_value(&*history).unwrap(),
            expected,
            "branch {branch}: the identities answer as hourly preparation does"
        );
        assert_eq!(
            &query(&source, "country-identities", &branch.to_string()).unwrap(),
            expected,
            "branch {branch}: the identity query"
        );
    }
    // The hourly walk then prepares every block; the history stays.
    let accepted = own(|| run(&source, None, &flag, |_| {})).unwrap();
    for branch in [1_u64, 2] {
        assert!(
            coverage(&source, None, branch, &flag)
                .unwrap()
                .is_some_and(|coverage| coverage.complete),
            "branch {branch}: the hourly walk covers it"
        );
    }
    let ready = accepted
        .sections
        .iter()
        .find(|section| section.id == "country-identities")
        .unwrap();
    assert_eq!(ready.state, State::Ready);
    for (branch, expected) in [1_u64, 2].into_iter().zip(&expected) {
        assert_eq!(
            &query(&source, "country-identities", &branch.to_string()).unwrap(),
            expected,
            "branch {branch}: after the hourly walk"
        );
    }
    clear(source);
}
