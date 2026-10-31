//! The event feed prepared on its own, before the hourly walk, with its own
//! read of the identity history: it publishes and saves exactly what hourly
//! preparation does across native declarations, nuclear strikes, existence
//! transitions, a guarantee read from one owner's relation slice (among
//! cancelled guarantees, other relations and other tables' slice reads,
//! which its kept frames leave out), a gap, two branches, append, resume and
//! refusals of the feed and of the identities it reads, with one and several
//! pre-pass workers; it observes every hour once, each from a frame its
//! workers read; and its query answers from the coverage of every family it
//! reads while the hourly walk has accepted nothing, as hourly preparation
//! answers it, and waits while one of them is still the walk's.
use super::family_lanes::{assert_payloads_equal, assert_versions, payloads};
use super::*;
use crate::prepared::prepass;
use dashboard_base::prepared::kept::counted;
use independent::Own;
use replay_data::{rows::RelationTupleRow, rows::SliceReadRow, rows::TableRows};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;

/// Branch 1's last hour.
const LAST: i32 = 115;

/// The feed's frame at `hour` of `branch`: the feed fixture's native
/// declaration (read at 102), nuclear strike (104), existence transitions
/// (105), factions, war, subject, advisor and focus changes, with A's
/// guarantee of B dated 103 and read in A's relation slice at 103, beside a
/// cancelled guarantee of C, another relation with B, and another table's
/// slice read of A at every hour.
fn feed_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = crate::products::event_feed::prepared::tests::frame(hour, 3);
    frame.branch = branch;
    let guarantee = engine_native::cguarantee_relation::TYPE;
    let mut relations = vec![
        RelationTupleRow {
            owner_country: A,
            target_country: B,
            type_id: guarantee,
            first_country: A,
            second_country: B,
            start_hour: 103,
            ..Default::default()
        },
        RelationTupleRow {
            owner_country: A,
            target_country: B,
            type_id: guarantee.wrapping_add(1),
            first_country: A,
            second_country: B,
            start_hour: 101,
            ..Default::default()
        },
        RelationTupleRow {
            owner_country: A,
            target_country: C,
            type_id: guarantee,
            first_country: A,
            second_country: C,
            start_hour: 102,
            cancel: 1,
            ..Default::default()
        },
    ];
    relations.sort_by_key(|row| (row.owner_country, row.target_country, row.type_id));
    let mut slices = vec![
        SliceReadRow {
            table: TableId::RelationTuples.index() as u32,
            owner: A,
            sample_hour: hour.min(103),
        },
        SliceReadRow {
            table: TableId::AvailableAdvisors.index() as u32,
            owner: A,
            sample_hour: hour,
        },
    ];
    slices.sort_by_key(|read| (read.table, read.owner));
    for rows in [
        TableRows::RelationTuples(if hour >= 103 { relations } else { Vec::new() }),
        TableRows::SliceReads(slices),
    ] {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

/// Branch 1's hours from 100 through `last`, without 109 and 110 (a gap
/// between blocks), in blocks of 3, then branch 2's one block (hours 100 to
/// 104) when `second`.
fn scenario(last: i32, second: bool) -> Replay {
    let hours: Vec<i32> = (100..=last)
        .filter(|hour| !(109..=110).contains(hour))
        .collect();
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| feed_frame(1, hour)).collect()
    };
    let mut blocks = hours.chunks(3);
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames(blocks.next().unwrap()))],
    )
    .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (100..=104).map(|hour| feed_frame(2, hour)).collect(),
            )])
            .unwrap();
    }
    replay
}

fn feed() -> &'static dyn Own {
    &independent::event_feed::FEED
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
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

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
}

/// Prepare a fresh source with `prepare` and take its payloads.
fn prepared(fixture: &mut Fixture, name: &str, prepare: &dyn Fn(&Source)) -> Payloads {
    let source = fixture.source(name);
    prepare(&source);
    let payloads = payloads(&source, None);
    clear(source);
    payloads
}

/// Sections only one mode writes: the checkpoint, the versions page (checked
/// against each run's plan), the families' own coverage pages and
/// continuations, the hourly branch state (compared apart), the feed's own read
/// of the identities and the identity history the walk keeps under territory's
/// name while the identities are prepared on their own.
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name == "preparation-versions-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || name.contains("-continuation-")
        || name.starts_with("event-feed-identities-")
        || name.starts_with("territory-identities-")
}

/// A continuation or branch state page as JSON.
fn decoded(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Families prepared on their own whose state hourly preparation keeps in
/// its branch state: their names and fields.
const KEPT: [(&str, &str); 2] = [("event_feed", "feed"), ("nukes", "nukes")];

/// Every section both modes publish is byte-identical (with `refusals`, but
/// for each branch's coverage page, whose refusals the families keep in
/// their own pages), the feed's facts among them, the hourly branch state is
/// the same apart from the fields of the families in `own` and, without
/// `refusals`, each one's continuation is the field hourly preparation keeps;
/// the feed's own read of the identities is the identity history hourly
/// preparation publishes.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, own: &[&str], refusals: bool) {
    assert_versions(actual, label);
    assert_versions(hourly, label);
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !(mode_specific(name) || (refusals && name.starts_with("preparation-coverage-")))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    assert!(
        actual.keys().any(|(name, _)| name == "event-feed-facts-1"),
        "{label}: no facts published"
    );
    if !refusals {
        for branch in [1, 2] {
            let read = actual
                .get(&(format!("event-feed-identities-{branch}"), 0))
                .unwrap_or_else(|| panic!("{label}: no own read of branch {branch}'s identities"));
            assert_eq!(
                read,
                &hourly[&(format!("country-identities-{branch}"), 0)],
                "{label}: branch {branch}'s identities as the feed read them"
            );
        }
    }
    let fields: Vec<_> = KEPT
        .iter()
        .filter(|(family, _)| own.contains(family))
        .collect();
    let mut states = 0;
    for ((name, range), bytes) in hourly {
        let Some(branch) = name.strip_prefix("preparation-branch-") else {
            continue;
        };
        let mut saved = decoded(bytes);
        let mut kept = decoded(
            actual
                .get(&(name.clone(), *range))
                .unwrap_or_else(|| panic!("{label}: no {name}")),
        );
        for (family, field) in &fields {
            let state = saved.as_object_mut().unwrap().remove(*field).unwrap();
            // The walk saves no state of a family it does not run.
            assert_eq!(
                kept.as_object_mut().unwrap().remove(*field),
                None,
                "{label}: branch {branch}'s saved state holds {field}"
            );
            if !refusals {
                let section = (format!("{family}-continuation-{branch}"), 0);
                let continuation = actual
                    .get(&section)
                    .unwrap_or_else(|| panic!("{label}: {family} saved no continuation"));
                assert_eq!(
                    decoded(continuation),
                    state,
                    "{label}: branch {branch}'s {family} continuation"
                );
                states += 1;
            }
        }
        assert_eq!(kept, saved, "{label}: branch {branch}'s hourly state");
    }
    assert!(refusals || states > 0, "{label}: no continuation");
}

/// Blocks the feed observed hour by hour, and folds that fell back.
fn notes() -> (usize, usize) {
    (
        prepass::noted("event_feed.sequential"),
        prepass::noted("event_feed.fallback"),
    )
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

#[test]
fn event_feed_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "feed-parity", &plain));
    // Alone, with the identity history it reads prepared on its own too, and
    // with the stockpiles its query reads.
    let registries: [Vec<&'static dyn Own>; 3] = [
        vec![feed()],
        vec![feed(), &independent::IDENTITIES],
        vec![
            feed(),
            &independent::IDENTITIES,
            &independent::census::NUKES,
        ],
    ];
    for families in registries {
        let own: Vec<&str> = families.iter().map(|own| own.family().name).collect();
        for (workers, together) in [(1, true), (3, false)] {
            let label = format!("{own:?}, {workers} workers, at once {together}");
            let before = (notes(), counted("event_feed"));
            let payloads = prepass::with_test_prepass(workers, None, || {
                independent::with_test_families(&families, || {
                    if together {
                        prepared(&mut fixture, "feed-parity", &plain)
                    } else {
                        independent::with_test_one_after_another(|| {
                            prepared(&mut fixture, "feed-parity", &plain)
                        })
                    }
                })
            });
            assert_same(&payloads, &hourly, &label, &own, false);
            assert_eq!(
                notes(),
                before.0,
                "{label}: every block folded from its extract"
            );
            // Every hour reads its own event pack: each is a frame a
            // worker read, observed once on the carried feed (counted
            // where the feed prepares on this thread).
            if !together || families.len() == 1 {
                let (observed, frames) = counted("event_feed");
                assert_eq!(
                    (observed - before.1.0, frames - before.1.1),
                    (recorded, recorded),
                    "{label}: every hour observed once from a worker's frame"
                );
            }
        }
    }
}

#[test]
fn event_feed_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    let first = scenario(106, false);
    let grown = scenario(LAST, true);
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    // Cancelled at the first accepted report, then prepared again; every
    // block commits on its own in both runs.
    let cancelled = |source: &Source| {
        let flag = AtomicBool::new(false);
        match run_measured_through(
            source,
            None,
            &flag,
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |_| flag.store(true, Ordering::Release),
        ) {
            Ok(_) => {}
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::Interrupted),
        }
        flag.store(false, Ordering::Release);
        measured(source);
    };
    let modes: [PrepareReloadMode; 5] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
        ("cancelled", &cancelled, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("feed-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("feed-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&[feed()], || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, &["event_feed"], false);
    }
}

/// The feed's refusal of each branch, from the checkpoint in hourly
/// preparation, or from its own coverage page.
fn feed_refusals(source: &Source, own: bool) -> BTreeMap<u64, Option<Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    [1, 2]
        .into_iter()
        .map(|branch| {
            let refusal = if own {
                store
                    .read(
                        &key,
                        &independent::coverage_section("event_feed", branch),
                        0,
                    )
                    .unwrap()
                    .and_then(|file| {
                        serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                            .unwrap()
                            .refused
                            .get("event_feed")
                            .cloned()
                    })
            } else {
                state
                    .refused
                    .get(&branch)
                    .and_then(|refused| refused.0.get("event_feed").cloned())
            };
            (branch, refusal)
        })
        .collect()
}

#[test]
fn event_feed_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    // Refusals match family and hour, so the unaffected branch must not
    // repeat the hours whose refusals this test injects.
    let replay = scenario(LAST, false)
        .append(&[records(
            2,
            (200..=204).map(|hour| feed_frame(2, hour)).collect(),
        )])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    // The feed's own refusal and one of the identities its observation
    // reads, which reaches it from them; at a block's first hour and inside
    // a block.
    for (family, from) in [("event_feed", None), ("identities", Some("identities"))] {
        for hour in [103, 104] {
            let label = format!("{family} refusing at {hour}");
            let refusals = [(family, hour)];
            let (hourly, hourly_refused) = independent::with_test_hourly(|| {
                lanes::with_test_refusals(&refusals, || {
                    let source = fixture.source("feed-refusal");
                    measured(&source);
                    let result = (payloads(&source, None), feed_refusals(&source, false));
                    clear(source);
                    result
                })
            });
            let source = fixture.source("feed-refusal");
            let sequential = prepass::noted("event_feed.sequential");
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || {
                    independent::with_test_families(&[feed()], || measured(&source))
                })
            });
            if family == "event_feed" {
                assert_eq!(
                    prepass::noted("event_feed.sequential") - sequential,
                    1,
                    "{label}: the refusing block is observed hour by hour"
                );
            }
            assert_same(
                &payloads(&source, None),
                &hourly,
                &label,
                &["event_feed"],
                true,
            );
            let own_refused = feed_refusals(&source, true);
            assert_eq!(own_refused, hourly_refused, "{label}");
            let refused = own_refused[&1].as_ref().unwrap();
            assert_eq!(refused.hour, hour, "{label}");
            assert_eq!(refused.from.as_deref(), from, "{label}: where it came from");
            assert!(own_refused[&2].is_none(), "{label}: branch 2 goes on");
            independent::with_test_families(&[feed()], || {
                let error = refusal(&source, None, 1, "event-feed", &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
                assert_eq!(
                    error.to_string(),
                    format!("Injected refusal of {family} at hour {hour}"),
                    "{label}"
                );
                refusal(&source, None, 2, "event-feed", &flag).unwrap();
            });
            clear(source);
        }
    }
}

fn query(source: &Source, params: serde_json::Value) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "event-feed".into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

/// The widget's default page of `branch` at `last`: every category.
fn page(branch: &str, last: i32) -> serde_json::Value {
    serde_json::json!({"branchId": branch, "lastHour": last, "pageSize": 256,
        "categories": ["warsPeace", "diplomacy", "politics", "milestones"]})
}

/// Every family whose pages the feed's query reads.
const READ: [&str; 8] = [
    "event_feed",
    "identities",
    "groups",
    "subjects",
    "ideas",
    "focus",
    "nukes",
    "projects",
];

#[test]
fn event_feed_answers_from_its_families_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let branches = [("1", LAST), ("2", 104)];
    // The replies of hourly preparation, with every block accepted.
    let expected: Vec<serde_json::Value> = independent::with_test_hourly(|| {
        let source = fixture.source("feed-hourly-replies");
        plain(&source);
        let replies = branches
            .iter()
            .map(|&(branch, last)| query(&source, page(branch, last)))
            .collect();
        clear(source);
        replies
    });
    assert!(
        expected.iter().all(|reply| !reply.is_null()),
        "hourly preparation answers"
    );
    // The hourly walk stalls: it is cancelled at its first commit.
    let stalled = |source: &Source| {
        let flag = AtomicBool::new(false);
        let mut commits = 0;
        let error = independent::with_test_one_after_another(|| {
            run_checked(
                source,
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
            unprepared_blocks(source, None).unwrap(),
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
    };
    let flag = AtomicBool::new(false);
    // Prepared on its own while another family it reads is still the walk's,
    // the feed waits for the walk.
    let without_nukes: Vec<&str> = READ
        .iter()
        .copied()
        .filter(|&family| family != "nukes")
        .collect();
    independent::with_test_only(&without_nukes, || {
        let source = fixture.source("feed-waiting");
        stalled(&source);
        for &(branch, last) in &branches {
            let id = branch.parse().unwrap();
            assert!(
                product_coverage(&source, None, id, "event-feed", &flag)
                    .unwrap()
                    .is_none(),
                "branch {branch}: the stockpiles' coverage is the walk's"
            );
            assert!(
                !query(&source, page(branch, last)).is_null(),
                "branch {branch}: the feed answers an empty pending prefix while the stockpiles are pending"
            );
        }
        clear(source);
    });
    // With every family it reads prepared on its own, it answers as hourly
    // preparation does.
    independent::with_test_only(&READ, || {
        let source = fixture.source("feed-stalled");
        stalled(&source);
        for (position, &(branch, last)) in branches.iter().enumerate() {
            let id = branch.parse().unwrap();
            assert!(
                coverage(&source, None, id, &flag).unwrap().is_none(),
                "branch {branch}: the hourly walk covers nothing"
            );
            assert!(
                complete(&source, None, id, "event-feed", &flag).unwrap(),
                "branch {branch}: the feed's families cover the branch"
            );
            let reply = query(&source, page(branch, last));
            assert!(!reply.is_null(), "branch {branch}: the feed answers");
            assert_eq!(
                reply, expected[position],
                "branch {branch}: the feed answers as hourly preparation does"
            );
        }
        clear(source);
    });
}
